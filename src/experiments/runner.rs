use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::{Context, anyhow};
use base64::Engine as _;
use uuid::Uuid;

use crate::api::experiments::{
    ExperimentLeaseCredentialsResponse, ExperimentLeaseJobResponse, ExperimentLeaseStatusRequest,
};
use crate::experiments::{
    ExperimentRunnerArtifactUpload, ExperimentRunnerCompletion, ExperimentRunnerJob,
    extract_metrics, validate_project_workdir_fragment,
};
use crate::tools::execution_backend::{
    CommandExecutionRequest, ExecutionBackend, LocalHostExecutionBackend, ScriptExecutionRequest,
};

/// Upper bound on artifact bytes we inline as base64 in a lease `/artifact` post.
/// Larger artifacts fall back to the pod-local-path breadcrumb only (no durable
/// upload). This caps memory/transport pressure from runaway logs while still
/// covering typical run logs and summary JSON.
// The gateway's global JSON request limit is 1 MiB. Keeping raw artifact bytes
// at 512 KiB leaves room for base64 expansion and the surrounding JSON object.
const MAX_INLINE_ARTIFACT_BYTES: u64 = 512 * 1024;
const MAX_LEASE_JOB_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LEASE_CREDENTIALS_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SUMMARY_JSON_BYTES: u64 = 8 * 1024 * 1024;
const RUNNER_GATEWAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RUNNER_GATEWAY_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn run_remote_runner(
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    workspace_root: Option<PathBuf>,
) -> anyhow::Result<()> {
    let gateway_url = validate_gateway_url(gateway_url)?;
    let client = reqwest::Client::builder()
        .user_agent(format!("ThinClaw/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(RUNNER_GATEWAY_CONNECT_TIMEOUT)
        .timeout(RUNNER_GATEWAY_REQUEST_TIMEOUT)
        .build()
        .context("failed to build experiment runner gateway client")?;
    let job_url = lease_url(&gateway_url, lease_id, "job")?;
    let credentials_url = lease_url(&gateway_url, lease_id, "credentials")?;

    let job_response = client
        .get(job_url)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to request lease job")?
        .error_for_status()
        .context("lease job request failed")?;
    let job = crate::http_response::bounded_json::<ExperimentLeaseJobResponse>(
        job_response,
        MAX_LEASE_JOB_RESPONSE_BYTES,
    )
    .await
    .context("invalid lease job response")?
    .job;

    let credentials_response = client
        .get(credentials_url)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to request lease credentials")?
        .error_for_status()
        .context("lease credentials request failed")?;
    let credentials = crate::http_response::bounded_json::<ExperimentLeaseCredentialsResponse>(
        credentials_response,
        MAX_LEASE_CREDENTIALS_RESPONSE_BYTES,
    )
    .await
    .context("invalid lease credentials response")?
    .credentials;

    let started_at = Instant::now();
    let mut terminal_completion_attempted = false;
    let mut completion_stage = "runner_setup".to_string();
    let mut persisted_log_path: Option<PathBuf> = None;
    let mut persisted_log_bytes: Option<Vec<u8>> = None;
    let backend = local_runner_execution_backend();
    let env = merge_env(&job, &credentials)?;

    let result: anyhow::Result<()> = async {
        post_status(
            &client,
            &gateway_url,
            lease_id,
            token,
            "runner_started",
            Some(serde_json::json!({ "backend": job.backend })),
        )
        .await
        .ok();

        completion_stage = "checkout".to_string();
        let checkout_dir = prepare_checkout_dir(workspace_root.clone(), lease_id)?;
        clone_checkout(&job, &checkout_dir, Arc::clone(&backend)).await?;

        let run_root = resolve_runner_workdir(&checkout_dir, &job.workdir).await?;

        let mut log = String::new();
        if let Some(prepare_command) = job.prepare_command.as_deref() {
            completion_stage = "prepare".to_string();
            post_event(
                &client,
                &gateway_url,
                lease_id,
                token,
                "running_prepare",
                Some(serde_json::json!({
                    "command": redact_sensitive_text(prepare_command, &env, token)
                })),
            )
            .await
            .ok();
            let mut output =
                run_shell_command(Arc::clone(&backend), &run_root, prepare_command, &env).await?;
            output.combined = redact_sensitive_text(&output.combined, &env, token);
            log.push_str("== prepare ==\n");
            log.push_str(&output.combined);
            log.push('\n');
            ensure_shell_step_succeeded("prepare", &output)?;
        }

        post_event(
            &client,
            &gateway_url,
            lease_id,
            token,
            "running_benchmark",
            Some(serde_json::json!({
                "command": redact_sensitive_text(&job.run_command, &env, token)
            })),
        )
        .await
        .ok();

        completion_stage = "run".to_string();
        let mut run_output =
            run_shell_command(Arc::clone(&backend), &run_root, &job.run_command, &env).await?;
        run_output.combined = redact_sensitive_text(&run_output.combined, &env, token);
        log.push_str("== run ==\n");
        log.push_str(&run_output.combined);

        let log_path = checkout_dir.join("run.log");
        let log_bytes = log.as_bytes().to_vec();
        thinclaw_platform::write_private_file_atomic_async(
            log_path.clone(),
            log_bytes.clone(),
            true,
        )
        .await
        .with_context(|| format!("failed to write {}", log_path.display()))?;
        persisted_log_path = Some(log_path.clone());
        persisted_log_bytes = Some(log_bytes);

        let summary_path = run_root.join("summary.json");
        let (summary_json, summary_artifact_bytes) =
            match thinclaw_platform::read_regular_file_bounded_async(
                summary_path.clone(),
                MAX_SUMMARY_JSON_BYTES,
            )
            .await
            {
                Ok(raw) => {
                    let mut summary = serde_json::from_slice::<serde_json::Value>(&raw)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    redact_sensitive_json(&mut summary, &env, token);
                    let sanitized = serde_json::to_vec(&summary).unwrap_or_else(|_| b"{}".to_vec());
                    (summary, Some(sanitized))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    (serde_json::json!({}), None)
                }
                Err(error) => {
                    tracing::warn!(
                        path = %summary_path.display(),
                        %error,
                        "Ignoring unsafe or unreadable experiment summary"
                    );
                    (serde_json::json!({}), None)
                }
            };
        let metrics = extract_metrics(
            &job.primary_metric,
            &job.secondary_metrics,
            &log,
            &summary_json,
        );
        let exit_code = run_output.exit_code;
        let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;

        let artifact_log = artifact_upload_from_bytes(
            "run_log",
            &log_path,
            persisted_log_bytes.as_deref().unwrap_or_default(),
            serde_json::json!({}),
        );
        post_artifact(&client, &gateway_url, lease_id, token, &artifact_log)
            .await
            .ok();
        ensure_shell_step_succeeded("run", &run_output)?;

        if let Some(summary_bytes) = summary_artifact_bytes.as_deref() {
            let artifact_summary = artifact_upload_from_bytes(
                "summary_json",
                &summary_path,
                summary_bytes,
                serde_json::json!({}),
            );
            post_artifact(&client, &gateway_url, lease_id, token, &artifact_summary)
                .await
                .ok();
        }

        let completion = ExperimentRunnerCompletion {
            exit_code: Some(exit_code),
            metrics_json: metrics,
            summary: Some(format!(
                "Remote runner finished with exit code {exit_code}."
            )),
            runtime_ms: Some(runtime_ms),
            attributed_cost_usd: None,
            log_preview_path: Some(log_path.to_string_lossy().to_string()),
            artifact_manifest_json: serde_json::json!({
                "stage": completion_stage,
                "checkout_dir": checkout_dir.to_string_lossy(),
                "summary_json_path": summary_path.to_string_lossy(),
            }),
        };
        completion_stage = "complete".to_string();
        client
            .post(lease_url(&gateway_url, lease_id, "complete")?)
            .bearer_auth(token)
            .json(&completion)
            .send()
            .await
            .context("failed to complete lease")?
            .error_for_status()
            .context("lease completion request failed")?;
        terminal_completion_attempted = true;

        Ok(())
    }
    .await;

    if let Err(err) = result {
        let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
        let error_text = redact_sensitive_text(&err.to_string(), &env, token);
        post_status(
            &client,
            &gateway_url,
            lease_id,
            token,
            "failed",
            Some(serde_json::json!({ "error": error_text.clone() })),
        )
        .await
        .ok();
        post_event(
            &client,
            &gateway_url,
            lease_id,
            token,
            "runner_failed",
            Some(serde_json::json!({ "error": error_text.clone() })),
        )
        .await
        .ok();
        if let (Some(log_path), Some(log_bytes)) =
            (persisted_log_path.as_ref(), persisted_log_bytes.as_deref())
        {
            let failure_log_artifact = artifact_upload_from_bytes(
                "run_log",
                log_path,
                log_bytes,
                serde_json::json!({ "failure": true }),
            );
            post_artifact(
                &client,
                &gateway_url,
                lease_id,
                token,
                &failure_log_artifact,
            )
            .await
            .ok();
        }
        if !terminal_completion_attempted {
            let failure = ExperimentRunnerCompletion {
                exit_code: Some(1),
                metrics_json: serde_json::json!({}),
                summary: Some(format!("Remote runner failed: {}", error_text)),
                runtime_ms: Some(runtime_ms),
                attributed_cost_usd: None,
                log_preview_path: persisted_log_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
                artifact_manifest_json: serde_json::json!({
                    "error": error_text.clone(),
                    "stage": completion_stage,
                    "log_path": persisted_log_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                }),
            };
            if let Ok(url) = lease_url(&gateway_url, lease_id, "complete") {
                let _ = client
                    .post(url)
                    .bearer_auth(token)
                    .json(&failure)
                    .send()
                    .await;
            }
        }
        return Err(err);
    }

    Ok(())
}

fn validate_gateway_url(gateway_url: &str) -> anyhow::Result<reqwest::Url> {
    let normalized = crate::experiments::adapters::validate_gateway_url(gateway_url)
        .map_err(|message| anyhow!(message))?;
    reqwest::Url::parse(&normalized).context("invalid normalized experiment gateway URL")
}

fn lease_url(
    gateway_url: &reqwest::Url,
    lease_id: Uuid,
    suffix: &str,
) -> anyhow::Result<reqwest::Url> {
    if !matches!(
        suffix,
        "job" | "credentials" | "status" | "event" | "artifact" | "complete"
    ) {
        return Err(anyhow!("invalid experiment lease endpoint"));
    }
    let mut url = gateway_url.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow!("experiment gateway URL cannot be used as a base URL"))?;
        segments.pop_if_empty();
        segments.extend([
            "api",
            "experiments",
            "leases",
            &lease_id.to_string(),
            suffix,
        ]);
    }
    Ok(url)
}

async fn post_status(
    client: &reqwest::Client,
    gateway_url: &reqwest::Url,
    lease_id: Uuid,
    token: &str,
    status: &str,
    metadata: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "status")?)
        .bearer_auth(token)
        .json(&ExperimentLeaseStatusRequest {
            status: status.to_string(),
            metadata,
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn post_event(
    client: &reqwest::Client,
    gateway_url: &reqwest::Url,
    lease_id: Uuid,
    token: &str,
    message: &str,
    metadata: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "event")?)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "message": message,
            "metadata": metadata,
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn post_artifact(
    client: &reqwest::Client,
    gateway_url: &reqwest::Url,
    lease_id: Uuid,
    token: &str,
    artifact: &ExperimentRunnerArtifactUpload,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "artifact")?)
        .bearer_auth(token)
        .json(artifact)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Build an artifact upload for already-validated bytes, attaching them inline
/// as base64 when they fit under [`MAX_INLINE_ARTIFACT_BYTES`] so the
/// gateway host can persist them durably. The pod-local path is retained as a
/// breadcrumb; `fetchable` stays `false` because the path itself does not survive
/// pod teardown — durability comes from the inline bytes the host stores.
fn artifact_upload_from_bytes(
    kind: &str,
    path: &Path,
    bytes: &[u8],
    metadata: serde_json::Value,
) -> ExperimentRunnerArtifactUpload {
    let size_bytes = u64::try_from(bytes.len()).ok();
    let content_base64 = size_bytes
        .filter(|length| *length <= MAX_INLINE_ARTIFACT_BYTES)
        .map(|_| base64::engine::general_purpose::STANDARD.encode(bytes));
    ExperimentRunnerArtifactUpload {
        kind: kind.to_string(),
        uri_or_local_path: path.to_string_lossy().to_string(),
        size_bytes,
        fetchable: false,
        metadata,
        content_base64,
    }
}

fn redact_sensitive_text(text: &str, env: &[(String, String)], token: &str) -> String {
    let mut secrets = env
        .iter()
        .map(|(_, value)| value.as_str())
        .chain(std::iter::once(token))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    secrets.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
    secrets.dedup();
    secrets
        .into_iter()
        .fold(text.to_string(), |redacted, secret| {
            redacted.replace(secret, "[REDACTED]")
        })
}

fn redact_sensitive_json(value: &mut serde_json::Value, env: &[(String, String)], token: &str) {
    fn visit(value: &mut serde_json::Value, env: &[(String, String)], token: &str, depth: usize) {
        if depth > 64 {
            *value = serde_json::Value::Null;
            return;
        }
        match value {
            serde_json::Value::String(text) => {
                *text = redact_sensitive_text(text, env, token);
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    visit(item, env, token, depth + 1);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values_mut() {
                    visit(item, env, token, depth + 1);
                }
            }
            _ => {}
        }
    }
    visit(value, env, token, 0);
}

async fn resolve_runner_workdir(checkout_dir: &Path, workdir: &str) -> anyhow::Result<PathBuf> {
    let fragment =
        validate_project_workdir_fragment(workdir).map_err(|message| anyhow!(message))?;
    let checkout_root = tokio::fs::canonicalize(checkout_dir)
        .await
        .with_context(|| format!("failed to resolve checkout root {}", checkout_dir.display()))?;
    let run_root = tokio::fs::canonicalize(checkout_root.join(fragment))
        .await
        .context("runner workdir does not exist")?;
    if !run_root.starts_with(&checkout_root) {
        return Err(anyhow!(
            "runner workdir resolves outside the checked-out repository"
        ));
    }
    Ok(run_root)
}

fn prepare_checkout_dir(
    workspace_root: Option<PathBuf>,
    lease_id: Uuid,
) -> anyhow::Result<PathBuf> {
    let root = workspace_root.unwrap_or_else(|| std::env::temp_dir().join("thinclaw-experiments"));
    std::fs::create_dir_all(&root)
        .with_context(|| format!("failed to create {}", root.display()))?;
    let root = std::fs::canonicalize(&root)
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let checkout_dir = root.join(lease_id.simple().to_string());
    match std::fs::symlink_metadata(&checkout_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            std::fs::remove_file(&checkout_dir)
                .with_context(|| format!("failed to clear {}", checkout_dir.display()))?;
        }
        Ok(_) => {
            std::fs::remove_dir_all(&checkout_dir)
                .with_context(|| format!("failed to clear {}", checkout_dir.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", checkout_dir.display()));
        }
    }
    Ok(checkout_dir)
}

fn local_runner_execution_backend() -> Arc<dyn ExecutionBackend> {
    LocalHostExecutionBackend::shared()
}

async fn clone_checkout(
    job: &ExperimentRunnerJob,
    checkout_dir: &Path,
    backend: Arc<dyn ExecutionBackend>,
) -> anyhow::Result<()> {
    let parent = checkout_dir
        .parent()
        .ok_or_else(|| anyhow!("checkout dir has no parent"))?;
    run_command_capture(
        Arc::clone(&backend),
        Some(parent),
        "git",
        &[
            "clone",
            "--",
            &job.repo_url,
            checkout_dir.to_string_lossy().as_ref(),
        ],
        &[],
    )
    .await?;
    run_command_capture(
        Arc::clone(&backend),
        Some(checkout_dir),
        "git",
        &["fetch", "--", "origin", &job.git_ref],
        &[],
    )
    .await?;
    run_command_capture(
        backend,
        Some(checkout_dir),
        "git",
        &["checkout", "--detach", "FETCH_HEAD"],
        &[],
    )
    .await?;
    Ok(())
}

fn merge_env(
    job: &ExperimentRunnerJob,
    credentials: &serde_json::Value,
) -> anyhow::Result<Vec<(String, String)>> {
    const MAX_ENV_VARS: usize = 256;
    const MAX_ENV_KEY_BYTES: usize = 128;
    const MAX_ENV_VALUE_BYTES: usize = 64 * 1024;
    const MAX_ENV_TOTAL_BYTES: usize = 2 * 1024 * 1024;

    let mut pairs = std::collections::BTreeMap::new();

    for map in [
        job.env_grants.as_object(),
        credentials.get("env").and_then(|value| value.as_object()),
    ]
    .into_iter()
    .flatten()
    {
        for (key, value) in map {
            if let Some(value) = value.as_str() {
                let valid_key = !key.is_empty()
                    && key.len() <= MAX_ENV_KEY_BYTES
                    && key.bytes().enumerate().all(|(index, byte)| {
                        byte == b'_'
                            || byte.is_ascii_alphabetic()
                            || (index > 0 && byte.is_ascii_digit())
                    });
                if !valid_key || value.len() > MAX_ENV_VALUE_BYTES {
                    return Err(anyhow!(
                        "lease credentials contain an invalid environment grant"
                    ));
                }
                pairs.insert(key.clone(), value.to_string());
            }
        }
    }
    let total_bytes = pairs
        .iter()
        .map(|(key, value)| key.len().saturating_add(value.len()))
        .sum::<usize>();
    if pairs.len() > MAX_ENV_VARS || total_bytes > MAX_ENV_TOTAL_BYTES {
        return Err(anyhow!(
            "lease credentials exceed the environment grant limits"
        ));
    }
    Ok(pairs.into_iter().collect())
}

#[allow(dead_code)]
struct RunnerCommandOutput {
    stdout: String,
    stderr: String,
    combined: String,
    exit_code: i32,
}

async fn run_shell_command(
    backend: Arc<dyn ExecutionBackend>,
    cwd: &Path,
    command: &str,
    env: &[(String, String)],
) -> anyhow::Result<RunnerCommandOutput> {
    let env_map = env.iter().cloned().collect();
    let output = backend
        .run_shell(CommandExecutionRequest {
            command: command.to_string(),
            workdir: cwd.to_path_buf(),
            timeout: std::time::Duration::from_secs(600),
            extra_env: env_map,
            allow_network: false,
        })
        .await
        .map_err(|err| anyhow!(err.to_string()))?;
    Ok(RunnerCommandOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        combined: output.output,
        exit_code: output.exit_code as i32,
    })
}

async fn run_command_capture(
    backend: Arc<dyn ExecutionBackend>,
    cwd: Option<&Path>,
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
) -> anyhow::Result<RunnerCommandOutput> {
    let env_map = env.iter().cloned().collect();
    let output = backend
        .run_script(ScriptExecutionRequest {
            program: binary.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            workdir: cwd
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            timeout: std::time::Duration::from_secs(600),
            extra_env: env_map,
            allow_network: true,
        })
        .await
        .with_context(|| format!("failed to run {binary}"))?;
    if output.exit_code != 0 {
        return Err(anyhow!(
            "{binary} exited with status {}{}",
            output.exit_code,
            if output.output.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", output.output.trim())
            }
        ));
    }
    Ok(RunnerCommandOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        combined: output.output,
        exit_code: output.exit_code as i32,
    })
}

fn ensure_shell_step_succeeded(stage: &str, output: &RunnerCommandOutput) -> anyhow::Result<()> {
    let exit_code = output.exit_code;
    if exit_code == 0 {
        return Ok(());
    }

    Err(anyhow!("{stage} command exited with code {exit_code}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_urls_are_origin_locked_and_path_segment_safe() {
        let lease_id = Uuid::nil();
        let gateway = validate_gateway_url("https://gateway.example/base").unwrap();
        let url = lease_url(&gateway, lease_id, "credentials").unwrap();
        assert_eq!(
            url.as_str(),
            "https://gateway.example/base/api/experiments/leases/00000000-0000-0000-0000-000000000000/credentials"
        );
        assert!(lease_url(&gateway, lease_id, "../../other").is_err());
        assert!(validate_gateway_url("http://gateway.example").is_err());
        assert!(validate_gateway_url("https://user:pass@gateway.example").is_err());
        assert!(validate_gateway_url("http://127.0.0.1:3001").is_ok());
    }

    #[test]
    fn runner_output_redaction_covers_env_values_and_lease_token() {
        let token = "exp_0123456789ab_0123456789abcdef0123456789abcdef";
        let env = vec![("API_KEY".to_string(), "super-secret-value".to_string())];
        let output = redact_sensitive_text(
            &format!("key=super-secret-value token={token}"),
            &env,
            token,
        );
        assert_eq!(output, "key=[REDACTED] token=[REDACTED]");

        let mut json = serde_json::json!({
            "nested": [format!("super-secret-value/{token}")]
        });
        redact_sensitive_json(&mut json, &env, token);
        assert!(!json.to_string().contains("super-secret-value"));
        assert!(!json.to_string().contains(token));
    }

    #[test]
    fn runner_environment_grants_are_bounded_and_validated() {
        let mut job = sample_job();
        job.env_grants = serde_json::json!({ "SAFE_NAME": "legacy" });
        let env = merge_env(
            &job,
            &serde_json::json!({ "env": { "SAFE_NAME": "current", "OTHER": "value" } }),
        )
        .unwrap();
        assert!(env.contains(&("SAFE_NAME".to_string(), "current".to_string())));
        assert!(merge_env(&job, &serde_json::json!({ "env": { "BAD-NAME": "x" } })).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runner_workdir_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let checkout = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), checkout.path().join("escape")).unwrap();
        assert!(
            resolve_runner_workdir(checkout.path(), "escape")
                .await
                .is_err()
        );
    }

    #[test]
    fn oversized_artifact_is_not_inlined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.log");
        let bytes = vec![b'x'; MAX_INLINE_ARTIFACT_BYTES as usize + 1];
        let artifact = artifact_upload_from_bytes("run_log", &path, &bytes, serde_json::json!({}));
        assert_eq!(artifact.size_bytes, Some(MAX_INLINE_ARTIFACT_BYTES + 1));
        assert!(artifact.content_base64.is_none());
    }

    fn sample_job() -> ExperimentRunnerJob {
        ExperimentRunnerJob {
            lease_id: Uuid::new_v4(),
            trial_id: Uuid::new_v4(),
            campaign_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            runner_profile_id: Uuid::new_v4(),
            backend: crate::experiments::ExperimentRunnerBackend::GenericRemoteRunner,
            repo_url: "https://example.com/repo.git".to_string(),
            git_ref: "codex/experiments/test".to_string(),
            workdir: ".".to_string(),
            prepare_command: None,
            run_command: "true".to_string(),
            primary_metric: crate::experiments::ExperimentMetricDefinition {
                name: "score".to_string(),
                regex: Some("score=(.*)".to_string()),
                ..Default::default()
            },
            secondary_metrics: Vec::new(),
            env_grants: serde_json::json!({}),
            artifact_paths: Vec::new(),
        }
    }
}

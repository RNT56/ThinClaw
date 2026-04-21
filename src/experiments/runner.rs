use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, anyhow};
use uuid::Uuid;

use crate::api::experiments::{
    ExperimentLeaseCredentialsResponse, ExperimentLeaseJobResponse, ExperimentLeaseStatusRequest,
};
use crate::experiments::{
    ExperimentRunnerArtifactUpload, ExperimentRunnerCompletion, ExperimentRunnerJob,
    extract_metrics,
};
use crate::tools::execution_backend::{
    CommandExecutionRequest, ExecutionBackend, LocalHostExecutionBackend, ScriptExecutionRequest,
};

pub async fn run_remote_runner(
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    workspace_root: Option<PathBuf>,
) -> anyhow::Result<()> {
    if gateway_url.trim().is_empty() {
        return Err(anyhow!(
            "gateway_url must be non-empty for remote experiment runner leases"
        ));
    }
    let client = reqwest::Client::new();
    let job_url = lease_url(gateway_url, lease_id, "job");
    let credentials_url = lease_url(gateway_url, lease_id, "credentials");

    let job = client
        .get(&job_url)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to request lease job")?
        .error_for_status()
        .context("lease job request failed")?
        .json::<ExperimentLeaseJobResponse>()
        .await
        .context("invalid lease job response")?
        .job;

    let credentials = client
        .get(&credentials_url)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to request lease credentials")?
        .error_for_status()
        .context("lease credentials request failed")?
        .json::<ExperimentLeaseCredentialsResponse>()
        .await
        .context("invalid lease credentials response")?
        .credentials;

    let started_at = Instant::now();
    let mut terminal_completion_attempted = false;
    let mut completion_stage = "runner_setup".to_string();
    let mut persisted_log_path: Option<PathBuf> = None;
    let backend = local_runner_execution_backend();

    let result: anyhow::Result<()> = async {
        post_status(
            &client,
            gateway_url,
            lease_id,
            token,
            "runner_started",
            Some(serde_json::json!({ "backend": job.backend })),
        )
        .await
        .ok();

        completion_stage = "checkout".to_string();
        let checkout_dir = prepare_checkout_dir(workspace_root, lease_id)?;
        clone_checkout(&job, &checkout_dir, Arc::clone(&backend)).await?;

        let run_root = checkout_dir.join(&job.workdir);
        if !run_root.exists() {
            return Err(anyhow!(
                "runner workdir does not exist: {}",
                run_root.display()
            ));
        }

        let env = merge_env(&job, &credentials);
        let mut log = String::new();
        if let Some(prepare_command) = job.prepare_command.as_deref() {
            completion_stage = "prepare".to_string();
            post_event(
                &client,
                gateway_url,
                lease_id,
                token,
                "running_prepare",
                Some(serde_json::json!({ "command": prepare_command })),
            )
            .await
            .ok();
            let output =
                run_shell_command(Arc::clone(&backend), &run_root, prepare_command, &env).await?;
            log.push_str("== prepare ==\n");
            log.push_str(&output.combined);
            log.push('\n');
            ensure_shell_step_succeeded("prepare", &output)?;
        }

        post_event(
            &client,
            gateway_url,
            lease_id,
            token,
            "running_benchmark",
            Some(serde_json::json!({ "command": job.run_command })),
        )
        .await
        .ok();

        completion_stage = "run".to_string();
        let run_output =
            run_shell_command(Arc::clone(&backend), &run_root, &job.run_command, &env).await?;
        log.push_str("== run ==\n");
        log.push_str(&run_output.combined);

        let log_path = checkout_dir.join("run.log");
        tokio::fs::write(&log_path, &log)
            .await
            .with_context(|| format!("failed to write {}", log_path.display()))?;
        persisted_log_path = Some(log_path.clone());

        let summary_path = run_root.join("summary.json");
        let summary_json = if summary_path.exists() {
            let raw = tokio::fs::read_to_string(&summary_path)
                .await
                .unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&raw)
                .unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };
        let metrics = extract_metrics(
            &job.primary_metric,
            &job.secondary_metrics,
            &log,
            &summary_json,
        );
        let exit_code = run_output.exit_code;
        let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;

        let artifact_log = ExperimentRunnerArtifactUpload {
            kind: "run_log".to_string(),
            uri_or_local_path: log_path.to_string_lossy().to_string(),
            size_bytes: Some(
                tokio::fs::metadata(&log_path)
                    .await
                    .map(|meta| meta.len())
                    .unwrap_or_default(),
            ),
            fetchable: false,
            metadata: serde_json::json!({}),
        };
        post_artifact(&client, gateway_url, lease_id, token, &artifact_log)
            .await
            .ok();
        ensure_shell_step_succeeded("run", &run_output)?;

        if summary_path.exists() {
            let artifact_summary = ExperimentRunnerArtifactUpload {
                kind: "summary_json".to_string(),
                uri_or_local_path: summary_path.to_string_lossy().to_string(),
                size_bytes: Some(
                    tokio::fs::metadata(&summary_path)
                        .await
                        .map(|meta| meta.len())
                        .unwrap_or_default(),
                ),
                fetchable: false,
                metadata: serde_json::json!({}),
            };
            post_artifact(&client, gateway_url, lease_id, token, &artifact_summary)
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
            .post(lease_url(gateway_url, lease_id, "complete"))
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
        let error_text = err.to_string();
        post_status(
            &client,
            gateway_url,
            lease_id,
            token,
            "failed",
            Some(serde_json::json!({ "error": error_text.clone() })),
        )
        .await
        .ok();
        post_event(
            &client,
            gateway_url,
            lease_id,
            token,
            "runner_failed",
            Some(serde_json::json!({ "error": error_text.clone() })),
        )
        .await
        .ok();
        if let Some(log_path) = persisted_log_path.as_ref() {
            let failure_log_artifact = ExperimentRunnerArtifactUpload {
                kind: "run_log".to_string(),
                uri_or_local_path: log_path.to_string_lossy().to_string(),
                size_bytes: Some(
                    tokio::fs::metadata(log_path)
                        .await
                        .map(|meta| meta.len())
                        .unwrap_or_default(),
                ),
                fetchable: false,
                metadata: serde_json::json!({ "failure": true }),
            };
            post_artifact(&client, gateway_url, lease_id, token, &failure_log_artifact)
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
            let _ = client
                .post(lease_url(gateway_url, lease_id, "complete"))
                .bearer_auth(token)
                .json(&failure)
                .send()
                .await;
        }
        return Err(err);
    }

    Ok(())
}

fn lease_url(gateway_url: &str, lease_id: Uuid, suffix: &str) -> String {
    format!(
        "{}/api/experiments/leases/{}/{}",
        gateway_url.trim_end_matches('/'),
        lease_id,
        suffix
    )
}

async fn post_status(
    client: &reqwest::Client,
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    status: &str,
    metadata: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "status"))
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
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    message: &str,
    metadata: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "event"))
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
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    artifact: &ExperimentRunnerArtifactUpload,
) -> anyhow::Result<()> {
    client
        .post(lease_url(gateway_url, lease_id, "artifact"))
        .bearer_auth(token)
        .json(artifact)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn prepare_checkout_dir(
    workspace_root: Option<PathBuf>,
    lease_id: Uuid,
) -> anyhow::Result<PathBuf> {
    let root = workspace_root.unwrap_or_else(|| std::env::temp_dir().join("thinclaw-experiments"));
    let checkout_dir = root.join(lease_id.simple().to_string());
    if checkout_dir.exists() {
        std::fs::remove_dir_all(&checkout_dir)
            .with_context(|| format!("failed to clear {}", checkout_dir.display()))?;
    }
    std::fs::create_dir_all(&root)
        .with_context(|| format!("failed to create {}", root.display()))?;
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
        &["fetch", "origin", &job.git_ref],
        &[],
    )
    .await?;
    run_command_capture(
        backend,
        Some(checkout_dir),
        "git",
        &["checkout", "FETCH_HEAD"],
        &[],
    )
    .await?;
    Ok(())
}

fn merge_env(job: &ExperimentRunnerJob, credentials: &serde_json::Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();

    if let Some(map) = job.env_grants.as_object() {
        for (key, value) in map {
            if let Some(value) = value.as_str() {
                pairs.push((key.clone(), value.to_string()));
            }
        }
    }
    if let Some(map) = credentials.get("env").and_then(|value| value.as_object()) {
        for (key, value) in map {
            if let Some(value) = value.as_str() {
                pairs.push((key.clone(), value.to_string()));
            }
        }
    }

    pairs
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
    if output.exit_code != 0 {
        return Err(anyhow!(
            "shell exited with status {}{}",
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

    Err(anyhow!(
        "{stage} command exited with code {exit_code}: {}",
        output.combined.trim()
    ))
}

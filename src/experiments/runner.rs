use std::path::{Path, PathBuf};
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

pub async fn run_remote_runner(
    gateway_url: &str,
    lease_id: Uuid,
    token: &str,
    workspace_root: Option<PathBuf>,
) -> anyhow::Result<()> {
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

        let checkout_dir = prepare_checkout_dir(workspace_root, lease_id)?;
        clone_checkout(&job, &checkout_dir).await?;

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
            let output = run_shell_command(&run_root, prepare_command, &env).await?;
            log.push_str("== prepare ==\n");
            log.push_str(&output);
            log.push('\n');
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

        let run_output = run_shell_command(&run_root, &job.run_command, &env).await?;
        log.push_str("== run ==\n");
        log.push_str(&run_output);

        let log_path = checkout_dir.join("run.log");
        tokio::fs::write(&log_path, &log)
            .await
            .with_context(|| format!("failed to write {}", log_path.display()))?;

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
        let exit_code = parse_exit_code(&run_output).unwrap_or(1);
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
                "checkout_dir": checkout_dir.to_string_lossy(),
                "summary_json_path": summary_path.to_string_lossy(),
            }),
        };
        terminal_completion_attempted = true;
        client
            .post(lease_url(gateway_url, lease_id, "complete"))
            .bearer_auth(token)
            .json(&completion)
            .send()
            .await
            .context("failed to complete lease")?
            .error_for_status()
            .context("lease completion request failed")?;

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
        if !terminal_completion_attempted {
            let failure = ExperimentRunnerCompletion {
                exit_code: Some(1),
                metrics_json: serde_json::json!({}),
                summary: Some(format!("Remote runner failed: {}", error_text)),
                runtime_ms: Some(runtime_ms),
                attributed_cost_usd: None,
                log_preview_path: None,
                artifact_manifest_json: serde_json::json!({
                    "error": error_text.clone(),
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

async fn clone_checkout(job: &ExperimentRunnerJob, checkout_dir: &Path) -> anyhow::Result<()> {
    let parent = checkout_dir
        .parent()
        .ok_or_else(|| anyhow!("checkout dir has no parent"))?;
    run_command_capture(
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
        Some(checkout_dir),
        "git",
        &["fetch", "origin", &job.git_ref],
        &[],
    )
    .await?;
    run_command_capture(Some(checkout_dir), "git", &["checkout", "FETCH_HEAD"], &[]).await?;
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

async fn run_shell_command(
    cwd: &Path,
    command: &str,
    env: &[(String, String)],
) -> anyhow::Result<String> {
    #[cfg(target_os = "windows")]
    let (shell, base_args) = ("cmd", vec!["/C"]);
    #[cfg(not(target_os = "windows"))]
    let (shell, base_args) = ("sh", vec!["-lc"]);

    let wrapped = format!("{command}; printf '\\n__THINCLAW_EXIT_CODE__:%s\\n' \"$?\"");
    let mut args = base_args;
    args.push(&wrapped);
    run_command_capture(Some(cwd), shell, &args, env).await
}

async fn run_command_capture(
    cwd: Option<&Path>,
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
) -> anyhow::Result<String> {
    let mut command = tokio::process::Command::new(binary);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .await
        .with_context(|| format!("failed to run {binary}"))?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(text)
}

fn parse_exit_code(output: &str) -> Option<i32> {
    output
        .lines()
        .find_map(|line| line.split("__THINCLAW_EXIT_CODE__:").nth(1))
        .and_then(|value| value.trim().parse::<i32>().ok())
}

//! Codex CLI bridge for sandboxed execution.
//!
//! Spawns the `codex` CLI inside a Docker container and streams its JSONL
//! output back to the orchestrator via HTTP. Supports follow-up prompts by
//! resuming the same Codex thread.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::error::WorkerError;
use crate::worker::api::{CompletionReport, JobEventPayload, WorkerHttpClient};
use crate::worker::bridge_common::{
    copy_auth_dir_from_mount, poll_for_prompt, post_job_event, truncate,
};

pub struct CodexBridgeConfig {
    pub job_id: Uuid,
    pub orchestrator_url: String,
    pub model: String,
    pub timeout: Duration,
}

pub struct CodexBridgeRuntime {
    config: CodexBridgeConfig,
    client: Arc<WorkerHttpClient>,
}

const CODEX_HOME_PATH: &str = "/home/sandbox/.codex";
const CODEX_SANDBOX_MODE: &str = "workspace-write";

impl CodexBridgeRuntime {
    pub fn new(config: CodexBridgeConfig) -> Result<Self, WorkerError> {
        let client = Arc::new(WorkerHttpClient::from_env(
            config.orchestrator_url.clone(),
            config.job_id,
        )?);
        Ok(Self { config, client })
    }

    fn copy_auth_from_mount(&self) -> Result<(), WorkerError> {
        let copied = copy_auth_dir_from_mount(
            std::path::Path::new("/home/sandbox/.codex-host"),
            std::path::Path::new(CODEX_HOME_PATH),
        )?;

        if copied > 0 {
            tracing::info!(
                job_id = %self.config.job_id,
                files_copied = copied,
                "Copied Codex auth config from host mount into container"
            );
        }

        Ok(())
    }

    pub async fn run(&self) -> Result<(), WorkerError> {
        self.copy_auth_from_mount()?;

        let job = self.client.get_job().await?;
        tracing::info!(
            job_id = %self.config.job_id,
            "Starting Codex bridge for: {}",
            truncate(&job.description, 100)
        );

        let credentials = self.client.fetch_credentials().await?;
        let mut extra_env = std::collections::HashMap::new();
        for cred in &credentials {
            extra_env.insert(cred.env_var.clone(), cred.value.clone());
        }

        let has_api_key =
            extra_env.contains_key("OPENAI_API_KEY") || std::env::var("OPENAI_API_KEY").is_ok();
        let has_auth_file = std::path::Path::new(CODEX_HOME_PATH)
            .join("auth.json")
            .is_file();
        if !has_api_key && !has_auth_file {
            tracing::warn!(
                job_id = %self.config.job_id,
                "No Codex auth available. Set OPENAI_API_KEY or populate CODEX_HOME/auth.json on the host."
            );
        }

        self.client
            .report_status(&crate::worker::api::StatusUpdate {
                state: "running".to_string(),
                message: Some("Spawning Codex CLI".to_string()),
                iteration: 0,
            })
            .await?;

        let session_id = match self
            .run_codex_session(&job.description, None, &extra_env)
            .await
        {
            Ok(sid) => sid,
            Err(e) => {
                tracing::error!(job_id = %self.config.job_id, "Codex session failed: {}", e);
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(format!("Codex CLI failed: {}", e)),
                        iterations: 1,
                    })
                    .await?;
                return Ok(());
            }
        };

        let mut iteration = 1u32;
        loop {
            match poll_for_prompt(&self.client).await {
                Ok(Some(prompt)) => {
                    if prompt.done {
                        tracing::info!(job_id = %self.config.job_id, "Orchestrator signaled done");
                        break;
                    }

                    iteration += 1;
                    if let Err(e) = self
                        .run_codex_session(&prompt.content, session_id.as_deref(), &extra_env)
                        .await
                    {
                        tracing::error!(
                            job_id = %self.config.job_id,
                            "Follow-up Codex session failed: {}", e
                        );
                        self.report_event(
                            "status",
                            &serde_json::json!({
                                "message": format!("Follow-up session failed: {}", e),
                            }),
                        )
                        .await;
                    }
                }
                Ok(None) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %self.config.job_id,
                        "Prompt polling error: {}", e
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        self.client
            .report_complete(&CompletionReport {
                success: true,
                message: Some("Codex session completed".to_string()),
                iterations: iteration,
            })
            .await?;

        Ok(())
    }

    async fn run_codex_session(
        &self,
        prompt: &str,
        resume_session_id: Option<&str>,
        extra_env: &std::collections::HashMap<String, String>,
    ) -> Result<Option<String>, WorkerError> {
        let mut cmd = Command::new("codex");
        cmd.args(codex_args(&self.config.model, prompt, resume_session_id))
            .env("CODEX_HOME", CODEX_HOME_PATH)
            .envs(extra_env)
            .current_dir("/workspace")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to spawn codex: {}", e),
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture codex stdout".to_string(),
            })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture codex stderr".to_string(),
            })?;

        let client_for_stderr = Arc::clone(&self.client);
        let job_id = self.config.job_id;
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(job_id = %job_id, "codex stderr: {}", line);
                post_job_event(
                    &client_for_stderr,
                    "status",
                    &serde_json::json!({ "message": line }),
                )
                .await;
            }
        });

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut session_id = resume_session_id.map(ToOwned::to_owned);

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<Value>(&line) {
                Ok(event) => {
                    let (captured_id, payloads) = codex_event_to_payloads(&event);
                    if captured_id.is_some() {
                        session_id = captured_id;
                    }

                    for payload in payloads {
                        self.report_event(&payload.event_type, &payload.data).await;
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        job_id = %self.config.job_id,
                        "Non-JSON codex output: {} (parse error: {})",
                        line,
                        e
                    );
                    self.report_event("status", &serde_json::json!({ "message": line }))
                        .await;
                }
            }
        }

        let status = match tokio::time::timeout(self.config.timeout, child.wait()).await {
            Ok(wait_result) => wait_result.map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed waiting for codex: {}", e),
            })?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(WorkerError::ExecutionFailed {
                    reason: format!(
                        "codex session timed out after {} seconds",
                        self.config.timeout.as_secs()
                    ),
                });
            }
        };

        let _ = stderr_handle.await;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            self.report_event(
                "result",
                &serde_json::json!({
                    "status": "error",
                    "exit_code": code,
                    "session_id": session_id,
                }),
            )
            .await;

            return Err(WorkerError::ExecutionFailed {
                reason: format!("codex exited with code {}", code),
            });
        }

        self.report_event(
            "result",
            &serde_json::json!({
                "status": "completed",
                "session_id": session_id,
            }),
        )
        .await;

        Ok(session_id)
    }

    async fn report_event(&self, event_type: &str, data: &Value) {
        post_job_event(&self.client, event_type, data).await;
    }
}

fn codex_args(model: &str, prompt: &str, resume_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "--ask-for-approval".to_string(),
        "never".to_string(),
        "--sandbox".to_string(),
        CODEX_SANDBOX_MODE.to_string(),
        "--skip-git-repo-check".to_string(),
        "-C".to_string(),
        "/workspace".to_string(),
    ];

    if !model.trim().is_empty() {
        args.push("--model".to_string());
        args.push(model.to_string());
    }

    if let Some(session_id) = resume_session_id {
        args.push("resume".to_string());
        args.push(session_id.to_string());
    }

    args.push(prompt.to_string());
    args
}

fn codex_event_to_payloads(event: &Value) -> (Option<String>, Vec<JobEventPayload>) {
    let mut payloads = Vec::new();
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let session_id = event
        .get("thread_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    match event_type {
        "thread.started" => {
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": "Codex session started",
                    "session_id": session_id,
                }),
            });
        }
        "turn.started" => {
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": "Codex turn started",
                    "turn_id": event.get("turn_id").cloned(),
                }),
            });
        }
        "turn.completed" => {
            if let Some(text) = extract_text(event.get("last_agent_message").unwrap_or(event)) {
                payloads.push(JobEventPayload {
                    event_type: "message".to_string(),
                    data: serde_json::json!({
                        "role": "assistant",
                        "content": text,
                    }),
                });
            }
        }
        "turn.failed" => {
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": event
                        .get("error")
                        .and_then(extract_text)
                        .unwrap_or_else(|| "Codex turn failed".to_string()),
                }),
            });
        }
        "item.started" => {
            if let Some(item) = event.get("item")
                && let Some(item_type) = item.get("type").and_then(Value::as_str)
                && let Some(tool_name) = item_tool_name(item_type, item)
            {
                payloads.push(JobEventPayload {
                    event_type: "tool_use".to_string(),
                    data: serde_json::json!({
                        "tool_name": tool_name,
                        "tool_use_id": item_identifier(item),
                        "input": item_input(item),
                    }),
                });
            }
        }
        "item.completed" => {
            if let Some(item) = event.get("item")
                && let Some(item_type) = item.get("type").and_then(Value::as_str)
            {
                match item_type {
                    "agent_message" => {
                        if let Some(text) = extract_text(item) {
                            payloads.push(JobEventPayload {
                                event_type: "message".to_string(),
                                data: serde_json::json!({
                                    "role": "assistant",
                                    "content": text,
                                }),
                            });
                        }
                    }
                    _ => {
                        if let Some(tool_name) = item_tool_name(item_type, item) {
                            payloads.push(JobEventPayload {
                                event_type: "tool_result".to_string(),
                                data: serde_json::json!({
                                    "tool_name": tool_name,
                                    "tool_use_id": item_identifier(item),
                                    "output": item_output(item),
                                }),
                            });
                        }
                    }
                }
            }
        }
        "error" => {
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": event
                        .get("message")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                        .or_else(|| event.get("error").and_then(extract_text))
                        .unwrap_or_else(|| "Codex error".to_string()),
                }),
            });
        }
        _ => {}
    }

    (session_id, payloads)
}

fn item_tool_name(item_type: &str, item: &Value) -> Option<String> {
    match item_type {
        "command_execution" => Some("shell".to_string()),
        "web_search" => Some("web_search".to_string()),
        "file_change" => Some("file_change".to_string()),
        "todo_list" => Some("todo_list".to_string()),
        "mcp_tool_call" | "collab_tool_call" => item
            .get("tool_name")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .or_else(|| Some(item_type.to_string())),
        "reasoning" => Some("reasoning".to_string()),
        "error" => Some("error".to_string()),
        "agent_message" => None,
        _ => Some(item_type.to_string()),
    }
}

fn item_identifier(item: &Value) -> Value {
    item.get("id")
        .cloned()
        .or_else(|| item.get("call_id").cloned())
        .or_else(|| item.get("tool_call_id").cloned())
        .unwrap_or(Value::Null)
}

fn item_input(item: &Value) -> Value {
    item.get("input")
        .cloned()
        .or_else(|| item.get("command").cloned())
        .or_else(|| item.get("query").cloned())
        .unwrap_or(Value::Null)
}

fn item_output(item: &Value) -> Value {
    if let Some(output) = item.get("output") {
        return output.clone();
    }

    let mut map = serde_json::Map::new();
    for key in [
        "stdout",
        "stderr",
        "exit_code",
        "status",
        "changes",
        "results",
    ] {
        if let Some(value) = item.get(key) {
            map.insert(key.to_string(), value.clone());
        }
    }

    if map.is_empty() {
        item.clone()
    } else {
        Value::Object(map)
    }
}

fn extract_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }

    if let Some(text) = value.get("text").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(content) = value.get("content").and_then(Value::as_array) {
        let joined = content
            .iter()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.trim().is_empty() {
            return Some(joined);
        }
    }

    if let Some(parts) = value.get("parts").and_then(Value::as_array) {
        let joined = parts
            .iter()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.trim().is_empty() {
            return Some(joined);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_started_maps_session_status() {
        let event = serde_json::json!({
            "type": "thread.started",
            "thread_id": "thread_123",
        });

        let (session_id, payloads) = codex_event_to_payloads(&event);
        assert_eq!(session_id.as_deref(), Some("thread_123"));
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "status");
        assert_eq!(payloads[0].data["session_id"], "thread_123");
    }

    #[test]
    fn agent_message_item_maps_to_message() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "agent_message",
                "content": [
                    { "text": "Made the requested changes." }
                ]
            }
        });

        let (_, payloads) = codex_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "message");
        assert_eq!(payloads[0].data["content"], "Made the requested changes.");
    }

    #[test]
    fn command_execution_started_maps_to_tool_use() {
        let event = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "cmd_1",
                "type": "command_execution",
                "command": { "cmd": "cargo test" }
            }
        });

        let (_, payloads) = codex_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "tool_use");
        assert_eq!(payloads[0].data["tool_name"], "shell");
        assert_eq!(payloads[0].data["tool_use_id"], "cmd_1");
    }

    #[test]
    fn command_execution_completed_maps_to_tool_result() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "cmd_1",
                "type": "command_execution",
                "stdout": "ok",
                "exit_code": 0
            }
        });

        let (_, payloads) = codex_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "tool_result");
        assert_eq!(payloads[0].data["tool_name"], "shell");
        assert_eq!(payloads[0].data["tool_use_id"], "cmd_1");
        assert_eq!(payloads[0].data["output"]["stdout"], "ok");
    }

    #[test]
    fn codex_exec_args_use_explicit_noninteractive_sandbox_flags() {
        let args = codex_args("gpt-5.3-codex", "fix the tests", None);

        assert!(
            args.windows(2)
                .any(|pair| pair == ["--ask-for-approval", "never"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--sandbox", CODEX_SANDBOX_MODE])
        );
        assert!(
            !args.iter().any(|arg| arg == "--full-auto"),
            "the bridge should use explicit flags instead of the full-auto shortcut"
        );
    }

    #[test]
    fn codex_exec_resume_args_append_session_before_prompt() {
        let args = codex_args("gpt-5.3-codex", "continue the refactor", Some("thread_123"));

        let resume_index = args
            .iter()
            .position(|arg| arg == "resume")
            .expect("resume subcommand missing");
        assert_eq!(args[resume_index + 1], "thread_123");
        assert_eq!(
            args.last().map(String::as_str),
            Some("continue the refactor")
        );
    }
}

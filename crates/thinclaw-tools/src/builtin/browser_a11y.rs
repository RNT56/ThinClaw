//! Accessibility-tree browser backend powered by the external `agent-browser` CLI.
//!
//! This backend is selected at tool-registration time and reuses the existing
//! `browser` tool name, but its actions are oriented around aria snapshots and
//! `@ref` selectors rather than Chromium CDP state kept in-process.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use tokio::process::Command;

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use super::browser::BrowserEgressRuntime;
use super::browser::is_network_url_allowed;
use super::capture_target::{CaptureFormat, CaptureTarget};
use crate::execution::bounded_command_output;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_AGENT_BROWSER_TEXT_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_AGENT_BROWSER_STDERR_BYTES: usize = 256 * 1024;
const MAX_AGENT_BROWSER_REFS: usize = 8192;
const MAX_AGENT_BROWSER_TEXT_INPUT_BYTES: usize = 64 * 1024;
const MAX_AGENT_BROWSER_EXPRESSION_BYTES: usize = 256 * 1024;

static REF_PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"ref="([^"]+)"|@ref=([A-Za-z0-9_-]+)"#).expect("valid agent-browser ref regex")
});

pub struct AgentBrowserTool {
    command: String,
    session: String,
    egress_runtime: Option<Arc<dyn BrowserEgressRuntime>>,
    operation_lock: tokio::sync::Mutex<()>,
    controlled_config: Result<tempfile::NamedTempFile, String>,
}

impl Default for AgentBrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBrowserTool {
    pub fn new() -> Self {
        Self::new_internal(None)
    }

    pub fn new_with_egress(egress_runtime: Arc<dyn BrowserEgressRuntime>) -> Self {
        Self::new_internal(Some(egress_runtime))
    }

    fn new_internal(egress_runtime: Option<Arc<dyn BrowserEgressRuntime>>) -> Self {
        let session = format!("thinclaw-{}", uuid::Uuid::new_v4().simple());
        Self {
            command: std::env::var("AGENT_BROWSER_BIN")
                .unwrap_or_else(|_| "agent-browser".to_string()),
            session,
            egress_runtime,
            operation_lock: tokio::sync::Mutex::new(()),
            controlled_config: create_controlled_agent_browser_config(),
        }
    }

    async fn run_text_command(&self, args: &[String]) -> Result<String, ToolError> {
        let egress_runtime = self.egress_runtime.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "agent-browser requires ThinClaw's pinned egress runtime".to_string(),
            )
        })?;
        let proxy = egress_runtime.start().await.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "failed to start agent-browser egress proxy: {error}"
            ))
        })?;
        let proxy_endpoint = proxy.authenticated_endpoint().map_err(|error| {
            ToolError::ExecutionFailed(format!("invalid agent-browser proxy: {error}"))
        })?;
        let config_path = self
            .controlled_config
            .as_ref()
            .map_err(|error| ToolError::ExecutionFailed(error.clone()))?
            .path();
        write_controlled_agent_browser_config(config_path, Some(&proxy_endpoint)).map_err(
            |error| {
                ToolError::ExecutionFailed(format!(
                    "failed to update agent-browser proxy config: {error}"
                ))
            },
        )?;

        let mut command = Command::new(&self.command);
        command
            .arg("--config")
            .arg(config_path)
            .arg("--session")
            .arg(&self.session)
            .args(args);
        let output = bounded_command_output(
            &mut command,
            COMMAND_TIMEOUT,
            MAX_AGENT_BROWSER_TEXT_OUTPUT_BYTES,
            MAX_AGENT_BROWSER_STDERR_BYTES,
            "agent-browser command",
        )
        .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let action = args.first().map(String::as_str).unwrap_or("command");
            return Err(ToolError::ExecutionFailed(format!(
                "`{}` {action} failed: {}",
                self.command,
                if stderr.is_empty() {
                    format!("exit {}", output.status)
                } else {
                    stderr
                }
            )));
        }

        String::from_utf8(output.stdout).map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "`{}` returned non-UTF8 output: {}",
                self.command, error
            ))
        })
    }

    fn snapshot_payload(snapshot: String, status: &str) -> serde_json::Value {
        let refs: Vec<String> = REF_PATTERN
            .captures_iter(&snapshot)
            .take(MAX_AGENT_BROWSER_REFS)
            .filter_map(|captures| {
                captures
                    .get(1)
                    .or_else(|| captures.get(2))
                    .map(|entry| entry.as_str().to_string())
            })
            .collect();

        serde_json::json!({
            "status": status,
            "backend": "agent_browser",
            "snapshot": snapshot,
            "element_count": refs.len(),
            "refs": refs,
        })
    }

    async fn navigate(&self, url: &str) -> Result<serde_json::Value, ToolError> {
        is_network_url_allowed(url)
            .await
            .map_err(ToolError::InvalidParameters)?;
        self.run_text_command(&["open".to_string(), url.to_string()])
            .await?;
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "navigated"))
    }

    async fn snapshot(&self) -> Result<serde_json::Value, ToolError> {
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "snapshot"))
    }

    async fn click(&self, reference: &str) -> Result<serde_json::Value, ToolError> {
        let reference = normalize_reference(reference)?;
        self.run_text_command(&["click".to_string(), reference])
            .await?;
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "clicked"))
    }

    async fn type_text(&self, reference: &str, text: &str) -> Result<serde_json::Value, ToolError> {
        let reference = normalize_reference(reference)?;
        if text.len() > MAX_AGENT_BROWSER_TEXT_INPUT_BYTES || text.contains('\0') {
            return Err(ToolError::InvalidParameters(format!(
                "browser text exceeds {MAX_AGENT_BROWSER_TEXT_INPUT_BYTES} bytes or contains NUL"
            )));
        }
        self.run_text_command(&["fill".to_string(), reference, text.to_string()])
            .await?;
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "typed"))
    }

    async fn scroll(&self, direction: &str) -> Result<serde_json::Value, ToolError> {
        if !matches!(direction, "up" | "down") {
            return Err(ToolError::InvalidParameters(
                "scroll direction must be up or down".to_string(),
            ));
        }
        self.run_text_command(&["scroll".to_string(), direction.to_string()])
            .await?;
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "scrolled"))
    }

    async fn console(&self, expression: &str) -> Result<serde_json::Value, ToolError> {
        if expression.is_empty()
            || expression.len() > MAX_AGENT_BROWSER_EXPRESSION_BYTES
            || expression.contains('\0')
        {
            return Err(ToolError::InvalidParameters(format!(
                "browser expression must be non-empty, at most {MAX_AGENT_BROWSER_EXPRESSION_BYTES} bytes, and contain no NUL"
            )));
        }
        let output = self
            .run_text_command(&["eval".to_string(), expression.to_string()])
            .await?;
        Ok(serde_json::json!({
            "backend": "agent_browser",
            "result": output.trim(),
        }))
    }

    async fn screenshot(&self) -> Result<serde_json::Value, ToolError> {
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let requested_path = thinclaw_platform::state_paths()
            .screenshots_dir
            .join(format!(
                "agent_browser_{ts}_{}.png",
                uuid::Uuid::new_v4().simple()
            ));
        let target = CaptureTarget::prepare(&requested_path, CaptureFormat::Png).await?;
        self.run_text_command(&[
            "screenshot".to_string(),
            target.staging_path().to_string_lossy().to_string(),
        ])
        .await?;
        let (path, size_bytes) = target.publish().await?;

        Ok(serde_json::json!({
            "status": "screenshot_taken",
            "backend": "agent_browser",
            "path": path.to_string_lossy(),
            "size_bytes": size_bytes,
        }))
    }

    async fn close(&self) -> Result<serde_json::Value, ToolError> {
        let close_result = self.run_text_command(&["close".to_string()]).await;
        let stop_result = if let Some(runtime) = self.egress_runtime.as_ref() {
            runtime.stop().await.map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to stop agent-browser egress proxy: {error}"
                ))
            })
        } else {
            Ok(())
        };
        close_result?;
        stop_result?;
        Ok(serde_json::json!({
            "status": "closed",
            "backend": "agent_browser",
        }))
    }
}

impl Drop for AgentBrowserTool {
    fn drop(&mut self) {
        let Some(runtime) = self.egress_runtime.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("agent-browser dropped outside a Tokio runtime; async cleanup skipped");
            return;
        };
        let command_name = self.command.clone();
        let session = self.session.clone();
        let cleanup_config = create_controlled_agent_browser_config().ok();
        handle.spawn(async move {
            let mut command = Command::new(command_name);
            if let Some(config) = cleanup_config.as_ref() {
                command.arg("--config").arg(config.path());
            }
            command.arg("--session").arg(session).arg("close");
            let _ = bounded_command_output(
                &mut command,
                Duration::from_secs(10),
                64 * 1024,
                64 * 1024,
                "agent-browser cleanup",
            )
            .await;
            if let Err(error) = runtime.stop().await {
                tracing::warn!(%error, "failed to stop dropped agent-browser egress runtime");
            }
        });
    }
}

fn normalize_reference(reference: &str) -> Result<String, ToolError> {
    let reference = reference.strip_prefix('@').unwrap_or(reference);
    if reference.len() < 2
        || reference.len() > 16
        || !reference.starts_with('e')
        || !reference[1..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ToolError::InvalidParameters(
            "browser ref must use the snapshot form e<number>".to_string(),
        ));
    }
    Ok(format!("@{reference}"))
}

#[async_trait]
impl Tool for AgentBrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browse the web using the external accessibility backend. Use this when you \
         need to inspect live websites, read page content, or interact with web UIs. \
         Navigate first, then use snapshot to get @ref selectors before click or type actions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "click", "type", "scroll", "console", "screenshot", "close"],
                    "description": "The browser action to perform."
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to for the navigate action."
                },
                "ref": {
                    "type": "string",
                    "description": "Aria snapshot @ref selector used by click and type."
                },
                "text": {
                    "type": "string",
                    "description": "Text to type for the type action."
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down"],
                    "description": "Scroll direction for the scroll action."
                },
                "expression": {
                    "type": "string",
                    "description": "JavaScript expression to execute for the console action."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let _operation_guard = self.operation_lock.lock().await;
        let action = params
            .get("action")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action' parameter".into()))?;

        let output = match action {
            "navigate" => {
                let url = params
                    .get("url")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("'navigate' requires 'url'".into())
                    })?;
                self.navigate(url).await?
            }
            "snapshot" => self.snapshot().await?,
            "click" => {
                let reference = params
                    .get("ref")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| ToolError::InvalidParameters("'click' requires 'ref'".into()))?;
                self.click(reference).await?
            }
            "type" => {
                let reference = params
                    .get("ref")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| ToolError::InvalidParameters("'type' requires 'ref'".into()))?;
                let text = params
                    .get("text")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| ToolError::InvalidParameters("'type' requires 'text'".into()))?;
                self.type_text(reference, text).await?
            }
            "scroll" => {
                let direction = params
                    .get("direction")
                    .and_then(|value| value.as_str())
                    .unwrap_or("down");
                self.scroll(direction).await?
            }
            "console" => {
                let expression = params
                    .get("expression")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("'console' requires 'expression'".into())
                    })?;
                self.console(expression).await?
            }
            "screenshot" => self.screenshot().await?,
            "close" => self.close().await?,
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown action '{}'. Supported actions: navigate, snapshot, click, type, scroll, console, screenshot",
                    action
                )));
            }
        };

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

fn create_controlled_agent_browser_config() -> Result<tempfile::NamedTempFile, String> {
    let file = tempfile::Builder::new()
        .prefix("thinclaw-agent-browser-")
        .suffix(".json")
        .tempfile()
        .map_err(|error| format!("failed to create agent-browser config: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to secure agent-browser config: {error}"))?;
    }
    write_controlled_agent_browser_config(file.path(), None)?;
    Ok(file)
}

fn write_controlled_agent_browser_config(
    path: &std::path::Path,
    proxy_endpoint: Option<&str>,
) -> Result<(), String> {
    let bypass_host = format!("{}.invalid", uuid::Uuid::new_v4().simple());
    let mut config = serde_json::json!({
        "autoConnect": false,
        "allowFileAccess": false,
        "contentBoundaries": true,
        "maxOutput": MAX_AGENT_BROWSER_TEXT_OUTPUT_BYTES,
        "proxyBypass": bypass_host,
        "args": "--proxy-bypass-list=<-loopback>",
        "headed": false,
    });
    if let Some(proxy_endpoint) = proxy_endpoint {
        config["proxy"] = serde_json::Value::String(proxy_endpoint.to_string());
    }
    let encoded = serde_json::to_vec(&config)
        .map_err(|error| format!("failed to encode agent-browser config: {error}"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to open agent-browser config: {error}"))?;
    file.write_all(&encoded)
        .map_err(|error| format!("failed to write agent-browser config: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush agent-browser config: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync agent-browser config: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_payload_extracts_refs() {
        let snapshot = "[button ref=\"e1\"] \"Search\"\n[@ref=e2] textbox";
        let payload = AgentBrowserTool::snapshot_payload(snapshot.to_string(), "snapshot");
        assert_eq!(payload["element_count"], 2);
    }

    #[test]
    fn schema_is_a11y_oriented() {
        let tool = AgentBrowserTool::new();
        let action_enum = tool.parameters_schema()["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .len();
        assert!(action_enum >= 6);
    }
}

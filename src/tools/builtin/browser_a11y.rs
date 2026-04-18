//! Accessibility-tree browser backend powered by the external `agent-browser` CLI.
//!
//! This backend is selected at tool-registration time and reuses the existing
//! `browser` tool name, but its actions are oriented around aria snapshots and
//! `@ref` selectors rather than Chromium CDP state kept in-process.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use regex::Regex;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCREENSHOT_PATH: &str = "thinclaw_agent_browser_screenshot.png";

static REF_PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"ref="([^"]+)"|@ref=([A-Za-z0-9_-]+)"#).expect("valid agent-browser ref regex")
});

pub struct AgentBrowserTool {
    command: String,
}

impl Default for AgentBrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBrowserTool {
    pub fn new() -> Self {
        Self {
            command: std::env::var("AGENT_BROWSER_BIN")
                .unwrap_or_else(|_| "agent-browser".to_string()),
        }
    }

    async fn run_text_command(&self, args: &[String]) -> Result<String, ToolError> {
        let mut command = Command::new(&self.command);
        command.args(args);
        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "`{}` timed out after {:?}",
                    self.command, COMMAND_TIMEOUT
                ))
            })?
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to launch `{}`: {}. Install agent-browser or switch browser_backend back to 'chromium'.",
                    self.command, error
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ToolError::ExecutionFailed(format!(
                "`{}` {} failed: {}",
                self.command,
                args.join(" "),
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

    async fn run_binary_command(&self, args: &[String]) -> Result<Vec<u8>, ToolError> {
        let mut command = Command::new(&self.command);
        command.args(args);
        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "`{}` timed out after {:?}",
                    self.command, COMMAND_TIMEOUT
                ))
            })?
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to launch `{}`: {}. Install agent-browser or switch browser_backend back to 'chromium'.",
                    self.command, error
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ToolError::ExecutionFailed(format!(
                "`{}` {} failed: {}",
                self.command,
                args.join(" "),
                if stderr.is_empty() {
                    format!("exit {}", output.status)
                } else {
                    stderr
                }
            )));
        }

        Ok(output.stdout)
    }

    fn snapshot_payload(snapshot: String, status: &str) -> serde_json::Value {
        let refs: Vec<String> = REF_PATTERN
            .captures_iter(&snapshot)
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
        let snapshot = self
            .run_text_command(&["navigate".to_string(), url.to_string()])
            .await?;
        Ok(Self::snapshot_payload(snapshot, "navigated"))
    }

    async fn snapshot(&self) -> Result<serde_json::Value, ToolError> {
        let snapshot = self.run_text_command(&["snapshot".to_string()]).await?;
        Ok(Self::snapshot_payload(snapshot, "snapshot"))
    }

    async fn click(&self, reference: &str) -> Result<serde_json::Value, ToolError> {
        let snapshot = self
            .run_text_command(&["click".to_string(), reference.to_string()])
            .await?;
        Ok(Self::snapshot_payload(snapshot, "clicked"))
    }

    async fn type_text(&self, reference: &str, text: &str) -> Result<serde_json::Value, ToolError> {
        let snapshot = self
            .run_text_command(&["type".to_string(), reference.to_string(), text.to_string()])
            .await?;
        Ok(Self::snapshot_payload(snapshot, "typed"))
    }

    async fn scroll(&self, direction: &str) -> Result<serde_json::Value, ToolError> {
        let snapshot = self
            .run_text_command(&["scroll".to_string(), direction.to_string()])
            .await?;
        Ok(Self::snapshot_payload(snapshot, "scrolled"))
    }

    async fn console(&self, expression: &str) -> Result<serde_json::Value, ToolError> {
        let output = self
            .run_text_command(&["console".to_string(), expression.to_string()])
            .await?;
        Ok(serde_json::json!({
            "backend": "agent_browser",
            "result": output.trim(),
        }))
    }

    async fn screenshot(&self) -> Result<serde_json::Value, ToolError> {
        let raw = self.run_binary_command(&["screenshot".to_string()]).await?;

        let png_bytes = if raw.starts_with(b"\x89PNG") {
            raw
        } else {
            let text = String::from_utf8(raw).map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "`{}` screenshot output was neither PNG bytes nor valid UTF-8/base64: {}",
                    self.command, error
                ))
            })?;
            base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|error| {
                    ToolError::ExecutionFailed(format!(
                        "`{}` screenshot output was not valid PNG or base64 PNG: {}",
                        self.command, error
                    ))
                })?
        };

        let path: PathBuf = std::env::temp_dir().join(SCREENSHOT_PATH);
        tokio::fs::write(&path, &png_bytes).await.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to save agent-browser screenshot: {}",
                error
            ))
        })?;

        Ok(serde_json::json!({
            "status": "screenshot_taken",
            "backend": "agent_browser",
            "path": path.to_string_lossy(),
            "size_bytes": png_bytes.len(),
        }))
    }
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
                    "enum": ["navigate", "snapshot", "click", "type", "scroll", "console", "screenshot"],
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

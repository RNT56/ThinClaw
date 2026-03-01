//! Canvas / A2UI tool — Agent-generated interactive UIs.
//!
//! Allows the agent to push interactive UI components to the user's
//! screen. The tool produces a structured JSON payload describing
//! the UI elements, which the frontend (Tauri WebView) renders.
//!
//! ## Supported Actions
//!
//! - `show` — Display a UI panel with components (text, buttons, forms, tables, charts)
//! - `update` — Update an existing panel's content
//! - `dismiss` — Close a displayed panel
//! - `notify` — Show a toast notification
//!
//! ## Architecture
//!
//! The tool outputs a `CanvasPayload` as its result. The orchestrator
//! or channel layer inspects the tool output and forwards it to the
//! appropriate frontend via the channel's status update mechanism or
//! a dedicated Tauri IPC event.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

// ── Canvas Payload Types ────────────────────────────────────────────

/// A canvas action produced by the tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CanvasAction {
    /// Display a UI panel.
    Show {
        /// Unique panel ID (agent-chosen, used for update/dismiss).
        panel_id: String,
        /// Panel title.
        title: String,
        /// UI components to render.
        components: Vec<UiComponent>,
        /// Panel position hint.
        #[serde(default)]
        position: PanelPosition,
        /// Whether the panel is modal (blocks interaction with chat).
        #[serde(default)]
        modal: bool,
    },
    /// Update an existing panel.
    Update {
        /// Panel ID to update.
        panel_id: String,
        /// Updated components (replaces all).
        components: Vec<UiComponent>,
    },
    /// Dismiss/close a panel.
    Dismiss {
        /// Panel ID to close.
        panel_id: String,
    },
    /// Show a toast notification.
    Notify {
        /// Notification message.
        message: String,
        /// Severity level.
        #[serde(default)]
        level: NotifyLevel,
        /// Auto-dismiss duration in seconds (0 = persistent).
        #[serde(default = "default_toast_duration")]
        duration_secs: u64,
    },
}

fn default_toast_duration() -> u64 {
    5
}

/// UI component that can be rendered in a panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiComponent {
    /// Markdown-formatted text block.
    Text { content: String },
    /// A heading.
    Heading {
        text: String,
        #[serde(default = "default_heading_level")]
        level: u8,
    },
    /// A data table.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A code block with syntax highlighting.
    Code { language: String, content: String },
    /// An image (base64 or URL).
    Image {
        src: String,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        width: Option<u32>,
    },
    /// A progress bar.
    Progress {
        #[serde(default)]
        label: String,
        value: f64,
        max: f64,
    },
    /// A key-value list.
    KeyValue { items: Vec<KvItem> },
    /// A separator / divider.
    Divider,
    /// A button (user interaction sends a message back to the agent).
    Button {
        label: String,
        /// Message sent back when clicked.
        action: String,
        #[serde(default)]
        style: ButtonStyle,
    },
    /// A form with input fields.
    Form {
        /// Form ID for response routing.
        form_id: String,
        fields: Vec<FormField>,
        submit_label: String,
    },
    /// JSON data rendered as a collapsible tree.
    Json {
        data: serde_json::Value,
        #[serde(default)]
        collapsed: bool,
    },
}

fn default_heading_level() -> u8 {
    2
}

/// Key-value item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvItem {
    pub key: String,
    pub value: String,
}

/// Form field definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormField {
    Text {
        name: String,
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        required: bool,
    },
    Number {
        name: String,
        label: String,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    Select {
        name: String,
        label: String,
        options: Vec<String>,
    },
    Checkbox {
        name: String,
        label: String,
        #[serde(default)]
        checked: bool,
    },
    Textarea {
        name: String,
        label: String,
        #[serde(default)]
        rows: Option<u32>,
    },
}

/// Panel position hint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelPosition {
    #[default]
    Right,
    Bottom,
    Center,
    Floating,
}

/// Notification severity.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// Button style.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Primary,
    Secondary,
    Danger,
    Ghost,
}

// ── Tool Implementation ─────────────────────────────────────────────

/// Canvas tool for agent-generated interactive UIs.
#[derive(Debug, Default)]
pub struct CanvasTool;

#[async_trait]
impl Tool for CanvasTool {
    fn name(&self) -> &str {
        "canvas"
    }

    fn description(&self) -> &str {
        "Display interactive UI panels, notifications, tables, forms, and visualizations \
         to the user. Use this when you need to present structured data, collect input, \
         or show progress in a rich visual format."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["show", "update", "dismiss", "notify"],
                    "description": "The canvas action to perform"
                },
                "panel_id": {
                    "type": "string",
                    "description": "Unique panel identifier (required for show/update/dismiss)"
                },
                "title": {
                    "type": "string",
                    "description": "Panel title (for show action)"
                },
                "components": {
                    "type": "array",
                    "description": "UI components to display (for show/update actions)",
                    "items": {
                        "type": "object",
                        "description": "A UI component with 'type' field (text, heading, table, code, image, progress, key_value, divider, button, form, json)"
                    }
                },
                "position": {
                    "type": "string",
                    "enum": ["right", "bottom", "center", "floating"],
                    "description": "Panel position hint (for show action)"
                },
                "modal": {
                    "type": "boolean",
                    "description": "Whether the panel blocks interaction (for show action)"
                },
                "message": {
                    "type": "string",
                    "description": "Notification message (for notify action)"
                },
                "level": {
                    "type": "string",
                    "enum": ["info", "success", "warning", "error"],
                    "description": "Notification severity (for notify action)"
                },
                "duration_secs": {
                    "type": "integer",
                    "description": "Auto-dismiss duration in seconds, 0 = persistent (for notify action)"
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

        let action_str = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action' parameter".to_string()))?;

        let canvas_action = match action_str {
            "show" => {
                let panel_id = params
                    .get("panel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();

                let title = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Agent Panel")
                    .to_string();

                let components = parse_components(&params)?;

                let position = params
                    .get("position")
                    .and_then(|v| v.as_str())
                    .map(|s| match s {
                        "bottom" => PanelPosition::Bottom,
                        "center" => PanelPosition::Center,
                        "floating" => PanelPosition::Floating,
                        _ => PanelPosition::Right,
                    })
                    .unwrap_or_default();

                let modal = params
                    .get("modal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                CanvasAction::Show {
                    panel_id,
                    title,
                    components,
                    position,
                    modal,
                }
            }
            "update" => {
                let panel_id = params
                    .get("panel_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'panel_id' for update action".to_string())
                    })?
                    .to_string();

                let components = parse_components(&params)?;

                CanvasAction::Update {
                    panel_id,
                    components,
                }
            }
            "dismiss" => {
                let panel_id = params
                    .get("panel_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'panel_id' for dismiss action".to_string())
                    })?
                    .to_string();

                CanvasAction::Dismiss { panel_id }
            }
            "notify" => {
                let message = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'message' for notify action".to_string())
                    })?
                    .to_string();

                let level = params
                    .get("level")
                    .and_then(|v| v.as_str())
                    .map(|s| match s {
                        "success" => NotifyLevel::Success,
                        "warning" => NotifyLevel::Warning,
                        "error" => NotifyLevel::Error,
                        _ => NotifyLevel::Info,
                    })
                    .unwrap_or_default();

                let duration_secs = params
                    .get("duration_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5);

                CanvasAction::Notify {
                    message,
                    level,
                    duration_secs,
                }
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown action: '{other}'. Use: show, update, dismiss, notify"
                )));
            }
        };

        // Serialize the action as the tool output
        let result = serde_json::to_value(&canvas_action).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to serialize canvas action: {e}"))
        })?;

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Canvas is display-only, no destructive operations
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false // Output is structured JSON, not user-facing text
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

/// Parse the `components` array from tool parameters.
fn parse_components(params: &serde_json::Value) -> Result<Vec<UiComponent>, ToolError> {
    let components_value = params
        .get("components")
        .ok_or_else(|| ToolError::InvalidParameters("Missing 'components' array".to_string()))?;

    let components: Vec<UiComponent> =
        serde_json::from_value(components_value.clone()).map_err(|e| {
            ToolError::InvalidParameters(format!(
                "Invalid components format: {e}. Each component needs a 'type' field."
            ))
        })?;

    if components.is_empty() {
        return Err(ToolError::InvalidParameters(
            "Components array must not be empty".to_string(),
        ));
    }

    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_show_action() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "show",
            "panel_id": "test-panel",
            "title": "Test Panel",
            "components": [
                {"type": "text", "content": "Hello, world!"},
                {"type": "heading", "text": "Section 1", "level": 2},
            ]
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();
        let action: CanvasAction = serde_json::from_value(result.result).unwrap();

        match action {
            CanvasAction::Show {
                panel_id,
                title,
                components,
                ..
            } => {
                assert_eq!(panel_id, "test-panel");
                assert_eq!(title, "Test Panel");
                assert_eq!(components.len(), 2);
            }
            _ => panic!("Expected Show action"),
        }
    }

    #[tokio::test]
    async fn test_notify_action() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "notify",
            "message": "Operation complete!",
            "level": "success",
            "duration_secs": 3
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();
        let action: CanvasAction = serde_json::from_value(result.result).unwrap();

        match action {
            CanvasAction::Notify {
                message,
                level,
                duration_secs,
            } => {
                assert_eq!(message, "Operation complete!");
                assert!(matches!(level, NotifyLevel::Success));
                assert_eq!(duration_secs, 3);
            }
            _ => panic!("Expected Notify action"),
        }
    }

    #[tokio::test]
    async fn test_dismiss_action() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "dismiss",
            "panel_id": "test-panel"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();
        let action: CanvasAction = serde_json::from_value(result.result).unwrap();

        match action {
            CanvasAction::Dismiss { panel_id } => {
                assert_eq!(panel_id, "test-panel");
            }
            _ => panic!("Expected Dismiss action"),
        }
    }

    #[tokio::test]
    async fn test_table_component() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "show",
            "panel_id": "data",
            "title": "Results",
            "components": [
                {
                    "type": "table",
                    "headers": ["Name", "Score"],
                    "rows": [["Alice", "95"], ["Bob", "87"]]
                }
            ]
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();
        assert!(result.result.is_object());
    }

    #[tokio::test]
    async fn test_invalid_action() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "explode"
        });

        let ctx = JobContext::default();
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_missing_components() {
        let tool = CanvasTool;
        let params = serde_json::json!({
            "action": "show",
            "panel_id": "p1",
            "title": "T"
        });

        let ctx = JobContext::default();
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("components"));
    }

    #[test]
    fn test_canvas_action_serialization() {
        let action = CanvasAction::Show {
            panel_id: "p1".to_string(),
            title: "Test".to_string(),
            components: vec![UiComponent::Text {
                content: "Hello".to_string(),
            }],
            position: PanelPosition::Right,
            modal: false,
        };

        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["action"], "show");
        assert_eq!(json["panel_id"], "p1");
    }

    #[test]
    fn test_tool_metadata() {
        let tool = CanvasTool;
        assert_eq!(tool.name(), "canvas");
        assert_eq!(tool.domain(), ToolDomain::Orchestrator);
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        ));
    }
}

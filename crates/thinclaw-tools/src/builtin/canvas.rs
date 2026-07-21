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

use std::time::Instant;

use async_trait::async_trait;
pub use thinclaw_tools_core::{CanvasAction, NotifyLevel, PanelPosition, UiComponent};

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};
use thinclaw_types::JobContext;

const MAX_PANEL_ID_BYTES: usize = 128;
const MAX_PANEL_TITLE_BYTES: usize = 512;
const MAX_COMPONENTS: usize = 100;
const MAX_COMPONENT_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_NOTIFICATION_BYTES: usize = 16 * 1024;

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
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "Unique panel identifier (required for show/update/dismiss)"
                },
                "title": {
                    "type": "string",
                    "maxLength": 512,
                    "description": "Panel title (for show action)"
                },
                "components": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 100,
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
                    "minLength": 1,
                    "maxLength": 16384,
                    "description": "Notification message (for notify action)"
                },
                "level": {
                    "type": "string",
                    "enum": ["info", "success", "warning", "error"],
                    "description": "Notification severity (for notify action)"
                },
                "duration_secs": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 86400,
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
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing 'action' parameter".to_string())
            })?;

        let canvas_action = match action_str {
            "show" => {
                let panel_id = params
                    .get("panel_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "Missing 'panel_id' for show action".to_string(),
                        )
                    })?
                    .to_string();
                validate_panel_id(&panel_id)?;

                let title = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Agent Panel")
                    .to_string();
                if title.len() > MAX_PANEL_TITLE_BYTES {
                    return Err(ToolError::InvalidParameters(
                        "Canvas title exceeds the 512-byte limit".to_string(),
                    ));
                }

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
                        ToolError::InvalidParameters(
                            "Missing 'panel_id' for update action".to_string(),
                        )
                    })?
                    .to_string();
                validate_panel_id(&panel_id)?;

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
                        ToolError::InvalidParameters(
                            "Missing 'panel_id' for dismiss action".to_string(),
                        )
                    })?
                    .to_string();
                validate_panel_id(&panel_id)?;

                CanvasAction::Dismiss { panel_id }
            }
            "notify" => {
                let message = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "Missing 'message' for notify action".to_string(),
                        )
                    })?
                    .to_string();
                if message.trim().is_empty() || message.len() > MAX_NOTIFICATION_BYTES {
                    return Err(ToolError::InvalidParameters(
                        "Canvas notification must contain 1 to 16384 bytes".to_string(),
                    ));
                }

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

                let duration_secs = match params.get("duration_secs") {
                    None => 5,
                    Some(value) => {
                        let duration = value.as_u64().ok_or_else(|| {
                            ToolError::InvalidParameters(
                                "Canvas duration_secs must be an integer from 0 to 86400"
                                    .to_string(),
                            )
                        })?;
                        if duration > 86_400 {
                            return Err(ToolError::InvalidParameters(
                                "Canvas duration_secs must be an integer from 0 to 86400"
                                    .to_string(),
                            ));
                        }
                        duration
                    }
                };

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

    let raw_components = components_value
        .as_array()
        .ok_or_else(|| ToolError::InvalidParameters("'components' must be an array".to_string()))?;
    if raw_components.len() > MAX_COMPONENTS {
        return Err(ToolError::InvalidParameters(format!(
            "Canvas panels support at most {MAX_COMPONENTS} components"
        )));
    }
    let payload_size = serde_json::to_vec(components_value)
        .map_err(|error| ToolError::InvalidParameters(format!("Invalid components: {error}")))?
        .len();
    if payload_size > MAX_COMPONENT_PAYLOAD_BYTES {
        return Err(ToolError::InvalidParameters(
            "Canvas component payload exceeds the 1 MiB limit".to_string(),
        ));
    }

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

fn validate_panel_id(panel_id: &str) -> Result<(), ToolError> {
    if panel_id.trim().is_empty()
        || panel_id.len() > MAX_PANEL_ID_BYTES
        || panel_id.chars().any(char::is_control)
    {
        return Err(ToolError::InvalidParameters(
            "Canvas panel_id must contain 1 to 128 bytes and no control characters".to_string(),
        ));
    }
    Ok(())
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

    #[tokio::test]
    async fn show_requires_a_valid_bounded_panel_id() {
        let tool = CanvasTool;
        let ctx = JobContext::default();
        for panel_id in [None, Some("   "), Some("bad\nname")] {
            let mut params = serde_json::json!({
                "action": "show",
                "components": [{"type": "text", "content": "hello"}]
            });
            if let Some(panel_id) = panel_id {
                params["panel_id"] = serde_json::json!(panel_id);
            }
            let error = tool.execute(params, &ctx).await.unwrap_err();
            assert!(error.to_string().contains("panel_id"));
        }

        let error = tool
            .execute(
                serde_json::json!({
                    "action": "show",
                    "panel_id": "x".repeat(MAX_PANEL_ID_BYTES + 1),
                    "components": [{"type": "text", "content": "hello"}]
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("panel_id"));
    }

    #[tokio::test]
    async fn canvas_payload_limits_are_enforced() {
        let tool = CanvasTool;
        let ctx = JobContext::default();
        let components = (0..=MAX_COMPONENTS)
            .map(|_| serde_json::json!({"type": "text", "content": "x"}))
            .collect::<Vec<_>>();
        let error = tool
            .execute(
                serde_json::json!({
                    "action": "show",
                    "panel_id": "many",
                    "components": components,
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("at most"));

        let error = tool
            .execute(
                serde_json::json!({
                    "action": "notify",
                    "message": "x".repeat(MAX_NOTIFICATION_BYTES + 1),
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("16384"));

        for duration in [serde_json::json!(-1), serde_json::json!(86_401)] {
            let error = tool
                .execute(
                    serde_json::json!({
                        "action": "notify",
                        "message": "hello",
                        "duration_secs": duration,
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("duration_secs"));
        }
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

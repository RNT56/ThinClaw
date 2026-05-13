#![allow(clippy::items_after_test_module)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};
use thinclaw_types::JobContext;

#[async_trait]
pub trait DesktopAutonomyPort: Send + Sync {
    async fn apps_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn ui_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn screen_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn calendar_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn numbers_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn pages_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    async fn status(&self) -> Result<serde_json::Value, String>;
    async fn pause(&self, reason: Option<String>);
    async fn resume(&self) -> Result<(), String>;
    async fn bootstrap(&self) -> Result<serde_json::Value, String>;
    async fn desktop_permission_status(&self) -> Result<serde_json::Value, String>;
    async fn rollback(&self) -> Result<serde_json::Value, String>;
    fn desktop_action_timeout_secs(&self) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopAutonomyToolKind {
    Apps,
    Ui,
    Screen,
    CalendarNative,
    NumbersNative,
    PagesNative,
    Control,
}

pub struct DesktopAutonomyTool {
    kind: DesktopAutonomyToolKind,
    manager: Arc<dyn DesktopAutonomyPort>,
}

impl DesktopAutonomyTool {
    pub fn apps(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::Apps,
            manager,
        }
    }

    pub fn ui(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::Ui,
            manager,
        }
    }

    pub fn screen(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::Screen,
            manager,
        }
    }

    pub fn calendar_native(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::CalendarNative,
            manager,
        }
    }

    pub fn numbers_native(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::NumbersNative,
            manager,
        }
    }

    pub fn pages_native(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::PagesNative,
            manager,
        }
    }

    pub fn control(manager: Arc<dyn DesktopAutonomyPort>) -> Self {
        Self {
            kind: DesktopAutonomyToolKind::Control,
            manager,
        }
    }

    fn action_enum(&self) -> &'static [&'static str] {
        match self.kind {
            DesktopAutonomyToolKind::Apps => &["list", "open", "focus", "quit", "windows", "menus"],
            DesktopAutonomyToolKind::Ui => &[
                "snapshot",
                "click",
                "double_click",
                "type_text",
                "set_value",
                "keypress",
                "chord",
                "select_menu",
                "scroll",
                "drag",
                "wait_for",
            ],
            DesktopAutonomyToolKind::Screen => &["capture", "window_capture", "ocr", "find_text"],
            DesktopAutonomyToolKind::CalendarNative => &[
                "list",
                "create",
                "update",
                "delete",
                "find",
                "ensure_calendar",
            ],
            DesktopAutonomyToolKind::NumbersNative => &[
                "create_doc",
                "open_doc",
                "read_range",
                "write_range",
                "set_formula",
                "run_table_action",
                "export",
            ],
            DesktopAutonomyToolKind::PagesNative => &[
                "create_doc",
                "open_doc",
                "insert_text",
                "replace_text",
                "export",
                "find",
            ],
            DesktopAutonomyToolKind::Control => &[
                "status",
                "pause",
                "resume",
                "bootstrap",
                "permissions",
                "rollback",
            ],
        }
    }

    fn execute_kind<'a>(
        &'a self,
        action: &'a str,
        params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'a>,
    > {
        Box::pin(async move {
            match self.kind {
                DesktopAutonomyToolKind::Apps => self.manager.apps_action(action, params).await,
                DesktopAutonomyToolKind::Ui => self.manager.ui_action(action, params).await,
                DesktopAutonomyToolKind::Screen => self.manager.screen_action(action, params).await,
                DesktopAutonomyToolKind::CalendarNative => {
                    self.manager.calendar_action(action, params).await
                }
                DesktopAutonomyToolKind::NumbersNative => {
                    self.manager.numbers_action(action, params).await
                }
                DesktopAutonomyToolKind::PagesNative => {
                    self.manager.pages_action(action, params).await
                }
                DesktopAutonomyToolKind::Control => match action {
                    "status" => self.manager.status().await,
                    "pause" => {
                        let reason = params
                            .get("reason")
                            .and_then(|value| value.as_str())
                            .map(str::to_string);
                        self.manager.pause(reason).await;
                        Ok(serde_json::json!({"paused": true}))
                    }
                    "resume" => {
                        self.manager.resume().await?;
                        Ok(serde_json::json!({"paused": false}))
                    }
                    "bootstrap" => self.manager.bootstrap().await,
                    "permissions" => self.manager.desktop_permission_status().await,
                    "rollback" => self.manager.rollback().await,
                    other => Err(format!("unsupported autonomy_control action '{other}'")),
                },
            }
        })
    }
}

impl std::fmt::Debug for DesktopAutonomyTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopAutonomyTool")
            .field("name", &self.name())
            .finish()
    }
}

#[async_trait]
impl Tool for DesktopAutonomyTool {
    fn name(&self) -> &str {
        match self.kind {
            DesktopAutonomyToolKind::Apps => "desktop_apps",
            DesktopAutonomyToolKind::Ui => "desktop_ui",
            DesktopAutonomyToolKind::Screen => "desktop_screen",
            DesktopAutonomyToolKind::CalendarNative => "desktop_calendar_native",
            DesktopAutonomyToolKind::NumbersNative => "desktop_numbers_native",
            DesktopAutonomyToolKind::PagesNative => "desktop_pages_native",
            DesktopAutonomyToolKind::Control => "autonomy_control",
        }
    }

    fn description(&self) -> &str {
        match self.kind {
            DesktopAutonomyToolKind::Apps => {
                "Interact with desktop applications by listing, opening, focusing, quitting, or inspecting windows and menus."
            }
            DesktopAutonomyToolKind::Ui => {
                "Capture desktop accessibility state and perform verified UI actions like click, type, keypress, drag, or wait-for."
            }
            DesktopAutonomyToolKind::Screen => {
                "Capture screenshots, run OCR, and locate visible text or windows for desktop evidence and fallback automation."
            }
            DesktopAutonomyToolKind::CalendarNative => {
                "Use the platform-native calendar adapter for structured event reads and writes."
            }
            DesktopAutonomyToolKind::NumbersNative => {
                "Use the platform-native spreadsheet adapter for structured sheet actions before falling back to generic UI automation."
            }
            DesktopAutonomyToolKind::PagesNative => {
                "Use the platform-native document editor adapter for structured edits and exports before falling back to generic UI automation."
            }
            DesktopAutonomyToolKind::Control => {
                "Inspect or control reckless desktop autonomy bootstrap, pause, resume, and rollback state."
            }
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": self.action_enum(),
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional desktop GUI session identifier. Defaults to the active autonomy session."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional pause reason used by autonomy_control pause."
                },
                "payload": {
                    "type": "object",
                    "description": "Action-specific structured parameters forwarded to the desktop sidecar."
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
            .ok_or_else(|| ToolError::InvalidParameters("missing action".to_string()))?;
        let mut forwarded = params
            .get("payload")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if !forwarded.is_object() {
            return Err(ToolError::InvalidParameters(
                "payload must be a JSON object".to_string(),
            ));
        }
        if let Some(session_id) = params.get("session_id").and_then(|value| value.as_str()) {
            forwarded["session_id"] = serde_json::json!(session_id);
        }
        if let Some(reason) = params.get("reason").and_then(|value| value.as_str()) {
            forwarded["reason"] = serde_json::json!(reason);
        }
        validate_tool_payload(self.kind, action, &forwarded)
            .map_err(ToolError::InvalidParameters)?;

        let result = self
            .execute_kind(action, forwarded)
            .await
            .map_err(ToolError::ExecutionFailed)?;
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(self.manager.desktop_action_timeout_secs().max(30))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubDesktopAutonomyPort;

    #[async_trait]
    impl DesktopAutonomyPort for StubDesktopAutonomyPort {
        async fn apps_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn ui_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn screen_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn calendar_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn numbers_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn pages_action(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn status(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn pause(&self, _reason: Option<String>) {}

        async fn resume(&self) -> Result<(), String> {
            Ok(())
        }

        async fn bootstrap(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn desktop_permission_status(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn rollback(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        fn desktop_action_timeout_secs(&self) -> u64 {
            60
        }
    }

    #[test]
    fn schemas_expose_action_enum() {
        let manager: Arc<dyn DesktopAutonomyPort> = Arc::new(StubDesktopAutonomyPort);
        let tool = DesktopAutonomyTool::apps(manager);
        let schema = tool.parameters_schema();
        assert!(schema.to_string().contains("list"));
        assert_eq!(tool.name(), "desktop_apps");
    }

    #[test]
    fn numbers_tool_rejects_missing_table_action_requirements() {
        let err = validate_tool_payload(
            DesktopAutonomyToolKind::NumbersNative,
            "run_table_action",
            &serde_json::json!({
                "table": "Table 1",
                "table_action": "add_row_below",
            }),
        )
        .expect_err("missing row_index should fail");
        assert!(err.contains("row_index"));
    }
}

fn validate_tool_payload(
    kind: DesktopAutonomyToolKind,
    action: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
    if kind == DesktopAutonomyToolKind::NumbersNative && action == "run_table_action" {
        let obj = payload
            .as_object()
            .ok_or_else(|| "desktop_numbers_native payload must be a JSON object".to_string())?;
        let table_action = obj
            .get("table_action")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "run_table_action requires payload.table_action".to_string())?;
        if obj
            .get("table")
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("run_table_action requires payload.table".to_string());
        }
        match table_action {
            "add_row_above" | "add_row_below" | "delete_row" => {
                if obj
                    .get("row_index")
                    .and_then(|value| value.as_i64())
                    .is_none()
                {
                    return Err(format!(
                        "run_table_action '{table_action}' requires payload.row_index"
                    ));
                }
            }
            "add_column_before"
            | "add_column_after"
            | "delete_column"
            | "sort_column_ascending"
            | "sort_column_descending" => {
                if obj
                    .get("column_index")
                    .and_then(|value| value.as_i64())
                    .is_none()
                {
                    return Err(format!(
                        "run_table_action '{table_action}' requires payload.column_index"
                    ));
                }
            }
            "clear_range" => {
                if obj
                    .get("range")
                    .and_then(|value| value.as_str())
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err("run_table_action 'clear_range' requires payload.range".to_string());
                }
            }
            other => {
                return Err(format!(
                    "unsupported run_table_action '{other}' for desktop_numbers_native"
                ));
            }
        }
    }
    Ok(())
}

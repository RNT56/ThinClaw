//! Conversion layer: IronClaw StatusUpdate → Scrappy UiEvent
//!
//! IronClaw's Channel trait receives StatusUpdate variants during a turn.
//! This module converts them to UiEvent variants that the frontend consumes.

use ironclaw::channels::StatusUpdate;
use serde_json::Value;

use super::sanitizer::strip_llm_tokens;
use super::ui_types::UiEvent;

/// Convert an IronClaw `StatusUpdate` to a Scrappy `UiEvent`.
///
/// The `session_key` and `run_id` are injected from the channel's routing
/// metadata (extracted from IncomingMessage::metadata).
pub fn status_to_ui_event(
    status: StatusUpdate,
    session_key: &str,
    run_id: Option<&str>,
    message_id: &str,
) -> Option<UiEvent> {
    let session_key = session_key.to_string();
    let run_id = run_id.map(|s| s.to_string());

    match status {
        StatusUpdate::Thinking(_text) => Some(UiEvent::RunStatus {
            session_key,
            run_id,
            status: "in_flight".into(),
            error: None,
        }),

        StatusUpdate::StreamChunk(delta) => Some(UiEvent::AssistantDelta {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            delta: strip_llm_tokens(&delta),
        }),

        StatusUpdate::ToolStarted { name } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: "started".into(),
            input: Value::Null,
            output: Value::Null,
        }),

        StatusUpdate::ToolCompleted { name, success } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: if success { "ok" } else { "error" }.into(),
            input: Value::Null,
            output: Value::Null,
        }),

        StatusUpdate::ToolResult { name, preview } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: "stream".into(),
            input: Value::Null,
            output: Value::String(preview),
        }),

        StatusUpdate::Status(text) => Some(UiEvent::RunStatus {
            session_key,
            run_id,
            status: text,
            error: None,
        }),

        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description: _,
            parameters,
        } => Some(UiEvent::ApprovalRequested {
            approval_id: request_id,
            session_key,
            tool_name,
            input: parameters,
        }),

        StatusUpdate::AuthRequired {
            extension_name,
            auth_url,
            ..
        } => Some(UiEvent::WebLogin {
            provider: extension_name,
            qr_code: None,
            status: auth_url.unwrap_or_else(|| "auth_required".into()),
        }),

        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => Some(UiEvent::WebLogin {
            provider: extension_name,
            qr_code: None,
            status: if success {
                "authenticated".into()
            } else {
                format!("failed: {}", message)
            },
        }),

        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => Some(UiEvent::CanvasUpdate {
            session_key,
            run_id,
            content: serde_json::json!({
                "job_id": job_id,
                "title": title,
                "browse_url": &browse_url,
            })
            .to_string(),
            content_type: "json".into(),
            url: Some(browse_url),
        }),

        StatusUpdate::Error { message, code } => Some(UiEvent::Error {
            code: code.unwrap_or_else(|| "turn_failed".into()),
            message,
            details: Value::Null,
        }),
    }
}

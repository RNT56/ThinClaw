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
        StatusUpdate::Thinking(text) => Some(UiEvent::AssistantInternal {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            text,
        }),

        StatusUpdate::StreamChunk(delta) => Some(UiEvent::AssistantDelta {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            delta: strip_llm_tokens(&delta),
        }),

        StatusUpdate::ToolStarted { name, parameters } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: "started".into(),
            input: parameters.unwrap_or(Value::Null),
            output: Value::Null,
        }),

        StatusUpdate::ToolCompleted {
            name,
            success,
            result_preview,
        } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: if success { "ok" } else { "error" }.into(),
            input: Value::Null,
            output: result_preview.map(Value::String).unwrap_or(Value::Null),
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

        StatusUpdate::CanvasAction(action) => Some(UiEvent::CanvasUpdate {
            session_key,
            run_id,
            content: serde_json::to_string(&action).unwrap_or_default(),
            content_type: "canvas_action".into(),
            url: None,
        }),

        StatusUpdate::AgentMessage {
            content,
            message_type,
        } => Some(UiEvent::AgentMessage {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            content: strip_llm_tokens(&content),
            message_type,
        }),

        // Lifecycle events — signal turn start/end to the frontend.
        //
        // LifecycleStart: emit AssistantInternal so the existing auto-activate path
        //   (OpenClawChatView ~line 711) fires — it creates activeRun + sets isSending=true
        //   from a single code path. Emitting RunStatus{status:"thinking"} here would
        //   double-fire the active-run branch (status not in TERMINAL_STATUSES → else branch).
        //
        // LifecycleEnd: normalise phase → a status the frontend TERMINAL_STATUSES list knows.
        //   ironclaw phase "response" → frontend "done"   (not in list otherwise)
        //   "interrupted" and "error" pass through unchanged (already in the list).
        StatusUpdate::LifecycleStart {
            run_id: lifecycle_run_id,
        } => Some(UiEvent::AssistantInternal {
            session_key,
            run_id: Some(lifecycle_run_id),
            message_id: message_id.to_string(),
            text: "Thinking...".into(),
        }),

        StatusUpdate::LifecycleEnd {
            run_id: lifecycle_run_id,
            phase,
        } => {
            // "response" is ironclaw's success phase name, but the frontend only
            // clears isSending on: ok | error | aborted | done | interrupted | rejected.
            let terminal_status = match phase.as_str() {
                "response" => "done".to_string(),
                other => other.to_string(),
            };
            Some(UiEvent::RunStatus {
                session_key,
                run_id: Some(lifecycle_run_id),
                status: terminal_status,
                error: None,
            })
        }

        // ── Sub-agent lifecycle → SubAgentUpdate ─────────────────────────
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
        } => Some(UiEvent::SubAgentUpdate {
            parent_session: session_key,
            child_session: agent_id,
            task: format!("[{}] {}", name, task),
            status: "running".into(),
            progress: Some(0.0),
            result_preview: None,
        }),

        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => Some(UiEvent::SubAgentUpdate {
            parent_session: session_key,
            child_session: agent_id,
            task: message,
            status: format!("running:{}", category),
            progress: None,
            result_preview: None,
        }),

        StatusUpdate::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
        } => {
            let preview = if response.len() > 200 {
                let mut end = 200;
                while !response.is_char_boundary(end) { end -= 1; }
                format!("{}…", &response[..end])
            } else {
                response
            };
            Some(UiEvent::SubAgentUpdate {
                parent_session: session_key,
                child_session: agent_id,
                task: format!(
                    "[{}] {} ({} iters, {:.1}s)",
                    name,
                    if success { "completed" } else { "failed" },
                    iterations,
                    duration_ms as f64 / 1000.0
                ),
                status: if success {
                    "completed".into()
                } else {
                    "failed".into()
                },
                progress: Some(1.0),
                result_preview: Some(preview),
            })
        }
    }
}

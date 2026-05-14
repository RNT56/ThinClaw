use thinclaw_channels_core::StatusUpdate;

use crate::web::types::SseEvent;

pub fn status_update_to_sse_event(status: StatusUpdate, thread_id: Option<String>) -> SseEvent {
    match status {
        StatusUpdate::Thinking(msg) => SseEvent::Thinking {
            message: msg,
            thread_id,
        },
        StatusUpdate::ToolStarted { name, .. } => SseEvent::ToolStarted { name, thread_id },
        StatusUpdate::ToolCompleted { name, success, .. } => SseEvent::ToolCompleted {
            name,
            success,
            thread_id,
        },
        StatusUpdate::ToolResult {
            name,
            preview,
            artifacts,
        } => SseEvent::ToolResult {
            name,
            preview,
            artifacts,
            thread_id,
        },
        StatusUpdate::StreamChunk(content) => SseEvent::StreamChunk { content, thread_id },
        StatusUpdate::Status(msg) => SseEvent::Status {
            message: msg,
            thread_id,
        },
        StatusUpdate::Plan { entries } => SseEvent::Status {
            message: serde_json::to_string(&entries).unwrap_or_else(|_| "plan update".to_string()),
            thread_id,
        },
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => SseEvent::Status {
            message: format!(
                "usage: {} input + {} output tokens{}{}",
                input_tokens,
                output_tokens,
                cost_usd
                    .map(|cost| format!(", ${cost:.6}"))
                    .unwrap_or_default(),
                model
                    .as_deref()
                    .map(|model| format!(" ({model})"))
                    .unwrap_or_default()
            ),
            thread_id,
        },
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => SseEvent::JobStarted {
            job_id,
            title,
            browse_url,
        },
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
        } => SseEvent::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters: serde_json::to_string_pretty(&parameters)
                .unwrap_or_else(|_| parameters.to_string()),
            thread_id,
        },
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id,
        } => SseEvent::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id.or(thread_id),
        },
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id,
        } => SseEvent::AuthCompleted {
            extension_name,
            success,
            message,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id.or(thread_id),
        },
        StatusUpdate::Error { message, code } => SseEvent::Status {
            message: format!(
                "[error{}] {}",
                code.as_ref().map(|c| format!(": {c}")).unwrap_or_default(),
                message
            ),
            thread_id,
        },
        StatusUpdate::CanvasAction(ref action) => {
            let (action_name, panel_id, content) = match action {
                thinclaw_tools_core::CanvasAction::Show {
                    panel_id,
                    title,
                    components,
                    ..
                } => (
                    "show",
                    panel_id.clone(),
                    Some(serde_json::json!({
                        "title": title,
                        "components": components,
                    })),
                ),
                thinclaw_tools_core::CanvasAction::Update {
                    panel_id,
                    components,
                } => (
                    "update",
                    panel_id.clone(),
                    Some(serde_json::json!({
                        "components": components,
                    })),
                ),
                thinclaw_tools_core::CanvasAction::Dismiss { panel_id } => {
                    ("dismiss", panel_id.clone(), None)
                }
                thinclaw_tools_core::CanvasAction::Notify {
                    message,
                    level,
                    duration_secs,
                } => (
                    "notify",
                    String::new(),
                    Some(serde_json::json!({
                        "message": message,
                        "level": format!("{:?}", level).to_lowercase(),
                        "duration_secs": duration_secs,
                    })),
                ),
            };
            SseEvent::CanvasUpdate {
                panel_id,
                action: action_name.to_string(),
                content,
            }
        }
        StatusUpdate::AgentMessage {
            content,
            message_type,
        } => {
            let prefix = match message_type.as_str() {
                "warning" => "⚠️ ",
                "question" => "❓ ",
                _ => "",
            };
            SseEvent::Response {
                content: format!("{}{}", prefix, content),
                thread_id: thread_id.unwrap_or_default(),
            }
        }
        StatusUpdate::LifecycleStart { run_id } => SseEvent::Status {
            message: format!("{{\"lifecycle\":\"start\",\"runId\":\"{}\"}}", run_id),
            thread_id,
        },
        StatusUpdate::LifecycleEnd { run_id, phase } => SseEvent::Status {
            message: format!(
                "{{\"lifecycle\":\"end\",\"runId\":\"{}\",\"phase\":\"{}\"}}",
                run_id, phase
            ),
            thread_id,
        },
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
        } => SseEvent::SubagentSpawned {
            agent_id,
            name,
            task,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => SseEvent::SubagentProgress {
            agent_id,
            message,
            category,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
        StatusUpdate::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
        } => SseEvent::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
    }
}

//! Conversion layer: IronClaw StatusUpdate → ThinClaw Desktop UiEvent
//!
//! IronClaw's Channel trait receives StatusUpdate variants during a turn.
//! This module converts them to UiEvent variants that the frontend consumes.

use ironclaw::channels::StatusUpdate;
use serde_json::Value;

use super::sanitizer::strip_llm_tokens;
use super::ui_types::{UiEvent, UiUsage};

fn session_key_from_metadata(metadata: &Value) -> Option<&str> {
    metadata
        .get("thread_id")
        .or_else(|| metadata.get("session_key"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn run_id_from_metadata(metadata: &Value) -> Option<&str> {
    metadata
        .get("run_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn message_id_from_metadata(metadata: &Value) -> Option<&str> {
    metadata
        .get("message_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn thread_id_from_status(status: &StatusUpdate) -> Option<&str> {
    match status {
        StatusUpdate::AuthRequired { thread_id, .. }
        | StatusUpdate::AuthCompleted { thread_id, .. } => thread_id.as_deref(),
        _ => None,
    }
}

/// Resolve routing identifiers for a local `StatusUpdate`.
///
/// Metadata wins over embedded status fields. If no route exists, fall back to
/// `agent:main` instead of the most recently active session so concurrent runs
/// cannot leak events into an unrelated user-selected thread.
pub fn routing_from_status<'a>(
    status: &'a StatusUpdate,
    metadata: &'a Value,
) -> (String, Option<&'a str>, &'a str) {
    let session_key = session_key_from_metadata(metadata)
        .or_else(|| thread_id_from_status(status))
        .unwrap_or("agent:main")
        .to_string();
    let run_id = run_id_from_metadata(metadata);
    let message_id = message_id_from_metadata(metadata).unwrap_or("unknown");
    (session_key, run_id, message_id)
}

/// Convert an IronClaw `StatusUpdate` to a ThinClaw Desktop `UiEvent`.
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

        StatusUpdate::ToolResult { name, preview, .. } => Some(UiEvent::ToolUpdate {
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
            thread_id,
            ..
        } => Some(UiEvent::WebLogin {
            session_key: thread_id.or(Some(session_key)),
            run_id,
            provider: extension_name,
            qr_code: None,
            status: auth_url.unwrap_or_else(|| "auth_required".into()),
        }),

        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
            thread_id,
            ..
        } => Some(UiEvent::WebLogin {
            session_key: thread_id.or(Some(session_key)),
            run_id,
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
        } => Some(UiEvent::JobUpdate {
            session_key: Some(session_key),
            run_id,
            job_id,
            title: Some(title),
            status: "started".into(),
            url: Some(browse_url),
            payload: Value::Null,
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

        // Lifecycle events — signal turn start/end to the frontend with an
        // explicit event shape while preserving the terminal status vocabulary
        // ThinClawChatView already understands.
        StatusUpdate::LifecycleStart {
            run_id: lifecycle_run_id,
        } => Some(UiEvent::LifecycleUpdate {
            session_key,
            run_id: lifecycle_run_id,
            phase: "start".into(),
            status: "started".into(),
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
            Some(UiEvent::LifecycleUpdate {
                session_key,
                run_id: lifecycle_run_id,
                phase: "end".into(),
                status: terminal_status,
            })
        }

        // ── Sub-agent lifecycle → SubAgentUpdate ─────────────────────────
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            ..
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
            ..
        } => {
            let preview = if response.len() > 200 {
                let mut end = 200;
                while !response.is_char_boundary(end) {
                    end -= 1;
                }
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

        StatusUpdate::Plan { entries } => Some(UiEvent::PlanUpdate {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            entries,
        }),

        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => {
            let input_tokens = input_tokens as u64;
            let output_tokens = output_tokens as u64;
            Some(UiEvent::UsageUpdate {
                session_key,
                run_id,
                message_id: message_id.to_string(),
                usage: UiUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                },
                cost_usd,
                model,
            })
        }
    }
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn value_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn value_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

fn value_usize(value: &Value, key: &str) -> usize {
    value_u64(value, key)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or_default()
}

fn value_f64(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|v| v.as_f64())
}

fn session_key_from_sse(value: &Value) -> Option<String> {
    value_string(value, "thread_id").or_else(|| value_string(value, "session_key"))
}

fn run_id_from_sse(value: &Value) -> Option<String> {
    value_string(value, "run_id")
}

fn message_id_from_sse(value: &Value, prefix: &str) -> String {
    value_string(value, "message_id")
        .or_else(|| value_string(value, "id").map(|id| format!("{prefix}-{id}")))
        .or_else(|| {
            session_key_from_sse(value).map(|session| {
                let run = run_id_from_sse(value).unwrap_or_else(|| "none".to_string());
                format!("{prefix}-{session}-{run}")
            })
        })
        .unwrap_or_else(|| format!("{prefix}-remote"))
}

fn gateway_event(event_type: &str, payload: Value) -> UiEvent {
    UiEvent::GatewayEvent {
        event_type: event_type.to_string(),
        session_key: session_key_from_sse(&payload),
        run_id: run_id_from_sse(&payload),
        payload,
    }
}

/// Convert a ThinClaw gateway SSE JSON event (`serde(tag = "type")`) to one
/// or more desktop UI events. Unknown but well-formed gateway events are still
/// forwarded as `GatewayEvent` so they remain visible on `thinclaw-event`.
pub fn gateway_sse_to_ui_events(value: Value) -> Vec<UiEvent> {
    let Some(event_type) = value
        .get("type")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return vec![gateway_event("unknown", value)];
    };

    let session_key = session_key_from_sse(&value);
    let run_id = run_id_from_sse(&value);

    match event_type.as_str() {
        "response" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::AssistantFinal {
                session_key,
                run_id,
                message_id: message_id_from_sse(&value, "remote-response"),
                text: value_string(&value, "content").unwrap_or_default(),
                usage: None,
            }]
        }
        "thinking" | "reasoning_content" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            let text_key = if event_type == "reasoning_content" {
                "content"
            } else {
                "message"
            };
            vec![UiEvent::AssistantInternal {
                session_key,
                run_id,
                message_id: message_id_from_sse(&value, "remote-thinking"),
                text: value_string(&value, text_key).unwrap_or_default(),
            }]
        }
        "stream_chunk" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::AssistantDelta {
                session_key,
                run_id,
                message_id: message_id_from_sse(&value, "remote-stream"),
                delta: strip_llm_tokens(&value_string(&value, "content").unwrap_or_default()),
            }]
        }
        "tool_started" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::ToolUpdate {
                session_key,
                run_id,
                tool_name: value_string(&value, "name").unwrap_or_default(),
                status: "started".into(),
                input: Value::Null,
                output: Value::Null,
            }]
        }
        "tool_completed" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::ToolUpdate {
                session_key,
                run_id,
                tool_name: value_string(&value, "name").unwrap_or_default(),
                status: if value_bool(&value, "success").unwrap_or(false) {
                    "ok".into()
                } else {
                    "error".into()
                },
                input: Value::Null,
                output: Value::Null,
            }]
        }
        "tool_result" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::ToolUpdate {
                session_key,
                run_id,
                tool_name: value_string(&value, "name").unwrap_or_default(),
                status: "stream".into(),
                input: Value::Null,
                output: value.get("preview").cloned().unwrap_or(Value::Null),
            }]
        }
        "status" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            let message = value_string(&value, "message").unwrap_or_default();
            if let Ok(lifecycle) = serde_json::from_str::<Value>(&message) {
                if lifecycle.get("lifecycle").and_then(|v| v.as_str()) == Some("start") {
                    let lifecycle_run_id = value_string(&lifecycle, "runId")
                        .or_else(|| run_id.clone())
                        .unwrap_or_else(|| "remote-run".to_string());
                    return vec![UiEvent::LifecycleUpdate {
                        session_key,
                        run_id: lifecycle_run_id,
                        phase: "start".into(),
                        status: "started".into(),
                    }];
                }
                if lifecycle.get("lifecycle").and_then(|v| v.as_str()) == Some("end") {
                    let lifecycle_run_id = value_string(&lifecycle, "runId")
                        .or_else(|| run_id.clone())
                        .unwrap_or_else(|| "remote-run".to_string());
                    let phase = value_string(&lifecycle, "phase").unwrap_or_else(|| "done".into());
                    let status = if phase == "response" { "done" } else { &phase }.to_string();
                    return vec![UiEvent::LifecycleUpdate {
                        session_key,
                        run_id: lifecycle_run_id,
                        phase: "end".into(),
                        status,
                    }];
                }
                if let Some(entries) = lifecycle.as_array() {
                    return vec![UiEvent::PlanUpdate {
                        session_key,
                        run_id,
                        message_id: message_id_from_sse(&value, "remote-plan"),
                        entries: entries.clone(),
                    }];
                }
            }
            vec![UiEvent::RunStatus {
                session_key,
                run_id,
                status: message,
                error: None,
            }]
        }
        "plan_update" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::PlanUpdate {
                session_key,
                run_id,
                message_id: message_id_from_sse(&value, "remote-plan"),
                entries: value
                    .get("entries")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
            }]
        }
        "usage_update" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            let input_tokens = value_u64(&value, "input_tokens").unwrap_or_default();
            let output_tokens = value_u64(&value, "output_tokens").unwrap_or_default();
            vec![UiEvent::UsageUpdate {
                session_key,
                run_id,
                message_id: message_id_from_sse(&value, "remote-usage"),
                usage: UiUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                },
                cost_usd: value_f64(&value, "cost_usd"),
                model: value_string(&value, "model"),
            }]
        }
        "approval_needed" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            let raw_parameters = value.get("parameters").cloned().unwrap_or(Value::Null);
            let parameters = raw_parameters
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(raw_parameters);
            vec![UiEvent::ApprovalRequested {
                approval_id: value_string(&value, "request_id").unwrap_or_default(),
                session_key,
                tool_name: value_string(&value, "tool_name").unwrap_or_default(),
                input: parameters,
            }]
        }
        "auth_required" => vec![UiEvent::WebLogin {
            session_key,
            run_id,
            provider: value_string(&value, "extension_name").unwrap_or_default(),
            qr_code: None,
            status: value_string(&value, "auth_url")
                .or_else(|| value_string(&value, "setup_url"))
                .unwrap_or_else(|| "auth_required".into()),
        }],
        "auth_completed" => vec![UiEvent::WebLogin {
            session_key,
            run_id,
            provider: value_string(&value, "extension_name").unwrap_or_default(),
            qr_code: None,
            status: if value_bool(&value, "success").unwrap_or(false) {
                "authenticated".into()
            } else {
                format!(
                    "failed: {}",
                    value_string(&value, "message").unwrap_or_default()
                )
            },
        }],
        "error" => vec![UiEvent::Error {
            code: "remote_error".into(),
            message: value_string(&value, "message").unwrap_or_default(),
            details: value.clone(),
        }],
        "job_started" => vec![UiEvent::JobUpdate {
            session_key,
            run_id,
            job_id: value_string(&value, "job_id").unwrap_or_default(),
            title: value_string(&value, "title"),
            status: "started".into(),
            url: value_string(&value, "browse_url"),
            payload: value.clone(),
        }],
        "job_status" | "job_result" | "job_session_result" => vec![UiEvent::JobUpdate {
            session_key,
            run_id,
            job_id: value_string(&value, "job_id").unwrap_or_default(),
            title: None,
            status: value_string(&value, "status")
                .or_else(|| value_string(&value, "message"))
                .unwrap_or_else(|| event_type.to_string()),
            url: None,
            payload: value.clone(),
        }],
        "canvas_update" => {
            let session_key = session_key.unwrap_or_else(|| "agent:main".to_string());
            vec![UiEvent::CanvasUpdate {
                session_key,
                run_id,
                content: value.to_string(),
                content_type: "gateway_canvas_update".into(),
                url: None,
            }]
        }
        "subagent_spawned" => {
            let parent_session = session_key.unwrap_or_else(|| "agent:main".to_string());
            let child_session = value_string(&value, "agent_id").unwrap_or_default();
            vec![UiEvent::SubAgentUpdate {
                parent_session,
                child_session,
                task: value_string(&value, "task").unwrap_or_default(),
                status: "running".into(),
                progress: Some(0.0),
                result_preview: None,
            }]
        }
        "subagent_progress" => {
            let parent_session = session_key.unwrap_or_else(|| "agent:main".to_string());
            let child_session = value_string(&value, "agent_id").unwrap_or_default();
            vec![UiEvent::SubAgentUpdate {
                parent_session,
                child_session,
                task: value_string(&value, "message").unwrap_or_default(),
                status: format!(
                    "running:{}",
                    value_string(&value, "category").unwrap_or_else(|| "progress".into())
                ),
                progress: None,
                result_preview: None,
            }]
        }
        "subagent_completed" => {
            let parent_session = session_key.unwrap_or_else(|| "agent:main".to_string());
            let child_session = value_string(&value, "agent_id").unwrap_or_default();
            let success = value_bool(&value, "success").unwrap_or(false);
            let response = value_string(&value, "response").unwrap_or_default();
            let preview = if response.len() > 200 {
                let mut end = 200;
                while !response.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &response[..end])
            } else {
                response
            };
            vec![UiEvent::SubAgentUpdate {
                parent_session,
                child_session,
                task: format!(
                    "[{}] {} ({} iters, {:.1}s)",
                    value_string(&value, "name").unwrap_or_else(|| "subagent".into()),
                    if success { "completed" } else { "failed" },
                    value_usize(&value, "iterations"),
                    value_u64(&value, "duration_ms").unwrap_or_default() as f64 / 1000.0
                ),
                status: if success {
                    "completed".into()
                } else {
                    "failed".into()
                },
                progress: Some(1.0),
                result_preview: Some(preview),
            }]
        }
        "routine_lifecycle" => vec![UiEvent::RoutineLifecycle {
            routine_name: value_string(&value, "routine_name").unwrap_or_default(),
            event: value_string(&value, "event").unwrap_or_default(),
            run_id,
            result_summary: value_string(&value, "result_summary"),
        }],
        "cost_alert" => vec![UiEvent::CostAlert {
            alert_type: value_string(&value, "alert_type").unwrap_or_default(),
            current_cost_usd: value_f64(&value, "current_cost_usd").unwrap_or_default(),
            limit_usd: value_f64(&value, "limit_usd").unwrap_or_default(),
            message: value_string(&value, "message"),
        }],
        "bootstrap_completed" => vec![UiEvent::BootstrapCompleted],
        _ => vec![gateway_event(&event_type, value)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subagent_packet() -> ironclaw::agent::subagent_executor::SubagentTaskPacket {
        ironclaw::agent::subagent_executor::SubagentTaskPacket {
            objective: "test objective".to_string(),
            ..Default::default()
        }
    }

    fn auth_required(thread_id: Option<&str>) -> StatusUpdate {
        StatusUpdate::AuthRequired {
            extension_name: "gmail".into(),
            instructions: Some("sign in".into()),
            auth_url: Some("https://example.test/auth".into()),
            setup_url: None,
            auth_mode: "oauth".into(),
            auth_status: "awaiting_authorization".into(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: thread_id.map(ToString::to_string),
        }
    }

    #[test]
    fn maps_every_current_status_update_variant() {
        let packet = subagent_packet();
        let statuses = vec![
            StatusUpdate::Thinking("thinking".into()),
            StatusUpdate::ToolStarted {
                name: "read_file".into(),
                parameters: Some(serde_json::json!({"path": "README.md"})),
            },
            StatusUpdate::ToolCompleted {
                name: "read_file".into(),
                success: true,
                result_preview: Some("ok".into()),
            },
            StatusUpdate::ToolResult {
                name: "read_file".into(),
                preview: "result".into(),
                artifacts: Vec::new(),
            },
            StatusUpdate::StreamChunk("chunk".into()),
            StatusUpdate::Status("running".into()),
            StatusUpdate::Plan {
                entries: vec![serde_json::json!({"step": "one"})],
            },
            StatusUpdate::Usage {
                input_tokens: 10,
                output_tokens: 5,
                cost_usd: Some(0.001),
                model: Some("model".into()),
            },
            StatusUpdate::JobStarted {
                job_id: "job-1".into(),
                title: "Job".into(),
                browse_url: "http://localhost/job-1".into(),
            },
            StatusUpdate::ApprovalNeeded {
                request_id: "approval-1".into(),
                tool_name: "shell".into(),
                description: "run".into(),
                parameters: serde_json::json!({"cmd": "true"}),
            },
            auth_required(Some("auth-thread")),
            StatusUpdate::AuthCompleted {
                extension_name: "gmail".into(),
                success: true,
                message: "ok".into(),
                auth_mode: Some("oauth".into()),
                auth_status: Some("authenticated".into()),
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
                thread_id: Some("auth-thread".into()),
            },
            StatusUpdate::Error {
                message: "failed".into(),
                code: Some("code".into()),
            },
            StatusUpdate::CanvasAction(ironclaw::tools::builtin::CanvasAction::Dismiss {
                panel_id: "panel-1".into(),
            }),
            StatusUpdate::AgentMessage {
                content: "checkpoint".into(),
                message_type: "progress".into(),
            },
            StatusUpdate::LifecycleStart {
                run_id: "run-1".into(),
            },
            StatusUpdate::LifecycleEnd {
                run_id: "run-1".into(),
                phase: "response".into(),
            },
            StatusUpdate::SubagentSpawned {
                agent_id: "child-1".into(),
                name: "researcher".into(),
                task: "research".into(),
                task_packet: packet.clone(),
                allowed_tools: Vec::new(),
                allowed_skills: Vec::new(),
                memory_mode: "provided_context_only".into(),
                tool_mode: "explicit_only".into(),
                skill_mode: "explicit_only".into(),
            },
            StatusUpdate::SubagentProgress {
                agent_id: "child-1".into(),
                message: "working".into(),
                category: "thinking".into(),
            },
            StatusUpdate::SubagentCompleted {
                agent_id: "child-1".into(),
                name: "researcher".into(),
                success: true,
                response: "done".into(),
                duration_ms: 1500,
                iterations: 2,
                task_packet: packet,
                allowed_tools: Vec::new(),
                allowed_skills: Vec::new(),
                memory_mode: "provided_context_only".into(),
                tool_mode: "explicit_only".into(),
                skill_mode: "explicit_only".into(),
            },
        ];

        for status in statuses {
            assert!(
                status_to_ui_event(status, "thread-a", Some("run-a"), "msg-a").is_some(),
                "StatusUpdate variant should map to a UiEvent"
            );
        }
    }

    #[test]
    fn routing_prefers_metadata_over_status_thread_id() {
        let status = auth_required(Some("auth-thread"));
        let metadata = serde_json::json!({
            "thread_id": "metadata-thread",
            "run_id": "run-1",
            "message_id": "msg-1"
        });

        let (session_key, run_id, message_id) = routing_from_status(&status, &metadata);

        assert_eq!(session_key, "metadata-thread");
        assert_eq!(run_id, Some("run-1"));
        assert_eq!(message_id, "msg-1");
    }

    #[test]
    fn routing_does_not_use_last_active_session_when_unscoped() {
        let status = StatusUpdate::Thinking("thinking".into());
        let (session_key, run_id, message_id) = routing_from_status(&status, &Value::Null);

        assert_eq!(session_key, "agent:main");
        assert_eq!(run_id, None);
        assert_eq!(message_id, "unknown");
    }

    #[test]
    fn maps_plan_status_to_ui_event() {
        let event = status_to_ui_event(
            StatusUpdate::Plan {
                entries: vec![serde_json::json!({
                    "step": "Inspect runtime wiring",
                    "status": "in_progress"
                })],
            },
            "agent:main",
            Some("run-1"),
            "msg-1",
        )
        .expect("plan status should map");

        match event {
            UiEvent::PlanUpdate {
                session_key,
                run_id,
                message_id,
                entries,
            } => {
                assert_eq!(session_key, "agent:main");
                assert_eq!(run_id.as_deref(), Some("run-1"));
                assert_eq!(message_id, "msg-1");
                assert_eq!(entries.len(), 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_usage_status_to_ui_event() {
        let event = status_to_ui_event(
            StatusUpdate::Usage {
                input_tokens: 123,
                output_tokens: 45,
                cost_usd: Some(0.0123),
                model: Some("provider/model".into()),
            },
            "thread-a",
            Some("run-2"),
            "msg-2",
        )
        .expect("usage status should map");

        match event {
            UiEvent::UsageUpdate {
                session_key,
                run_id,
                message_id,
                usage,
                cost_usd,
                model,
            } => {
                assert_eq!(session_key, "thread-a");
                assert_eq!(run_id.as_deref(), Some("run-2"));
                assert_eq!(message_id, "msg-2");
                assert_eq!(usage.input_tokens, 123);
                assert_eq!(usage.output_tokens, 45);
                assert_eq!(usage.total_tokens, 168);
                assert_eq!(cost_usd, Some(0.0123));
                assert_eq!(model.as_deref(), Some("provider/model"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_gateway_plan_usage_and_lifecycle_events() {
        let plan = gateway_sse_to_ui_events(serde_json::json!({
            "type": "plan_update",
            "thread_id": "thread-1",
            "run_id": "run-1",
            "entries": [{"step": "inspect"}]
        }));
        assert!(matches!(
            plan.as_slice(),
            [UiEvent::PlanUpdate {
                session_key,
                run_id,
                entries,
                ..
            }] if session_key == "thread-1"
                && run_id.as_deref() == Some("run-1")
                && entries.len() == 1
        ));

        let usage = gateway_sse_to_ui_events(serde_json::json!({
            "type": "usage_update",
            "thread_id": "thread-1",
            "run_id": "run-1",
            "input_tokens": 7,
            "output_tokens": 11,
            "cost_usd": 0.002,
            "model": "provider/model"
        }));
        assert!(matches!(
            usage.as_slice(),
            [UiEvent::UsageUpdate { usage, cost_usd, model, .. }]
                if usage.total_tokens == 18
                    && *cost_usd == Some(0.002)
                    && model.as_deref() == Some("provider/model")
        ));

        let lifecycle = gateway_sse_to_ui_events(serde_json::json!({
            "type": "status",
            "thread_id": "thread-1",
            "message": "{\"lifecycle\":\"end\",\"runId\":\"run-1\",\"phase\":\"response\"}"
        }));
        assert!(matches!(
            lifecycle.as_slice(),
            [UiEvent::LifecycleUpdate { session_key, run_id, phase, status }]
                if session_key == "thread-1"
                    && run_id == "run-1"
                    && phase == "end"
                    && status == "done"
        ));
    }

    #[test]
    fn maps_gateway_product_surface_events_without_dropping() {
        let approval = gateway_sse_to_ui_events(serde_json::json!({
            "type": "approval_needed",
            "thread_id": "thread-approval",
            "request_id": "approval-1",
            "tool_name": "shell",
            "parameters": "{\"cmd\":\"true\"}"
        }));
        assert!(matches!(
            approval.as_slice(),
            [UiEvent::ApprovalRequested { approval_id, session_key, tool_name, input }]
                if approval_id == "approval-1"
                    && session_key == "thread-approval"
                    && tool_name == "shell"
                    && input.get("cmd").and_then(Value::as_str) == Some("true")
        ));

        let auth = gateway_sse_to_ui_events(serde_json::json!({
            "type": "auth_required",
            "thread_id": "thread-auth",
            "run_id": "run-auth",
            "extension_name": "gmail",
            "auth_url": "https://example.test/oauth"
        }));
        assert!(matches!(
            auth.as_slice(),
            [UiEvent::WebLogin { session_key, run_id, provider, status, .. }]
                if session_key.as_deref() == Some("thread-auth")
                    && run_id.as_deref() == Some("run-auth")
                    && provider == "gmail"
                    && status == "https://example.test/oauth"
        ));

        let job = gateway_sse_to_ui_events(serde_json::json!({
            "type": "job_started",
            "thread_id": "thread-job",
            "run_id": "run-job",
            "job_id": "job-1",
            "title": "Build",
            "browse_url": "http://localhost/jobs/job-1"
        }));
        assert!(matches!(
            job.as_slice(),
            [UiEvent::JobUpdate { session_key, run_id, job_id, title, status, url, .. }]
                if session_key.as_deref() == Some("thread-job")
                    && run_id.as_deref() == Some("run-job")
                    && job_id == "job-1"
                    && title.as_deref() == Some("Build")
                    && status == "started"
                    && url.as_deref() == Some("http://localhost/jobs/job-1")
        ));

        let canvas = gateway_sse_to_ui_events(serde_json::json!({
            "type": "canvas_update",
            "thread_id": "thread-canvas",
            "run_id": "run-canvas",
            "panel_id": "panel-1"
        }));
        assert!(matches!(
            canvas.as_slice(),
            [UiEvent::CanvasUpdate { session_key, run_id, content_type, .. }]
                if session_key == "thread-canvas"
                    && run_id.as_deref() == Some("run-canvas")
                    && content_type == "gateway_canvas_update"
        ));

        let subagent = gateway_sse_to_ui_events(serde_json::json!({
            "type": "subagent_spawned",
            "thread_id": "thread-parent",
            "agent_id": "child-1",
            "task": "research"
        }));
        assert!(matches!(
            subagent.as_slice(),
            [UiEvent::SubAgentUpdate { parent_session, child_session, task, status, progress, .. }]
                if parent_session == "thread-parent"
                    && child_session == "child-1"
                    && task == "research"
                    && status == "running"
                    && *progress == Some(0.0)
        ));

        let routine = gateway_sse_to_ui_events(serde_json::json!({
            "type": "routine_lifecycle",
            "run_id": "routine-run-1",
            "routine_name": "daily brief",
            "event": "completed",
            "result_summary": "ok"
        }));
        assert!(matches!(
            routine.as_slice(),
            [UiEvent::RoutineLifecycle { routine_name, event, run_id, result_summary }]
                if routine_name == "daily brief"
                    && event == "completed"
                    && run_id.as_deref() == Some("routine-run-1")
                    && result_summary.as_deref() == Some("ok")
        ));

        let cost = gateway_sse_to_ui_events(serde_json::json!({
            "type": "cost_alert",
            "alert_type": "threshold",
            "current_cost_usd": 12.5,
            "limit_usd": 10.0,
            "message": "limit reached"
        }));
        assert!(matches!(
            cost.as_slice(),
            [UiEvent::CostAlert { alert_type, current_cost_usd, limit_usd, message }]
                if alert_type == "threshold"
                    && (*current_cost_usd - 12.5).abs() < f64::EPSILON
                    && (*limit_usd - 10.0).abs() < f64::EPSILON
                    && message.as_deref() == Some("limit reached")
        ));

        let bootstrap = gateway_sse_to_ui_events(serde_json::json!({
            "type": "bootstrap_completed"
        }));
        assert!(matches!(
            bootstrap.as_slice(),
            [UiEvent::BootstrapCompleted]
        ));
    }

    #[test]
    fn gateway_events_keep_concurrent_session_routes_isolated() {
        let first = gateway_sse_to_ui_events(serde_json::json!({
            "type": "stream_chunk",
            "thread_id": "thread-a",
            "run_id": "run-a",
            "content": "a"
        }));
        let second = gateway_sse_to_ui_events(serde_json::json!({
            "type": "stream_chunk",
            "thread_id": "thread-b",
            "run_id": "run-b",
            "content": "b"
        }));

        assert!(matches!(
            first.as_slice(),
            [UiEvent::AssistantDelta { session_key, run_id, delta, .. }]
                if session_key == "thread-a"
                    && run_id.as_deref() == Some("run-a")
                    && delta == "a"
        ));
        assert!(matches!(
            second.as_slice(),
            [UiEvent::AssistantDelta { session_key, run_id, delta, .. }]
                if session_key == "thread-b"
                    && run_id.as_deref() == Some("run-b")
                    && delta == "b"
        ));
    }

    #[test]
    fn maps_unknown_gateway_event_to_visible_passthrough() {
        let events = gateway_sse_to_ui_events(serde_json::json!({
            "type": "future_event",
            "thread_id": "thread-x",
            "run_id": "run-x",
            "payload": true
        }));

        assert!(matches!(
            events.as_slice(),
            [UiEvent::GatewayEvent { event_type, session_key, run_id, .. }]
                if event_type == "future_event"
                    && session_key.as_deref() == Some("thread-x")
                    && run_id.as_deref() == Some("run-x")
        ));
    }
}

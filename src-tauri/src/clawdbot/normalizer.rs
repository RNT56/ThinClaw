//! Event normalizer - converts upstream Moltbot events to stable UI contract
//!
//! This layer insulates the UI from protocol drift by parsing events
//! defensively and mapping them to a stable internal representation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use super::frames::WsFrame;

/// Stable UI event contract - what the Clawdbot chat UI consumes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UiEvent {
    /// Successfully connected to gateway
    Connected { protocol: u32 },

    /// Disconnected from gateway
    Disconnected { reason: String },

    /// List of available sessions
    SessionList { sessions: Vec<UiSession> },

    /// Chat history response
    History {
        session_key: String,
        messages: Vec<UiMessage>,
        has_more: bool,
        before: Option<String>,
    },

    /// Streaming assistant delta (append)
    AssistantDelta {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        delta: String,
    },

    /// Streaming assistant snapshot (replace)
    AssistantInternal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },
    AssistantSnapshot {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },

    /// Final assistant message (replace)
    AssistantFinal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
        usage: Option<UiUsage>,
    },

    /// Tool execution update
    ToolUpdate {
        session_key: String,
        run_id: Option<String>,
        tool_name: String,
        status: String, // started|stream|ok|error
        input: Value,
        output: Value,
    },

    /// Run status change
    RunStatus {
        session_key: String,
        run_id: Option<String>,
        status: String, // started|in_flight|ok|error|aborted
        error: Option<String>,
    },

    /// Approval requested for tool execution
    ApprovalRequested {
        approval_id: String,
        session_key: String,
        tool_name: String,
        input: Value,
    },
    /// Approval has been resolved (approved/denied)
    ApprovalResolved {
        approval_id: String,
        session_key: String,
        approved: bool,
    },

    /// Gateway error
    Error {
        code: String,
        message: String,
        details: Value,
    },

    /// Web login event (QR code, status)
    WebLogin {
        provider: String,
        qr_code: Option<String>,
        status: String,
    },
}

/// Session metadata for session list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSession {
    pub session_key: String,
    pub title: Option<String>,
    pub updated_at_ms: Option<u64>,
    pub source: Option<String>, // slack|telegram|webchat|...
}

/// Message in chat history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMessage {
    pub id: String,
    pub role: String, // user|assistant|tool|system
    pub ts_ms: u64,
    pub text: String,
    pub source: Option<String>,
}

/// Token usage stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Normalize a WsFrame::Event into a UiEvent
pub fn normalize_event(frame: &WsFrame) -> Option<UiEvent> {
    let WsFrame::Event { event, payload, .. } = frame else {
        return None;
    };

    // Debug: Log all incoming events
    info!(
        "[normalizer] Incoming event: {} | payload keys: {:?}",
        event,
        payload.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );

    match event.as_str() {
        "connect.challenge" => {
            // Handled in connection state machine, not exposed to UI
            None
        }

        "chat" => normalize_chat_event(payload),

        "health" | "status" | "tick" | "ping" => {
            // Heartbeats - completely silent to reduce log noise
            None
        }

        "exec.approval.requested" => {
            // ... (keep existing approval logic) ...
            let approval_id = payload
                .get("approvalId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let tool_name = payload
                .get("tool")
                .and_then(|t| t.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let input = payload
                .get("tool")
                .and_then(|t| t.get("input"))
                .cloned()
                .unwrap_or(Value::Null);

            Some(UiEvent::ApprovalRequested {
                approval_id,
                session_key,
                tool_name,
                input,
            })
        }

        "exec.approval.resolved" => {
            let approval_id = payload
                .get("approvalId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let approved = payload
                .get("approved")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            Some(UiEvent::ApprovalResolved {
                approval_id,
                session_key,
                approved,
            })
        }

        "agent" => normalize_agent_event(payload),

        "web.login.whatsapp" | "web.login.telegram" => {
            let provider = if event.contains("whatsapp") {
                "whatsapp"
            } else {
                "telegram"
            };
            let qr_code = payload
                .get("qr")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();

            Some(UiEvent::WebLogin {
                provider: provider.to_string(),
                qr_code,
                status,
            })
        }

        "tool.start" | "tool.end" | "tool.output" | "tool.error" => {
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let run_id = payload
                .get("runId")
                .and_then(Value::as_str)
                .map(|s| s.to_string());

            let status = match event.as_str() {
                "tool.start" => "started",
                "tool.end" | "tool.output" => "ok",
                "tool.error" => "error",
                _ => "running",
            };

            let tool_name = payload
                .get("tool")
                .and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else {
                        v.get("name").and_then(Value::as_str).map(|s| s.to_string())
                    }
                })
                .unwrap_or("unknown".to_string());

            let input = payload.get("input").cloned().unwrap_or(Value::Null);
            let output = payload.get("output").cloned().unwrap_or(Value::Null);

            info!("[normalizer] Top-level Tool event: {} -> {}", event, status);

            Some(UiEvent::ToolUpdate {
                session_key,
                run_id,
                tool_name,
                status: status.to_string(),
                input,
                output,
            })
        }

        _ => {
            // Log unhandled events as warnings to help debugging
            if !event.starts_with("sys.") {
                tracing::warn!(
                    "[normalizer] Dropped Unhandled Event: {} | payload keys: {:?}",
                    event,
                    payload.as_object().map(|o| o.keys().collect::<Vec<_>>())
                );
            }
            None
        }
    }
}

/// Normalize agent-specific events (tools, lifecycle, status)
fn normalize_agent_event(payload: &Value) -> Option<UiEvent> {
    let session_key = payload
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let run_id = payload
        .get("runId")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let stream = payload.get("stream").and_then(Value::as_str)?;
    let data = payload.get("data");

    // Log what stream values we're receiving
    info!(
        "[normalizer] agent event stream={:?} data_keys={:?}",
        stream,
        data.and_then(|d| d.as_object())
            .map(|o| o.keys().collect::<Vec<_>>())
    );

    match stream {
        "tool" => {
            let tool_name = data
                .and_then(|d| d.get("tool"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let phase = data
                .and_then(|d| d.get("phase"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            // Map phase to status
            let status = match phase {
                "start" => "started",
                "output" => "ok",
                "error" => "error",
                _ => "stream",
            };

            info!(
                "[normalizer] ToolUpdate: {} | phase={} status={}",
                tool_name, phase, status
            );

            return Some(UiEvent::ToolUpdate {
                session_key,
                run_id,
                tool_name,
                status: status.to_string(),
                input: data
                    .and_then(|d| d.get("input").or_else(|| d.get("arguments")))
                    .cloned()
                    .unwrap_or(Value::Null),
                output: data
                    .and_then(|d| d.get("output"))
                    .cloned()
                    .unwrap_or(Value::Null),
            });
        }
        "lifecycle" => {
            let phase = data
                .and_then(|d| d.get("phase")) // start, end, error
                .and_then(Value::as_str)
                .unwrap_or("");

            let status = match phase {
                "start" => "started",
                "end" => "ok",
                "error" => "error",
                _ => return None,
            };

            let error = data
                .and_then(|d| d.get("error"))
                .and_then(Value::as_str)
                .map(|s| s.to_string());

            info!(
                "[normalizer] Creating RunStatus: session={} status={} phase={}",
                session_key, status, phase
            );

            return Some(UiEvent::RunStatus {
                session_key,
                run_id,
                status: status.to_string(),
                error,
            });
        }
        "assistant" => {
            // Map token-by-token agent streams to Assistant snapshots
            let text = data
                .and_then(|d| d.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");

            return Some(UiEvent::AssistantSnapshot {
                session_key,
                run_id: run_id.clone(),
                message_id: run_id.unwrap_or_else(|| "assistant".to_string()),
                text: text.to_string(),
            });
        }
        "compaction" => {
            // Compact logic is internal to Moltbot
            None
        }
        other => {
            // Log unhandled stream types to discover what we're missing
            info!(
                "[normalizer] UNHANDLED agent stream type: {} | data={:?}",
                other,
                data.map(|d| d.to_string().chars().take(200).collect::<String>())
            );
            None
        }
    }
}

/// Normalize chat-specific events with fallback heuristics
fn normalize_chat_event(payload: &Value) -> Option<UiEvent> {
    let session_key = payload
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let run_id = payload
        .get("runId")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    // Check for 'state' field (Moltbot v2/v3 chat protocol)
    if let Some(state) = payload.get("state").and_then(Value::as_str) {
        match state {
            "delta" => {
                // Extract text from message.content[0].text
                let width_accumulated_text = payload
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                // Handle silent/internal replies
                if width_accumulated_text.contains("NO_REPL") {
                    let clean_text = width_accumulated_text
                        .replace("NO_REPL", "")
                        .trim()
                        .to_string();
                    if !clean_text.is_empty() {
                        return Some(UiEvent::AssistantInternal {
                            session_key: session_key.clone(),
                            run_id: run_id.clone(),
                            message_id: run_id.clone().unwrap_or_else(|| "internal".to_string()),
                            text: clean_text,
                        });
                    }
                    return None;
                }

                return Some(UiEvent::AssistantSnapshot {
                    session_key: session_key.clone(),
                    run_id: run_id.clone(),
                    message_id: run_id.clone().unwrap_or_else(|| "assistant".to_string()),
                    text: width_accumulated_text.to_string(),
                });
            }
            "final" => {
                let text = payload
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                // Filter silent replies
                if text.contains("NO_REPL") {
                    return None;
                }

                return Some(UiEvent::AssistantFinal {
                    session_key: session_key.clone(),
                    run_id: run_id.clone(),
                    message_id: run_id.clone().unwrap_or_else(|| "assistant".to_string()),
                    text: text.to_string(),
                    usage: None,
                });
            }
            "error" => {
                let error_msg = payload
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown error");

                return Some(UiEvent::RunStatus {
                    session_key: session_key.clone(),
                    run_id: run_id.clone(),
                    status: "error".into(),
                    error: Some(error_msg.to_string()),
                });
            }
            _ => {}
        }
    }

    // If upstream provides a kind/type field, use it
    if let Some(kind) = payload.get("kind").and_then(Value::as_str) {
        match kind {
            "assistant.delta" => {
                let delta = payload.get("delta").and_then(Value::as_str).unwrap_or("");

                // Filter silent tokens from stream
                if delta.contains("NO_REPL") {
                    return None;
                }

                let msg_id = payload
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .or_else(|| run_id.clone())
                    .unwrap_or_else(|| "assistant".to_string());

                return Some(UiEvent::AssistantDelta {
                    session_key,
                    run_id,
                    message_id: msg_id,
                    delta: delta.to_string(),
                });
            }
            "assistant.final" => {
                let text = payload.get("text").and_then(Value::as_str).unwrap_or("");

                if text.contains("NO_REPL") {
                    return None;
                }

                let msg_id = payload
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .or_else(|| run_id.clone())
                    .unwrap_or_else(|| "assistant".to_string());

                return Some(UiEvent::AssistantFinal {
                    session_key,
                    run_id,
                    message_id: msg_id,
                    text: text.to_string(),
                    usage: payload.get("usage").and_then(|u| {
                        Some(UiUsage {
                            input_tokens: u.get("inputTokens")?.as_u64()?,
                            output_tokens: u.get("outputTokens")?.as_u64()?,
                            total_tokens: u.get("totalTokens")?.as_u64()?,
                        })
                    }),
                });
            }
            "tool" => {
                let tool = payload.get("tool").cloned().unwrap_or(Value::Null);
                return Some(UiEvent::ToolUpdate {
                    session_key,
                    run_id,
                    tool_name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string(),
                    status: tool
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    input: tool
                        .get("input")
                        .or_else(|| tool.get("arguments"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    output: tool.get("output").cloned().unwrap_or(Value::Null),
                });
            }
            "run.status" => {
                return Some(UiEvent::RunStatus {
                    session_key,
                    run_id,
                    status: payload
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    error: payload
                        .get("error")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string()),
                });
            }
            _ => {}
        }
    }

    // Fallback heuristics for older protocol versions
    // 1) If payload looks like { delta: "..." } assume assistant delta
    if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
        if delta.contains("NO_REPL") {
            return None;
        }

        let msg_id = payload
            .get("messageId")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| run_id.clone())
            .unwrap_or_else(|| "assistant".to_string());

        return Some(UiEvent::AssistantDelta {
            session_key,
            run_id,
            message_id: msg_id,
            delta: delta.to_string(),
        });
    }

    // 2) If payload has full text field
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        if text.contains("NO_REPL") {
            return None;
        }

        let msg_id = payload
            .get("messageId")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| run_id.clone())
            .unwrap_or_else(|| "assistant".to_string());

        return Some(UiEvent::AssistantFinal {
            session_key,
            run_id,
            message_id: msg_id,
            text: text.to_string(),
            usage: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_assistant_delta() {
        let frame = WsFrame::Event {
            event: "chat".into(),
            payload: serde_json::json!({
                "kind": "assistant.delta",
                "sessionKey": "main",
                "messageId": "msg-1",
                "delta": "Hello"
            }),
            seq: None,
            state_version: None,
        };

        let event = normalize_event(&frame).unwrap();
        match event {
            UiEvent::AssistantDelta { delta, .. } => {
                assert_eq!(delta, "Hello");
            }
            _ => panic!("Expected AssistantDelta"),
        }
    }

    #[test]
    fn test_normalize_fallback_delta() {
        // Old format without kind field
        let frame = WsFrame::Event {
            event: "chat".into(),
            payload: serde_json::json!({
                "sessionKey": "main",
                "delta": "World"
            }),
            seq: None,
            state_version: None,
        };

        let event = normalize_event(&frame).unwrap();
        match event {
            UiEvent::AssistantDelta { delta, .. } => {
                assert_eq!(delta, "World");
            }
            _ => panic!("Expected AssistantDelta from fallback"),
        }
    }
}

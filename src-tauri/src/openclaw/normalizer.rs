//! Event normalizer - converts upstream OpenClawEngine events to stable UI contract
//!
//! This layer insulates the UI from protocol drift by parsing events
//! defensively and mapping them to a stable internal representation.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use tracing::info;

use super::frames::WsFrame;

// ---------------------------------------------------------------------------
// LLM token sanitizer — strips leaked ChatML / Jinja template tokens
// ---------------------------------------------------------------------------

/// Compiled regexes for stripping LLM control tokens from output text.
/// These patterns catch ChatML (Qwen, Mistral, etc.), Llama, and common
/// template artifacts that local models sometimes emit.
static LLM_TOKEN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // ChatML block markers: <|im_start|> optionally followed by a role word (assistant, user, etc.)
        Regex::new(r"<\|im_start\|>\w*").unwrap(),
        Regex::new(r"<\|im_end\|>").unwrap(),
        // Generic special tokens
        Regex::new(r"<\|end\|>").unwrap(),
        Regex::new(r"<\|endoftext\|>").unwrap(),
        Regex::new(r"<\|eot_id\|>").unwrap(),
        // Llama header blocks: <|start_header_id|>role<|end_header_id|> as a single unit
        Regex::new(r"<\|start_header_id\|>\w*<\|end_header_id\|>").unwrap(),
        // Fallback: catch orphaned header tokens that appear without the other half
        Regex::new(r"<\|start_header_id\|>").unwrap(),
        Regex::new(r"<\|end_header_id\|>").unwrap(),
        // Thinking blocks: <think>...</think>
        Regex::new(r"(?s)<think>.*?</think>").unwrap(),
        // Bare role markers that sometimes leak mid-text
        Regex::new(r"(?m)^(user|assistant|system|tool)>\s*$").unwrap(),
    ]
});

/// Strip leaked LLM template tokens from text before it reaches the UI.
/// This is applied in the normalizer so ALL consumers (chat, fleet, live status)
/// receive clean text.
fn strip_llm_tokens(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in LLM_TOKEN_PATTERNS.iter() {
        result = pattern.replace_all(&result, "").to_string();
    }
    // Collapse runs of 3+ newlines into 2
    let collapse = Regex::new(r"\n{3,}").unwrap();
    result = collapse.replace_all(&result, "\n\n").to_string();
    result.trim().to_string()
}

/// Stable UI event contract - what the OpenClaw chat UI consumes
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

    /// Canvas update
    CanvasUpdate {
        session_key: String,
        run_id: Option<String>,
        content: String,
        content_type: String, // "html" | "json"
        url: Option<String>,
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

        "canvas" => {
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let run_id = payload
                .get("runId")
                .and_then(Value::as_str)
                .map(|s| s.to_string());

            let content = payload
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let content_type = payload
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("html")
                .to_string();
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .map(|s| s.to_string());

            Some(UiEvent::CanvasUpdate {
                session_key,
                run_id,
                content,
                content_type,
                url,
            })
        }

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
                text: strip_llm_tokens(text),
            });
        }
        "compaction" => {
            // Compact logic is internal to OpenClawEngine
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

    // Check for 'state' field (OpenClawEngine v2/v3 chat protocol)
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
                            text: strip_llm_tokens(&clean_text),
                        });
                    }
                    return None;
                }

                return Some(UiEvent::AssistantSnapshot {
                    session_key: session_key.clone(),
                    run_id: run_id.clone(),
                    message_id: run_id.clone().unwrap_or_else(|| "assistant".to_string()),
                    text: strip_llm_tokens(width_accumulated_text),
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
                    text: strip_llm_tokens(text),
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
                    delta: strip_llm_tokens(delta),
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
                    text: strip_llm_tokens(text),
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
            delta: strip_llm_tokens(delta),
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
            text: strip_llm_tokens(text),
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

    #[test]
    fn test_strip_chatml_tokens() {
        // <|im_start|>assistant\n gets stripped (including "assistant")
        // <|im_end|> gets stripped
        // "user>" inline is NOT stripped (only stripped when on its own line)
        let input = "Hello<|im_end|>\n<|im_start|>assistant\nI'm fine<|im_end|>";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Hello\n\nI'm fine");
    }

    #[test]
    fn test_strip_thinking_blocks() {
        let input =
            "Let me help. <think>I should check the weather first...</think>Here's the plan:";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Let me help. Here's the plan:");
    }

    #[test]
    fn test_strip_llama_tokens() {
        let input = "Hello<|eot_id|><|start_header_id|>assistant<|end_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_orphaned_header_tokens() {
        // Orphaned tokens without the matching pair should still be stripped
        let input = "Hello<|start_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_preserves_normal_text() {
        let input = "This is a normal response with **markdown** and `code`.";
        let result = strip_llm_tokens(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_collapses_newlines() {
        let input = "Part 1\n\n\n\n\nPart 2";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Part 1\n\nPart 2");
    }
}

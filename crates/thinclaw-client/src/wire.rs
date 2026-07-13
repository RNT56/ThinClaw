//! Wire DTOs mirroring the gateway's `web::types`.
//!
//! These are hand-written (not a dependency on `thinclaw-gateway`) so the SDK
//! stays a light, standalone crate. A contract test
//! (`tests/wire_contract.rs`) guards against drift from the real server types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /api/chat/send`.
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
}

/// Response from `POST /api/chat/send`. The send is async-accepted; the actual
/// assistant content arrives later over the SSE stream, correlated by
/// `thread_id`.
#[derive(Debug, Clone, Deserialize)]
pub struct SendMessageResponse {
    pub message_id: Uuid,
    pub status: String,
}

/// A conversation thread summary.
#[derive(Debug, Clone, Deserialize)]
pub struct ThreadInfo {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thread_type: Option<String>,
}

/// Response from `GET /api/chat/threads`.
#[derive(Debug, Clone, Deserialize)]
pub struct ThreadListResponse {
    #[serde(default)]
    pub assistant_thread: Option<ThreadInfo>,
    #[serde(default)]
    pub threads: Vec<ThreadInfo>,
    #[serde(default)]
    pub active_thread: Option<Uuid>,
}

/// A recorded tool call within a turn (transcript projection).
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

/// A single conversation turn.
#[derive(Debug, Clone, Deserialize)]
pub struct TurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    #[serde(default)]
    pub hide_user_input: bool,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallInfo>,
}

/// Response from `GET /api/chat/history`.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryResponse {
    pub thread_id: Uuid,
    #[serde(default)]
    pub turns: Vec<TurnInfo>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub oldest_timestamp: Option<String>,
}

/// Request body for `POST /api/chat/approval`.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    /// "approve", "always", or "deny".
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
}

/// The action to take on a pending approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    /// Approve this one invocation.
    Approve,
    /// Approve and remember for the session.
    Always,
    /// Deny.
    Deny,
}

impl ApprovalAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Always => "always",
            Self::Deny => "deny",
        }
    }
}

/// A server-sent event streamed from `GET /api/chat/events`.
///
/// This is a **curated subset** of the gateway's `SseEvent` — the variants a
/// client typically consumes — plus an [`SseEvent::Unknown`] fallback so a
/// server that adds new event types never breaks an older client. The contract
/// test asserts these stay compatible with the real server type.
#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    /// A final assistant response for a thread.
    Response { content: String, thread_id: String },
    /// A "thinking" status line.
    Thinking {
        message: String,
        thread_id: Option<String>,
    },
    /// Extended reasoning / chain-of-thought text.
    ReasoningContent {
        content: String,
        thread_id: Option<String>,
    },
    /// A tool started executing.
    ToolStarted {
        name: String,
        thread_id: Option<String>,
    },
    /// A tool finished executing.
    ToolCompleted {
        name: String,
        success: bool,
        thread_id: Option<String>,
    },
    /// A tool produced a result preview.
    ToolResult {
        name: String,
        preview: String,
        thread_id: Option<String>,
    },
    /// A streamed token chunk.
    StreamChunk {
        content: String,
        thread_id: Option<String>,
    },
    /// A generic status line.
    Status {
        message: String,
        thread_id: Option<String>,
    },
    /// A structured context-window capacity transition.
    ContextPressure {
        level: String,
        usage_percent: Option<f64>,
        thread_id: Option<String>,
    },
    /// A structured internal agent lifecycle transition.
    AgentLifecycle {
        phase: String,
        label: String,
        detail: Option<String>,
        thread_id: Option<String>,
    },
    /// A token/cost usage update.
    UsageUpdate {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
        model: Option<String>,
        thread_id: Option<String>,
    },
    /// A tool invocation needs operator approval.
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        thread_id: Option<String>,
    },
    /// An error status.
    Error {
        message: String,
        thread_id: Option<String>,
    },
    /// A keep-alive heartbeat.
    Heartbeat,
    /// Any event type this client version does not model. The full JSON payload
    /// is preserved so callers can inspect it if needed.
    Unknown {
        event_type: String,
        raw: serde_json::Value,
    },
}

impl SseEvent {
    /// The `thread_id` this event is scoped to, if any. Used to correlate a
    /// streamed response back to the message that triggered it.
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::Response { thread_id, .. } => Some(thread_id.as_str()),
            Self::Thinking { thread_id, .. }
            | Self::ReasoningContent { thread_id, .. }
            | Self::ToolStarted { thread_id, .. }
            | Self::ToolCompleted { thread_id, .. }
            | Self::ToolResult { thread_id, .. }
            | Self::StreamChunk { thread_id, .. }
            | Self::Status { thread_id, .. }
            | Self::ContextPressure { thread_id, .. }
            | Self::AgentLifecycle { thread_id, .. }
            | Self::UsageUpdate { thread_id, .. }
            | Self::ApprovalNeeded { thread_id, .. }
            | Self::Error { thread_id, .. } => thread_id.as_deref(),
            Self::Heartbeat | Self::Unknown { .. } => None,
        }
    }

    /// Parse an event from its decoded JSON object. Unknown `type` values (or a
    /// missing `type`) become [`SseEvent::Unknown`] rather than an error.
    pub fn from_json(value: serde_json::Value) -> Self {
        let event_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let s = |k: &str| value.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let opt = |k: &str| value.get(k).and_then(|v| v.as_str()).map(str::to_string);

        match event_type.as_str() {
            "response" => Self::Response {
                content: s("content").unwrap_or_default(),
                thread_id: s("thread_id").unwrap_or_default(),
            },
            "thinking" => Self::Thinking {
                message: s("message").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "reasoning_content" => Self::ReasoningContent {
                content: s("content").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "tool_started" => Self::ToolStarted {
                name: s("name").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "tool_completed" => Self::ToolCompleted {
                name: s("name").unwrap_or_default(),
                success: value
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                thread_id: opt("thread_id"),
            },
            "tool_result" => Self::ToolResult {
                name: s("name").unwrap_or_default(),
                preview: s("preview").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "stream_chunk" => Self::StreamChunk {
                content: s("content").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "status" => Self::Status {
                message: s("message").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "context_pressure" => Self::ContextPressure {
                level: s("level").unwrap_or_else(|| "none".to_string()),
                usage_percent: value.get("usage_percent").and_then(|value| value.as_f64()),
                thread_id: opt("thread_id"),
            },
            "agent_lifecycle" => Self::AgentLifecycle {
                phase: s("phase").unwrap_or_default(),
                label: s("label").unwrap_or_default(),
                detail: opt("detail"),
                thread_id: opt("thread_id"),
            },
            "usage_update" => Self::UsageUpdate {
                input_tokens: value.get("input_tokens").and_then(|v| v.as_u64()),
                output_tokens: value.get("output_tokens").and_then(|v| v.as_u64()),
                cost_usd: value.get("cost_usd").and_then(|v| v.as_f64()),
                model: opt("model"),
                thread_id: opt("thread_id"),
            },
            "approval_needed" => Self::ApprovalNeeded {
                request_id: s("request_id").unwrap_or_default(),
                tool_name: s("tool_name").unwrap_or_default(),
                description: s("description").unwrap_or_default(),
                thread_id: opt("thread_id"),
            },
            "heartbeat" => Self::Heartbeat,
            _ => Self::Unknown {
                event_type,
                raw: value,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_event() {
        let v = serde_json::json!({
            "type": "response",
            "content": "hi there",
            "thread_id": "t-1"
        });
        assert_eq!(
            SseEvent::from_json(v),
            SseEvent::Response {
                content: "hi there".into(),
                thread_id: "t-1".into()
            }
        );
    }

    #[test]
    fn unknown_event_preserves_payload() {
        let v = serde_json::json!({ "type": "future_thing", "x": 1 });
        match SseEvent::from_json(v) {
            SseEvent::Unknown { event_type, raw } => {
                assert_eq!(event_type, "future_thing");
                assert_eq!(raw.get("x").and_then(|n| n.as_i64()), Some(1));
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn parses_structured_agent_lifecycle_event() {
        let event = SseEvent::from_json(serde_json::json!({
            "type": "agent_lifecycle",
            "phase": "advisor_consultation",
            "label": "Consulting the advisor lane",
            "detail": "confidence below threshold",
            "thread_id": "t-2"
        }));

        assert_eq!(
            event,
            SseEvent::AgentLifecycle {
                phase: "advisor_consultation".into(),
                label: "Consulting the advisor lane".into(),
                detail: Some("confidence below threshold".into()),
                thread_id: Some("t-2".into()),
            }
        );
        assert_eq!(event.thread_id(), Some("t-2"));
    }

    #[test]
    fn parses_structured_context_pressure_event() {
        let event = SseEvent::from_json(serde_json::json!({
            "type": "context_pressure",
            "level": "warning",
            "usage_percent": 88.5,
            "thread_id": "t-3"
        }));

        assert_eq!(
            event,
            SseEvent::ContextPressure {
                level: "warning".into(),
                usage_percent: Some(88.5),
                thread_id: Some("t-3".into()),
            }
        );
        assert_eq!(event.thread_id(), Some("t-3"));
    }

    #[test]
    fn thread_id_correlation() {
        let ev = SseEvent::ToolStarted {
            name: "shell".into(),
            thread_id: Some("t-2".into()),
        };
        assert_eq!(ev.thread_id(), Some("t-2"));
        assert_eq!(SseEvent::Heartbeat.thread_id(), None);
    }

    #[test]
    fn approval_action_strings() {
        assert_eq!(ApprovalAction::Approve.as_str(), "approve");
        assert_eq!(ApprovalAction::Always.as_str(), "always");
        assert_eq!(ApprovalAction::Deny.as_str(), "deny");
    }
}

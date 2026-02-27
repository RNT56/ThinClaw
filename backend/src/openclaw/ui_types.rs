//! Stable UI event types — the frontend event contract for OpenClaw
//!
//! These types define the shape of every `"openclaw-event"` emission.
//! The frontend's `OpenClawChatView.tsx` pattern-matches on `kind` to
//! decide how to render each event.
//!
//! After IronClaw integration, these are emitted by `TauriChannel`
//! instead of the old WS normalizer. The types themselves don't change.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable UI event contract — what the OpenClaw chat UI consumes.
///
/// Tagged with `#[serde(tag = "kind")]` so JSON looks like:
/// `{ "kind": "AssistantDelta", "session_key": "...", "delta": "..." }`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UiEvent {
    /// Successfully connected to engine
    Connected { protocol: u32 },

    /// Disconnected from engine
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

    /// Streaming assistant delta (append to current text)
    AssistantDelta {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        delta: String,
    },

    /// Internal assistant thinking (renders 🧠 indicator)
    AssistantInternal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },

    /// Streaming assistant snapshot (replace current text)
    AssistantSnapshot {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
    },

    /// Final assistant message (replace, includes usage stats)
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

    /// Engine error
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

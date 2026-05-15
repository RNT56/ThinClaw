//! Stable UI event types — the frontend event contract for ThinClaw
//!
//! These types define the shape of every `"thinclaw-event"` emission.
//! The frontend's `ThinClawChatView.tsx` pattern-matches on `kind` to
//! decide how to render each event.
//!
//! After IronClaw integration, these are emitted by `TauriChannel`
//! instead of the old WS normalizer. The types themselves don't change.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable UI event contract — what the ThinClaw chat UI consumes.
///
/// Tagged with `#[serde(tag = "kind")]` so JSON looks like:
/// `{ "kind": "AssistantDelta", "session_key": "...", "delta": "..." }`
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

    /// Explicit run lifecycle transition.
    LifecycleUpdate {
        session_key: String,
        run_id: String,
        phase: String, // start|end
        status: String,
    },

    /// Structured plan/progress update from the ThinClaw agent loop.
    PlanUpdate {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        entries: Vec<Value>,
    },

    /// Token and cost usage for the most recent model turn.
    UsageUpdate {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        usage: UiUsage,
        cost_usd: Option<f64>,
        model: Option<String>,
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
        session_key: Option<String>,
        run_id: Option<String>,
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

    /// Sub-agent progress update (relayed to parent session's UI).
    ///
    /// Emitted when a child session spawned by `thinclaw_spawn_session`
    /// changes state. The frontend can use this to show a progress panel
    /// in the parent session's chat view.
    SubAgentUpdate {
        parent_session: String,
        child_session: String,
        task: String,
        status: String,        // "running" | "completed" | "failed"
        progress: Option<f32>, // 0.0–1.0
        result_preview: Option<String>,
    },

    /// Sandbox/job lifecycle update.
    JobUpdate {
        session_key: Option<String>,
        run_id: Option<String>,
        job_id: String,
        title: Option<String>,
        status: String,
        url: Option<String>,
        payload: Value,
    },

    /// Mid-loop agent message — rendered as a persistent chat bubble.
    ///
    /// Emitted by the `emit_user_message` tool. The agent is still working
    /// (the agentic loop continues), so the processing indicator should
    /// stay active. These are NOT ephemeral status text.
    AgentMessage {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        content: String,
        message_type: String, // "progress" | "warning" | "question" | "interim_result"
    },

    /// Factory reset completed — frontend must clear all cached state
    FactoryReset,

    /// Routine lifecycle event — fired when a routine starts, completes, or fails.
    /// The frontend Automations panel and Console can display these as live status.
    RoutineLifecycle {
        routine_name: String,
        event: String, // "started" | "completed" | "failed"
        run_id: Option<String>,
        result_summary: Option<String>,
    },

    /// Cost/budget event from the gateway/runtime.
    CostAlert {
        alert_type: String,
        current_cost_usd: f64,
        limit_usd: f64,
        message: Option<String>,
    },

    /// Typed catch-all for ThinClaw gateway events that do not yet have a
    /// dedicated desktop rendering surface. Keeping these on `thinclaw-event`
    /// prevents silent drops while frontend surfaces catch up.
    GatewayEvent {
        event_type: String,
        session_key: Option<String>,
        run_id: Option<String>,
        payload: Value,
    },

    /// Real-time log entry push from the internal tracing subscriber.
    /// Sent for every DEBUG+ event so the UI Logs tab updates live without polling.
    LogEntry {
        timestamp: String,
        level: String, // "DEBUG" | "INFO" | "WARN" | "ERROR"
        target: String,
        message: String,
    },

    /// Bootstrap ritual completed — BOOTSTRAP.md was deleted by the agent.
    /// Frontend should update bootstrapNeeded → false and hide the boot button.
    BootstrapCompleted,

    /// Agent created a file via write_file tool.
    /// Frontend should show a clickable Finder-reveal link in the chat.
    FileCreated {
        /// Absolute path of the created file on disk.
        path: String,
        /// Relative path from workspace root (user-friendly display).
        relative_path: String,
        /// Size in bytes.
        #[specta(type = f64)]
        bytes: u64,
    },
}

/// Session metadata for session list
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UiSession {
    pub session_key: String,
    pub title: Option<String>,
    #[specta(type = Option<f64>)]
    pub updated_at_ms: Option<u64>,
    pub source: Option<String>, // slack|telegram|webchat|...
}

/// Message in chat history
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UiMessage {
    pub id: String,
    pub role: String, // user|assistant|tool|system
    #[specta(type = f64)]
    pub ts_ms: u64,
    pub text: String,
    pub source: Option<String>,
}

/// Token usage stats
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UiUsage {
    #[specta(type = f64)]
    pub input_tokens: u64,
    #[specta(type = f64)]
    pub output_tokens: u64,
    #[specta(type = f64)]
    pub total_tokens: u64,
}

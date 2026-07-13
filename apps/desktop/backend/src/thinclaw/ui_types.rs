//! Stable UI event types — the frontend event contract for ThinClaw
//!
//! These types define the shape of every `"thinclaw-event"` emission.
//! The frontend's `ThinClawChatView.tsx` pattern-matches on `kind` to
//! decide how to render each event.
//!
//! After ThinClaw integration, these are emitted by `TauriChannel`
//! instead of the old WS normalizer. The types themselves don't change.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Tool-execution status carried by [`UiEvent::ToolUpdate`].
///
/// The wire strings are preserved exactly (`started` | `stream` | `ok` |
/// `error`) so existing frontend runtime checks keep working, while the
/// exported TypeScript type becomes an exhaustive string-literal union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Tool call has started executing.
    Started,
    /// Intermediate streamed tool result.
    Stream,
    /// Tool completed successfully.
    Ok,
    /// Tool failed.
    Error,
}

/// Run lifecycle status carried by [`UiEvent::RunStatus`].
///
/// Historically a free-form string (`StatusUpdate::Status(text)` and the
/// gateway `status` message forward arbitrary human-readable text). The known
/// terminal/active states are modelled as named variants with their exact wire
/// strings preserved; any other value round-trips losslessly through
/// [`RunStatus::Other`]. The exported TypeScript type is therefore a union of
/// the known literals plus `string`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(untagged, rename_all = "snake_case")]
pub enum RunStatus {
    /// One of the recognised run states.
    Known(RunStatusKnown),
    /// Any other free-form status string (preserved verbatim).
    Other(String),
}

/// The recognised, closed set of [`RunStatus`] values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusKnown {
    Started,
    InFlight,
    Ok,
    Error,
    Aborted,
    Done,
    Interrupted,
    Rejected,
}

impl RunStatus {
    /// Build from a free-form wire string, mapping recognised values to a
    /// [`RunStatusKnown`] variant and preserving anything else verbatim.
    pub fn from_wire(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "started" => Self::Known(RunStatusKnown::Started),
            "in_flight" => Self::Known(RunStatusKnown::InFlight),
            "ok" => Self::Known(RunStatusKnown::Ok),
            "error" => Self::Known(RunStatusKnown::Error),
            "aborted" => Self::Known(RunStatusKnown::Aborted),
            "done" => Self::Known(RunStatusKnown::Done),
            "interrupted" => Self::Known(RunStatusKnown::Interrupted),
            "rejected" => Self::Known(RunStatusKnown::Rejected),
            _ => Self::Other(value),
        }
    }
}

/// Sub-agent progress status carried by [`UiEvent::SubAgentUpdate`].
///
/// Known lifecycle states use their exact wire strings; the running-with-
/// category form (`running:<category>`) and any operator-supplied status from
/// the `thinclaw_update_sub_agent_status` RPC round-trip losslessly through
/// [`SubAgentStatus::Other`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(untagged, rename_all = "snake_case")]
pub enum SubAgentStatus {
    /// One of the recognised sub-agent states.
    Known(SubAgentStatusKnown),
    /// Any other status string, e.g. `running:thinking` (preserved verbatim).
    Other(String),
}

/// The recognised, closed set of [`SubAgentStatus`] values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentStatusKnown {
    Running,
    Completed,
    Failed,
}

impl SubAgentStatus {
    /// Build from a free-form wire string, mapping recognised values to a
    /// [`SubAgentStatusKnown`] variant and preserving anything else (such as
    /// the `running:<category>` form) verbatim.
    pub fn from_wire(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "running" => Self::Known(SubAgentStatusKnown::Running),
            "completed" => Self::Known(SubAgentStatusKnown::Completed),
            "failed" => Self::Known(SubAgentStatusKnown::Failed),
            _ => Self::Other(value),
        }
    }
}

/// Mid-loop agent message classification carried by [`UiEvent::AgentMessage`].
///
/// Matches the `emit_user_message` tool schema (`progress` | `interim_result`
/// | `question` | `warning`). The tool default is `progress`, but the value is
/// not strictly clamped upstream, so any other classification round-trips
/// through [`MessageType::Other`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(untagged, rename_all = "snake_case")]
pub enum MessageType {
    /// One of the recognised message classifications.
    Known(MessageTypeKnown),
    /// Any other classification string (preserved verbatim).
    Other(String),
}

/// The recognised, closed set of [`MessageType`] values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MessageTypeKnown {
    Progress,
    InterimResult,
    Question,
    Warning,
}

impl MessageType {
    /// Build from a free-form wire string, mapping recognised values to a
    /// [`MessageTypeKnown`] variant and preserving anything else verbatim.
    pub fn from_wire(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "progress" => Self::Known(MessageTypeKnown::Progress),
            "interim_result" => Self::Known(MessageTypeKnown::InterimResult),
            "question" => Self::Known(MessageTypeKnown::Question),
            "warning" => Self::Known(MessageTypeKnown::Warning),
            _ => Self::Other(value),
        }
    }
}

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
        /// started | stream | ok | error
        status: ToolStatus,
        input: Value,
        output: Value,
    },

    /// Run status change
    RunStatus {
        session_key: String,
        run_id: Option<String>,
        /// started | in_flight | ok | error | aborted | done | … | free-form
        status: RunStatus,
        error: Option<String>,
    },

    /// Explicit run lifecycle transition.
    LifecycleUpdate {
        session_key: String,
        run_id: String,
        phase: String, // start|end
        status: String,
    },

    /// Agent lifecycle activity (context compaction, advisor consultation, …)
    /// surfaced as a transient, human-readable status for the Event Inspector.
    /// Distinct from `LifecycleUpdate` (run start/end) — these are mid-run
    /// internal phases the agent passes through.
    AgentLifecycleEvent {
        session_key: String,
        run_id: Option<String>,
        /// Machine phase key, e.g. "context_compaction" | "advisor_consultation".
        phase: String,
        /// Human-readable label, e.g. "Compacting context and retrying".
        label: String,
        /// Optional extra detail (token counts, trigger reason, …).
        detail: Option<String>,
    },

    /// Metadata-only core observer event/metric forwarded by Desktop.
    ObserverRecord { record: UiObserverRecord },

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

    /// Agent requests a credential; the UI shows an inline masked-input card.
    /// Carries NO secret value — the typed value is submitted out-of-band via
    /// `thinclaw_repo_projects_set_credential`, bypassing the engine and model.
    CredentialPrompt {
        prompt_id: String,
        session_key: String,
        run_id: Option<String>,
        secret_name: String,
        provider: String,
        reason: String,
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
        /// "running" | "completed" | "failed" | "running:<category>" | free-form
        status: SubAgentStatus,
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
        /// "progress" | "warning" | "question" | "interim_result" | free-form
        message_type: MessageType,
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

    /// Channel connectivity changed in either the local runtime or remote gateway.
    ChannelStatus {
        channel_id: String,
        state: String,
        error: Option<String>,
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

/// Privacy-safe observer metadata surfaced to the Desktop Event Inspector.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UiObserverRecord {
    /// `event` or `metric`.
    pub record_type: String,
    /// Stable machine name such as `llm_response` or `loop_run`.
    pub name: String,
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
    /// Redacted, bounded metadata. Prompt/message bodies are never included.
    pub attributes: BTreeMap<String, String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn json(value: &impl Serialize) -> String {
        serde_json::to_value(value).unwrap().to_string()
    }

    #[test]
    fn tool_status_preserves_wire_strings() {
        assert_eq!(json(&ToolStatus::Started), "\"started\"");
        assert_eq!(json(&ToolStatus::Stream), "\"stream\"");
        assert_eq!(json(&ToolStatus::Ok), "\"ok\"");
        assert_eq!(json(&ToolStatus::Error), "\"error\"");

        // Round-trip from the wire strings the frontend still relies on.
        let parsed: ToolStatus = serde_json::from_str("\"ok\"").unwrap();
        assert_eq!(parsed, ToolStatus::Ok);
    }

    #[test]
    fn run_status_maps_known_and_preserves_unknown() {
        assert_eq!(
            RunStatus::from_wire("in_flight"),
            RunStatus::Known(RunStatusKnown::InFlight)
        );
        assert_eq!(json(&RunStatus::from_wire("done")), "\"done\"");

        // Free-form status text (e.g. "Compacting context…") is preserved.
        let freeform = RunStatus::from_wire("Compacting context and retrying");
        assert_eq!(
            freeform,
            RunStatus::Other("Compacting context and retrying".to_string())
        );
        assert_eq!(json(&freeform), "\"Compacting context and retrying\"");

        // Untagged deserialization recovers the same value.
        let round: RunStatus = serde_json::from_str("\"done\"").unwrap();
        assert_eq!(round, RunStatus::Known(RunStatusKnown::Done));
        let round_other: RunStatus = serde_json::from_str("\"paused\"").unwrap();
        assert_eq!(round_other, RunStatus::Other("paused".to_string()));
    }

    #[test]
    fn sub_agent_status_preserves_running_category_form() {
        assert_eq!(
            SubAgentStatus::from_wire("completed"),
            SubAgentStatus::Known(SubAgentStatusKnown::Completed)
        );
        let category = SubAgentStatus::Other("running:thinking".to_string());
        assert_eq!(json(&category), "\"running:thinking\"");
        assert_eq!(
            SubAgentStatus::from_wire("running:thinking"),
            SubAgentStatus::Other("running:thinking".to_string())
        );
    }

    #[test]
    fn message_type_maps_known_and_preserves_unknown() {
        assert_eq!(
            MessageType::from_wire("interim_result"),
            MessageType::Known(MessageTypeKnown::InterimResult)
        );
        assert_eq!(json(&MessageType::from_wire("progress")), "\"progress\"");
        assert_eq!(
            MessageType::from_wire("custom"),
            MessageType::Other("custom".to_string())
        );
    }

    #[test]
    fn channel_status_uses_the_generated_discriminated_shape() {
        let event = UiEvent::ChannelStatus {
            channel_id: "telegram".into(),
            state: "degraded".into(),
            error: Some("webhook timeout".into()),
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "kind": "ChannelStatus",
                "channel_id": "telegram",
                "state": "degraded",
                "error": "webhook timeout"
            })
        );
    }
}

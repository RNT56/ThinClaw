//! Channel trait and message types.

use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use uuid::Uuid;

use crate::error::ChannelError;
use crate::media::MediaContent;

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Channel this message came from.
    pub channel: String,
    /// User identifier within the channel.
    pub user_id: String,
    /// Optional display name.
    pub user_name: Option<String>,
    /// Message content.
    pub content: String,
    /// Thread/conversation ID for threaded conversations.
    pub thread_id: Option<String>,
    /// When the message was received.
    pub received_at: DateTime<Utc>,
    /// Channel-specific metadata.
    pub metadata: serde_json::Value,
    /// Media attachments (images, PDFs, audio files, etc.).
    pub attachments: Vec<MediaContent>,
}

impl IncomingMessage {
    /// Create a new incoming message.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel: channel.into(),
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
            attachments: Vec::new(),
        }
    }

    /// Set the thread ID.
    pub fn with_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set user name.
    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Add media attachments.
    pub fn with_attachments(mut self, attachments: Vec<MediaContent>) -> Self {
        self.attachments = attachments;
        self
    }
}

/// Stream of incoming messages.
pub type MessageStream = Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>;

/// Response to send back to a channel.
#[derive(Debug, Clone)]
pub struct OutgoingResponse {
    /// The content to send.
    pub content: String,
    /// Optional thread ID to reply in.
    pub thread_id: Option<String>,
    /// Channel-specific metadata for the response.
    pub metadata: serde_json::Value,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the thread ID for the response.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }
}

/// Status update types for showing agent activity.
#[derive(Debug, Clone)]
pub enum StatusUpdate {
    /// Agent is thinking/processing.
    Thinking(String),
    /// Tool execution started.
    ToolStarted {
        name: String,
        /// Tool input parameters (optional — may be omitted for performance).
        parameters: Option<serde_json::Value>,
    },
    /// Tool execution completed.
    ToolCompleted {
        name: String,
        success: bool,
        /// Brief preview of the result (truncated to keep events small).
        result_preview: Option<String>,
    },
    /// Brief preview of tool execution output.
    ToolResult { name: String, preview: String },
    /// Streaming text chunk.
    StreamChunk(String),
    /// General status message.
    Status(String),
    /// A sandbox job has started (shown as a clickable card in the UI).
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    /// Tool requires user approval before execution.
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
    },
    /// Extension needs user authentication (token or OAuth).
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
        auth_url: Option<String>,
        setup_url: Option<String>,
    },
    /// Extension authentication completed.
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },
    /// Turn-level error surfaced to the UI (e.g., LLM unreachable, safety rejection).
    ///
    /// Emitted by the API layer when a spawned agent turn fails. Without this,
    /// turn errors are only logged and the UI shows an infinite spinner.
    Error {
        message: String,
        code: Option<String>,
    },
    /// Canvas / A2UI action — agent wants to show, update, or dismiss a UI panel.
    ///
    /// Emitted after the `canvas` tool executes successfully. The channel
    /// layer forwards this to the frontend (Tauri event, SSE, etc.) for
    /// immediate rendering, while the agent loop also persists the panel
    /// in the `CanvasStore` for HTTP access.
    CanvasAction(crate::tools::builtin::CanvasAction),
    /// Agent-initiated progress message sent via the `emit_user_message` tool.
    ///
    /// Unlike `Thinking` (ephemeral status), this is a persistent message the
    /// agent wants the user to see. Channels should render it as a real chat
    /// message or notification, not a transient indicator.
    AgentMessage {
        content: String,
        message_type: String,
    },

    /// Run lifecycle start — emitted immediately when a run is accepted,
    /// before any LLM call. Lets the frontend show a thinking indicator
    /// instantly, matching openclaw's `lifecycle: phase=start` event.
    LifecycleStart {
        /// Unique ID for this run (correlates Start ↔ End events).
        run_id: String,
    },

    /// Run lifecycle end — emitted after the final response is produced
    /// or when the run terminates (error or interrupt).
    LifecycleEnd {
        /// Unique ID matching the corresponding LifecycleStart.
        run_id: String,
        /// How the run ended: "response" | "interrupted" | "error".
        phase: String,
    },

    // ── Sub-agent lifecycle events ─────────────────────────────────────
    /// A sub-agent was spawned by the main agent.
    SubagentSpawned {
        /// Unique sub-agent ID.
        agent_id: String,
        /// Human-readable name (e.g., "researcher").
        name: String,
        /// Task description.
        task: String,
    },

    /// A running sub-agent reports progress (tool use, thinking, etc.).
    SubagentProgress {
        /// Sub-agent ID.
        agent_id: String,
        /// Progress message.
        message: String,
        /// Message category: "tool" | "thinking" | "question".
        category: String,
    },

    /// A sub-agent completed, failed, or was cancelled.
    SubagentCompleted {
        /// Sub-agent ID.
        agent_id: String,
        /// Sub-agent name.
        name: String,
        /// Whether it succeeded.
        success: bool,
        /// The sub-agent's final response / findings.
        response: String,
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Number of tool iterations used.
        iterations: usize,
    },
}

// ── Streaming draft replies ───────────────────────────────────────────

/// Per-channel streaming mode for partial reply rendering.
///
/// Configurable via `CHANNEL_STREAM_MODE` env var or per-channel config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamMode {
    /// No streaming — wait for the full response, then send once.
    #[default]
    None,
    /// Send-then-edit: post an initial message, then edit it as chunks arrive.
    /// The first version is "\u2726 typing..." and updates accumulate.
    EditFirst,
    /// Status line: send a single updating status line that shows the
    /// current assistant state (like a progress bar), then a final message.
    StatusLine,
}

impl StreamMode {
    /// Parse from a string value (env var or config).
    pub fn from_str_value(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "edit" | "edit_first" | "editfirst" | "fulledit" | "full_edit" => Self::EditFirst,
            "status" | "status_line" | "statusline" => Self::StatusLine,
            _ => Self::None,
        }
    }
}

/// Minimum interval between draft edits (to avoid Discord/Slack rate limits).
const DRAFT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Tracks the state of an in-progress streaming draft reply.
#[derive(Debug)]
pub struct DraftReplyState {
    /// The message ID of the draft we're editing (platform-specific).
    pub message_id: Option<String>,
    /// Channel ID / conversation target.
    pub channel_id: String,
    /// Accumulated text so far.
    pub accumulated: String,
    /// When the last edit was sent.
    pub last_edit_at: Instant,
    /// Whether the initial placeholder has been posted.
    pub posted: bool,
}

impl DraftReplyState {
    /// Create a new draft state for a channel.
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            message_id: None,
            channel_id: channel_id.into(),
            accumulated: String::new(),
            last_edit_at: Instant::now() - DRAFT_DEBOUNCE, // allow immediate first edit
            posted: false,
        }
    }

    /// Append a chunk and return true if enough time has passed to send an edit.
    pub fn append(&mut self, chunk: &str) -> bool {
        self.accumulated.push_str(chunk);
        self.last_edit_at.elapsed() >= DRAFT_DEBOUNCE
    }

    /// Mark that an edit was just sent.
    pub fn mark_sent(&mut self, message_id: Option<String>) {
        self.last_edit_at = Instant::now();
        self.posted = true;
        if let Some(id) = message_id {
            self.message_id = Some(id);
        }
    }

    /// Get the current accumulated text with a typing indicator.
    pub fn display_text(&self) -> String {
        format!("{} \u{2726}", self.accumulated)
    }

    /// Get the final accumulated text (no typing indicator).
    pub fn final_text(&self) -> &str {
        &self.accumulated
    }
}

/// Trait for message channels.
///
/// Channels receive messages from external sources and convert them to
/// a unified format. They also handle sending responses back.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Get the channel name (e.g., "cli", "slack", "telegram", "http").
    fn name(&self) -> &str;

    /// Start listening for messages.
    ///
    /// Returns a stream of incoming messages. The channel should handle
    /// reconnection and error recovery internally.
    async fn start(&self) -> Result<MessageStream, ChannelError>;

    /// Send a response back to the user.
    ///
    /// The response is sent in the context of the original message
    /// (same channel, same thread if applicable).
    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError>;

    /// Send a status update (thinking, tool execution, etc.).
    ///
    /// The metadata contains channel-specific routing info (e.g., Telegram chat_id)
    /// needed to deliver the status to the correct destination.
    ///
    /// Default implementation does nothing (for channels that don't support status).
    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a proactive message without a prior incoming message.
    ///
    /// Used for alerts, heartbeat notifications, and other agent-initiated communication.
    /// The user_id helps target a specific user within the channel.
    ///
    /// Default implementation does nothing (for channels that don't support broadcast).
    async fn broadcast(
        &self,
        _user_id: &str,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a streaming draft update for progressive message rendering.
    ///
    /// Channels that support message editing (Slack, Discord) can override this
    /// to post an initial placeholder and then edit it as chunks arrive.
    ///
    /// Returns the platform message ID (for subsequent edits).
    async fn send_draft(
        &self,
        _draft: &DraftReplyState,
        _metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        Ok(None)
    }

    /// Get the stream mode for this channel.
    ///
    /// Default: StreamMode::None (no streaming drafts).
    fn stream_mode(&self) -> StreamMode {
        StreamMode::None
    }

    /// Update the stream mode at runtime (e.g., from WebUI settings).
    ///
    /// Default implementation does nothing. Channels that support runtime
    /// stream mode changes (e.g., WASM Telegram) should override this.
    async fn set_stream_mode(&self, _mode: StreamMode) {
        // No-op by default
    }

    /// Check if the channel is healthy.
    async fn health_check(&self) -> Result<(), ChannelError>;

    /// React to a message with an emoji.
    ///
    /// Default implementation does nothing (for channels that don't support reactions).
    async fn react(
        &self,
        _chat_id: &str,
        _message_id: &str,
        _emoji: &str,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a poll to a chat.
    ///
    /// Default implementation does nothing (for channels that don't support polls).
    async fn poll(
        &self,
        _chat_id: &str,
        _question: &str,
        _options: &[String],
        _is_anonymous: bool,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Gracefully shut down the channel.
    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a typing indicator to a chat.
    ///
    /// Platforms like Telegram, Discord, and Slack show a "... is typing"
    /// indicator. The `chat_id` is channel-specific (e.g. Telegram chat ID,
    /// Discord channel ID).
    ///
    /// Default implementation does nothing (for channels without typing support).
    async fn send_typing(&self, _chat_id: &str) -> Result<(), ChannelError> {
        Ok(())
    }
}

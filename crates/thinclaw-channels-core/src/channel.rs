//! Core channel message and streaming-draft types.

use std::pin::Pin;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures::Stream;
use thinclaw_identity::{IncomingIdentityMessage, ResolvedIdentity};
use thinclaw_types::media::MediaContent;
use uuid::Uuid;

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub id: Uuid,
    pub channel: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub content: String,
    pub thread_id: Option<String>,
    pub received_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub identity: Option<ResolvedIdentity>,
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
            identity: None,
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

    /// Attach a resolved identity.
    pub fn with_identity(mut self, identity: ResolvedIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Resolve the message identity, deriving stable defaults when needed.
    pub fn resolved_identity(&self) -> ResolvedIdentity {
        ResolvedIdentity::from_message(self)
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

impl IncomingIdentityMessage for IncomingMessage {
    fn channel(&self) -> &str {
        &self.channel
    }

    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    fn metadata(&self) -> &serde_json::Value {
        &self.metadata
    }

    fn identity(&self) -> Option<&ResolvedIdentity> {
        self.identity.as_ref()
    }
}

/// Response to send back to a channel.
#[derive(Debug, Clone)]
pub struct OutgoingResponse {
    pub content: String,
    pub thread_id: Option<String>,
    pub metadata: serde_json::Value,
    pub attachments: Vec<MediaContent>,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            metadata: serde_json::Value::Null,
            attachments: Vec::new(),
        }
    }

    /// Set the thread ID for the response.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Attach outbound media files to the response.
    pub fn with_attachments(mut self, attachments: Vec<MediaContent>) -> Self {
        self.attachments = attachments;
        self
    }
}

/// Per-channel streaming mode for partial reply rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamMode {
    #[default]
    None,
    EditFirst,
    StatusLine,
    EventChunks,
}

impl StreamMode {
    /// Parse from a string value (env var or config).
    pub fn from_str_value(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "edit" | "edit_first" | "editfirst" | "fulledit" | "full_edit" => Self::EditFirst,
            "status" | "status_line" | "statusline" => Self::StatusLine,
            "event" | "events" | "chunk" | "chunks" | "event_chunks" | "eventchunks" => {
                Self::EventChunks
            }
            _ => Self::None,
        }
    }
}

/// Minimum interval between draft edits.
const DRAFT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Tracks the state of an in-progress streaming draft reply.
#[derive(Debug)]
pub struct DraftReplyState {
    pub message_id: Option<String>,
    pub channel_id: String,
    pub accumulated: String,
    pub last_edit_at: Instant,
    pub posted: bool,
    pub overflow: bool,
}

impl DraftReplyState {
    /// Create a new draft state for a channel.
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            message_id: None,
            channel_id: channel_id.into(),
            accumulated: String::new(),
            last_edit_at: Instant::now()
                .checked_sub(DRAFT_DEBOUNCE)
                .unwrap_or_else(Instant::now),
            posted: false,
            overflow: false,
        }
    }

    /// Append a chunk and return true if enough time has passed to send an edit.
    pub fn append(&mut self, chunk: &str) -> bool {
        self.accumulated.push_str(chunk);
        if self.overflow {
            return false;
        }
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

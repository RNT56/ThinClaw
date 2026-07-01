//! Signal channel via signal-cli daemon HTTP/JSON-RPC.
//!
//! Connects to a running `signal-cli daemon --http <host:port>`.
//! Listens for messages via SSE at `/api/v1/events` and sends via
//! JSON-RPC at `/api/v1/rpc`.

use std::num::NonZeroUsize;

use serde::Deserialize;

const GROUP_TARGET_PREFIX: &str = "group:";
const SIGNAL_HEALTH_ENDPOINT: &str = "/api/v1/check";

const MAX_SSE_BUFFER_SIZE: usize = 1024 * 1024;
const MAX_SSE_EVENT_SIZE: usize = 256 * 1024;
const MAX_HTTP_RESPONSE_SIZE: usize = 10 * 1024 * 1024;
const MAX_REPLY_TARGETS: usize = 10000;
const MAX_ERROR_LOG_BODY: usize = 1024;

const REPLY_TARGETS_CAP: NonZeroUsize =
    NonZeroUsize::new(MAX_REPLY_TARGETS).expect("MAX_REPLY_TARGETS is non-zero");

#[derive(Debug, Clone)]
pub struct SignalConfig {
    pub http_url: String,
    pub account: String,
    pub allow_from: Vec<String>,
    pub allow_from_groups: Vec<String>,
    pub dm_policy: String,
    pub group_policy: String,
    pub group_allow_from: Vec<String>,
    pub ignore_attachments: bool,
    pub ignore_stories: bool,
}

/// Recipient classification for outbound messages.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecipientTarget {
    Direct(String),
    Group(String),
}

// ── signal-cli SSE event JSON shapes ────────────────────────────

#[derive(Debug, Deserialize)]
struct SseEnvelope {
    #[serde(default)]
    envelope: Option<Envelope>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "sourceNumber", default)]
    source_number: Option<String>,
    #[serde(rename = "sourceName", default)]
    source_name: Option<String>,
    #[serde(rename = "sourceUuid", default)]
    source_uuid: Option<String>,
    #[serde(rename = "dataMessage", default)]
    data_message: Option<DataMessage>,
    #[serde(rename = "storyMessage", default)]
    story_message: Option<serde_json::Value>,
    #[serde(default)]
    timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    timestamp: Option<u64>,
    #[serde(rename = "groupInfo", default)]
    group_info: Option<GroupInfo>,
    #[serde(default)]
    attachments: Option<Vec<SignalAttachment>>,
}

/// Signal attachment from signal-cli SSE events.
///
/// signal-cli stores downloaded attachments locally. The `id` field is the
/// local filename under `~/.local/share/signal-cli/attachments/`.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SignalAttachment {
    /// MIME type (e.g. "image/jpeg", "audio/aac").
    #[serde(rename = "contentType", default)]
    content_type: Option<String>,
    /// Original filename.
    #[serde(default)]
    filename: Option<String>,
    /// File size in bytes.
    #[serde(default)]
    size: Option<u64>,
    /// Local attachment ID — the filename in signal-cli's attachment store.
    #[serde(default)]
    id: Option<String>,
}

/// Maximum single attachment size we'll read from disk (20 MB).
const MAX_SIGNAL_ATTACHMENT_SIZE: u64 = 20 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct GroupInfo {
    #[serde(rename = "groupId", default)]
    group_id: Option<String>,
}

mod attachments;
mod channel;

pub use channel::SignalChannel;

//! Host state for WASM channel execution.
//!
//! Extends the base tool host state with channel-specific functionality:
//! - Message emission (queueing messages to send to the agent)
//! - Workspace write access (scoped to channel namespace)
//! - Rate limiting for message emission

use std::time::{SystemTime, UNIX_EPOCH};

use crate::channels::wasm::capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, to_root_tool_capabilities,
};
use crate::channels::wasm::error::WasmChannelError;
use crate::tools::wasm::{HostState, LogLevel};

/// Maximum emitted messages per callback execution.
const MAX_EMITS_PER_EXECUTION: usize = 100;

/// Maximum message content size (64 KB).
const MAX_MESSAGE_CONTENT_SIZE: usize = 64 * 1024;

/// Maximum single attachment size (20 MB).
const MAX_ATTACHMENT_SIZE: usize = 20 * 1024 * 1024;

/// Maximum total attachment payload per message (50 MB).
const MAX_TOTAL_ATTACHMENT_SIZE: usize = 50 * 1024 * 1024;

/// A message emitted by a WASM channel to be sent to the agent.
#[derive(Debug, Clone)]
pub struct EmittedMessage {
    /// User identifier within the channel.
    pub user_id: String,

    /// Optional user display name.
    pub user_name: Option<String>,

    /// Message content.
    pub content: String,

    /// Optional thread ID for threaded conversations.
    pub thread_id: Option<String>,

    /// Channel-specific metadata as JSON string.
    pub metadata_json: String,

    /// Timestamp when the message was emitted.
    pub emitted_at_millis: u64,

    /// Binary media attachments (images, documents, etc.).
    pub attachments: Vec<MediaAttachment>,
}

/// A binary media attachment from a WASM channel.
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    /// MIME type (e.g., "image/jpeg").
    pub mime_type: String,
    /// Raw binary data.
    pub data: Vec<u8>,
    /// Optional filename.
    pub filename: Option<String>,
}

impl MediaAttachment {
    /// Convert to the agent's MediaContent type.
    pub fn to_media_content(&self) -> crate::media::MediaContent {
        let mc = crate::media::MediaContent::new(self.data.clone(), &self.mime_type);
        if let Some(ref filename) = self.filename {
            mc.with_filename(filename.clone())
        } else {
            mc
        }
    }
}

impl EmittedMessage {
    /// Create a new emitted message.
    pub fn new(user_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            emitted_at_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            attachments: Vec::new(),
        }
    }

    /// Set the user name.
    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Set the thread ID.
    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Set metadata JSON.
    pub fn with_metadata(mut self, metadata_json: impl Into<String>) -> Self {
        self.metadata_json = metadata_json.into();
        self
    }
}

/// A pending workspace write operation.
#[derive(Debug, Clone)]
pub struct PendingWorkspaceWrite {
    /// Full path (already prefixed with channel namespace).
    pub path: String,

    /// Content to write.
    pub content: String,
}

/// Host state for WASM channel callbacks.
///
/// Maintains all side effects during callback execution and enforces limits.
/// This is the channel-specific equivalent of HostState for tools.
pub struct ChannelHostState {
    /// Base tool host state (logging, time, HTTP, etc.).
    base: HostState,

    /// Channel name (for error messages).
    channel_name: String,

    /// Channel capabilities.
    capabilities: ChannelCapabilities,

    /// Emitted messages (queued for delivery).
    emitted_messages: Vec<EmittedMessage>,

    /// Pending workspace writes.
    pending_writes: Vec<PendingWorkspaceWrite>,

    /// Emit count for rate limiting within this execution.
    emit_count: u32,

    /// Whether emit is still allowed (false after rate limit hit).
    emit_enabled: bool,

    /// Count of emits dropped due to rate limiting.
    emits_dropped: usize,
}

impl std::fmt::Debug for ChannelHostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelHostState")
            .field("channel_name", &self.channel_name)
            .field("emitted_messages_count", &self.emitted_messages.len())
            .field("pending_writes_count", &self.pending_writes.len())
            .field("emit_count", &self.emit_count)
            .field("emit_enabled", &self.emit_enabled)
            .field("emits_dropped", &self.emits_dropped)
            .finish()
    }
}

impl ChannelHostState {
    /// Create a new channel host state.
    pub fn new(channel_name: impl Into<String>, capabilities: ChannelCapabilities) -> Self {
        let base = HostState::new(to_root_tool_capabilities(
            &capabilities.tool_capabilities,
            None,
        ));

        Self::with_base(channel_name, capabilities, base)
    }

    /// Create channel host state with root tool capabilities supplied by an adapter.
    pub(crate) fn with_root_tool_capabilities(
        channel_name: impl Into<String>,
        capabilities: ChannelCapabilities,
        tool_capabilities: crate::tools::wasm::Capabilities,
    ) -> Self {
        Self::with_base(
            channel_name,
            capabilities,
            HostState::new(tool_capabilities),
        )
    }

    fn with_base(
        channel_name: impl Into<String>,
        capabilities: ChannelCapabilities,
        base: HostState,
    ) -> Self {
        Self {
            base,
            channel_name: channel_name.into(),
            capabilities,
            emitted_messages: Vec::new(),
            pending_writes: Vec::new(),
            emit_count: 0,
            emit_enabled: true,
            emits_dropped: 0,
        }
    }

    /// Get the channel name.
    pub fn channel_name(&self) -> &str {
        &self.channel_name
    }

    /// Get the capabilities.
    pub fn capabilities(&self) -> &ChannelCapabilities {
        &self.capabilities
    }

    /// Get the base host state for tool capabilities.
    pub fn base(&self) -> &HostState {
        &self.base
    }

    /// Get mutable access to the base host state.
    pub fn base_mut(&mut self) -> &mut HostState {
        &mut self.base
    }

    /// Emit a message from the channel.
    ///
    /// Messages are queued and delivered after callback execution completes.
    /// Rate limiting is enforced per-execution and globally.
    pub fn emit_message(&mut self, mut msg: EmittedMessage) -> Result<(), WasmChannelError> {
        // Check per-execution limit
        if !self.emit_enabled {
            self.emits_dropped += 1;
            return Ok(()); // Silently drop, don't fail execution
        }

        if self.emitted_messages.len() >= MAX_EMITS_PER_EXECUTION {
            self.emit_enabled = false;
            self.emits_dropped += 1;
            tracing::warn!(
                channel = %self.channel_name,
                limit = MAX_EMITS_PER_EXECUTION,
                "Channel emit limit reached, further messages dropped"
            );
            return Ok(());
        }

        // Validate attachment sizes — drop oversized attachments individually
        let original_count = msg.attachments.len();
        let mut total_size: usize = 0;
        msg.attachments.retain(|att| {
            if att.data.len() > MAX_ATTACHMENT_SIZE {
                tracing::warn!(
                    channel = %self.channel_name,
                    size = att.data.len(),
                    max = MAX_ATTACHMENT_SIZE,
                    mime = %att.mime_type,
                    "Dropping oversized attachment"
                );
                return false;
            }
            total_size += att.data.len();
            if total_size > MAX_TOTAL_ATTACHMENT_SIZE {
                tracing::warn!(
                    channel = %self.channel_name,
                    total = total_size,
                    max = MAX_TOTAL_ATTACHMENT_SIZE,
                    "Dropping attachment: total payload exceeds limit"
                );
                return false;
            }
            true
        });
        if msg.attachments.len() < original_count {
            tracing::info!(
                channel = %self.channel_name,
                kept = msg.attachments.len(),
                dropped = original_count - msg.attachments.len(),
                "Some attachments dropped due to size limits"
            );
        }

        // Validate message content size
        if msg.content.len() > MAX_MESSAGE_CONTENT_SIZE {
            tracing::warn!(
                channel = %self.channel_name,
                size = msg.content.len(),
                max = MAX_MESSAGE_CONTENT_SIZE,
                "Message content too large, truncating"
            );
            let safe_end = crate::util::floor_char_boundary(&msg.content, MAX_MESSAGE_CONTENT_SIZE);
            let mut truncated = msg.content[..safe_end].to_string();
            truncated.push_str("... (truncated)");
            let msg = EmittedMessage {
                content: truncated,
                ..msg
            };
            self.emitted_messages.push(msg);
        } else {
            self.emitted_messages.push(msg);
        }

        self.emit_count += 1;
        Ok(())
    }

    /// Take all emitted messages (clears the queue).
    pub fn take_emitted_messages(&mut self) -> Vec<EmittedMessage> {
        std::mem::take(&mut self.emitted_messages)
    }

    /// Get the number of emitted messages.
    pub fn emitted_count(&self) -> usize {
        self.emitted_messages.len()
    }

    /// Get the number of emits dropped due to rate limiting.
    pub fn emits_dropped(&self) -> usize {
        self.emits_dropped
    }

    /// Write to workspace (scoped to channel namespace).
    ///
    /// Writes are queued and committed after callback execution completes.
    pub fn workspace_write(&mut self, path: &str, content: String) -> Result<(), WasmChannelError> {
        // Validate and prefix path
        let full_path = self
            .capabilities
            .validate_workspace_path(path)
            .map_err(|reason| WasmChannelError::WorkspaceEscape {
                name: self.channel_name.clone(),
                path: reason,
            })?;

        self.pending_writes.push(PendingWorkspaceWrite {
            path: full_path,
            content,
        });

        Ok(())
    }

    /// Take all pending workspace writes (clears the queue).
    pub fn take_pending_writes(&mut self) -> Vec<PendingWorkspaceWrite> {
        std::mem::take(&mut self.pending_writes)
    }

    /// Get the number of pending workspace writes.
    pub fn pending_writes_count(&self) -> usize {
        self.pending_writes.len()
    }

    /// Log a message (delegates to base).
    pub fn log(
        &mut self,
        level: LogLevel,
        message: String,
    ) -> Result<(), crate::tools::wasm::WasmError> {
        self.base.log(level, message)
    }

    /// Get current timestamp in milliseconds (delegates to base).
    pub fn now_millis(&self) -> u64 {
        self.base.now_millis()
    }

    /// Read from workspace (delegates to base).
    pub fn workspace_read(
        &self,
        path: &str,
    ) -> Result<Option<String>, crate::tools::wasm::WasmError> {
        // Prefix the path with channel namespace before reading
        let full_path = self.capabilities.prefix_workspace_path(path);
        self.base.workspace_read(&full_path)
    }

    /// Check if a secret exists (delegates to base).
    pub fn secret_exists(&self, name: &str) -> bool {
        self.base.secret_exists(name)
    }

    /// Check if HTTP is allowed (delegates to base).
    pub fn check_http_allowed(&self, url: &str, method: &str) -> Result<(), String> {
        self.base.check_http_allowed(url, method)
    }

    /// Record an HTTP request (delegates to base).
    pub fn record_http_request(&mut self) -> Result<(), String> {
        self.base.record_http_request()
    }

    /// Take logs (delegates to base).
    pub fn take_logs(&mut self) -> Vec<crate::tools::wasm::LogEntry> {
        self.base.take_logs()
    }
}

/// Workspace store for WASM channels with optional disk persistence.
///
/// Persists workspace writes across callback invocations. When a
/// `persist_path` is configured, the store also survives process restarts
/// by loading state from disk on construction and flushing after every
/// batch of writes that actually changes the serialized content.
///
/// To minimize unnecessary I/O (e.g. ephemeral timestamp keys that change
/// every poll tick), the store tracks the last-flushed serialized snapshot
/// and only writes to disk when the content has actually changed.
///
/// Uses `std::sync::RwLock` (not tokio) because WASM execution runs
/// inside `spawn_blocking`.
pub struct ChannelWorkspaceStore {
    data: std::sync::RwLock<std::collections::HashMap<String, String>>,
    /// Optional path for disk persistence. When set, the store is loaded
    /// from and saved to this JSON file.
    persist_path: Option<std::path::PathBuf>,
    /// Serialized snapshot of the last content flushed to disk.
    /// Compared against new serialization to skip redundant writes.
    last_flushed: std::sync::Mutex<Vec<u8>>,
}

impl ChannelWorkspaceStore {
    const MANAGED_PRIVATE_TOPICS_SUFFIX: &str = "/state/managed_private_topics";

    /// Create a new empty workspace store (in-memory only, no disk persistence).
    #[allow(dead_code)] // Used by tests in wrapper.rs and host.rs
    pub fn new() -> Self {
        Self {
            data: std::sync::RwLock::new(std::collections::HashMap::new()),
            persist_path: None,
            last_flushed: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create a workspace store backed by a JSON file on disk.
    ///
    /// Loads existing state from `path` if the file exists. Subsequent
    /// calls to [`commit_writes`] will flush the full store back to disk
    /// only when the serialized content has actually changed.
    pub fn with_persistence(path: std::path::PathBuf) -> Self {
        let (data, initial_snapshot) = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    match serde_json::from_str::<std::collections::HashMap<String, String>>(
                        &content,
                    ) {
                        Ok(map) => {
                            // Pre-compute initial snapshot for change detection
                            let snapshot = Self::serialize_deterministic(&map).unwrap_or_default();
                            (map, snapshot)
                        }
                        Err(err) => {
                            tracing::warn!(
                                path = %path.display(),
                                error = %err,
                                "Failed to parse persisted channel workspace, starting fresh"
                            );
                            (std::collections::HashMap::new(), Vec::new())
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "Failed to read persisted channel workspace, starting fresh"
                    );
                    (std::collections::HashMap::new(), Vec::new())
                }
            }
        } else {
            (std::collections::HashMap::new(), Vec::new())
        };

        tracing::debug!(
            path = %path.display(),
            entries = data.len(),
            "Loaded channel workspace store from disk"
        );

        Self {
            data: std::sync::RwLock::new(data),
            persist_path: Some(path),
            last_flushed: std::sync::Mutex::new(initial_snapshot),
        }
    }

    /// Commit pending writes from a callback execution into the store.
    ///
    /// If a persist path is configured, the store is flushed to disk only
    /// when the serialized content has actually changed since the last flush.
    /// The disk flush happens outside the data write lock to avoid blocking
    /// concurrent readers.
    pub fn commit_writes(&self, writes: &[PendingWorkspaceWrite]) {
        if writes.is_empty() {
            return;
        }

        // Apply writes under the lock, then snapshot for out-of-lock flush.
        let snapshot = {
            let mut data = match self.data.write() {
                Ok(guard) => guard,
                Err(_poisoned) => return,
            };
            for write in writes {
                tracing::debug!(
                    path = %write.path,
                    content_len = write.content.len(),
                    "Committing workspace write to channel store"
                );

                if Self::is_managed_private_topics_path(&write.path) {
                    let merged = data
                        .get(&write.path)
                        .map(|existing| {
                            Self::merge_managed_private_topic_registry(existing, &write.content)
                        })
                        .unwrap_or_else(|| write.content.clone());
                    data.insert(write.path.clone(), merged);
                } else {
                    data.insert(write.path.clone(), write.content.clone());
                }
            }

            // Only clone for flush if persistence is configured
            if self.persist_path.is_some() {
                Some(data.clone())
            } else {
                None
            }
            // data write lock is released here
        };

        // Flush to disk outside the data lock
        if let (Some(snapshot), Some(persist_path)) = (snapshot, &self.persist_path) {
            self.flush_if_changed(persist_path, &snapshot);
        }
    }

    fn is_managed_private_topics_path(path: &str) -> bool {
        path.ends_with(Self::MANAGED_PRIVATE_TOPICS_SUFFIX)
    }

    fn merge_managed_private_topic_registry(existing: &str, incoming: &str) -> String {
        let existing_json = match serde_json::from_str::<serde_json::Value>(existing) {
            Ok(value) => value,
            Err(_) => return incoming.to_string(),
        };
        let incoming_json = match serde_json::from_str::<serde_json::Value>(incoming) {
            Ok(value) => value,
            Err(_) => return incoming.to_string(),
        };

        let mut merged = existing_json;
        Self::merge_json_in_place(&mut merged, incoming_json);
        serde_json::to_string(&merged).unwrap_or_else(|_| incoming.to_string())
    }

    fn merge_json_in_place(base: &mut serde_json::Value, incoming: serde_json::Value) {
        match (base, incoming) {
            (serde_json::Value::Object(base_map), serde_json::Value::Object(incoming_map)) => {
                for (key, value) in incoming_map {
                    if let Some(existing_value) = base_map.get_mut(&key) {
                        Self::merge_json_in_place(existing_value, value);
                    } else {
                        base_map.insert(key, value);
                    }
                }
            }
            (slot, value) => {
                *slot = value;
            }
        }
    }

    /// Serialize a HashMap deterministically using BTreeMap ordering.
    ///
    /// JSON serialization of HashMap is non-deterministic (iteration order
    /// varies). Using BTreeMap ensures identical content always produces
    /// identical bytes, enabling reliable snapshot comparison.
    fn serialize_deterministic(
        data: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<u8>, serde_json::Error> {
        let sorted: std::collections::BTreeMap<_, _> = data.iter().collect();
        serde_json::to_vec_pretty(&sorted)
    }

    /// Flush to disk only if the serialized content has changed since the
    /// last successful flush.
    fn flush_if_changed(
        &self,
        path: &std::path::Path,
        data: &std::collections::HashMap<String, String>,
    ) {
        let serialized = match Self::serialize_deterministic(data) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Failed to serialize channel workspace store"
                );
                return;
            }
        };

        // Compare with last-flushed snapshot to skip redundant writes
        if let Ok(last) = self.last_flushed.lock()
            && *last == serialized
        {
            return; // Content unchanged, skip disk I/O
        }

        // Content changed — write to disk atomically
        Self::write_to_disk(path, &serialized);

        // Update last-flushed snapshot
        if let Ok(mut last) = self.last_flushed.lock() {
            *last = serialized;
        }
    }

    /// Write serialized bytes to disk atomically (write-tmp + rename).
    fn write_to_disk(path: &std::path::Path, serialized: &[u8]) {
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(
                path = %parent.display(),
                error = %err,
                "Failed to create directory for channel workspace persistence"
            );
            return;
        }

        let tmp_path = path.with_extension("tmp");
        if let Err(err) =
            std::fs::write(&tmp_path, serialized).and_then(|()| std::fs::rename(&tmp_path, path))
        {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "Failed to persist channel workspace store to disk"
            );
        }
    }
}

impl crate::tools::wasm::WorkspaceReader for ChannelWorkspaceStore {
    fn read(&self, path: &str) -> Option<String> {
        self.data.read().ok()?.get(path).cloned()
    }
}

/// Rate limiter for channel message emission.
///
/// Tracks emission rates across multiple executions.
pub struct ChannelEmitRateLimiter {
    config: EmitRateLimitConfig,
    minute_window: RateWindow,
    hour_window: RateWindow,
}

struct RateWindow {
    count: u32,
    window_start: u64,
    window_duration_ms: u64,
}

impl RateWindow {
    fn new(duration_ms: u64) -> Self {
        Self {
            count: 0,
            window_start: 0,
            window_duration_ms: duration_ms,
        }
    }

    fn check_and_record(&mut self, now_ms: u64, limit: u32) -> bool {
        // Reset window if expired
        if now_ms.saturating_sub(self.window_start) > self.window_duration_ms {
            self.count = 0;
            self.window_start = now_ms;
        }

        if self.count >= limit {
            return false;
        }

        self.count += 1;
        true
    }
}

#[allow(dead_code)]
impl ChannelEmitRateLimiter {
    /// Create a new rate limiter with the given config.
    pub fn new(config: EmitRateLimitConfig) -> Self {
        Self {
            config,
            minute_window: RateWindow::new(60_000), // 1 minute
            hour_window: RateWindow::new(3_600_000), // 1 hour
        }
    }

    /// Check if an emit is allowed and record it if so.
    ///
    /// Returns true if the emit is allowed, false if rate limited.
    pub fn check_and_record(&mut self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Check both windows
        let minute_ok = self
            .minute_window
            .check_and_record(now, self.config.messages_per_minute);
        let hour_ok = self
            .hour_window
            .check_and_record(now, self.config.messages_per_hour);

        minute_ok && hour_ok
    }

    /// Get the current emission count for the minute window.
    pub fn minute_count(&self) -> u32 {
        self.minute_window.count
    }

    /// Get the current emission count for the hour window.
    pub fn hour_count(&self) -> u32 {
        self.hour_window.count
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
    use crate::channels::wasm::host::{
        ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MAX_EMITS_PER_EXECUTION,
    };

    #[test]
    fn test_emit_message_basic() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello, world!");
        state.emit_message(msg).unwrap();

        assert_eq!(state.emitted_count(), 1);

        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].user_id, "user123");
        assert_eq!(messages[0].content, "Hello, world!");

        // Queue should be cleared
        assert_eq!(state.emitted_count(), 0);
    }

    #[test]
    fn test_emit_message_with_metadata() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello")
            .with_user_name("John Doe")
            .with_thread_id("thread-1")
            .with_metadata(r#"{"key": "value"}"#);

        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages[0].user_name, Some("John Doe".to_string()));
        assert_eq!(messages[0].thread_id, Some("thread-1".to_string()));
        assert_eq!(messages[0].metadata_json, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_emit_per_execution_limit() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        // Fill up to limit
        for i in 0..MAX_EMITS_PER_EXECUTION {
            let msg = EmittedMessage::new("user", format!("Message {}", i));
            state.emit_message(msg).unwrap();
        }

        // This should be dropped silently
        let msg = EmittedMessage::new("user", "Should be dropped");
        state.emit_message(msg).unwrap();

        assert_eq!(state.emitted_count(), MAX_EMITS_PER_EXECUTION);
        assert_eq!(state.emits_dropped(), 1);
    }

    #[test]
    fn test_workspace_write_prefixing() {
        let caps = ChannelCapabilities::for_channel("slack");
        let mut state = ChannelHostState::new("slack", caps);

        state
            .workspace_write("state.json", "{}".to_string())
            .unwrap();

        let writes = state.take_pending_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "channels/slack/state.json");
    }

    #[test]
    fn test_workspace_write_path_traversal_blocked() {
        let caps = ChannelCapabilities::for_channel("slack");
        let mut state = ChannelHostState::new("slack", caps);

        // Try to escape namespace
        let result = state.workspace_write("../secrets.json", "{}".to_string());
        assert!(result.is_err());

        // Absolute path
        let result = state.workspace_write("/etc/passwd", "{}".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_rate_limiter_basic() {
        let config = EmitRateLimitConfig {
            messages_per_minute: 10,
            messages_per_hour: 100,
        };
        let mut limiter = ChannelEmitRateLimiter::new(config);

        // Should allow 10 messages
        for _ in 0..10 {
            assert!(limiter.check_and_record());
        }

        // 11th should be blocked
        assert!(!limiter.check_and_record());
    }

    #[test]
    fn test_channel_name() {
        let caps = ChannelCapabilities::for_channel("telegram");
        let state = ChannelHostState::new("telegram", caps);

        assert_eq!(state.channel_name(), "telegram");
    }

    #[test]
    fn test_channel_workspace_store_commit_and_read() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();

        // Initially empty
        assert!(store.read("channels/telegram/offset").is_none());

        // Commit some writes
        let writes = vec![
            PendingWorkspaceWrite {
                path: "channels/telegram/offset".to_string(),
                content: "103".to_string(),
            },
            PendingWorkspaceWrite {
                path: "channels/telegram/state.json".to_string(),
                content: r#"{"ok":true}"#.to_string(),
            },
        ];
        store.commit_writes(&writes);

        // Should be readable
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("103".to_string())
        );
        assert_eq!(
            store.read("channels/telegram/state.json"),
            Some(r#"{"ok":true}"#.to_string())
        );

        // Overwrite a value
        let writes2 = vec![PendingWorkspaceWrite {
            path: "channels/telegram/offset".to_string(),
            content: "200".to_string(),
        }];
        store.commit_writes(&writes2);
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );

        // Empty writes are a no-op
        store.commit_writes(&[]);
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );
    }

    #[test]
    fn test_channel_workspace_store_disk_persistence_round_trip() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};
        use crate::tools::wasm::WorkspaceReader;

        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("test_workspace.json");

        // Write data with a disk-backed store
        {
            let store = ChannelWorkspaceStore::with_persistence(path.clone());
            assert!(
                store
                    .read("channels/telegram/state/managed_private_topics")
                    .is_none()
            );

            let writes = vec![PendingWorkspaceWrite {
                path: "channels/telegram/state/managed_private_topics".to_string(),
                content: r#"{"chats":{"123":{"general_thread_id":42}}}"#.to_string(),
            }];
            store.commit_writes(&writes);

            assert_eq!(
                store.read("channels/telegram/state/managed_private_topics"),
                Some(r#"{"chats":{"123":{"general_thread_id":42}}}"#.to_string())
            );
        }

        // Verify the file exists on disk
        assert!(path.exists(), "workspace file should be persisted to disk");

        // Load a fresh store from the same path — simulates a restart
        {
            let store2 = ChannelWorkspaceStore::with_persistence(path.clone());
            assert_eq!(
                store2.read("channels/telegram/state/managed_private_topics"),
                Some(r#"{"chats":{"123":{"general_thread_id":42}}}"#.to_string()),
                "managed topic registry should survive restart"
            );
        }
    }

    #[test]
    fn test_channel_workspace_store_skips_redundant_flush() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("skip_redundant.json");

        let store = ChannelWorkspaceStore::with_persistence(path.clone());

        // First write — should create the file
        let writes = vec![PendingWorkspaceWrite {
            path: "channels/telegram/state/managed_private_topics".to_string(),
            content: r#"{"chats":{"123":{"general_thread_id":42}}}"#.to_string(),
        }];
        store.commit_writes(&writes);
        assert!(path.exists());

        let mtime_after_first = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Small sleep to ensure filesystem timestamp granularity
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Second write with identical content — should NOT touch the file
        store.commit_writes(&writes);

        let mtime_after_second = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert_eq!(
            mtime_after_first, mtime_after_second,
            "Redundant write should not update the file"
        );

        // Third write with CHANGED content — should update the file
        std::thread::sleep(std::time::Duration::from_millis(50));
        let writes_changed = vec![PendingWorkspaceWrite {
            path: "channels/telegram/state/managed_private_topics".to_string(),
            content: r#"{"chats":{"123":{"general_thread_id":99}}}"#.to_string(),
        }];
        store.commit_writes(&writes_changed);

        let mtime_after_third = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert_ne!(
            mtime_after_first, mtime_after_third,
            "Changed content should update the file"
        );
    }

    #[test]
    fn test_channel_workspace_store_merges_managed_private_topic_registry_updates() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();
        let path = "channels/telegram/state/managed_private_topics".to_string();

        store.commit_writes(&[PendingWorkspaceWrite {
            path: path.clone(),
            content: r#"{"chats":{"123":{"onboarding_thread_id":61419}}}"#.to_string(),
        }]);

        store.commit_writes(&[PendingWorkspaceWrite {
            path: path.clone(),
            content: r#"{"chats":{"123":{"general_thread_id":7}}}"#.to_string(),
        }]);

        let raw = store
            .read(&path)
            .expect("managed topic registry should be present");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("managed topic registry should be valid JSON");

        assert_eq!(
            parsed["chats"]["123"]["onboarding_thread_id"],
            serde_json::json!(61419)
        );
        assert_eq!(
            parsed["chats"]["123"]["general_thread_id"],
            serde_json::json!(7)
        );
    }

    #[test]
    fn test_channel_workspace_store_merge_preserves_explicit_null_override() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();
        let path = "channels/telegram/state/managed_private_topics".to_string();

        store.commit_writes(&[PendingWorkspaceWrite {
            path: path.clone(),
            content: r#"{"chats":{"123":{"onboarding_thread_id":61419,"general_thread_id":7}}}"#
                .to_string(),
        }]);

        store.commit_writes(&[PendingWorkspaceWrite {
            path: path.clone(),
            content: r#"{"chats":{"123":{"general_thread_id":null}}}"#.to_string(),
        }]);

        let raw = store
            .read(&path)
            .expect("managed topic registry should be present");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("managed topic registry should be valid JSON");

        assert_eq!(
            parsed["chats"]["123"]["onboarding_thread_id"],
            serde_json::json!(61419)
        );
        assert_eq!(
            parsed["chats"]["123"]["general_thread_id"],
            serde_json::Value::Null
        );
    }
}

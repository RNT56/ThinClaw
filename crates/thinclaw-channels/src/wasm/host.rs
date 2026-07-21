//! Host state for WASM channel execution.
//!
//! Extends the base tool host state with channel-specific functionality:
//! - Message emission (queueing messages to send to the agent)
//! - Workspace write access (scoped to channel namespace)
//! - Rate limiting for message emission

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
use crate::wasm::error::WasmChannelError;

/// Log levels matching the channel WIT interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp_millis: u64,
}

pub trait WorkspaceReader: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
}

fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }

    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn workspace_prefix_matches(value: &str, prefix: &str) -> bool {
    value == prefix
        || value
            .strip_prefix(prefix)
            .is_some_and(|remainder| prefix.ends_with('/') || remainder.starts_with('/'))
}

/// Maximum emitted messages per callback execution.
const MAX_EMITS_PER_EXECUTION: usize = 100;

/// Maximum message content size (64 KB).
const MAX_MESSAGE_CONTENT_SIZE: usize = 64 * 1024;

/// Maximum single attachment size (20 MB).
const MAX_ATTACHMENT_SIZE: usize = 20 * 1024 * 1024;

/// Maximum total attachment payload per message (40 MB).
const MAX_TOTAL_ATTACHMENT_SIZE: usize = 40 * 1024 * 1024;
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
const MAX_MESSAGE_IDENTIFIER_SIZE: usize = 4096;
const MAX_MESSAGE_USER_NAME_SIZE: usize = 1024;
const MAX_MESSAGE_METADATA_SIZE: usize = 256 * 1024;
const MAX_WORKSPACE_PATH_SIZE: usize = 1024;
const MAX_WORKSPACE_WRITE_SIZE: usize = 1024 * 1024;
const MAX_WORKSPACE_WRITES_PER_EXECUTION: usize = 64;
const MAX_WORKSPACE_WRITE_TOTAL: usize = 4 * 1024 * 1024;
const MAX_WORKSPACE_STORE_ENTRIES: usize = 4096;
const MAX_WORKSPACE_STORE_BYTES: usize = 8 * 1024 * 1024;
const MAX_WORKSPACE_STORE_FILE_BYTES: usize = 12 * 1024 * 1024;

/// A message emitted by a WASM channel to be sent to the agent.
#[derive(Clone)]
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

impl std::fmt::Debug for EmittedMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmittedMessage")
            .field("user_id", &self.user_id)
            .field("user_name", &self.user_name)
            .field("content_bytes", &self.content.len())
            .field("thread_id", &self.thread_id)
            .field("metadata_bytes", &self.metadata_json.len())
            .field("emitted_at_millis", &self.emitted_at_millis)
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

/// A binary media attachment from a WASM channel.
#[derive(Clone)]
pub struct MediaAttachment {
    /// MIME type (e.g., "image/jpeg").
    pub mime_type: String,
    /// Raw binary data.
    pub data: Vec<u8>,
    /// Optional filename.
    pub filename: Option<String>,
}

impl std::fmt::Debug for MediaAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MediaAttachment")
            .field("mime_type", &self.mime_type)
            .field("data_bytes", &self.data.len())
            .field("filename", &self.filename)
            .finish()
    }
}

impl MediaAttachment {
    /// Convert to the agent's MediaContent type.
    pub fn to_media_content(&self) -> thinclaw_media::MediaContent {
        let mc = thinclaw_media::MediaContent::new(self.data.clone(), &self.mime_type);
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
#[derive(Clone)]
pub struct PendingWorkspaceWrite {
    /// Full path (already prefixed with channel namespace).
    pub path: String,

    /// Content to write.
    pub content: String,
}

impl std::fmt::Debug for PendingWorkspaceWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingWorkspaceWrite")
            .field("path", &self.path)
            .field("content_bytes", &self.content.len())
            .finish()
    }
}

/// Host state for WASM channel callbacks.
///
/// Maintains all side effects during callback execution and enforces limits.
/// This is the channel-specific equivalent of HostState for tools.
pub struct ChannelHostState {
    /// Channel name (for error messages).
    channel_name: String,

    /// Channel capabilities.
    capabilities: ChannelCapabilities,

    workspace_reader: Option<Arc<dyn WorkspaceReader>>,

    logs: Vec<LogEntry>,

    logging_enabled: bool,

    logs_dropped: usize,

    http_request_count: u32,

    /// Emitted messages (queued for delivery).
    emitted_messages: Vec<EmittedMessage>,

    /// Pending workspace writes.
    pending_writes: Vec<PendingWorkspaceWrite>,

    pending_write_bytes: usize,

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
        Self::with_workspace_reader(channel_name, capabilities, None)
    }

    pub(crate) fn with_workspace_reader(
        channel_name: impl Into<String>,
        capabilities: ChannelCapabilities,
        workspace_reader: Option<Arc<dyn WorkspaceReader>>,
    ) -> Self {
        Self {
            channel_name: channel_name.into(),
            capabilities,
            workspace_reader,
            logs: Vec::new(),
            logging_enabled: true,
            logs_dropped: 0,
            http_request_count: 0,
            emitted_messages: Vec::new(),
            pending_writes: Vec::new(),
            pending_write_bytes: 0,
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

    /// Emit a message from the channel.
    ///
    /// Messages are queued and delivered after callback execution completes.
    /// Rate limiting is enforced per-execution and globally.
    pub fn emit_message(&mut self, msg: EmittedMessage) -> Result<(), WasmChannelError> {
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

        let total_attachment_size = msg
            .attachments
            .iter()
            .try_fold(0usize, |total, attachment| {
                total.checked_add(attachment.data.len())
            });
        let max_content_size = self
            .capabilities
            .max_message_size
            .min(MAX_MESSAGE_CONTENT_SIZE);
        let valid_identifier = |value: &str, max: usize| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        if !valid_identifier(&msg.user_id, MAX_MESSAGE_IDENTIFIER_SIZE)
            || msg
                .user_name
                .as_deref()
                .is_some_and(|value| !valid_identifier(value, MAX_MESSAGE_USER_NAME_SIZE))
            || msg
                .thread_id
                .as_deref()
                .is_some_and(|value| !valid_identifier(value, MAX_MESSAGE_IDENTIFIER_SIZE))
            || msg.metadata_json.len() > MAX_MESSAGE_METADATA_SIZE
            || serde_json::from_str::<serde_json::Value>(&msg.metadata_json).is_err()
            || msg.content.len() > max_content_size
            || msg.attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE
            || total_attachment_size.is_none_or(|total| total > MAX_TOTAL_ATTACHMENT_SIZE)
            || msg.attachments.iter().any(|attachment| {
                attachment.data.len() > MAX_ATTACHMENT_SIZE
                    || !valid_identifier(&attachment.mime_type, 256)
                    || attachment.filename.as_deref().is_some_and(|value| {
                        !valid_identifier(value, 255) || value.contains(['/', '\\'])
                    })
            })
        {
            return Err(WasmChannelError::CallbackFailed {
                name: self.channel_name.clone(),
                reason: "emitted message is malformed or oversized".to_string(),
            });
        }

        self.emitted_messages.push(msg);

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

        if full_path.len() > MAX_WORKSPACE_PATH_SIZE
            || content.len() > MAX_WORKSPACE_WRITE_SIZE
            || self.pending_writes.len() >= MAX_WORKSPACE_WRITES_PER_EXECUTION
            || self.pending_write_bytes.saturating_add(content.len()) > MAX_WORKSPACE_WRITE_TOTAL
        {
            return Err(WasmChannelError::CallbackFailed {
                name: self.channel_name.clone(),
                reason: "workspace write is oversized or exceeds the per-execution limit"
                    .to_string(),
            });
        }
        self.pending_write_bytes += content.len();

        self.pending_writes.push(PendingWorkspaceWrite {
            path: full_path,
            content,
        });

        Ok(())
    }

    /// Take all pending workspace writes (clears the queue).
    pub fn take_pending_writes(&mut self) -> Vec<PendingWorkspaceWrite> {
        self.pending_write_bytes = 0;
        std::mem::take(&mut self.pending_writes)
    }

    /// Get the number of pending workspace writes.
    pub fn pending_writes_count(&self) -> usize {
        self.pending_writes.len()
    }

    pub fn log(&mut self, level: LogLevel, message: String) -> Result<(), String> {
        const MAX_LOG_ENTRIES: usize = 1000;
        const MAX_LOG_MESSAGE_BYTES: usize = 4096;

        if !self.logging_enabled {
            self.logs_dropped += 1;
            return Ok(());
        }

        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logging_enabled = false;
            self.logs_dropped += 1;
            return Ok(());
        }

        let message = if message.len() > MAX_LOG_MESSAGE_BYTES {
            let end = floor_char_boundary(&message, MAX_LOG_MESSAGE_BYTES);
            format!("{}... (truncated)", &message[..end])
        } else {
            message
        };

        self.logs.push(LogEntry {
            level,
            message,
            timestamp_millis: self.now_millis(),
        });
        Ok(())
    }

    pub fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn workspace_read(&self, path: &str) -> Result<Option<String>, String> {
        let full_path = self.capabilities.validate_workspace_path(path)?;
        if full_path.len() > MAX_WORKSPACE_PATH_SIZE {
            return Err("workspace path is oversized".to_string());
        }
        if let Some(workspace) = &self.capabilities.tool_capabilities.workspace_read
            && !workspace.allowed_prefixes.is_empty()
            && !workspace.allowed_prefixes.iter().any(|prefix| {
                workspace_prefix_matches(&full_path, prefix)
                    || workspace_prefix_matches(path, prefix)
            })
        {
            return Ok(None);
        }

        Ok(self
            .workspace_reader
            .as_ref()
            .and_then(|reader| reader.read(&full_path)))
    }

    pub fn secret_exists(&self, name: &str) -> bool {
        if name.is_empty()
            || name.len() > 256
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return false;
        }
        self.capabilities
            .tool_capabilities
            .secrets
            .as_ref()
            .is_some_and(|secrets| secrets.is_allowed(name))
    }

    pub fn check_http_allowed(&self, url: &str, method: &str) -> Result<(), String> {
        let http = self
            .capabilities
            .tool_capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;
        let parsed = url::Url::parse(url).map_err(|error| format!("invalid URL: {error}"))?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err("HTTP request requires a credential-free HTTPS URL".to_string());
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?;
        let path = parsed.path();
        if !http
            .allowlist
            .iter()
            .any(|pattern| pattern.matches(host, path, method))
        {
            return Err(format!(
                "HTTP {method} request is not allowed by channel capability"
            ));
        }

        // Reject special-use literal addresses here. The transport performs a
        // second DNS-resolving guard and pins the resulting public addresses so
        // hostnames cannot rebind between authorization and connection.
        let ip_literal = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = ip_literal.parse::<std::net::IpAddr>()
            && !thinclaw_tools_core::is_public_outbound_ip(ip)
        {
            return Err(format!(
                "HTTP request to a private/loopback address is not allowed: {host}"
            ));
        }
        Ok(())
    }

    pub fn record_http_request(&mut self) -> Result<(), String> {
        self.capabilities
            .tool_capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;
        self.http_request_count += 1;
        const MAX_REQUESTS_PER_EXECUTION: u32 = 50;
        if self.http_request_count > MAX_REQUESTS_PER_EXECUTION {
            return Err(format!(
                "Too many HTTP requests in single execution (max {MAX_REQUESTS_PER_EXECUTION})"
            ));
        }
        Ok(())
    }

    pub fn take_logs(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.logs)
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
    /// Serializes the read-modify-persist-publish transaction so concurrent
    /// callback completions cannot overwrite one another with stale snapshots.
    commit_lock: std::sync::Mutex<()>,
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
            commit_lock: std::sync::Mutex::new(()),
        }
    }

    /// Create a workspace store backed by a JSON file on disk.
    ///
    /// Loads existing state from `path` if the file exists. Subsequent
    /// calls to [`commit_writes`] will flush the full store back to disk
    /// only when the serialized content has actually changed.
    pub fn with_persistence(path: std::path::PathBuf) -> Self {
        let (data, initial_snapshot) = match Self::load_from_disk(&path) {
            Ok(Some(map)) => {
                let snapshot = Self::serialize_deterministic(&map).unwrap_or_default();
                (map, snapshot)
            }
            Ok(None) => (std::collections::HashMap::new(), Vec::new()),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "Rejected persisted channel workspace, starting fresh"
                );
                (std::collections::HashMap::new(), Vec::new())
            }
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
            commit_lock: std::sync::Mutex::new(()),
        }
    }

    fn load_from_disk(
        path: &std::path::Path,
    ) -> Result<Option<std::collections::HashMap<String, String>>, String> {
        use std::io::Read as _;

        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot inspect workspace file: {error}")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("workspace persistence path is not a regular file".to_string());
        }
        if metadata.len() > MAX_WORKSPACE_STORE_FILE_BYTES as u64 {
            return Err("workspace persistence file is oversized".to_string());
        }

        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(path)
            .map_err(|error| format!("cannot open workspace file: {error}"))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| format!("cannot inspect opened workspace file: {error}"))?;
        if !opened_metadata.is_file()
            || opened_metadata.len() > MAX_WORKSPACE_STORE_FILE_BYTES as u64
        {
            return Err("opened workspace persistence file is invalid or oversized".to_string());
        }

        let mut bytes = Vec::with_capacity(
            usize::try_from(opened_metadata.len())
                .unwrap_or(MAX_WORKSPACE_STORE_FILE_BYTES)
                .min(MAX_WORKSPACE_STORE_FILE_BYTES),
        );
        std::io::Read::by_ref(&mut file)
            .take((MAX_WORKSPACE_STORE_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read workspace file: {error}"))?;
        if bytes.len() > MAX_WORKSPACE_STORE_FILE_BYTES {
            return Err("workspace persistence file is oversized".to_string());
        }
        let map: std::collections::HashMap<String, String> = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid workspace JSON: {error}"))?;
        Self::validate_store_data(&map)?;
        Ok(Some(map))
    }

    /// Commit pending writes from a callback execution into the store.
    ///
    /// If a persist path is configured, the store is flushed to disk only
    /// when the serialized content has actually changed since the last flush.
    /// The disk flush happens outside the data write lock to avoid blocking
    /// concurrent readers.
    pub fn commit_writes(&self, writes: &[PendingWorkspaceWrite]) -> Result<(), String> {
        if writes.is_empty() {
            return Ok(());
        }
        if writes.len() > MAX_WORKSPACE_WRITES_PER_EXECUTION {
            return Err("workspace write batch exceeds the entry limit".to_string());
        }
        let _commit_guard = self
            .commit_lock
            .lock()
            .map_err(|_| "workspace commit lock is poisoned".to_string())?;
        let mut candidate = self
            .data
            .read()
            .map_err(|_| "workspace data lock is poisoned".to_string())?
            .clone();

        for write in writes {
            Self::validate_write(write)?;
            let value = if Self::is_managed_private_topics_path(&write.path) {
                candidate
                    .get(&write.path)
                    .map(|existing| {
                        Self::merge_managed_private_topic_registry(existing, &write.content)
                    })
                    .unwrap_or_else(|| write.content.clone())
            } else {
                write.content.clone()
            };
            if value.len() > MAX_WORKSPACE_WRITE_SIZE {
                return Err("merged workspace value exceeds the per-entry limit".to_string());
            }
            candidate.insert(write.path.clone(), value);
        }
        Self::validate_store_data(&candidate)?;

        // Persist the complete candidate before publishing it to readers. A
        // disk failure therefore leaves both the durable and in-memory views
        // on the previous successful state.
        if let Some(persist_path) = &self.persist_path {
            self.flush_if_changed(persist_path, &candidate)?;
        }
        *self
            .data
            .write()
            .map_err(|_| "workspace data lock is poisoned".to_string())? = candidate;
        Ok(())
    }

    fn validate_write(write: &PendingWorkspaceWrite) -> Result<(), String> {
        if write.path.is_empty()
            || write.path.len() > MAX_WORKSPACE_PATH_SIZE
            || write.path.starts_with('/')
            || write.path.contains('\\')
            || write.path.chars().any(char::is_control)
            || write
                .path
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err("workspace write path is invalid".to_string());
        }
        if write.content.len() > MAX_WORKSPACE_WRITE_SIZE {
            return Err("workspace write value exceeds the per-entry limit".to_string());
        }
        Ok(())
    }

    fn validate_store_data(data: &std::collections::HashMap<String, String>) -> Result<(), String> {
        if data.len() > MAX_WORKSPACE_STORE_ENTRIES {
            return Err("workspace store exceeds the entry limit".to_string());
        }
        let mut total = 0usize;
        for (path, content) in data {
            Self::validate_write(&PendingWorkspaceWrite {
                path: path.clone(),
                content: content.clone(),
            })?;
            total = total
                .checked_add(path.len())
                .and_then(|value| value.checked_add(content.len()))
                .ok_or_else(|| "workspace store size overflow".to_string())?;
            if total > MAX_WORKSPACE_STORE_BYTES {
                return Err("workspace store exceeds the aggregate byte limit".to_string());
            }
        }
        Ok(())
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
    ) -> Result<(), String> {
        let serialized = Self::serialize_deterministic(data)
            .map_err(|error| format!("cannot serialize workspace store: {error}"))?;
        if serialized.len() > MAX_WORKSPACE_STORE_FILE_BYTES {
            return Err("serialized workspace store exceeds the file limit".to_string());
        }

        // Acquire the snapshot lock before I/O so a poisoned lock cannot leave
        // disk advanced while memory remains on the previous candidate.
        let mut last = self
            .last_flushed
            .lock()
            .map_err(|_| "workspace snapshot lock is poisoned".to_string())?;
        if *last == serialized {
            return Ok(()); // Content unchanged, skip disk I/O
        }

        // Content changed — write to disk atomically
        Self::write_to_disk(path, &serialized)?;

        // Update last-flushed snapshot only after the durable replacement.
        *last = serialized;
        Ok(())
    }

    /// Write serialized bytes to disk atomically (write-tmp + rename).
    fn write_to_disk(path: &std::path::Path, serialized: &[u8]) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create workspace directory: {error}"))?;
        }

        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "workspace persistence path has no valid filename".to_string())?;
        let tmp_path = parent.join(format!(
            ".{file_name}.{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> Result<(), std::io::Error> {
            let mut file = options.open(&tmp_path)?;
            std::io::Write::write_all(&mut file, serialized)?;
            file.sync_all()?;
            std::fs::rename(&tmp_path, path)?;
            if let Ok(directory) = std::fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        result.map_err(|error| format!("cannot persist workspace store: {error}"))
    }
}

impl Default for ChannelWorkspaceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceReader for ChannelWorkspaceStore {
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
    use crate::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
    use crate::wasm::host::{
        ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MAX_EMITS_PER_EXECUTION,
    };

    #[test]
    fn blocked_http_ips_cover_private_loopback_and_metadata() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1", // IPv4-mapped loopback
        ] {
            assert!(
                !thinclaw_tools_core::is_public_outbound_ip(ip.parse().unwrap()),
                "{ip} should be blocked"
            );
        }
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(
                thinclaw_tools_core::is_public_outbound_ip(ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
    }

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
        use crate::wasm::host::WorkspaceReader;
        use crate::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

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
        store.commit_writes(&writes).unwrap();

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
        store.commit_writes(&writes2).unwrap();
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );

        // Empty writes are a no-op
        store.commit_writes(&[]).unwrap();
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );
    }

    #[test]
    fn test_channel_workspace_store_disk_persistence_round_trip() {
        use crate::wasm::host::WorkspaceReader;
        use crate::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

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
            store.commit_writes(&writes).unwrap();

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
        use crate::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("skip_redundant.json");

        let store = ChannelWorkspaceStore::with_persistence(path.clone());

        // First write — should create the file
        let writes = vec![PendingWorkspaceWrite {
            path: "channels/telegram/state/managed_private_topics".to_string(),
            content: r#"{"chats":{"123":{"general_thread_id":42}}}"#.to_string(),
        }];
        store.commit_writes(&writes).unwrap();
        assert!(path.exists());

        let mtime_after_first = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Small sleep to ensure filesystem timestamp granularity
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Second write with identical content — should NOT touch the file
        store.commit_writes(&writes).unwrap();

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
        store.commit_writes(&writes_changed).unwrap();

        let mtime_after_third = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert_ne!(
            mtime_after_first, mtime_after_third,
            "Changed content should update the file"
        );
    }

    #[test]
    fn test_channel_workspace_store_merges_managed_private_topic_registry_updates() {
        use crate::wasm::host::WorkspaceReader;
        use crate::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

        let store = ChannelWorkspaceStore::new();
        let path = "channels/telegram/state/managed_private_topics".to_string();

        store
            .commit_writes(&[PendingWorkspaceWrite {
                path: path.clone(),
                content: r#"{"chats":{"123":{"onboarding_thread_id":61419}}}"#.to_string(),
            }])
            .unwrap();

        store
            .commit_writes(&[PendingWorkspaceWrite {
                path: path.clone(),
                content: r#"{"chats":{"123":{"general_thread_id":7}}}"#.to_string(),
            }])
            .unwrap();

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
        use crate::wasm::host::WorkspaceReader;
        use crate::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};

        let store = ChannelWorkspaceStore::new();
        let path = "channels/telegram/state/managed_private_topics".to_string();

        store
            .commit_writes(&[PendingWorkspaceWrite {
                path: path.clone(),
                content:
                    r#"{"chats":{"123":{"onboarding_thread_id":61419,"general_thread_id":7}}}"#
                        .to_string(),
            }])
            .unwrap();

        store
            .commit_writes(&[PendingWorkspaceWrite {
                path: path.clone(),
                content: r#"{"chats":{"123":{"general_thread_id":null}}}"#.to_string(),
            }])
            .unwrap();

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

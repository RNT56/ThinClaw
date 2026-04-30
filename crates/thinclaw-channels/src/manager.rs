//! Channel manager for coordinating multiple input channels.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use chrono::Utc;
use futures::stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};

use crate::status_view::{ChannelStatusEntry, ChannelViewState};
use thinclaw_channels_core::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
use thinclaw_types::error::ChannelError;

const LEGACY_WEB_CHANNEL_ALIAS: &str = "web";
const GATEWAY_CHANNEL_NAME: &str = "gateway";

/// Descriptor for a native channel surface that the runtime should expose even
/// when a full transport has not landed yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDescriptor {
    pub name: String,
    pub channel_type: String,
    pub enabled: bool,
    pub available: bool,
    pub description: String,
}

impl ChannelDescriptor {
    pub fn native_placeholder(
        name: impl Into<String>,
        enabled: bool,
        available: bool,
        description: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            channel_type: "native-placeholder".to_string(),
            enabled,
            available,
            description: description.into(),
        }
    }
}

/// Raw platform-neutral event shape used by channel drivers before an event is
/// turned into ThinClaw's canonical [`IncomingMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    pub platform: String,
    pub chat_type: String,
    pub chat_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Parsed slash command produced by the centralized channel command parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub command: String,
    pub args: String,
}

/// Gateway-neutral channel lifecycle event emitted by the runtime manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStatusChangeEvent {
    pub channel: String,
    pub status: String,
    pub message: Option<String>,
}

type ChannelStatusSink = Arc<dyn Fn(ChannelStatusChangeEvent) + Send + Sync>;

/// Standard ThinClaw session-key format for gateway and chat platform ingress.
pub fn mint_session_key(platform: &str, chat_type: &str, chat_id: &str) -> String {
    format!(
        "agent:main:{}:{}:{}",
        sanitize_session_key_part(platform),
        sanitize_session_key_part(chat_type),
        sanitize_session_key_part(chat_id)
    )
}

/// Compatibility aliases for persisted sessions created before the unified key
/// format landed. New channel drivers should write only `mint_session_key`.
pub fn legacy_session_key_aliases(platform: &str, chat_type: &str, chat_id: &str) -> Vec<String> {
    let platform = sanitize_session_key_part(platform);
    let chat_type = sanitize_session_key_part(chat_type);
    let chat_id = sanitize_session_key_part(chat_id);
    let mut aliases = vec![
        format!("{platform}:{chat_id}"),
        format!("{platform}:{chat_type}:{chat_id}"),
        format!("agent:main:{platform}:{chat_id}"),
    ];
    aliases.sort();
    aliases.dedup();
    aliases
}

/// Convert a platform-neutral event into the canonical incoming-message shape.
pub fn normalize_incoming_event(event: IncomingEvent) -> IncomingMessage {
    let thread_id = mint_session_key(&event.platform, &event.chat_type, &event.chat_id);
    let aliases = legacy_session_key_aliases(&event.platform, &event.chat_type, &event.chat_id);
    let metadata = serde_json::json!({
        "platform": event.platform.clone(),
        "chat_type": event.chat_type.clone(),
        "chat_id": event.chat_id.clone(),
        "session_key": thread_id.clone(),
        "legacy_session_key_aliases": aliases,
        "raw": event.metadata,
    });

    let mut message = IncomingMessage::new(event.platform, event.user_id, event.text)
        .with_thread(thread_id)
        .with_metadata(metadata);
    if let Some(name) = event.user_name {
        message = message.with_user_name(name);
    }
    message
}

/// Parse a leading slash command without each channel re-implementing prefix
/// handling. Returns `None` for regular user messages.
pub fn parse_slash_command(content: &str) -> Option<SlashCommand> {
    let trimmed = content.trim();
    let rest = trimmed.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (command, args) = rest
        .split_once(char::is_whitespace)
        .map(|(cmd, args)| (cmd, args.trim()))
        .unwrap_or((rest, ""));
    if command.is_empty() {
        return None;
    }
    Some(SlashCommand {
        command: command.to_ascii_lowercase(),
        args: args.to_string(),
    })
}

fn sanitize_session_key_part(value: &str) -> String {
    let cleaned = value
        .trim()
        .chars()
        .map(|ch| match ch {
            ':' | '\n' | '\r' | '\t' => '_',
            _ => ch,
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// Per-channel atomic message counters.
struct ChannelCounters {
    received: AtomicU64,
    sent: AtomicU64,
    errors: AtomicU64,
    last_message_at: std::sync::RwLock<Option<String>>,
    last_error: std::sync::RwLock<Option<String>>,
    last_error_at: std::sync::RwLock<Option<String>>,
}

impl Default for ChannelCounters {
    fn default() -> Self {
        Self {
            received: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_message_at: std::sync::RwLock::new(None),
            last_error: std::sync::RwLock::new(None),
            last_error_at: std::sync::RwLock::new(None),
        }
    }
}

/// Manages multiple input channels and merges their message streams.
///
/// Includes an injection channel so background tasks (e.g., job monitors) can
/// push messages into the agent loop without being a full `Channel` impl.
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Box<dyn Channel>>>>,
    descriptors: Arc<RwLock<HashMap<String, ChannelDescriptor>>>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    /// Taken once in `start_all()` and merged into the stream.
    inject_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Per-channel message counters (received/sent/errors).
    counters: Arc<RwLock<HashMap<String, Arc<ChannelCounters>>>>,
    /// Time when the manager was created (for uptime calculation).
    started_at: Instant,
    /// Optional gateway adapter for channel status change events.
    status_sink: RwLock<Option<ChannelStatusSink>>,
}

impl ChannelManager {
    /// Create a new channel manager.
    pub fn new() -> Self {
        let (inject_tx, inject_rx) = mpsc::channel(64);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            descriptors: Arc::new(RwLock::new(HashMap::new())),
            inject_tx,
            inject_rx: tokio::sync::Mutex::new(Some(inject_rx)),
            counters: Arc::new(RwLock::new(HashMap::new())),
            started_at: Instant::now(),
            status_sink: RwLock::new(None),
        }
    }

    /// Install a gateway-neutral status change sink.
    pub async fn set_status_change_sink<F>(&self, sink: F)
    where
        F: Fn(ChannelStatusChangeEvent) + Send + Sync + 'static,
    {
        *self.status_sink.write().await = Some(Arc::new(sink));
    }

    /// Compatibility hook for older root wiring. Gateway-specific SSE mapping
    /// is intentionally kept outside this runtime crate.
    pub async fn set_sse_sender<T>(&self, _tx: tokio::sync::broadcast::Sender<T>)
    where
        T: Clone + Send + Sync + 'static,
    {
    }

    async fn emit_channel_status_change(
        &self,
        channel: impl Into<String>,
        status: impl Into<String>,
        message: Option<String>,
    ) {
        let sink = self.status_sink.read().await.clone();
        if let Some(sink) = sink {
            sink(ChannelStatusChangeEvent {
                channel: channel.into(),
                status: status.into(),
                message,
            });
        }
    }

    /// Get or create the counter entry for a channel (lock-free fast path via read).
    async fn counter_for(&self, name: &str) -> Arc<ChannelCounters> {
        // Fast path: counter already exists.
        {
            let guard = self.counters.read().await;
            if let Some(c) = guard.get(name) {
                return Arc::clone(c);
            }
        }
        // Slow path: insert new counter.
        let mut guard = self.counters.write().await;
        guard
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(ChannelCounters::default()))
            .clone()
    }

    fn record_channel_error(counter: &ChannelCounters, error: &ChannelError) {
        let failed_at = Utc::now().to_rfc3339();
        counter.errors.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut guard) = counter.last_error.write() {
            *guard = Some(error.to_string());
        }
        if let Ok(mut guard) = counter.last_error_at.write() {
            *guard = Some(failed_at);
        }
    }

    fn clear_channel_error(counter: &ChannelCounters) {
        if let Ok(mut guard) = counter.last_error.write() {
            *guard = None;
        }
        if let Ok(mut guard) = counter.last_error_at.write() {
            *guard = None;
        }
    }

    fn resolve_channel_name<'a>(
        requested: &'a str,
        channels: &'a HashMap<String, Box<dyn Channel>>,
    ) -> &'a str {
        if channels.contains_key(requested) {
            requested
        } else if requested == LEGACY_WEB_CHANNEL_ALIAS
            && channels.contains_key(GATEWAY_CHANNEL_NAME)
        {
            GATEWAY_CHANNEL_NAME
        } else {
            requested
        }
    }

    /// Get a clone of the injection sender.
    ///
    /// Background tasks (like job monitors) use this to push messages into the
    /// agent loop without being a full `Channel` implementation.
    pub fn inject_sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.inject_tx.clone()
    }

    /// Add a channel to the manager.
    pub async fn add(&self, channel: Box<dyn Channel>) {
        let name = channel.name().to_string();
        self.channels.write().await.insert(name.clone(), channel);
        let _ = self.counter_for(&name).await;
        tracing::debug!("Added channel: {}", name);
    }

    /// Add or update a channel descriptor that is visible in status output even
    /// when there is no active `Channel` transport registered.
    pub async fn add_descriptor(&self, descriptor: ChannelDescriptor) {
        let name = descriptor.name.clone();
        self.descriptors
            .write()
            .await
            .insert(name.clone(), descriptor);
        let _ = self.counter_for(&name).await;
        tracing::debug!("Added channel descriptor: {}", name);
    }

    /// Hot-add a channel to a running agent.
    ///
    /// Starts the channel, registers it in the channels map for `respond()`/`broadcast()`,
    /// and spawns a task that forwards its stream messages through `inject_tx` into
    /// the agent loop.
    pub async fn hot_add(&self, channel: Box<dyn Channel>) -> Result<(), ChannelError> {
        let name = channel.name().to_string();
        let stream = channel.start().await?;

        // Register for respond/broadcast/send_status
        self.channels.write().await.insert(name.clone(), channel);
        let _ = self.counter_for(&name).await;

        // Forward stream messages through inject_tx
        let tx = self.inject_tx.clone();
        let spawn_name = name.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                if tx.send(msg).await.is_err() {
                    tracing::warn!(channel = %spawn_name, "Inject channel closed, stopping hot-added channel");
                    break;
                }
            }
            tracing::info!(channel = %spawn_name, "Hot-added channel stream ended");
        });

        self.emit_channel_status_change(
            name.clone(),
            "online",
            Some(format!("Channel '{}' activated", name)),
        )
        .await;

        Ok(())
    }

    /// Hot-remove a channel from a running agent.
    ///
    /// Shuts down the channel and removes it from the channels map.
    /// The channel's stream task will end naturally when the channel is dropped.
    pub async fn hot_remove(&self, name: &str) -> Result<(), ChannelError> {
        let channel = self.channels.write().await.remove(name);

        if let Some(channel) = channel {
            if let Err(e) = channel.shutdown().await {
                tracing::warn!(channel = %name, error = %e, "Error shutting down hot-removed channel");
            }

            // Clean up counters
            self.counters.write().await.remove(name);

            self.emit_channel_status_change(
                name.to_string(),
                "removed",
                Some(format!("Channel '{}' removed", name)),
            )
            .await;

            tracing::info!(channel = %name, "Hot-removed channel");
            Ok(())
        } else {
            tracing::debug!(channel = %name, "Channel not found for hot-remove (may not have been active)");
            Ok(())
        }
    }

    /// Start all channels and return a merged stream of messages.
    ///
    /// Also merges the injection channel so background tasks can push messages
    /// into the same stream.
    pub async fn start_all(&self) -> Result<MessageStream, ChannelError> {
        let channels = self.channels.read().await;
        let mut streams: Vec<MessageStream> = Vec::new();

        for (name, channel) in channels.iter() {
            match channel.start().await {
                Ok(stream) => {
                    tracing::info!("Started channel: {}", name);
                    streams.push(stream);
                }
                Err(e) => {
                    tracing::error!("Failed to start channel {}: {}", name, e);
                    // Continue with other channels, don't fail completely
                }
            }
        }

        if streams.is_empty() {
            return Err(ChannelError::StartupFailed {
                name: "all".to_string(),
                reason: "No channels started successfully".to_string(),
            });
        }

        // Take the injection receiver (can only be taken once)
        if let Some(inject_rx) = self.inject_rx.lock().await.take() {
            let inject_stream = tokio_stream::wrappers::ReceiverStream::new(inject_rx);
            streams.push(Box::pin(inject_stream));
            tracing::debug!("Injection channel merged into message stream");
        }

        // Merge all streams into one
        let merged = stream::select_all(streams);
        Ok(Box::pin(merged))
    }

    /// Increment the received counter for an incoming message's channel.
    ///
    /// Call this once per message received from a channel before processing.
    pub async fn record_received(&self, channel_name: &str) {
        let counter = self.counter_for(channel_name).await;
        counter.received.fetch_add(1, Ordering::Relaxed);
        Self::clear_channel_error(counter.as_ref());
        if let Ok(mut guard) = counter.last_message_at.write() {
            *guard = Some(Utc::now().to_rfc3339());
        }
    }

    /// Send a response to a specific channel.
    pub async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let requested_channel_name = msg.channel.clone();
        let resolved_channel_name = {
            let channels = self.channels.read().await;
            Self::resolve_channel_name(&requested_channel_name, &channels).to_string()
        };
        let result = {
            let channels = self.channels.read().await;
            if let Some(channel) = channels.get(&resolved_channel_name) {
                channel.respond(msg, response).await
            } else {
                return Err(ChannelError::SendFailed {
                    name: requested_channel_name,
                    reason: "Channel not found".to_string(),
                });
            }
        }; // lock guard drops here
        let counter = self.counter_for(&resolved_channel_name).await;
        if result.is_ok() {
            counter.sent.fetch_add(1, Ordering::Relaxed);
            Self::clear_channel_error(counter.as_ref());
        } else if let Err(ref err) = result {
            Self::record_channel_error(counter.as_ref(), err);
        }
        result
    }

    /// Send a status update to a specific channel.
    ///
    /// The metadata contains channel-specific routing info (e.g., Telegram chat_id)
    /// needed to deliver the status to the correct destination.
    pub async fn send_status(
        &self,
        channel_name: &str,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        let channel_name = Self::resolve_channel_name(channel_name, &channels);
        if let Some(channel) = channels.get(channel_name) {
            channel.send_status(status, metadata).await
        } else {
            // Silently ignore if channel not found (status is best-effort)
            Ok(())
        }
    }

    /// Broadcast a message to a specific user on a specific channel.
    ///
    /// Used for proactive notifications like heartbeat alerts.
    pub async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let resolved_channel_name = {
            let channels = self.channels.read().await;
            Self::resolve_channel_name(channel_name, &channels).to_string()
        };
        let result = {
            let channels = self.channels.read().await;
            if let Some(channel) = channels.get(&resolved_channel_name) {
                channel.broadcast(user_id, response).await
            } else {
                return Err(ChannelError::SendFailed {
                    name: channel_name.to_string(),
                    reason: "Channel not found".to_string(),
                });
            }
        }; // lock drops here
        let counter = self.counter_for(&resolved_channel_name).await;
        if result.is_ok() {
            counter.sent.fetch_add(1, Ordering::Relaxed);
        } else if let Err(ref err) = result {
            Self::record_channel_error(counter.as_ref(), err);
        }
        result
    }

    /// Broadcast a message to all channels.
    ///
    /// Sends to the specified user on every registered channel.
    pub async fn broadcast_all(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Vec<(String, Result<(), ChannelError>)> {
        let names: Vec<String> = self.channels.read().await.keys().cloned().collect();
        let mut results = Vec::new();

        for name in &names {
            let result = {
                let channels = self.channels.read().await;
                if let Some(channel) = channels.get(name.as_str()) {
                    channel.broadcast(user_id, response.clone()).await
                } else {
                    continue;
                }
            };
            let counter = self.counter_for(name).await;
            if result.is_ok() {
                counter.sent.fetch_add(1, Ordering::Relaxed);
            } else if let Err(ref err) = result {
                Self::record_channel_error(counter.as_ref(), err);
            }
            results.push((name.clone(), result));
        }

        results
    }

    /// Check health of all channels.
    pub async fn health_check_all(&self) -> HashMap<String, Result<(), ChannelError>> {
        let channels = self.channels.read().await;
        let mut results = HashMap::new();

        for (name, channel) in channels.iter() {
            results.insert(name.clone(), channel.health_check().await);
        }

        results
    }

    /// Shutdown all channels.
    pub async fn shutdown_all(&self) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            if let Err(e) = channel.shutdown().await {
                tracing::error!("Error shutting down channel {}: {}", name, e);
            }
        }
        Ok(())
    }

    /// Get list of channel names.
    pub async fn channel_names(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }

    pub async fn channel_descriptors(&self) -> Vec<ChannelDescriptor> {
        self.descriptors.read().await.values().cloned().collect()
    }

    /// Return formatting guidance from the active channel implementation.
    pub async fn formatting_hints_for(&self, channel_name: &str) -> Option<String> {
        let channels = self.channels.read().await;
        let channel_name = Self::resolve_channel_name(channel_name, &channels);
        channels
            .get(channel_name)
            .and_then(|channel| channel.formatting_hints())
    }

    /// Return channel-specific diagnostics when the implementation exposes them.
    pub async fn channel_diagnostics(&self, channel_name: &str) -> Option<serde_json::Value> {
        let channels = self.channels.read().await;
        let channel_name = Self::resolve_channel_name(channel_name, &channels);
        let channel = channels.get(channel_name)?;
        channel.diagnostics().await
    }

    /// Return live `ChannelStatusEntry` list for `openclaw_channel_status_list`.
    ///
    /// Combines channel names with real atomic counters and uptime.
    /// State is derived: channels that exist and have been started are "Running".
    pub async fn status_entries(&self) -> Vec<ChannelStatusEntry> {
        let uptime_secs = self.started_at.elapsed().as_secs();
        let names = self.channel_names().await;
        let counters_guard = self.counters.read().await;
        let descriptors = self.channel_descriptors().await;
        let mut entries = Vec::with_capacity(names.len() + descriptors.len());
        for name in &names {
            let (received, sent, errors) = if let Some(c) = counters_guard.get(name.as_str()) {
                (
                    c.received.load(Ordering::Relaxed),
                    c.sent.load(Ordering::Relaxed),
                    c.errors.load(Ordering::Relaxed) as u32,
                )
            } else {
                (0, 0, 0)
            };

            let (last_message_at, last_error, last_error_at) =
                if let Some(c) = counters_guard.get(name.as_str()) {
                    (
                        c.last_message_at
                            .read()
                            .ok()
                            .and_then(|guard| guard.clone()),
                        c.last_error.read().ok().and_then(|guard| guard.clone()),
                        c.last_error_at.read().ok().and_then(|guard| guard.clone()),
                    )
                } else {
                    (None, None, None)
                };
            let state = if let (Some(error), Some(failed_at)) =
                (last_error.clone(), last_error_at.clone())
            {
                ChannelViewState::Failed { error, failed_at }
            } else {
                ChannelViewState::Running { uptime_secs }
            };

            entries.push(ChannelStatusEntry {
                name: name.clone(),
                channel_type: name.clone(),
                state,
                last_message_at,
                last_error,
                messages_received: received,
                messages_sent: sent,
                errors,
            });
        }
        for descriptor in descriptors {
            if names.iter().any(|name| name == &descriptor.name) {
                continue;
            }
            let (received, sent, errors) =
                if let Some(c) = counters_guard.get(descriptor.name.as_str()) {
                    (
                        c.received.load(Ordering::Relaxed),
                        c.sent.load(Ordering::Relaxed),
                        c.errors.load(Ordering::Relaxed) as u32,
                    )
                } else {
                    (0, 0, 0)
                };
            let last_error = match (descriptor.enabled, descriptor.available) {
                (true, true) => Some(format!(
                    "{} is configured as a native lifecycle placeholder; transport not implemented yet",
                    descriptor.description
                )),
                (true, false) => Some(format!(
                    "{} is configured, but this build does not include the required feature",
                    descriptor.description
                )),
                (false, _) => None,
            };
            entries.push(ChannelStatusEntry {
                name: descriptor.name,
                channel_type: descriptor.channel_type,
                state: ChannelViewState::Disabled,
                last_message_at: None,
                last_error,
                messages_received: received,
                messages_sent: sent,
                errors,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Get the stream mode for a specific channel.
    ///
    /// Returns `StreamMode::None` if the channel is not found.
    pub async fn stream_mode(&self, channel_name: &str) -> StreamMode {
        let channels = self.channels.read().await;
        channels
            .get(channel_name)
            .map(|c| c.stream_mode())
            .unwrap_or_default()
    }

    /// Send a streaming draft update to a specific channel.
    ///
    /// Returns the platform message ID for subsequent edits.
    pub async fn send_draft(
        &self,
        channel_name: &str,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.send_draft(draft, metadata).await
        } else {
            Ok(None)
        }
    }

    /// Delete a previously sent message (best-effort).
    ///
    /// Used by the streaming fallback path to remove partial streaming
    /// messages before resending the complete response via `on_respond()`.
    pub async fn delete_message(
        &self,
        channel_name: &str,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.delete_message(message_id, metadata).await
        } else {
            Ok(())
        }
    }

    /// Update the stream mode for a specific channel at runtime.
    ///
    /// This allows the WebUI to change telegram streaming mode without restart.
    pub async fn set_channel_stream_mode(&self, channel_name: &str, mode: StreamMode) {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.set_stream_mode(mode).await;
        } else {
            tracing::debug!(
                channel = %channel_name,
                "Cannot set stream mode: channel not found"
            );
        }
    }

    /// Update channel-specific runtime config values before an in-place restart.
    pub async fn update_channel_runtime_config(
        &self,
        channel_name: &str,
        updates: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        let Some(channel) = channels.get(channel_name) else {
            return Err(ChannelError::SendFailed {
                name: channel_name.to_string(),
                reason: "Channel not found".to_string(),
            });
        };
        channel.update_runtime_config(updates).await;
        Ok(())
    }

    /// Clear transient connection state before a manual reconnect.
    pub async fn reset_channel_connection_state(
        &self,
        channel_name: &str,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        let Some(channel) = channels.get(channel_name) else {
            return Err(ChannelError::SendFailed {
                name: channel_name.to_string(),
                reason: "Channel not found".to_string(),
            });
        };
        channel.reset_connection_state().await
    }

    /// Restart a channel in-place: shutdown → re-start → merge new stream.
    ///
    /// The channel stays registered in the map so `respond()`/`broadcast()`
    /// continue to work. Only the underlying transport is recycled.
    ///
    /// Used by `ChannelHealthMonitor` for auto-restart after consecutive failures.
    pub async fn restart_channel(&self, name: &str) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        let Some(channel) = channels.get(name) else {
            return Err(ChannelError::SendFailed {
                name: name.to_string(),
                reason: "Channel not found".to_string(),
            });
        };

        // Shutdown the old transport (best-effort).
        if let Err(e) = channel.shutdown().await {
            tracing::warn!(
                channel = %name,
                error = %e,
                "Error shutting down channel during restart (continuing)"
            );
        }

        // Re-start to get a fresh stream.
        let stream = channel.start().await.map_err(|e| {
            tracing::error!(channel = %name, error = %e, "Failed to restart channel");
            e
        })?;

        // Drop the read guard before spawning (we don't need it anymore).
        drop(channels);

        // Forward the new stream through inject_tx.
        let tx = self.inject_tx.clone();
        let spawn_name = name.to_string();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                if tx.send(msg).await.is_err() {
                    tracing::warn!(
                        channel = %spawn_name,
                        "Inject channel closed, stopping restarted channel"
                    );
                    break;
                }
            }
            tracing::info!(channel = %spawn_name, "Restarted channel stream ended");
        });

        self.emit_channel_status_change(
            name.to_string(),
            "online",
            Some(format!("Channel '{}' restarted", name)),
        )
        .await;

        // Reset error counter on successful restart.
        self.counters.write().await.remove(name);

        tracing::info!(channel = %name, "Channel restarted successfully");
        Ok(())
    }

    /// Toggle debug mode on a specific channel.
    ///
    /// Returns the new debug state (`true` = on, `false` = off).
    /// For channels that don't support debug mode (e.g., REPL), returns `false`.
    pub async fn toggle_debug_mode(&self, channel_name: &str) -> bool {
        let channels = self.channels.read().await;
        let channel_name = Self::resolve_channel_name(channel_name, &channels);
        if let Some(channel) = channels.get(channel_name) {
            channel.toggle_debug_mode().await
        } else {
            false
        }
    }
}

#[cfg(test)]
mod normalization_tests {
    use super::*;

    #[test]
    fn mints_standard_session_key_and_aliases() {
        assert_eq!(
            mint_session_key("matrix", "room", "!abc:def"),
            "agent:main:matrix:room:!abc_def"
        );
        let aliases = legacy_session_key_aliases("matrix", "room", "!abc:def");
        assert!(aliases.contains(&"matrix:!abc_def".to_string()));
        assert!(aliases.contains(&"matrix:room:!abc_def".to_string()));
    }

    #[test]
    fn normalizes_incoming_event_with_metadata() {
        let message = normalize_incoming_event(IncomingEvent {
            platform: "sms".to_string(),
            chat_type: "dm".to_string(),
            chat_id: "+15551234567".to_string(),
            user_id: "+15551234567".to_string(),
            user_name: Some("Pat".to_string()),
            text: "/help please".to_string(),
            metadata: serde_json::json!({ "provider": "twilio" }),
        });
        assert_eq!(message.channel, "sms");
        assert_eq!(
            message.thread_id.as_deref(),
            Some("agent:main:sms:dm:+15551234567")
        );
        assert_eq!(
            parse_slash_command(&message.content).unwrap().command,
            "help"
        );
        assert_eq!(message.metadata["raw"]["provider"], "twilio");
    }

    #[test]
    fn native_lifecycle_placeholders_use_shared_ingress_helpers() {
        for (platform, chat_type, chat_id) in [
            ("matrix", "room", "!room:example.org"),
            ("voice-call", "call", "call-123"),
            ("apns", "device", "device-token"),
            ("browser-push", "subscription", "endpoint-123"),
        ] {
            let expected = mint_session_key(platform, chat_type, chat_id);
            let message = normalize_incoming_event(IncomingEvent {
                platform: platform.to_string(),
                chat_type: chat_type.to_string(),
                chat_id: chat_id.to_string(),
                user_id: "user-1".to_string(),
                user_name: None,
                text: "/status now".to_string(),
                metadata: serde_json::json!({"placeholder": true}),
            });

            assert_eq!(message.channel, platform);
            assert_eq!(message.thread_id.as_deref(), Some(expected.as_str()));
            assert_eq!(
                message.metadata["session_key"].as_str(),
                Some(expected.as_str())
            );
            assert!(
                message
                    .metadata
                    .get("legacy_session_key_aliases")
                    .and_then(|value| value.as_array())
                    .is_some_and(|aliases| !aliases.is_empty())
            );
            let command = parse_slash_command(&message.content)
                .expect("placeholder ingress should use shared slash parsing");
            assert_eq!(command.command, "status");
            assert_eq!(command.args, "now");
        }
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockChannelState {
        broadcasts: Mutex<Vec<(String, String)>>,
        diagnostics_calls: Mutex<usize>,
    }

    struct MockChannel {
        name: String,
        state: Arc<MockChannelState>,
    }

    impl MockChannel {
        fn new(name: &str, state: Arc<MockChannelState>) -> Self {
            Self {
                name: name.to_string(),
                state,
            }
        }
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Ok(Box::pin(stream::empty()))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        async fn broadcast(
            &self,
            user_id: &str,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            self.state
                .broadcasts
                .lock()
                .await
                .push((user_id.to_string(), response.content));
            Ok(())
        }

        async fn diagnostics(&self) -> Option<serde_json::Value> {
            let mut calls = self.state.diagnostics_calls.lock().await;
            *calls += 1;
            Some(serde_json::json!({"channel": self.name}))
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn broadcast_resolves_legacy_web_alias_to_gateway() {
        let manager = ChannelManager::new();
        let state = Arc::new(MockChannelState::default());
        manager
            .add(Box::new(MockChannel::new("gateway", Arc::clone(&state))))
            .await;

        manager
            .broadcast("web", "user-1", OutgoingResponse::text("hello"))
            .await
            .expect("legacy web alias should reach gateway channel");

        let broadcasts = state.broadcasts.lock().await;
        assert_eq!(
            broadcasts.as_slice(),
            &[("user-1".to_string(), "hello".to_string())]
        );
    }

    #[tokio::test]
    async fn channel_diagnostics_resolves_legacy_web_alias_to_gateway() {
        let manager = ChannelManager::new();
        let state = Arc::new(MockChannelState::default());
        manager
            .add(Box::new(MockChannel::new("gateway", Arc::clone(&state))))
            .await;

        let diagnostics = manager
            .channel_diagnostics("web")
            .await
            .expect("legacy web alias should resolve diagnostics");
        assert_eq!(
            diagnostics.get("channel").and_then(|value| value.as_str()),
            Some("gateway")
        );
        assert_eq!(*state.diagnostics_calls.lock().await, 1);
    }

    #[tokio::test]
    async fn status_entries_include_native_placeholders_without_shadowing_active_channels() {
        let manager = ChannelManager::new();
        manager
            .add_descriptor(ChannelDescriptor::native_placeholder(
                "matrix",
                true,
                true,
                "Matrix rooms and DMs",
            ))
            .await;
        manager
            .add_descriptor(ChannelDescriptor::native_placeholder(
                "gateway",
                true,
                true,
                "Gateway placeholder should be shadowed",
            ))
            .await;
        manager
            .add(Box::new(MockChannel::new(
                "gateway",
                Arc::new(MockChannelState::default()),
            )))
            .await;

        let entries = manager.status_entries().await;
        let matrix = entries
            .iter()
            .find(|entry| entry.name == "matrix")
            .expect("matrix placeholder should be visible");
        assert_eq!(matrix.channel_type, "native-placeholder");
        assert_eq!(matrix.state, ChannelViewState::Disabled);
        assert!(
            matrix
                .last_error
                .as_deref()
                .is_some_and(|err| err.contains("placeholder"))
        );

        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.name == "gateway")
                .count(),
            1
        );
        let gateway = entries
            .iter()
            .find(|entry| entry.name == "gateway")
            .expect("active gateway entry should remain");
        assert_eq!(gateway.channel_type, "gateway");
    }
}

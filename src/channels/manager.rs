//! Channel manager for coordinating multiple input channels.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use chrono::Utc;
use futures::stream;
use tokio::sync::{RwLock, mpsc};

use crate::channels::status_view::{ChannelStatusEntry, ChannelViewState};
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

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
    inject_tx: mpsc::Sender<IncomingMessage>,
    /// Taken once in `start_all()` and merged into the stream.
    inject_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Per-channel message counters (received/sent/errors).
    counters: Arc<RwLock<HashMap<String, Arc<ChannelCounters>>>>,
    /// Time when the manager was created (for uptime calculation).
    started_at: Instant,
    /// Optional SSE broadcast sender for channel status change events.
    sse_tx: RwLock<Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>>,
}

impl ChannelManager {
    /// Create a new channel manager.
    pub fn new() -> Self {
        let (inject_tx, inject_rx) = mpsc::channel(64);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            inject_tx,
            inject_rx: tokio::sync::Mutex::new(Some(inject_rx)),
            counters: Arc::new(RwLock::new(HashMap::new())),
            started_at: Instant::now(),
            sse_tx: RwLock::new(None),
        }
    }

    /// Set the SSE broadcast sender for channel status events.
    pub async fn set_sse_sender(
        &self,
        tx: tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>,
    ) {
        *self.sse_tx.write().await = Some(tx);
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

        // Emit channel status change event
        {
            let guard = self.sse_tx.read().await;
            if let Some(ref tx) = *guard {
                let _ = tx.send(crate::channels::web::types::SseEvent::ChannelStatusChange {
                    channel: name.clone(),
                    status: "online".to_string(),
                    message: Some(format!("Channel '{}' activated", name)),
                });
            }
        }

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

            // Emit channel status change event
            {
                let guard = self.sse_tx.read().await;
                if let Some(ref tx) = *guard {
                    let _ = tx.send(crate::channels::web::types::SseEvent::ChannelStatusChange {
                        channel: name.to_string(),
                        status: "removed".to_string(),
                        message: Some(format!("Channel '{}' removed", name)),
                    });
                }
            }

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
        let channel_name = msg.channel.clone();
        let result = {
            let channels = self.channels.read().await;
            if let Some(channel) = channels.get(&channel_name) {
                channel.respond(msg, response).await
            } else {
                return Err(ChannelError::SendFailed {
                    name: channel_name,
                    reason: "Channel not found".to_string(),
                });
            }
        }; // lock guard drops here
        let counter = self.counter_for(&channel_name).await;
        if result.is_ok() {
            counter.sent.fetch_add(1, Ordering::Relaxed);
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
        let result = {
            let channels = self.channels.read().await;
            if let Some(channel) = channels.get(channel_name) {
                channel.broadcast(user_id, response).await
            } else {
                return Err(ChannelError::SendFailed {
                    name: channel_name.to_string(),
                    reason: "Channel not found".to_string(),
                });
            }
        }; // lock drops here
        let counter = self.counter_for(channel_name).await;
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

    /// Return formatting guidance from the active channel implementation.
    pub async fn formatting_hints_for(&self, channel_name: &str) -> Option<String> {
        self.channels
            .read()
            .await
            .get(channel_name)
            .and_then(|channel| channel.formatting_hints())
    }

    /// Return channel-specific diagnostics when the implementation exposes them.
    pub async fn channel_diagnostics(&self, channel_name: &str) -> Option<serde_json::Value> {
        let channels = self.channels.read().await;
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
        let mut entries = Vec::with_capacity(names.len());
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
        entries
    }

    /// Get the stream mode for a specific channel.
    ///
    /// Returns `StreamMode::None` if the channel is not found.
    pub async fn stream_mode(&self, channel_name: &str) -> crate::channels::StreamMode {
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
        draft: &crate::channels::DraftReplyState,
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
    pub async fn set_channel_stream_mode(
        &self,
        channel_name: &str,
        mode: crate::channels::StreamMode,
    ) {
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

        // Emit channel status change event.
        {
            let guard = self.sse_tx.read().await;
            if let Some(ref tx) = *guard {
                let _ = tx.send(crate::channels::web::types::SseEvent::ChannelStatusChange {
                    channel: name.to_string(),
                    status: "online".to_string(),
                    message: Some(format!("Channel '{}' restarted", name)),
                });
            }
        }

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
        if let Some(channel) = channels.get(channel_name) {
            channel.toggle_debug_mode().await
        } else {
            false
        }
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

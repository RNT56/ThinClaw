//! WASM channel wrapper implementing the Channel trait.
//!
//! Wraps a prepared WASM channel module and provides the Channel interface.
//! Each callback (on_start, on_http_request, on_poll, on_respond) creates
//! a fresh WASM instance for isolation.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                    WasmChannel                               │
//! │                                                              │
//! │   ┌─────────────┐   call_on_*   ┌──────────────────────┐    │
//! │   │   Channel   │ ────────────> │   execute_callback   │    │
//! │   │    Trait    │               │   (fresh instance)   │    │
//! │   └─────────────┘               └──────────┬───────────┘    │
//! │                                            │                 │
//! │                                            ▼                 │
//! │   ┌──────────────────────────────────────────────────────┐  │
//! │   │               ChannelStoreData                       │  │
//! │   │  ┌─────────────┐  ┌──────────────────────────────┐   │  │
//! │   │  │   limiter   │  │      ChannelHostState        │   │  │
//! │   │  └─────────────┘  │  - emitted_messages          │   │  │
//! │   │                   │  - pending_writes            │   │  │
//! │   │                   │  - base HostState (logging)  │   │  │
//! │   │                   └──────────────────────────────┘   │  │
//! │   └──────────────────────────────────────────────────────┘  │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module layout
//!
//! This façade owns the [`WasmChannel`] type, its construction/config surface,
//! and the [`Channel`] / [`SharedWasmChannel`] trait wiring. The heavier
//! concerns are split into focused submodules:
//!
//! - [`store`]: per-execution [`ChannelStoreData`](store::ChannelStoreData) and
//!   the generated channel-host bindings.
//! - [`callbacks`]: channel-agnostic WASM callback execution, typing/polling
//!   tasks, and status dispatch.
//! - [`conversions`]: WIT <-> host type conversions, message normalization, and
//!   the public [`HttpResponse`] type.
//! - [`telegram_transport`]: Telegram-specific direct-API behavior behind the
//!   [`WasmChannelTransport`](telegram_transport::WasmChannelTransport) adapter.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::pairing::PairingStore;
use crate::wasm::capabilities::ChannelCapabilities;
use crate::wasm::error::WasmChannelError;
use crate::wasm::host::{ChannelEmitRateLimiter, ChannelWorkspaceStore};
use crate::wasm::router::RegisteredEndpoint;
use crate::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime};
use crate::wasm::schema::ChannelConfig;
use thinclaw_channels_core::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
use thinclaw_types::error::ChannelError;

use store::ToolEventEntry;
use telegram_transport::WasmChannelTransport;

mod callbacks;
mod conversions;
mod store;
mod telegram_transport;

#[cfg(test)]
mod tests;

pub use conversions::HttpResponse;

/// Version of the channel WIT contract (`near:agent` package in
/// `wit/channel.wit`) used for host/artifact capability negotiation. Bumped on
/// additive contract changes such as new `status-type` variants. Must stay in
/// sync with the `@x.y.z` version on the WIT package declaration.
pub const CHANNEL_WIT_VERSION: &str = "0.2.0";

// Generate component model bindings from the WIT file
wasmtime::component::bindgen!({
    path: "../../wit/channel.wit",
    world: "sandboxed-channel",
    with: {
        // Use our own store data type
    },
});

// Type aliases for the generated WIT types (exported interface)
use exports::near::agent::channel as wit_channel;

/// A WASM-based channel implementing the Channel trait.
#[allow(dead_code)]
pub struct WasmChannel {
    /// Channel name.
    name: String,

    /// Runtime for WASM execution.
    runtime: Arc<WasmChannelRuntime>,

    /// Prepared module (compiled WASM).
    prepared: Arc<PreparedChannelModule>,

    /// Channel capabilities.
    capabilities: ChannelCapabilities,

    /// Channel configuration JSON (passed to on_start).
    /// Wrapped in RwLock to allow updating before start.
    config_json: RwLock<String>,

    /// Channel configuration returned by on_start.
    channel_config: RwLock<Option<ChannelConfig>>,

    /// Optional platform formatting guidance exposed to prompt assembly.
    formatting_hints: Option<String>,

    /// Message sender (for emitting messages to the stream).
    /// Wrapped in Arc for sharing with the polling task.
    message_tx: Arc<RwLock<Option<mpsc::Sender<IncomingMessage>>>>,

    /// Pending responses (for synchronous response handling).
    pending_responses: RwLock<HashMap<Uuid, oneshot::Sender<String>>>,

    /// Rate limiter for message emission.
    /// Wrapped in Arc for sharing with the polling task.
    rate_limiter: Arc<RwLock<ChannelEmitRateLimiter>>,

    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Polling shutdown signal sender (keeps polling alive while held).
    poll_shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Registered HTTP endpoints.
    endpoints: RwLock<Vec<RegisteredEndpoint>>,

    /// Injected credentials for HTTP requests (e.g., bot tokens).
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    /// Wrapped in Arc for sharing with the polling task.
    credentials: Arc<RwLock<HashMap<String, String>>>,

    /// Background task that repeats typing indicators every 4 seconds.
    /// Telegram's "typing..." indicator expires after ~5s, so we refresh it.
    typing_task: RwLock<Option<tokio::task::JoinHandle<()>>>,

    /// Pairing store for DM pairing (guest access control).
    pairing_store: Arc<PairingStore>,

    /// In-memory workspace store persisting writes across callback invocations.
    /// Ensures WASM channels can maintain state (e.g., polling offsets) between ticks.
    workspace_store: Arc<ChannelWorkspaceStore>,

    /// Stream mode for progressive message rendering via sendMessageDraft.
    /// Configured via TELEGRAM_STREAM_MODE env var or DB setting. Default: None.
    /// Wrapped in RwLock for runtime hot-reload via WebUI settings.
    stream_mode: std::sync::RwLock<StreamMode>,

    /// When true, forward verbose status events (tool calls, subagent
    /// lifecycle, canvas actions) to the channel. When false (default),
    /// these events are silently suppressed to keep chat clean. Toggle
    /// at runtime via `/debug`.
    debug_mode: std::sync::RwLock<bool>,

    /// Accumulated tool events for batched delivery in debug mode.
    /// Flushed as a single formatted summary before each response.
    pending_tool_events: tokio::sync::RwLock<Vec<ToolEventEntry>>,
}

impl WasmChannel {
    /// Create a new WASM channel.
    pub fn new(
        runtime: Arc<WasmChannelRuntime>,
        prepared: Arc<PreparedChannelModule>,
        capabilities: ChannelCapabilities,
        config_json: String,
        formatting_hints: Option<String>,
        pairing_store: Arc<PairingStore>,
    ) -> Self {
        let name = prepared.name.clone();
        let rate_limiter = ChannelEmitRateLimiter::new(capabilities.emit_rate_limit.clone());
        let formatting_hints =
            formatting_hints.or_else(|| conversions::default_wasm_channel_formatting_hints(&name));

        // Read stream mode from env for Telegram and Discord channels
        let stream_mode = if prepared.name == "telegram" {
            std::env::var("TELEGRAM_STREAM_MODE")
                .map(|v| StreamMode::from_str_value(&v))
                .unwrap_or_default()
        } else if prepared.name == "discord" {
            std::env::var("DISCORD_STREAM_MODE")
                .map(|v| StreamMode::from_str_value(&v))
                .unwrap_or_default()
        } else {
            StreamMode::None
        };

        // Use disk-backed workspace store so WASM channel state (e.g.,
        // Telegram managed topic registry) survives process restarts.
        let workspace_persist_path = thinclaw_platform::state_paths()
            .channels_dir
            .join(format!("{}.workspace.json", &name));
        let workspace_store = Arc::new(ChannelWorkspaceStore::with_persistence(
            workspace_persist_path,
        ));

        Self {
            name,
            runtime,
            prepared,
            capabilities,
            config_json: RwLock::new(config_json),
            channel_config: RwLock::new(None),
            formatting_hints,
            message_tx: Arc::new(RwLock::new(None)),
            pending_responses: RwLock::new(HashMap::new()),
            rate_limiter: Arc::new(RwLock::new(rate_limiter)),
            shutdown_tx: RwLock::new(None),
            poll_shutdown_tx: RwLock::new(None),
            endpoints: RwLock::new(Vec::new()),
            credentials: Arc::new(RwLock::new(HashMap::new())),
            typing_task: RwLock::new(None),
            pairing_store,
            workspace_store,
            stream_mode: std::sync::RwLock::new(stream_mode),
            debug_mode: std::sync::RwLock::new(false),
            pending_tool_events: tokio::sync::RwLock::new(Vec::new()),
        }
    }

    /// Update the channel config before starting.
    ///
    /// Merges the provided values into the existing config JSON.
    /// Call this before `start()` to inject runtime values like tunnel_url.
    pub async fn update_config(&self, updates: HashMap<String, serde_json::Value>) {
        if let Some(mode) = updates.get("stream_mode").and_then(|v| v.as_str()) {
            let parsed = StreamMode::from_str_value(mode);
            if let Ok(mut g) = self.stream_mode.write() {
                *g = parsed;
            }
        }

        let mut config_guard = self.config_json.write().await;

        // Parse existing config
        let mut config: HashMap<String, serde_json::Value> =
            serde_json::from_str(&config_guard).unwrap_or_default();

        // Merge updates
        for (key, value) in updates {
            config.insert(key, value);
        }

        // Serialize back
        *config_guard = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());

        tracing::debug!(
            channel = %self.name,
            config = %*config_guard,
            "Updated channel config"
        );
    }

    /// Set a credential for URL injection.
    pub async fn set_credential(&self, name: &str, value: String) {
        self.credentials
            .write()
            .await
            .insert(name.to_string(), value);
    }

    /// Get a snapshot of credentials for use in callbacks.
    pub async fn get_credentials(&self) -> HashMap<String, String> {
        self.credentials.read().await.clone()
    }

    /// Get the channel name.
    pub fn channel_name(&self) -> &str {
        &self.name
    }

    /// Get the channel capabilities.
    pub fn capabilities(&self) -> &ChannelCapabilities {
        &self.capabilities
    }

    /// Get the registered endpoints.
    pub async fn endpoints(&self) -> Vec<RegisteredEndpoint> {
        self.endpoints.read().await.clone()
    }

    /// Prime and cache the on_start configuration without starting the channel.
    ///
    /// This lets the host register the actual webhook endpoints before the
    /// HTTP server starts, while keeping `start()` idempotent.
    pub async fn prime_on_start_config(&self) -> Result<ChannelConfig, WasmChannelError> {
        self.ensure_on_start_config(false).await
    }

    /// Force a fresh on_start call and replace the cached config/endpoints.
    pub async fn refresh_on_start_config(&self) -> Result<ChannelConfig, WasmChannelError> {
        self.ensure_on_start_config(true).await
    }
}

#[async_trait]
impl Channel for WasmChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn formatting_hints(&self) -> Option<String> {
        self.formatting_hints.clone()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Create message channel
        let (tx, rx) = mpsc::channel(256);
        *self.message_tx.write().await = Some(tx);

        // Create shutdown channel
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        // Call on_start to get configuration, unless we already primed it.
        let config =
            self.ensure_on_start_config(false)
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: self.name.clone(),
                    reason: e.to_string(),
                })?;

        // Start polling if configured
        if let Some(poll_config) = &config.poll
            && poll_config.enabled
        {
            let interval = self
                .capabilities
                .validate_poll_interval(poll_config.interval_ms)
                .map_err(|e| ChannelError::StartupFailed {
                    name: self.name.clone(),
                    reason: e,
                })?;

            // Create shutdown channel for polling and store the sender to keep it alive
            let (poll_shutdown_tx, poll_shutdown_rx) = oneshot::channel();
            *self.poll_shutdown_tx.write().await = Some(poll_shutdown_tx);

            self.start_polling(Duration::from_millis(interval as u64), poll_shutdown_rx);
        }

        tracing::info!(
            channel = %self.name,
            display_name = %config.display_name,
            endpoints = config.http_endpoints.len(),
            "WASM channel started"
        );

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Stop the typing indicator, we're about to send the actual response
        self.cancel_typing_task().await;

        // Flush accumulated tool events as a single summary message before the response.
        self.flush_tool_events(&msg.metadata).await;
        let outbound_content = conversions::response_content_for_wasm(&self.name, &response);

        // Check if there's a pending synchronous response waiter
        if let Some(tx) = self.pending_responses.write().await.remove(&msg.id) {
            let _ = tx.send(outbound_content.clone());
        }

        // Send outbound media attachments directly via Telegram API
        // (before the text response, so files arrive first)
        if !response.attachments.is_empty() {
            let chat_id = msg.metadata.get("chat_id").and_then(|v| v.as_i64());
            let message_thread_id = msg
                .metadata
                .get("message_thread_id")
                .and_then(|v| v.as_i64());
            if let Some(chat_id) = chat_id {
                self.transport_send_attachments(chat_id, message_thread_id, &response.attachments)
                    .await;
            }
        }

        // Merge original routing metadata with any response-specific overrides.
        // Response metadata wins on conflicts, and outbound attachments are
        // tunneled through `response_attachments` for WASM channels.
        let metadata_json = serde_json::to_string(&conversions::merged_response_metadata(
            &msg.metadata,
            &response,
        ))
        .unwrap_or_default();
        self.call_on_respond(
            msg.id,
            &outbound_content,
            response.thread_id.as_deref(),
            &metadata_json,
        )
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: self.name.clone(),
            reason: e.to_string(),
        })?;

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Delegate to the typing indicator implementation
        self.handle_status_update(status, metadata).await
    }

    fn stream_mode(&self) -> StreamMode {
        *self.stream_mode.read().unwrap_or_else(|e| e.into_inner())
    }

    async fn set_stream_mode(&self, mode: StreamMode) {
        if let Ok(mut g) = self.stream_mode.write() {
            *g = mode;
        }
        tracing::info!(
            channel = %self.name,
            mode = ?mode,
            "Stream mode updated at runtime"
        );
    }

    async fn update_runtime_config(
        &self,
        updates: std::collections::HashMap<String, serde_json::Value>,
    ) {
        self.update_config(updates).await;
    }

    async fn send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        self.transport_send_draft(draft, metadata).await
    }

    async fn delete_message(
        &self,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.transport_delete_message(message_id, metadata).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.transport_health_check().await
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        self.transport_diagnostics().await
    }

    async fn reset_connection_state(&self) -> Result<(), ChannelError> {
        self.transport_reset_connection_state().await
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // Cancel typing indicator
        self.cancel_typing_task().await;

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }

        // Stop polling by dropping the sender (receiver will complete)
        let _ = self.poll_shutdown_tx.write().await.take();

        // Clear the message sender
        *self.message_tx.write().await = None;

        tracing::info!(
            channel = %self.name,
            "WASM channel shut down"
        );

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Channel-specialized broadcast (e.g. WhatsApp routed delivery). When
        // the transport fully handles it, we are done.
        if self.transport_try_broadcast(user_id, &response).await? {
            return Ok(());
        }

        // For WASM channels, broadcast routes through on_respond with a
        // synthetic metadata containing the user_id as the chat_id.
        // This works because on_respond just needs chat_id to know where
        // to send the message.
        //
        // If user_id is not a valid numeric ID (e.g. "default"), we can't
        // address a specific chat — skip gracefully.
        let chat_id: i64 = match user_id.parse() {
            Ok(id) => id,
            Err(_) => {
                tracing::debug!(
                    channel = %self.name,
                    user_id = %user_id,
                    "WASM broadcast: skipping — user_id is not a numeric chat ID"
                );
                return Ok(());
            }
        };

        // Send outbound media attachments directly via Telegram API
        if !response.attachments.is_empty() {
            self.transport_send_attachments(chat_id, None, &response.attachments)
                .await;
        }

        // Build minimal metadata that on_respond can parse.
        // message_id=0 means "don't reply to a specific message".
        let base_metadata = serde_json::json!({
            "chat_id": chat_id,
            "message_id": 0,
            "user_id": chat_id,
            "is_private": true,
        });
        let metadata_json = serde_json::to_string(&conversions::merged_response_metadata(
            &base_metadata,
            &response,
        ))
        .unwrap_or_default();
        let outbound_content = conversions::response_content_for_wasm(&self.name, &response);

        tracing::info!(
            channel = %self.name,
            chat_id = chat_id,
            content_len = outbound_content.len(),
            "WASM broadcast: sending proactive message via on_respond"
        );

        let result = self
            .call_on_respond(
                uuid::Uuid::new_v4(),
                &outbound_content,
                response.thread_id.as_deref(),
                &metadata_json,
            )
            .await;

        match &result {
            Ok(()) => tracing::info!(
                channel = %self.name,
                chat_id = chat_id,
                "WASM broadcast: on_respond completed without error"
            ),
            Err(e) => tracing::error!(
                channel = %self.name,
                chat_id = chat_id,
                error = %e,
                "WASM broadcast: on_respond FAILED"
            ),
        }

        result.map_err(|e| ChannelError::SendFailed {
            name: self.name.clone(),
            reason: format!("broadcast via on_respond: {}", e),
        })
    }

    async fn toggle_debug_mode(&self) -> bool {
        let new_state = match self.debug_mode.write() {
            Ok(mut g) => {
                *g = !*g;
                *g
            }
            Err(e) => {
                let mut g = e.into_inner();
                *g = !*g;
                *g
            }
        };
        tracing::info!(
            channel = %self.name,
            debug_mode = new_state,
            "Debug mode toggled"
        );
        new_state
    }
}

impl std::fmt::Debug for WasmChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmChannel")
            .field("name", &self.name)
            .field("prepared", &self.prepared.name)
            .finish()
    }
}

// ============================================================================
// Shared Channel Wrapper
// ============================================================================

/// A wrapper around `Arc<WasmChannel>` that implements `Channel`.
///
/// This allows sharing the same WasmChannel instance between:
/// - The WasmChannelRouter (for webhook handling)
/// - The ChannelManager (for message streaming and responses)
pub struct SharedWasmChannel {
    inner: Arc<WasmChannel>,
}

impl SharedWasmChannel {
    /// Create a new shared wrapper.
    pub fn new(channel: Arc<WasmChannel>) -> Self {
        Self { inner: channel }
    }

    /// Get the inner Arc.
    pub fn inner(&self) -> &Arc<WasmChannel> {
        &self.inner
    }
}

impl std::fmt::Debug for SharedWasmChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedWasmChannel")
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait]
impl Channel for SharedWasmChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn formatting_hints(&self) -> Option<String> {
        self.inner.formatting_hints()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    fn stream_mode(&self) -> StreamMode {
        self.inner.stream_mode()
    }

    async fn set_stream_mode(&self, mode: StreamMode) {
        self.inner.set_stream_mode(mode).await
    }

    async fn update_runtime_config(
        &self,
        updates: std::collections::HashMap<String, serde_json::Value>,
    ) {
        self.inner.update_runtime_config(updates).await
    }

    async fn send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        self.inner.send_draft(draft, metadata).await
    }

    async fn delete_message(
        &self,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.inner.delete_message(message_id, metadata).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        self.inner.diagnostics().await
    }

    async fn reset_connection_state(&self) -> Result<(), ChannelError> {
        self.inner.reset_connection_state().await
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.inner.shutdown().await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn toggle_debug_mode(&self) -> bool {
        self.inner.toggle_debug_mode().await
    }
}

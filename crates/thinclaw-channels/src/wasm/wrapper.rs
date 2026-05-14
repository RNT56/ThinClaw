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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::manager::{IncomingEvent, normalize_incoming_event, parse_slash_command};
use crate::pairing::PairingStore;
use crate::wasm::capabilities::{ChannelCapabilities, WorkspaceCapability};
use crate::wasm::error::WasmChannelError;
use crate::wasm::host::{
    ChannelEmitRateLimiter, ChannelHostState, ChannelWorkspaceStore, EmittedMessage,
};
use crate::wasm::host::{LogLevel, WorkspaceReader};
use crate::wasm::limits::WasmResourceLimiter;
use crate::wasm::router::RegisteredEndpoint;
use crate::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime};
use crate::wasm::schema::ChannelConfig;
use thinclaw_channels_core::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
use thinclaw_safety::LeakDetector;
use thinclaw_types::error::ChannelError;

// Generate component model bindings from the WIT file
wasmtime::component::bindgen!({
    path: "../../wit/channel.wit",
    world: "sandboxed-channel",
    with: {
        // Use our own store data type
    },
});

/// A single tool lifecycle event accumulated during a turn.
///
/// Collected while processing and flushed as a single summary
/// message before the response is sent (debug mode only).
#[derive(Debug, Clone)]
enum ToolEventEntry {
    /// Tool execution started.
    Started { name: String },
    /// Tool execution completed (success or failure).
    Completed { name: String, success: bool },
    /// Tool returned a result preview.
    Result { preview: String },
}

/// Escape HTML entities for safe embedding in Telegram HTML messages.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Store data for WASM channel execution.
///
/// Contains the resource limiter, channel-specific host state, and WASI context.
struct ChannelStoreData {
    limiter: WasmResourceLimiter,
    host_state: ChannelHostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL substitution (e.g., bot tokens).
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    credentials: HashMap<String, String>,
    /// Pairing store for DM pairing (guest access control).
    pairing_store: Arc<PairingStore>,
    /// Dedicated tokio runtime for HTTP requests, lazily initialized.
    /// Reused across multiple `http_request` calls within one execution.
    http_runtime: Option<tokio::runtime::Runtime>,
}

impl ChannelStoreData {
    fn new(
        memory_limit: u64,
        channel_name: &str,
        capabilities: ChannelCapabilities,
        credentials: HashMap<String, String>,
        pairing_store: Arc<PairingStore>,
        workspace_store: Arc<ChannelWorkspaceStore>,
    ) -> Self {
        // Create a minimal WASI context (no filesystem, no env vars for security)
        let wasi = WasiCtxBuilder::new().build();
        let workspace_reader = Some(workspace_store as Arc<dyn WorkspaceReader>);

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: ChannelHostState::with_workspace_reader(
                channel_name,
                capabilities,
                workspace_reader,
            ),
            wasi,
            table: ResourceTable::new(),
            credentials,
            pairing_store,
            http_runtime: None,
        }
    }

    /// Inject credentials into a string by replacing placeholders.
    ///
    /// Replaces patterns like `{TELEGRAM_BOT_TOKEN}` or `{WHATSAPP_ACCESS_TOKEN}`
    /// with actual values from the injected credentials map. This allows WASM
    /// channels to reference credentials without ever seeing the actual values.
    ///
    /// Works on URLs, headers, or any string with credential placeholders.
    fn inject_credentials(&self, input: &str, context: &str) -> String {
        let mut result = input.to_string();

        tracing::debug!(
            input_preview = %input.chars().take(100).collect::<String>(),
            context = %context,
            credential_count = self.credentials.len(),
            credential_names = ?self.credentials.keys().collect::<Vec<_>>(),
            "Injecting credentials"
        );

        // Replace all known placeholders from the credentials map
        for (name, value) in &self.credentials {
            let placeholder = format!("{{{}}}", name);
            if result.contains(&placeholder) {
                tracing::debug!(
                    placeholder = %placeholder,
                    context = %context,
                    "Found and replacing credential placeholder"
                );
                result = result.replace(&placeholder, value);
            }
        }

        // Check if any placeholders remain (indicates missing credential)
        if result.contains('{') && result.contains('}') {
            // Only warn if it looks like an unresolved placeholder (not JSON braces)
            let brace_pattern = regex::Regex::new(r"\{[A-Z_]+\}").ok();
            if let Some(re) = brace_pattern
                && re.is_match(&result)
            {
                tracing::warn!(
                    context = %context,
                    "String may contain unresolved credential placeholders"
                );
            }
        }

        result
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages, logs, or
    /// return values to WASM. reqwest::Error includes the full URL in its
    /// Display output, so any error from an injected-URL request will
    /// contain the raw credential unless we scrub it.
    fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, value) in &self.credentials {
            if !value.is_empty() {
                result = result.replace(value, &format!("[REDACTED:{}]", name));
            }
        }
        result
    }

    fn leak_detector(&self) -> LeakDetector {
        LeakDetector::with_exact_values(self.credentials.values().cloned())
    }
}

// Implement WasiView to provide WASI context and resource table
impl WasiView for ChannelStoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Implement the generated Host trait for channel-host interface
impl near::agent::channel_host::Host for ChannelStoreData {
    fn log(&mut self, level: near::agent::channel_host::LogLevel, message: String) {
        let log_level = match level {
            near::agent::channel_host::LogLevel::Trace => LogLevel::Trace,
            near::agent::channel_host::LogLevel::Debug => LogLevel::Debug,
            near::agent::channel_host::LogLevel::Info => LogLevel::Info,
            near::agent::channel_host::LogLevel::Warn => LogLevel::Warn,
            near::agent::channel_host::LogLevel::Error => LogLevel::Error,
        };
        let _ = self.host_state.log(log_level, message);
    }

    fn now_millis(&mut self) -> u64 {
        self.host_state.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host_state.workspace_read(&path).ok().flatten()
    }

    fn workspace_write(&mut self, path: String, content: String) -> Result<(), String> {
        self.host_state
            .workspace_write(&path, content)
            .map_err(|e| e.to_string())
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<near::agent::channel_host::HttpResponse, String> {
        tracing::info!(
            method = %method,
            original_url = %url,
            body_len = body.as_ref().map(|b| b.len()).unwrap_or(0),
            "WASM http_request called"
        );

        let leak_detector = self.leak_detector();
        let raw_headers: std::collections::HashMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();
        let raw_header_vec: Vec<(String, String)> = raw_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        leak_detector
            .scan_http_request(&url, &raw_header_vec, body.as_deref())
            .map_err(|e| format!("Potential secret leak blocked: {}", e))?;

        // Inject credentials into URL (e.g., replace {TELEGRAM_BOT_TOKEN} with actual token)
        let injected_url = self.inject_credentials(&url, "url");

        // Log whether injection happened (without revealing the token)
        let url_changed = injected_url != url;
        tracing::info!(url_changed = url_changed, "URL after credential injection");

        // Inject credentials into header values
        // This allows patterns like "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
        let headers: std::collections::HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let headers_changed = headers
            .values()
            .any(|v| v.contains("Bearer ") && !v.contains('{'));
        tracing::debug!(
            header_count = headers.len(),
            headers_changed = headers_changed,
            "Parsed and injected request headers"
        );

        let url = injected_url;
        let body = body.map(|body_bytes| {
            std::str::from_utf8(&body_bytes)
                .map(|text| self.inject_credentials(text, "body").into_bytes())
                .unwrap_or(body_bytes)
        });

        self.host_state
            .check_http_allowed(&url, &method)
            .map_err(|e| {
                tracing::error!(error = %e, "HTTP not allowed");
                format!("HTTP not allowed: {}", e)
            })?;

        // Record the request for rate limiting
        self.host_state.record_http_request().map_err(|e| {
            tracing::error!(error = %e, "Rate limit exceeded");
            format!("Rate limit exceeded: {}", e)
        })?;
        // Get the max response size from capabilities (default 10MB).
        let max_response_bytes = self
            .host_state
            .capabilities()
            .tool_capabilities
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Make the HTTP request using a dedicated single-threaded runtime.
        // We're inside spawn_blocking, so we can't rely on the main runtime's
        // I/O driver (it may be busy with WASM compilation or other startup work).
        // A dedicated runtime gives us our own I/O driver and avoids contention.
        // The runtime is lazily created and reused across calls within one execution.
        if self.http_runtime.is_none() {
            self.http_runtime = Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create HTTP runtime: {e}"))?,
            );
        }
        let rt = self.http_runtime.as_ref().expect("just initialized");
        let result = rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => return Err(format!("Unsupported HTTP method: {}", method)),
            };

            // Add headers
            for (key, value) in headers {
                request = request.header(&key, &value);
            }

            // Add body if present
            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            }

            // Send request with caller-specified timeout (default 30s, max 5min).
            let timeout_ms = timeout_ms.unwrap_or(30_000).min(300_000) as u64;
            let timeout = std::time::Duration::from_millis(timeout_ms);
            let response = request.timeout(timeout).send().await.map_err(|e| {
                // Walk the full error chain so we get the actual root cause
                // (DNS, TLS, connection refused, etc.) instead of just
                // "error sending request for url (...)".
                let mut chain = format!("HTTP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                chain
            })?;

            let status = response.status().as_u16();
            let response_headers: std::collections::HashMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
                })
                .collect();
            let headers_json = serde_json::to_string(&response_headers).unwrap_or_default();

            // Enforce max response body size to prevent memory exhaustion.
            let max_response = max_response_bytes;
            if let Some(cl) = response.content_length()
                && cl as usize > max_response
            {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    cl, max_response
                ));
            }
            let body = response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if body.len() > max_response {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    body.len(),
                    max_response
                ));
            }
            let mut body = body.to_vec();

            tracing::info!(
                status = status,
                body_len = body.len(),
                "HTTP response received"
            );

            // Log response body for debugging (truncated at char boundary)
            if let Ok(body_str) = std::str::from_utf8(&body) {
                let truncated = if body_str.chars().count() > 500 {
                    format!("{}...", body_str.chars().take(500).collect::<String>())
                } else {
                    body_str.to_string()
                };
                tracing::debug!(body = %truncated, "Response body");
            }

            // Leak detection on response body (best-effort)
            if let Ok(body_str) = std::str::from_utf8(&body) {
                let cleaned = leak_detector
                    .scan_and_clean(body_str)
                    .map_err(|e| format!("Potential secret leak in response: {}", e))?;
                if cleaned != body_str {
                    body = cleaned.into_bytes();
                }
            }

            Ok(near::agent::channel_host::HttpResponse {
                status,
                headers_json,
                body,
            })
        });

        // Scrub credential values from error messages before logging or returning
        // to WASM. reqwest::Error includes the full URL (with injected credentials)
        // in its Display output.
        let result = result.map_err(|e| self.redact_credentials(&e));

        match &result {
            Ok(resp) => {
                tracing::info!(status = resp.status, "http_request completed successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "http_request failed");
            }
        }

        result
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host_state.secret_exists(&name)
    }

    fn emit_message(&mut self, msg: near::agent::channel_host::EmittedMessage) {
        tracing::info!(
            user_id = %msg.user_id,
            user_name = ?msg.user_name,
            content_len = msg.content.len(),
            attachment_count = msg.attachments.len(),
            "WASM emit_message called"
        );

        let mut emitted = EmittedMessage::new(msg.user_id.clone(), msg.content.clone());
        if let Some(name) = msg.user_name {
            emitted = emitted.with_user_name(name);
        }
        if let Some(tid) = msg.thread_id {
            emitted = emitted.with_thread_id(tid);
        }
        emitted = emitted.with_metadata(msg.metadata_json);

        // Convert WIT media-attachment records to MediaAttachment
        for att in msg.attachments {
            emitted
                .attachments
                .push(crate::wasm::host::MediaAttachment {
                    mime_type: att.mime_type,
                    data: att.data,
                    filename: att.filename,
                });
        }

        match self.host_state.emit_message(emitted) {
            Ok(()) => {
                tracing::info!("Message emitted to host state successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to emit message to host state");
            }
        }
    }

    fn pairing_upsert_request(
        &mut self,
        channel: String,
        id: String,
        meta_json: String,
    ) -> Result<near::agent::channel_host::PairingUpsertResult, String> {
        let meta = if meta_json.is_empty() {
            None
        } else {
            serde_json::from_str(&meta_json).ok()
        };
        match self.pairing_store.upsert_request(&channel, &id, meta) {
            Ok(r) => Ok(near::agent::channel_host::PairingUpsertResult {
                code: r.code,
                created: r.created,
            }),
            Err(e) => Err(e.to_string()),
        }
    }

    fn pairing_is_allowed(
        &mut self,
        channel: String,
        id: String,
        username: Option<String>,
    ) -> Result<bool, String> {
        self.pairing_store
            .is_sender_allowed(&channel, &id, username.as_deref())
            .map_err(|e| e.to_string())
    }

    fn pairing_read_allow_from(&mut self, channel: String) -> Result<Vec<String>, String> {
        self.pairing_store
            .read_allow_from(&channel)
            .map_err(|e| e.to_string())
    }

    fn markdown_to_telegram_html(&mut self, markdown: String) -> String {
        super::telegram_html::markdown_to_telegram_html(&markdown)
    }
}

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
            formatting_hints.or_else(|| default_wasm_channel_formatting_hints(&name));

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

    /// Flush accumulated tool events as a single formatted summary message.
    ///
    /// Called at the start of `respond()` so the tool activity block appears
    /// before the final response.  Does nothing if the accumulator is empty.
    async fn flush_tool_events(&self, metadata: &serde_json::Value) {
        let events: Vec<ToolEventEntry> = {
            let mut guard = self.pending_tool_events.write().await;
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        // Build a grouped, single-message summary.
        //
        // Format (Telegram HTML):
        //   🔧 <b>Tool Activity</b>
        //   ✅ web_search
        //      "query text…"
        //   ✅ read_file
        //      main.rs (1,234 chars)
        //   ❌ list_dir — failed
        //   ─────────────
        //   3 calls · 2✅ 1❌

        let mut lines: Vec<String> = vec!["🔧 <b>Tool Activity</b>".to_string()];

        // Walk through events in order, emitting one visual block per tool
        let mut succeeded = 0u32;
        let mut failed = 0u32;
        let mut total_calls = 0u32;

        for event in &events {
            match event {
                ToolEventEntry::Started { name } => {
                    // We'll render the tool line when we see the Completed event.
                    // If we never get a Completed (edge case), the Started is
                    // just informational.  We track it for ordering context.
                    let _ = name; // used below via Completed
                }
                ToolEventEntry::Completed { name, success } => {
                    total_calls += 1;
                    let icon = if *success { "✅" } else { "❌" };
                    let suffix = if *success { "" } else { " — failed" };
                    lines.push(format!(
                        "{} <code>{}</code>{}",
                        icon,
                        html_escape(name),
                        suffix
                    ));
                    if *success {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                }
                ToolEventEntry::Result { preview } => {
                    if !preview.is_empty() {
                        // Truncate long results
                        let display: String = if preview.chars().count() > 120 {
                            let truncated: String = preview.chars().take(117).collect();
                            format!("{}…", truncated)
                        } else {
                            preview.clone()
                        };
                        lines.push(format!("   <i>{}</i>", html_escape(&display)));
                    }
                }
            }
        }

        // Footer
        if total_calls > 0 {
            lines.push("───────────────".to_string());
            let mut footer_parts = vec![format!(
                "{} call{}",
                total_calls,
                if total_calls == 1 { "" } else { "s" }
            )];
            if succeeded > 0 {
                footer_parts.push(format!("{}✅", succeeded));
            }
            if failed > 0 {
                footer_parts.push(format!("{}❌", failed));
            }
            lines.push(footer_parts.join(" · "));
        }

        let summary = lines.join("\n");

        // Send as a single message via on_respond (not on_status, which
        // would go through the WASM status handler and might be dropped)
        let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
        if let Err(e) = self
            .call_on_respond(uuid::Uuid::new_v4(), &summary, None, &metadata_json)
            .await
        {
            tracing::debug!(
                channel = %self.name,
                error = %e,
                "Failed to send tool summary (best-effort)"
            );
        }
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

    fn registered_endpoints_from_config(&self, config: &ChannelConfig) -> Vec<RegisteredEndpoint> {
        let mut endpoints = Vec::new();

        for endpoint in &config.http_endpoints {
            if !self.capabilities.is_path_allowed(&endpoint.path) {
                tracing::warn!(
                    channel = %self.name,
                    path = %endpoint.path,
                    "HTTP endpoint path not allowed by capabilities"
                );
                continue;
            }

            endpoints.push(RegisteredEndpoint {
                channel_name: self.name.clone(),
                path: endpoint.path.clone(),
                methods: endpoint.methods.clone(),
                require_secret: endpoint.require_secret,
            });
        }

        endpoints
    }

    async fn cache_channel_config(&self, config: &ChannelConfig) {
        *self.channel_config.write().await = Some(config.clone());
        *self.endpoints.write().await = self.registered_endpoints_from_config(config);
    }

    async fn ensure_on_start_config(
        &self,
        force_refresh: bool,
    ) -> Result<ChannelConfig, WasmChannelError> {
        if !force_refresh && let Some(existing) = self.channel_config.read().await.clone() {
            return Ok(existing);
        }

        let config = self.call_on_start().await?;
        self.cache_channel_config(&config).await;
        Ok(config)
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

    /// Inject the workspace store as the reader into a capabilities clone.
    ///
    /// Ensures `workspace_read` capability is present with the store as its reader,
    /// so WASM callbacks can read previously written workspace state.
    fn inject_workspace_reader(
        capabilities: &ChannelCapabilities,
        store: &Arc<ChannelWorkspaceStore>,
    ) -> ChannelCapabilities {
        let mut caps = capabilities.clone();
        let ws_cap = caps
            .tool_capabilities
            .workspace_read
            .get_or_insert_with(|| WorkspaceCapability {
                allowed_prefixes: Vec::new(),
            });
        let _ = (store, ws_cap);
        caps
    }

    /// Add channel host functions to the linker using generated bindings.
    ///
    /// Uses the wasmtime::component::bindgen! generated `add_to_linker` function
    /// to properly register all host functions with correct component model signatures.
    fn add_host_functions(linker: &mut Linker<ChannelStoreData>) -> Result<(), WasmChannelError> {
        // Add WASI support (required by the component adapter)
        wasmtime_wasi::p2::add_to_linker_sync(linker).map_err(|e| {
            WasmChannelError::Config(format!("Failed to add WASI functions: {}", e))
        })?;

        // Use the generated add_to_linker function from bindgen for our custom interface
        near::agent::channel_host::add_to_linker::<
            ChannelStoreData,
            wasmtime::component::HasSelf<ChannelStoreData>,
        >(linker, |state| state)
        .map_err(|e| WasmChannelError::Config(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    /// Create a fresh store configured for WASM execution.
    fn create_store(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
        credentials: HashMap<String, String>,
        pairing_store: Arc<PairingStore>,
        workspace_store: Arc<ChannelWorkspaceStore>,
    ) -> Result<Store<ChannelStoreData>, WasmChannelError> {
        let engine = runtime.engine();
        let limits = &prepared.limits;

        // Create fresh store with channel state (NEAR pattern: fresh instance per call)
        let store_data = ChannelStoreData::new(
            limits.memory_bytes,
            &prepared.name,
            capabilities.clone(),
            credentials,
            pairing_store,
            workspace_store,
        );
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmChannelError::Config(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline for timeout backup
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        Ok(store)
    }

    /// Instantiate the WASM component using generated bindings.
    fn instantiate_component(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        store: &mut Store<ChannelStoreData>,
    ) -> Result<SandboxedChannel, WasmChannelError> {
        let engine = runtime.engine();

        // Use the pre-compiled component (no recompilation needed)
        let component = prepared
            .component()
            .ok_or_else(|| {
                WasmChannelError::Compilation("No compiled component available".to_string())
            })?
            .clone();

        // Create linker and add host functions
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance = SandboxedChannel::instantiate(store, &component, &linker)
            .map_err(|e| WasmChannelError::Instantiation(e.to_string()))?;

        Ok(instance)
    }

    /// Map WASM execution errors to our error types.
    fn map_wasm_error(e: anyhow::Error, name: &str, fuel_limit: u64) -> WasmChannelError {
        let error_str = e.to_string();
        if error_str.contains("out of fuel") {
            WasmChannelError::FuelExhausted {
                name: name.to_string(),
                limit: fuel_limit,
            }
        } else if error_str.contains("unreachable") {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: "unreachable code executed".to_string(),
            }
        } else {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: error_str,
            }
        }
    }

    /// Extract host state after callback execution.
    fn extract_host_state(
        store: &mut Store<ChannelStoreData>,
        channel_name: &str,
        capabilities: &ChannelCapabilities,
    ) -> ChannelHostState {
        std::mem::replace(
            &mut store.data_mut().host_state,
            ChannelHostState::new(channel_name, capabilities.clone()),
        )
    }

    /// Execute the on_start callback.
    ///
    /// Returns the channel configuration for HTTP endpoint registration.
    /// Call the WASM module's `on_start` callback.
    ///
    /// Typically called once during `start()`, but can be called again after
    /// credentials are refreshed to re-trigger webhook registration and
    /// other one-time setup that depends on credentials.
    pub async fn call_on_start(&self) -> Result<ChannelConfig, WasmChannelError> {
        // If no WASM bytes, return default config (for testing)
        if self.prepared.component().is_none() {
            tracing::info!(
                channel = %self.name,
                "WASM channel on_start called (no WASM module, returning defaults)"
            );
            return Ok(ChannelConfig {
                display_name: self.prepared.description.clone(),
                http_endpoints: Vec::new(),
                poll: None,
            });
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let config_json = self.apply_telegram_runtime_state(
            self.config_json.read().await.clone(),
            &self.load_runtime_state(),
        );
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_start using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_start(&mut store, &config_json)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                // Convert the result
                let config = match wasm_result {
                    Ok(wit_config) => convert_channel_config(wit_config),
                    Err(err_msg) => {
                        return Err(WasmChannelError::CallbackFailed {
                            name: prepared.name.clone(),
                            reason: err_msg,
                        });
                    }
                };

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok((config, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok((config, mut host_state))) => {
                // Surface WASM guest logs (errors/warnings from webhook setup, etc.)
                for entry in host_state.take_logs() {
                    match entry.level {
                        LogLevel::Error => {
                            tracing::error!(channel = %self.name, "{}", entry.message);
                        }
                        LogLevel::Warn => {
                            tracing::warn!(channel = %self.name, "{}", entry.message);
                        }
                        _ => {
                            tracing::debug!(channel = %self.name, "{}", entry.message);
                        }
                    }
                }
                tracing::info!(
                    channel = %self.name,
                    display_name = %config.display_name,
                    endpoints = config.http_endpoints.len(),
                    "WASM channel on_start completed"
                );
                Ok(config)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_start".to_string(),
            }),
        }
    }

    /// Execute the on_http_request callback.
    ///
    /// Called when an HTTP request arrives at a registered endpoint.
    pub async fn call_on_http_request(
        &self,
        method: &str,
        path: &str,
        headers: &HashMap<String, String>,
        query: &HashMap<String, String>,
        body: &[u8],
        secret_validated: bool,
    ) -> Result<HttpResponse, WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            method = method,
            path = path,
            body_len = body.len(),
            secret_validated = secret_validated,
            "call_on_http_request invoked (webhook received)"
        );

        // Log the body for debugging (truncated at char boundary)
        if let Ok(body_str) = std::str::from_utf8(body) {
            let truncated = if body_str.chars().count() > 1000 {
                format!("{}...", body_str.chars().take(1000).collect::<String>())
            } else {
                body_str.to_string()
            };
            tracing::debug!(body = %truncated, "Webhook request body");
        }

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_http_request"
        );

        // If no WASM bytes, return 200 OK (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                method = method,
                path = path,
                "WASM channel on_http_request called (no WASM module)"
            );
            return Ok(HttpResponse::ok());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare request data
        let method = method.to_string();
        let path = path.to_string();
        let headers_json = serde_json::to_string(&headers).unwrap_or_default();
        let query_json = serde_json::to_string(&query).unwrap_or_default();
        let body = body.to_vec();

        let channel_name = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT request type
                let wit_request = wit_channel::IncomingHttpRequest {
                    method,
                    path,
                    headers_json,
                    query_json,
                    body,
                    secret_validated,
                };

                // Call on_http_request using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wit_response = channel_iface
                    .call_on_http_request(&mut store, &wit_request)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let response = convert_http_response(wit_response);
                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok((response, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok((response, mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    status = response.status,
                    "WASM channel on_http_request completed"
                );
                Ok(response)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_http_request".to_string(),
            }),
        }
    }

    /// Execute the on_poll callback.
    ///
    /// Called periodically if polling is configured.
    pub async fn call_on_poll(&self) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(((), host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    "WASM channel on_poll completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Execute the on_respond callback.
    ///
    /// Called when the agent has a response to send back.
    pub async fn call_on_respond(
        &self,
        message_id: Uuid,
        content: &str,
        thread_id: Option<&str>,
        metadata_json: &str,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_id = %message_id,
            content_len = content.len(),
            thread_id = ?thread_id,
            "call_on_respond invoked"
        );

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_respond"
        );

        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                message_id = %message_id,
                "WASM channel on_respond called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare response data
        let message_id_str = message_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let metadata_json = metadata_json.to_string();

        // Execute in blocking task with timeout
        tracing::info!(channel = %channel_name, "Starting on_respond WASM execution");

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                tracing::info!("Creating WASM store for on_respond");
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;

                tracing::info!("Instantiating WASM component for on_respond");
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT response type
                let wit_response = wit_channel::AgentResponse {
                    message_id: message_id_str,
                    content: content.clone(),
                    thread_id,
                    metadata_json,
                };

                // Truncate at char boundary for logging (avoid panic on multi-byte UTF-8)
                let content_preview: String = content.chars().take(50).collect();
                tracing::info!(
                    content_preview = %content_preview,
                    "Calling WASM on_respond"
                );

                // Call on_respond using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_respond(&mut store, &wit_response)
                    .map_err(|e| {
                        tracing::error!(error = %e, "WASM on_respond call failed");
                        Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel)
                    })?;

                tracing::info!(wasm_result = ?wasm_result, "WASM on_respond returned");

                // Check for WASM-level errors
                if let Err(ref err_msg) = wasm_result {
                    tracing::error!(error = %err_msg, "WASM on_respond returned error");
                    return Err(WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: err_msg.clone(),
                    });
                }

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store
                // so state mutations from on_respond survive restarts.
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);
                tracing::info!("on_respond WASM execution completed successfully");
                Ok(((), host_state))
            })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "spawn_blocking panicked");
                WasmChannelError::ExecutionPanicked {
                    name: channel_name.clone(),
                    reason: e.to_string(),
                }
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), _host_state))) => {
                tracing::info!(
                    channel = %channel_name,
                    message_id = %message_id,
                    "WASM channel on_respond completed successfully"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_respond".to_string(),
            }),
        }
    }

    /// Execute the on_status callback.
    ///
    /// Called to notify the WASM channel of agent status changes (e.g., typing).
    pub async fn call_on_status(
        &self,
        status: &StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        let wit_update = status_to_wit(status, metadata);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store
                // so state mutations from on_status survive restarts.
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => {
                tracing::debug!(
                    channel = %self.name,
                    "WASM channel on_status completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Execute a single on_status callback with a fresh WASM instance.
    ///
    /// Static method for use by the background typing repeat task (which
    /// doesn't have access to `&self`).
    #[allow(clippy::too_many_arguments)]
    async fn execute_status(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        workspace_store: &Arc<ChannelWorkspaceStore>,
        pairing_store: Arc<PairingStore>,
        timeout: Duration,
        wit_update: wit_channel::StatusUpdate,
    ) -> Result<(), WasmChannelError> {
        if prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store for
                // background typing/status callbacks.
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Cancel the background typing indicator task if running.
    async fn cancel_typing_task(&self) {
        if let Some(handle) = self.typing_task.write().await.take() {
            handle.abort();
        }
    }

    /// Handle a status update, managing the typing repeat timer.
    ///
    /// On Thinking: fires on_status once, then spawns a background task
    /// that repeats the call every 4 seconds (Telegram's typing indicator
    /// expires after ~5s).
    ///
    /// On terminal or user-action-required states: cancels the repeat task,
    /// then fires on_status once.
    ///
    /// On intermediate progress states (tool/auth/job/status updates), keeps
    /// the typing repeater running and fires on_status once.
    /// On StreamChunk: no-op (too noisy).
    async fn handle_status_update(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        fn is_terminal_text_status(msg: &str) -> bool {
            let trimmed = msg.trim();
            trimmed.eq_ignore_ascii_case("done")
                || trimmed.eq_ignore_ascii_case("interrupted")
                || trimmed.eq_ignore_ascii_case("awaiting approval")
                || trimmed.eq_ignore_ascii_case("rejected")
        }

        match &status {
            StatusUpdate::Thinking(_) => {
                // Cancel any existing typing task
                self.cancel_typing_task().await;

                // Diagnostic: log the metadata to verify message_thread_id propagation
                tracing::info!(
                    channel = %self.name,
                    metadata = %metadata,
                    thread_id = ?metadata.get("message_thread_id"),
                    "handle_status_update: Thinking with metadata"
                );

                // Fire once immediately
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status(Thinking) failed (best-effort)"
                    );
                }

                // Spawn background repeater
                let channel_name = self.name.clone();
                let runtime = Arc::clone(&self.runtime);
                let prepared = Arc::clone(&self.prepared);
                let capabilities = self.capabilities.clone();
                let credentials = self.credentials.clone();
                let workspace_store = self.workspace_store.clone();
                let pairing_store = self.pairing_store.clone();
                let callback_timeout = self.runtime.config().callback_timeout;
                let wit_update = status_to_wit(&status, metadata);

                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(4));
                    // Skip the first tick (we already fired above)
                    interval.tick().await;

                    loop {
                        interval.tick().await;

                        let wit_update_clone = clone_wit_status_update(&wit_update);

                        if let Err(e) = Self::execute_status(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            &workspace_store,
                            pairing_store.clone(),
                            callback_timeout,
                            wit_update_clone,
                        )
                        .await
                        {
                            tracing::debug!(
                                channel = %channel_name,
                                error = %e,
                                "Typing repeat on_status failed (best-effort)"
                            );
                        }
                    }
                });

                *self.typing_task.write().await = Some(handle);
            }
            StatusUpdate::StreamChunk(_) => {
                // No-op, too noisy
            }
            StatusUpdate::ApprovalNeeded {
                tool_name,
                description,
                parameters,
                ..
            } => {
                // WASM channels (Telegram, Slack, etc.) cannot render
                // interactive approval overlays.  Send the approval prompt
                // as an actual message so the user can reply yes/no.
                self.cancel_typing_task().await;

                let params_preview = parameters
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| {
                                let val = match v {
                                    serde_json::Value::String(s) => {
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("\"{}...\"", truncated)
                                        } else {
                                            format!("\"{}\"", s)
                                        }
                                    }
                                    other => {
                                        let s = other.to_string();
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("{}...", truncated)
                                        } else {
                                            s
                                        }
                                    }
                                };
                                format!("  {}: {}", k, val)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                let prompt = format!(
                    "Approval needed: {tool_name}\n\
                     {description}\n\
                     \n\
                     Parameters:\n\
                     {params_preview}\n\
                     \n\
                     Reply \"yes\" to approve, \"no\" to deny, or \"always\" to auto-approve."
                );

                let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
                if let Err(e) = self
                    .call_on_respond(uuid::Uuid::new_v4(), &prompt, None, &metadata_json)
                    .await
                {
                    tracing::warn!(
                        channel = %self.name,
                        error = %e,
                        "Failed to send approval prompt via on_respond, falling back to on_status"
                    );
                    // Fall back to status update (typing indicator)
                    let _ = self.call_on_status(&status, metadata).await;
                }
            }
            StatusUpdate::AuthRequired { .. } => {
                // Waiting on user action: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            StatusUpdate::Status(msg) if is_terminal_text_status(msg) => {
                // Waiting on user or terminal states: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            StatusUpdate::ToolStarted { name, .. } => {
                // Accumulate in debug mode; suppress entirely in standard mode.
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Started { name: name.clone() });
                }
            }
            StatusUpdate::ToolCompleted { name, success, .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Completed {
                            name: name.clone(),
                            success: *success,
                        });
                }
            }
            StatusUpdate::ToolResult { preview, .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Result {
                            preview: preview.clone(),
                        });
                }
            }
            // Sub-agent lifecycle: debug-only (noisy orchestration detail).
            StatusUpdate::SubagentSpawned { .. }
            | StatusUpdate::SubagentProgress { .. }
            | StatusUpdate::SubagentCompleted { .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    let _ = self.call_on_status(&status, metadata).await;
                } else {
                    tracing::trace!(
                        channel = %self.name,
                        "Suppressed subagent status (enable /debug to show)"
                    );
                }
            }
            // Canvas actions: debug-only (UI panels have no chat equivalent).
            StatusUpdate::CanvasAction(_) => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    let _ = self.call_on_status(&status, metadata).await;
                }
            }
            // Lifecycle markers are internal bookkeeping, never user-facing.
            StatusUpdate::LifecycleStart { .. } | StatusUpdate::LifecycleEnd { .. } => {}
            _ => {
                // Other intermediate progress status: keep any existing typing task alive.
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
        }

        Ok(())
    }

    /// Process emitted messages from a callback.
    async fn process_emitted_messages(
        &self,
        messages: Vec<EmittedMessage>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_count = messages.len(),
            "Processing emitted messages from WASM callback"
        );

        if messages.is_empty() {
            tracing::debug!(channel = %self.name, "No messages emitted");
            return Ok(());
        }

        let tx_guard = self.message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %self.name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut rate_limiter = self.rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !rate_limiter.check_and_record() {
                tracing::warn!(
                    channel = %self.name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: self.name.clone(),
                });
            }

            let msg = emitted_message_to_incoming_message(&self.name, emitted);

            // Send to stream
            tracing::info!(
                channel = %self.name,
                user_id = %msg.user_id,
                content_len = msg.content.len(),
                "Sending emitted message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %self.name,
                    "Failed to send emitted message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %self.name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }

    /// Start the polling loop if configured.
    ///
    /// Since we can't hold `Arc<Self>` from `&self`, we pass all the components
    /// needed for polling to a spawned task. Each poll tick creates a fresh WASM
    /// instance (matching our "fresh instance per callback" pattern).
    fn start_polling(&self, interval: Duration, shutdown_rx: oneshot::Receiver<()>) {
        let channel_name = self.name.clone();
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let message_tx = self.message_tx.clone();
        let rate_limiter = self.rate_limiter.clone();
        let credentials = self.credentials.clone();
        let pairing_store = self.pairing_store.clone();
        let callback_timeout = self.runtime.config().callback_timeout;
        let workspace_store = self.workspace_store.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            let mut shutdown = std::pin::pin!(shutdown_rx);

            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        tracing::debug!(
                            channel = %channel_name,
                            "Polling tick - calling on_poll"
                        );

                        // Execute on_poll with fresh WASM instance
                        let result = Self::execute_poll(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            pairing_store.clone(),
                            callback_timeout,
                            &workspace_store,
                        ).await;

                        match result {
                            Ok(emitted_messages) => {
                                // Process any emitted messages
                                if !emitted_messages.is_empty()
                                    && let Err(e) = Self::dispatch_emitted_messages(
                                        &channel_name,
                                        emitted_messages,
                                        &message_tx,
                                        &rate_limiter,
                                    ).await {
                                        tracing::warn!(
                                            channel = %channel_name,
                                            error = %e,
                                            "Failed to dispatch emitted messages from poll"
                                        );
                                    }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    channel = %channel_name,
                                    error = %e,
                                    "Polling callback failed"
                                );
                            }
                        }
                    }
                    _ = &mut shutdown => {
                        tracing::info!(
                            channel = %channel_name,
                            "Polling stopped"
                        );
                        break;
                    }
                }
            }
        });
    }

    /// Execute a single poll callback with a fresh WASM instance.
    ///
    /// Returns any emitted messages from the callback. Pending workspace writes
    /// are committed to the shared `ChannelWorkspaceStore` so state persists
    /// across poll ticks (e.g., Telegram polling offset).
    #[allow(clippy::too_many_arguments)]
    async fn execute_poll(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        pairing_store: Arc<PairingStore>,
        timeout: Duration,
        workspace_store: &Arc<ChannelWorkspaceStore>,
    ) -> Result<Vec<EmittedMessage>, WasmChannelError> {
        // Skip if no WASM bytes (testing mode)
        if prepared.component().is_none() {
            tracing::debug!(
                channel = %channel_name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(Vec::new());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(host_state)
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(mut host_state)) => {
                let emitted = host_state.take_emitted_messages();
                tracing::debug!(
                    channel = %channel_name,
                    emitted_count = emitted.len(),
                    "WASM channel on_poll completed"
                );
                Ok(emitted)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Dispatch emitted messages to the message channel.
    ///
    /// This is a static helper used by the polling loop since it doesn't have
    /// access to `&self`.
    async fn dispatch_emitted_messages(
        channel_name: &str,
        messages: Vec<EmittedMessage>,
        message_tx: &RwLock<Option<mpsc::Sender<IncomingMessage>>>,
        rate_limiter: &RwLock<ChannelEmitRateLimiter>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %channel_name,
            message_count = messages.len(),
            "Processing emitted messages from polling callback"
        );

        let tx_guard = message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %channel_name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut limiter = rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !limiter.check_and_record() {
                tracing::warn!(
                    channel = %channel_name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: channel_name.to_string(),
                });
            }

            let msg = emitted_message_to_incoming_message(channel_name, emitted);

            // Send to stream
            tracing::info!(
                channel = %channel_name,
                user_id = %msg.user_id,
                content_len = msg.content.len(),
                "Sending polled message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %channel_name,
                    "Failed to send polled message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %channel_name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct TelegramWebhookInfoEnvelope {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Option<TelegramWebhookInfo>,
}

#[derive(Debug, serde::Deserialize)]
struct TelegramWebhookInfo {
    #[serde(default)]
    url: String,
    #[serde(default)]
    pending_update_count: u64,
    #[serde(default)]
    last_error_date: Option<i64>,
    #[serde(default)]
    last_error_message: Option<String>,
}

const TELEGRAM_POLLING_OVERRIDE: &str = "polling";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PersistedChannelRuntimeState {
    #[serde(default)]
    transport_override: Option<String>,
    #[serde(default)]
    fallback_from_webhook_url: Option<String>,
}

impl WasmChannel {
    fn runtime_state_path(&self) -> std::path::PathBuf {
        thinclaw_platform::state_paths()
            .channels_dir
            .join(format!("{}.runtime.json", self.name))
    }

    fn load_runtime_state(&self) -> PersistedChannelRuntimeState {
        let path = self.runtime_state_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return PersistedChannelRuntimeState::default();
        };

        serde_json::from_str(&content).unwrap_or_else(|error| {
            tracing::warn!(
                channel = %self.name,
                path = %path.display(),
                error = %error,
                "Failed to parse persisted channel runtime state, ignoring"
            );
            PersistedChannelRuntimeState::default()
        })
    }

    fn save_runtime_state(
        &self,
        state: &PersistedChannelRuntimeState,
    ) -> Result<(), std::io::Error> {
        let path = self.runtime_state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let serialized = serde_json::to_vec_pretty(state)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let tmp_path = path.with_extension("runtime.json.tmp");
        std::fs::write(&tmp_path, serialized)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn clear_runtime_state(&self) {
        let path = self.runtime_state_path();
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                channel = %self.name,
                path = %path.display(),
                error = %error,
                "Failed to clear persisted channel runtime state"
            );
        }
    }

    fn telegram_webhook_url_from_tunnel_url(tunnel_url: &str) -> String {
        format!("{}/webhook/telegram", tunnel_url.trim_end_matches('/'))
    }

    fn tunnel_url_from_config(config_json: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(config_json)
            .ok()
            .and_then(|value| {
                value
                    .get("tunnel_url")
                    .and_then(|entry| entry.as_str())
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
    }

    fn apply_telegram_runtime_state(
        &self,
        config_json: String,
        state: &PersistedChannelRuntimeState,
    ) -> String {
        if state.transport_override.as_deref() != Some(TELEGRAM_POLLING_OVERRIDE) {
            return config_json;
        }

        let current_webhook_url = Self::tunnel_url_from_config(&config_json)
            .map(|url| Self::telegram_webhook_url_from_tunnel_url(&url));
        if let (Some(expected_previous), Some(current)) = (
            state.fallback_from_webhook_url.as_deref(),
            current_webhook_url.as_deref(),
        ) && expected_previous != current
        {
            tracing::info!(
                channel = %self.name,
                previous = %expected_previous,
                current = %current,
                "Telegram webhook URL changed, clearing persisted polling fallback"
            );
            self.clear_runtime_state();
            return config_json;
        }

        let mut value = serde_json::from_str::<serde_json::Value>(&config_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        if !value.is_object() {
            value = serde_json::json!({});
        }
        let object = value
            .as_object_mut()
            .expect("fallback configuration should be a JSON object");
        object.insert(
            "transport_override".to_string(),
            serde_json::Value::String(TELEGRAM_POLLING_OVERRIDE.to_string()),
        );
        object.insert("tunnel_url".to_string(), serde_json::Value::Null);

        serde_json::to_string(&value).unwrap_or(config_json)
    }

    fn read_workspace_state(&self, path: &str) -> Option<String> {
        self.workspace_store
            .read(&self.capabilities.prefix_workspace_path(path))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn read_workspace_state_u64(&self, path: &str) -> Option<u64> {
        self.read_workspace_state(path)?.parse::<u64>().ok()
    }

    fn iso_timestamp_from_millis(millis: Option<u64>) -> Option<String> {
        let millis = millis?;
        let millis = i64::try_from(millis).ok()?;
        Utc.timestamp_millis_opt(millis)
            .single()
            .map(|ts| ts.to_rfc3339())
    }

    fn iso_timestamp_from_seconds(seconds: Option<i64>) -> Option<String> {
        let seconds = seconds?;
        Utc.timestamp_opt(seconds, 0)
            .single()
            .map(|ts| ts.to_rfc3339())
    }

    async fn telegram_live_webhook_info(&self) -> Result<Option<TelegramWebhookInfo>, String> {
        let bot_token = self
            .credentials
            .read()
            .await
            .get("TELEGRAM_BOT_TOKEN")
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Missing TELEGRAM_BOT_TOKEN".to_string())?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| format!("Failed to build Telegram client: {}", error))?;
        let response = client
            .get(format!(
                "https://api.telegram.org/bot{}/getWebhookInfo",
                bot_token
            ))
            .send()
            .await
            .map_err(|error| format!("getWebhookInfo request failed: {}", error))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read getWebhookInfo response: {}", error))?;
        if !status.is_success() {
            return Err(format!("getWebhookInfo returned {}: {}", status, body));
        }

        let envelope: TelegramWebhookInfoEnvelope = serde_json::from_str(&body)
            .map_err(|error| format!("Failed to parse getWebhookInfo: {}", error))?;
        if !envelope.ok {
            return Err(envelope
                .description
                .unwrap_or_else(|| "Telegram webhook lookup failed".to_string()));
        }
        Ok(envelope.result)
    }

    fn telegram_polling_unhealthy_reason(
        now_ms: u64,
        last_poll_success_ms: Option<u64>,
        last_poll_started_ms: Option<u64>,
        last_poll_error: Option<&str>,
        poll_stale_after_ms: u64,
    ) -> Option<String> {
        match last_poll_success_ms {
            Some(last_success_ms)
                if now_ms.saturating_sub(last_success_ms) > poll_stale_after_ms =>
            {
                Some(match last_poll_error {
                    Some(error) if !error.trim().is_empty() => {
                        format!("polling stalled: {}", error.trim())
                    }
                    _ => "polling stalled with no recent successful poll".to_string(),
                })
            }
            None if last_poll_started_ms.is_none() => {
                Some("polling has not started yet".to_string())
            }
            None => last_poll_error
                .filter(|error| !error.trim().is_empty())
                .map(|error| format!("polling has not completed successfully: {}", error.trim())),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn telegram_webhook_unhealthy_reason(
        now_ms: u64,
        expected_webhook_url: Option<&str>,
        registered_webhook_url: Option<&str>,
        last_webhook_register_error: Option<&str>,
        registered_webhook_error: Option<&str>,
        pending_updates: Option<u64>,
        last_webhook_register_ms: Option<u64>,
        last_inbound_ms: Option<u64>,
    ) -> Option<String> {
        if let Some(error) = last_webhook_register_error.filter(|error| !error.trim().is_empty()) {
            return Some(format!("webhook registration failed: {}", error.trim()));
        }

        if let Some(error) = registered_webhook_error.filter(|error| !error.trim().is_empty()) {
            return Some(format!("Telegram webhook error: {}", error.trim()));
        }

        match (expected_webhook_url, registered_webhook_url) {
            (Some(expected), Some(registered)) if expected != registered => {
                return Some(format!(
                    "webhook URL mismatch (expected {}, registered {})",
                    expected, registered
                ));
            }
            (Some(_), None) => {
                return Some("Telegram webhook is not registered".to_string());
            }
            (None, _) => {
                return Some("missing expected webhook URL".to_string());
            }
            _ => {}
        }

        let pending_updates = pending_updates.unwrap_or(0);
        if pending_updates == 0 {
            return None;
        }

        let pending_backlog_stale_after_ms = 90_000;
        if let Some(last_inbound_ms) = last_inbound_ms {
            if now_ms.saturating_sub(last_inbound_ms) > pending_backlog_stale_after_ms {
                return Some(format!(
                    "Telegram has {} pending webhook update(s) but inbound delivery is stalled",
                    pending_updates
                ));
            }
            return None;
        }

        let registration_grace_ms = 30_000;
        let registered_long_enough = last_webhook_register_ms
            .map(|registered_at| now_ms.saturating_sub(registered_at) > registration_grace_ms)
            .unwrap_or(true);
        if registered_long_enough {
            return Some(format!(
                "Telegram has {} pending webhook update(s) but ThinClaw has not received any inbound webhook events",
                pending_updates
            ));
        }

        None
    }

    async fn telegram_diagnostics_payload(&self) -> serde_json::Value {
        let runtime_state = self.load_runtime_state();
        let config_snapshot =
            serde_json::from_str::<serde_json::Value>(&self.config_json.read().await.clone())
                .unwrap_or_else(|_| serde_json::json!({}));
        let transport_preference = config_snapshot
            .get("transport_preference")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let transport_reason = config_snapshot
            .get("transport_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let host_tunnel_url = config_snapshot
            .get("host_tunnel_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let host_webhook_capable = config_snapshot
            .get("host_webhook_capable")
            .and_then(|value| value.as_bool());
        let host_transport_reason = config_snapshot
            .get("host_transport_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let transport_mode = self
            .read_workspace_state("state/transport_mode")
            .unwrap_or_else(|| "unknown".to_string());
        let expected_webhook_url = self.read_workspace_state("state/expected_webhook_url");
        let last_webhook_register_ms =
            self.read_workspace_state_u64("state/last_webhook_register_at");
        let last_webhook_register_at = Self::iso_timestamp_from_millis(last_webhook_register_ms);
        let last_poll_started_at = Self::iso_timestamp_from_millis(
            self.read_workspace_state_u64("state/last_poll_started_at"),
        );
        let last_poll_success_at = Self::iso_timestamp_from_millis(
            self.read_workspace_state_u64("state/last_poll_success_at"),
        );
        let last_inbound_at =
            Self::iso_timestamp_from_millis(self.read_workspace_state_u64("state/last_inbound_at"));
        let last_webhook_register_error =
            self.read_workspace_state("state/last_webhook_register_error");
        let last_poll_error = self.read_workspace_state("state/last_poll_error");
        let last_transport_error = self.read_workspace_state("state/last_transport_error");
        let last_update_id = self
            .read_workspace_state("state/last_emitted_update_id")
            .and_then(|value| value.parse::<i64>().ok());

        let mut registered_webhook_url = None;
        let mut registered_webhook_error = None;
        let mut registered_webhook_error_at = None;
        let mut pending_updates = None;

        if transport_mode == "webhook" {
            match self.telegram_live_webhook_info().await {
                Ok(Some(info)) => {
                    registered_webhook_url =
                        (!info.url.trim().is_empty()).then(|| info.url.trim().to_string());
                    registered_webhook_error = info
                        .last_error_message
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                    registered_webhook_error_at =
                        Self::iso_timestamp_from_seconds(info.last_error_date);
                    pending_updates = Some(info.pending_update_count);
                }
                Ok(None) => {}
                Err(error) => {
                    registered_webhook_error = Some(error);
                }
            }
        }

        let last_inbound_ms = self.read_workspace_state_u64("state/last_inbound_at");
        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        let poll_interval_ms = self
            .channel_config
            .read()
            .await
            .as_ref()
            .and_then(|config| config.poll.as_ref().map(|poll| u64::from(poll.interval_ms)))
            .unwrap_or(5_000);
        let poll_stale_after_ms = poll_interval_ms.saturating_mul(6).max(90_000);
        let last_poll_success_ms = self.read_workspace_state_u64("state/last_poll_success_at");
        let last_poll_started_ms = self.read_workspace_state_u64("state/last_poll_started_at");

        let unhealthy_reason = if self.message_tx.read().await.is_none() {
            Some("transport not started".to_string())
        } else if transport_mode == "polling" {
            Self::telegram_polling_unhealthy_reason(
                now_ms,
                last_poll_success_ms,
                last_poll_started_ms,
                last_poll_error.as_deref(),
                poll_stale_after_ms,
            )
        } else if transport_mode == "webhook" {
            Self::telegram_webhook_unhealthy_reason(
                now_ms,
                expected_webhook_url.as_deref(),
                registered_webhook_url.as_deref(),
                last_webhook_register_error.as_deref(),
                registered_webhook_error.as_deref(),
                pending_updates,
                last_webhook_register_ms,
                last_inbound_ms,
            )
        } else {
            None
        };

        serde_json::json!({
            "transport_mode": transport_mode,
            "transport_preference": transport_preference,
            "transport_reason": transport_reason,
            "transport_override": runtime_state.transport_override,
            "fallback_from_webhook_url": runtime_state.fallback_from_webhook_url,
            "host_tunnel_url": host_tunnel_url,
            "host_webhook_capable": host_webhook_capable,
            "host_transport_reason": host_transport_reason,
            "expected_webhook_url": expected_webhook_url,
            "registered_webhook_url": registered_webhook_url,
            "registered_webhook_error": registered_webhook_error,
            "registered_webhook_error_at": registered_webhook_error_at,
            "pending_update_count": pending_updates,
            "last_webhook_register_at": last_webhook_register_at,
            "last_webhook_register_error": last_webhook_register_error,
            "last_poll_started_at": last_poll_started_at,
            "last_poll_success_at": last_poll_success_at,
            "last_poll_error": last_poll_error,
            "last_inbound_at": last_inbound_at,
            "last_transport_error": last_transport_error,
            "last_update_id": last_update_id,
            "unhealthy_reason": unhealthy_reason,
        })
    }

    fn arm_telegram_polling_fallback(&self, diagnostics: &serde_json::Value) {
        if self.name != "telegram" {
            return;
        }

        let transport_mode = diagnostics
            .get("transport_mode")
            .and_then(|value| value.as_str())
            .map(str::trim);
        let unhealthy_reason = diagnostics
            .get("unhealthy_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if transport_mode != Some("webhook") || unhealthy_reason.is_none() {
            return;
        }

        let mut state = self.load_runtime_state();
        if state.transport_override.as_deref() == Some(TELEGRAM_POLLING_OVERRIDE) {
            return;
        }

        state.transport_override = Some(TELEGRAM_POLLING_OVERRIDE.to_string());
        state.fallback_from_webhook_url = diagnostics
            .get("expected_webhook_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        match self.save_runtime_state(&state) {
            Ok(()) => {
                tracing::warn!(
                    channel = %self.name,
                    reason = %unhealthy_reason.unwrap_or("unknown"),
                    expected_webhook_url = ?state.fallback_from_webhook_url,
                    "Telegram webhook unhealthy; forcing polling fallback on next restart"
                );
            }
            Err(error) => {
                tracing::error!(
                    channel = %self.name,
                    error = %error,
                    "Failed to persist Telegram polling fallback state"
                );
            }
        }
    }

    // ── Telegram outbound media attachments ─────────────────────────

    /// Send outbound media attachments to a Telegram chat.
    ///
    /// Uses the Telegram Bot API directly (bypassing the WASM sandbox),
    /// matching the pattern used by `send_draft` and `delete_message`.
    /// Each attachment is sent via the appropriate endpoint based on media
    /// type: `sendPhoto` for images, `sendAudio` for audio, `sendVideo`
    /// for video, and `sendDocument` for everything else.
    ///
    /// Failures are logged but do not abort the response (best-effort).
    async fn send_telegram_attachments(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        attachments: &[thinclaw_media::MediaContent],
    ) {
        if self.name != "telegram" || attachments.is_empty() {
            return;
        }

        // Get bot token from credentials
        let creds = self.credentials.read().await;
        let token = match creds.get("TELEGRAM_BOT_TOKEN").cloned() {
            Some(t) => t,
            None => {
                tracing::debug!("send_telegram_attachments: no TELEGRAM_BOT_TOKEN, skipping");
                return;
            }
        };
        drop(creds);

        let client = reqwest::Client::new();

        for attachment in attachments {
            use thinclaw_media::MediaType;

            // Pick the right Telegram API endpoint based on media type
            let (api_method, field_name) = match attachment.media_type {
                MediaType::Image => ("sendPhoto", "photo"),
                MediaType::Audio => ("sendAudio", "audio"),
                MediaType::Video => ("sendVideo", "video"),
                // PDFs, documents, unknown — all go through sendDocument
                _ => ("sendDocument", "document"),
            };

            let url = format!("https://api.telegram.org/bot{}/{}", token, api_method);

            let filename = attachment
                .filename
                .as_deref()
                .unwrap_or("attachment")
                .to_string();

            let file_part = match reqwest::multipart::Part::bytes(attachment.data.clone())
                .file_name(filename.clone())
                .mime_str(&attachment.mime_type)
            {
                Ok(part) => part,
                Err(e) => {
                    tracing::warn!(
                        channel = %self.name,
                        error = %e,
                        mime = %attachment.mime_type,
                        "Telegram: invalid MIME for attachment, skipping"
                    );
                    continue;
                }
            };

            let mut form = reqwest::multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part(field_name, file_part);

            if let Some(thread_id) = message_thread_id {
                form = form.text("message_thread_id", thread_id.to_string());
            }

            match client
                .post(&url)
                .multipart(form)
                .timeout(Duration::from_secs(120))
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!(
                            channel = %self.name,
                            chat_id = chat_id,
                            method = api_method,
                            filename = %filename,
                            size = attachment.data.len(),
                            "Telegram: attachment sent successfully"
                        );
                    } else {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            channel = %self.name,
                            chat_id = chat_id,
                            method = api_method,
                            status = %status,
                            body = %body,
                            "Telegram: attachment send returned error"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %self.name,
                        chat_id = chat_id,
                        method = api_method,
                        error = %e,
                        "Telegram: attachment HTTP request failed"
                    );
                }
            }
        }
    }
}

fn default_wasm_channel_formatting_hints(channel_name: &str) -> Option<String> {
    match channel_name {
        "telegram" => Some(
            "Prefer Telegram HTML-style formatting for emphasis and links; standard Markdown is also supported as a fallback. Keep code blocks short, avoid markdown tables, and expect long replies to be split into multiple messages."
                .to_string(),
        ),
        "slack" => Some(
            "Use Slack mrkdwn formatting, not GitHub-flavored markdown. Keep replies easy to skim and avoid relying on raw HTML.".to_string(),
        ),
        "whatsapp" => Some(
            "Use WhatsApp-friendly plain text with light emphasis only. Avoid markdown tables, long fenced code blocks, and dense nested structure."
                .to_string(),
        ),
        "discord" => Some(
            "Discord supports markdown and fenced code blocks. Keep long answers readable with short sections and avoid overly wide tables."
                .to_string(),
        ),
        _ => None,
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
        let outbound_content = response_content_for_wasm(&self.name, &response);

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
                self.send_telegram_attachments(chat_id, message_thread_id, &response.attachments)
                    .await;
            }
        }

        // Merge original routing metadata with any response-specific overrides.
        // Response metadata wins on conflicts, and outbound attachments are
        // tunneled through `response_attachments` for WASM channels.
        let metadata_json =
            serde_json::to_string(&merged_response_metadata(&msg.metadata, &response))
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
        // Only Telegram channels support streaming via edit
        if self.name != "telegram" {
            return Ok(None);
        }

        // Extract chat_id and optional message_thread_id from metadata
        let chat_id = metadata.get("chat_id").and_then(|v| v.as_i64());
        let message_thread_id = metadata.get("message_thread_id").and_then(|v| v.as_i64());

        let Some(chat_id) = chat_id else {
            tracing::debug!("send_draft: no chat_id in metadata, skipping");
            return Ok(None);
        };

        // Get bot token from credentials
        let creds = self.credentials.read().await;
        let token = creds.get("TELEGRAM_BOT_TOKEN").cloned();
        drop(creds);

        let Some(token) = token else {
            tracing::debug!("send_draft: no TELEGRAM_BOT_TOKEN in credentials, skipping");
            return Ok(None);
        };

        let client = reqwest::Client::new();

        // Strategy: sendMessage (first call) → editMessageText (subsequent)
        // This is the standard, reliable approach for streaming in Telegram.
        // sendMessageDraft is unreliable (RANDOM_ID_INVALID errors).
        if !draft.posted {
            // ── First chunk: send a new message ──────────────────────────
            let html = super::telegram_html::markdown_to_telegram_html(&draft.accumulated);
            let mut payload = serde_json::json!({
                "chat_id": chat_id,
                "text": html,
                "parse_mode": "HTML",
            });

            if let Some(thread_id) = message_thread_id {
                payload["message_thread_id"] = serde_json::json!(thread_id);
            }

            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        tracing::warn!(
                            channel = %self.name,
                            status = %status,
                            body = %body,
                            "send_draft: initial sendMessage failed"
                        );
                        return Ok(None);
                    }

                    // Extract message_id from the Telegram response
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
                        && let Some(msg_id) = parsed
                            .get("result")
                            .and_then(|r| r.get("message_id"))
                            .and_then(|v| v.as_i64())
                    {
                        tracing::debug!(
                            channel = %self.name,
                            chat_id = chat_id,
                            message_id = msg_id,
                            thread_id = ?message_thread_id,
                            text_len = draft.accumulated.len(),
                            "send_draft: initial message sent"
                        );
                        // Return the message_id as string so DraftReplyState can
                        // track it for subsequent editMessageText calls
                        return Ok(Some(msg_id.to_string()));
                    }
                    tracing::warn!(
                        "send_draft: could not extract message_id from sendMessage response"
                    );
                    Ok(None)
                }
                Err(e) => {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "send_draft: sendMessage HTTP request failed (non-fatal)"
                    );
                    Ok(None)
                }
            }
        } else {
            // ── Subsequent chunks: edit the existing message ─────────────
            let Some(ref msg_id_str) = draft.message_id else {
                // No message_id to edit — skip
                return Ok(None);
            };

            let msg_id: i64 = match msg_id_str.parse() {
                Ok(id) => id,
                Err(_) => return Ok(None),
            };

            let html = super::telegram_html::markdown_to_telegram_html(&draft.accumulated);

            // Telegram's max message length is 4096 chars. Use a lower
            // threshold (3800) to account for HTML tag expansion and
            // avoid edge-case truncation. When exceeded, signal overflow
            // so the dispatcher falls back to on_respond() which splits.
            const TELEGRAM_MAX_SAFE_EDIT_LENGTH: usize = 3800;
            if html.len() > TELEGRAM_MAX_SAFE_EDIT_LENGTH {
                tracing::info!(
                    channel = %self.name,
                    html_len = html.len(),
                    "send_draft: response exceeds Telegram limit, signaling overflow"
                );
                return Err(ChannelError::MessageTooLong {
                    channel: self.name.clone(),
                    length: html.len(),
                    max: TELEGRAM_MAX_SAFE_EDIT_LENGTH,
                });
            }

            let payload = serde_json::json!({
                "chat_id": chat_id,
                "message_id": msg_id,
                "text": html,
                "parse_mode": "HTML",
            });

            let url = format!("https://api.telegram.org/bot{}/editMessageText", token);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        // 400 "message is not modified" is expected when text hasn't
                        // changed enough — treat as non-fatal
                        if body.contains("message is not modified") {
                            return Ok(Some(msg_id_str.clone()));
                        }
                        tracing::debug!(
                            channel = %self.name,
                            status = %status,
                            body = %body,
                            "send_draft: editMessageText failed (non-fatal)"
                        );
                        return Ok(Some(msg_id_str.clone()));
                    }
                    tracing::trace!(
                        channel = %self.name,
                        chat_id = chat_id,
                        message_id = msg_id,
                        text_len = draft.accumulated.len(),
                        "send_draft: message edited"
                    );
                    Ok(Some(msg_id_str.clone()))
                }
                Err(e) => {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "send_draft: editMessageText HTTP request failed (non-fatal)"
                    );
                    Ok(Some(msg_id_str.clone()))
                }
            }
        }
    }

    async fn delete_message(
        &self,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Only Telegram channels support message deletion in this context
        if !self.name.starts_with("telegram") {
            return Ok(());
        }

        // Get bot token from credentials (same pattern as send_draft)
        let creds = self.credentials.read().await;
        let token = creds.get("TELEGRAM_BOT_TOKEN").cloned();
        drop(creds);

        let Some(token) = token else {
            return Ok(());
        };

        // Extract chat_id from metadata (same pattern as send_draft)
        let chat_id = metadata.get("chat_id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        });

        let Some(chat_id) = chat_id else {
            return Ok(());
        };

        let msg_id: i64 = match message_id.parse() {
            Ok(id) => id,
            Err(_) => return Ok(()),
        };

        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/deleteMessage", token);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": msg_id,
        });

        match client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    tracing::debug!(
                        channel = %self.name,
                        message_id = msg_id,
                        "delete_message: message deleted successfully"
                    );
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::debug!(
                        channel = %self.name,
                        message_id = msg_id,
                        body = %body,
                        "delete_message: deleteMessage API failed (non-fatal)"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(
                    channel = %self.name,
                    error = %e,
                    "delete_message: HTTP request failed (non-fatal)"
                );
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.name == "telegram" {
            let diagnostics = self.telegram_diagnostics_payload().await;
            self.arm_telegram_polling_fallback(&diagnostics);
            if diagnostics
                .get("unhealthy_reason")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                return Err(ChannelError::HealthCheckFailed {
                    name: self.name.clone(),
                });
            }
            return Ok(());
        }

        if self.message_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: self.name.clone(),
            })
        }
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        if self.name == "telegram" {
            Some(self.telegram_diagnostics_payload().await)
        } else {
            None
        }
    }

    async fn reset_connection_state(&self) -> Result<(), ChannelError> {
        if self.name == "telegram" {
            self.clear_runtime_state();
        }
        Ok(())
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
        if self.name == "whatsapp" {
            let metadata = merged_response_metadata(&serde_json::Value::Null, &response);
            let has_route = metadata
                .get("phone_number_id")
                .and_then(|value| value.as_str())
                .is_some()
                && metadata
                    .get("recipient_phone")
                    .and_then(|value| value.as_str())
                    .is_some();

            if has_route {
                let metadata_json = serde_json::to_string(&metadata).unwrap_or_default();
                return self
                    .call_on_respond(
                        uuid::Uuid::new_v4(),
                        &response.content,
                        response.thread_id.as_deref(),
                        &metadata_json,
                    )
                    .await
                    .map_err(|e| ChannelError::SendFailed {
                        name: self.name.clone(),
                        reason: format!("broadcast via on_respond: {}", e),
                    });
            }

            tracing::warn!(
                channel = %self.name,
                user_id = %user_id,
                "WASM broadcast: WhatsApp requires explicit route metadata"
            );
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
            self.send_telegram_attachments(chat_id, None, &response.attachments)
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
        let metadata_json =
            serde_json::to_string(&merged_response_metadata(&base_metadata, &response))
                .unwrap_or_default();
        let outbound_content = response_content_for_wasm(&self.name, &response);

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

// ============================================================================
// WIT Type Conversion Helpers
// ============================================================================

// Type aliases for the generated WIT types (exported interface)
use exports::near::agent::channel as wit_channel;

/// Convert WIT-generated ChannelConfig to our internal type.
fn convert_channel_config(wit: wit_channel::ChannelConfig) -> ChannelConfig {
    ChannelConfig {
        display_name: wit.display_name,
        http_endpoints: wit
            .http_endpoints
            .into_iter()
            .map(|ep| crate::wasm::schema::HttpEndpointConfigSchema {
                path: ep.path,
                methods: ep.methods,
                require_secret: ep.require_secret,
            })
            .collect(),
        poll: wit.poll.map(|p| crate::wasm::schema::PollConfigSchema {
            interval_ms: p.interval_ms,
            enabled: p.enabled,
        }),
    }
}

/// Convert WIT-generated OutgoingHttpResponse to our HttpResponse type.
fn convert_http_response(wit: wit_channel::OutgoingHttpResponse) -> HttpResponse {
    let headers = serde_json::from_str(&wit.headers_json).unwrap_or_default();
    HttpResponse {
        status: wit.status,
        headers,
        body: wit.body,
    }
}

/// Convert a StatusUpdate + metadata into the WIT StatusUpdate type.
fn truncate_status_text(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn status_to_wit(status: &StatusUpdate, metadata: &serde_json::Value) -> wit_channel::StatusUpdate {
    let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

    match status {
        StatusUpdate::Thinking(msg) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: msg.clone(),
            metadata_json,
        },
        StatusUpdate::ToolStarted { name, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolStarted,
            message: format!("Tool started: {}", name),
            metadata_json,
        },
        StatusUpdate::ToolCompleted { name, success, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolCompleted,
            message: format!(
                "Tool completed: {} ({})",
                name,
                if *success { "ok" } else { "failed" }
            ),
            metadata_json,
        },
        StatusUpdate::ToolResult { name, preview, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolResult,
            message: format!(
                "Tool result: {}\n{}",
                name,
                truncate_status_text(preview, 280)
            ),
            metadata_json,
        },
        StatusUpdate::StreamChunk(chunk) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: chunk.clone(),
            metadata_json,
        },
        StatusUpdate::Status(msg) => {
            // Map well-known status strings to WIT types (case-insensitive
            // to stay consistent with is_terminal_text_status and the
            // Telegram-side classify_status_update).
            let trimmed = msg.trim();
            let status_type = if trimmed.eq_ignore_ascii_case("done") {
                wit_channel::StatusType::Done
            } else if trimmed.eq_ignore_ascii_case("interrupted") {
                wit_channel::StatusType::Interrupted
            } else {
                wit_channel::StatusType::Status
            };
            wit_channel::StatusUpdate {
                status: status_type,
                message: msg.clone(),
                metadata_json,
            }
        }
        StatusUpdate::Plan { entries } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[plan] {}",
                serde_json::to_string(entries).unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[usage] {} input + {} output tokens{}{}",
                input_tokens,
                output_tokens,
                cost_usd
                    .map(|cost| format!(", ${cost:.6}"))
                    .unwrap_or_default(),
                model
                    .as_deref()
                    .map(|model| format!(" ({model})"))
                    .unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ApprovalNeeded,
            message: format!(
                "Approval needed for tool '{}'. {}\nRequest ID: {}\nReply with: yes (or /approve), no (or /deny), or always (or /always).",
                tool_name, description, request_id
            ),
            metadata_json,
        },
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::JobStarted,
            message: format!("Job started: {} ({})\n{}", title, job_id, browse_url),
            metadata_json,
        },
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthRequired,
            message: {
                let mut lines = vec![format!("Authentication required for {}.", extension_name)];
                if let Some(text) = instructions
                    && !text.trim().is_empty()
                {
                    lines.push(text.trim().to_string());
                }
                if let Some(url) = auth_url {
                    lines.push(format!("Auth URL: {}", url));
                }
                if let Some(url) = setup_url {
                    lines.push(format!("Setup URL: {}", url));
                }
                lines.join("\n")
            },
            metadata_json,
        },
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthCompleted,
            message: format!(
                "Authentication {} for {}. {}",
                if *success { "completed" } else { "failed" },
                extension_name,
                message
            ),
            metadata_json,
        },
        StatusUpdate::Error { message, code } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[error{}] {}",
                code.as_ref().map(|c| format!(": {c}")).unwrap_or_default(),
                message
            ),
            metadata_json,
        },
        StatusUpdate::CanvasAction(action) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[canvas] {}",
                serde_json::to_string(action).unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::AgentMessage {
            content,
            message_type,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!("[agent_message:{}] {}", message_type, content),
            metadata_json,
        },
        StatusUpdate::LifecycleStart { run_id } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: format!("lifecycle:start:{}", run_id),
            metadata_json,
        },
        StatusUpdate::LifecycleEnd { run_id, phase } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Done,
            message: format!("lifecycle:end:{}:{}", phase, run_id),
            metadata_json,
        },
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:spawned:{}] {}",
                agent_id,
                serde_json::to_string(&serde_json::json!({
                    "name": name,
                    "task": task,
                }))
                .unwrap_or_else(|_| format!("{} - {}", name, task))
            ),
            metadata_json,
        },
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:progress:{}:{}] {}",
                agent_id,
                category,
                serde_json::to_string(&serde_json::json!({
                    "message": message,
                }))
                .unwrap_or_else(|_| message.clone())
            ),
            metadata_json,
        },
        StatusUpdate::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:{}:{}] {}",
                if *success { "completed" } else { "failed" },
                agent_id,
                serde_json::to_string(&serde_json::json!({
                    "name": name,
                    "success": success,
                    "response": response,
                    "duration_ms": duration_ms,
                    "iterations": iterations,
                }))
                .unwrap_or_else(|_| format!(
                    "{} ({:.1}s)",
                    name,
                    *duration_ms as f64 / 1000.0
                ))
            ),
            metadata_json,
        },
    }
}

/// Clone a WIT StatusUpdate (the generated type doesn't derive Clone).
fn clone_wit_status_update(update: &wit_channel::StatusUpdate) -> wit_channel::StatusUpdate {
    wit_channel::StatusUpdate {
        status: match update.status {
            wit_channel::StatusType::Thinking => wit_channel::StatusType::Thinking,
            wit_channel::StatusType::Done => wit_channel::StatusType::Done,
            wit_channel::StatusType::Interrupted => wit_channel::StatusType::Interrupted,
            wit_channel::StatusType::ToolStarted => wit_channel::StatusType::ToolStarted,
            wit_channel::StatusType::ToolCompleted => wit_channel::StatusType::ToolCompleted,
            wit_channel::StatusType::ToolResult => wit_channel::StatusType::ToolResult,
            wit_channel::StatusType::ApprovalNeeded => wit_channel::StatusType::ApprovalNeeded,
            wit_channel::StatusType::Status => wit_channel::StatusType::Status,
            wit_channel::StatusType::JobStarted => wit_channel::StatusType::JobStarted,
            wit_channel::StatusType::AuthRequired => wit_channel::StatusType::AuthRequired,
            wit_channel::StatusType::AuthCompleted => wit_channel::StatusType::AuthCompleted,
        },
        message: update.message.clone(),
        metadata_json: update.metadata_json.clone(),
    }
}

fn emitted_message_to_incoming_message(
    channel_name: &str,
    emitted: EmittedMessage,
) -> IncomingMessage {
    let parsed_metadata = serde_json::from_str::<serde_json::Value>(&emitted.metadata_json)
        .unwrap_or(serde_json::Value::Null);
    let legacy_thread_id = emitted.thread_id.clone();

    let mut msg = wasm_emitted_incoming_event(channel_name, &emitted, &parsed_metadata)
        .map(normalize_incoming_event)
        .unwrap_or_else(|| {
            let mut msg = IncomingMessage::new(channel_name, &emitted.user_id, &emitted.content);
            if let Some(thread_id) = emitted.thread_id.clone() {
                msg = msg.with_thread(thread_id);
            }
            msg
        });

    if let Some(name) = emitted.user_name {
        msg = msg.with_user_name(name);
    }

    let mut metadata = metadata_object(&parsed_metadata, "package_metadata");
    for (key, value) in metadata_object(&msg.metadata, "normalized_metadata") {
        let collision_key = match key.as_str() {
            "chat_id" if metadata.contains_key("chat_id") => Some("canonical_chat_id"),
            "chat_type" if metadata.contains_key("chat_type") => Some("canonical_chat_type"),
            _ => None,
        };
        if let Some(collision_key) = collision_key {
            metadata.insert(collision_key.to_string(), value);
        } else {
            metadata.insert(key, value);
        }
    }
    if let Some(legacy_thread_id) = legacy_thread_id.as_deref() {
        add_legacy_thread_aliases(&mut metadata, channel_name, legacy_thread_id);
        metadata.insert(
            "package_thread_id".to_string(),
            serde_json::Value::String(legacy_thread_id.to_string()),
        );
    }
    if let Some(command) = parse_slash_command(&msg.content) {
        metadata.insert(
            "slash_command".to_string(),
            serde_json::json!({
                "command": command.command,
                "args": command.args,
            }),
        );
    }
    if !metadata.contains_key("conversation_kind")
        && let Some(chat_type) = metadata.get("chat_type").and_then(|value| value.as_str())
    {
        let conversation_kind = if chat_type == "dm" { "direct" } else { "group" };
        metadata.insert(
            "conversation_kind".to_string(),
            serde_json::Value::String(conversation_kind.to_string()),
        );
    }
    msg = msg.with_metadata(serde_json::Value::Object(metadata));

    for att in &emitted.attachments {
        msg.attachments.push(att.to_media_content());
    }

    msg
}

fn wasm_emitted_incoming_event(
    channel_name: &str,
    emitted: &EmittedMessage,
    metadata: &serde_json::Value,
) -> Option<IncomingEvent> {
    let chat_type = wasm_emitted_chat_type(channel_name, metadata);
    let chat_id = wasm_emitted_chat_id(channel_name, &chat_type, emitted, metadata)?;

    Some(IncomingEvent {
        platform: channel_name.to_string(),
        chat_type,
        chat_id,
        user_id: emitted.user_id.clone(),
        user_name: emitted.user_name.clone(),
        text: emitted.content.clone(),
        metadata: metadata.clone(),
    })
}

fn wasm_emitted_chat_type(channel_name: &str, metadata: &serde_json::Value) -> String {
    if let Some(chat_type) = metadata_string(metadata, "chat_type") {
        return normalize_chat_type(&chat_type);
    }
    if let Some(kind) = metadata_string(metadata, "conversation_kind") {
        return normalize_chat_type(&kind);
    }
    if let Some(is_private) = metadata_bool(metadata, "is_private") {
        return if is_private { "dm" } else { "group" }.to_string();
    }
    if metadata_bool(metadata, "is_group").unwrap_or(false) {
        return "group".to_string();
    }
    match channel_name {
        "whatsapp" => "dm".to_string(),
        "discord" | "slack" => "group".to_string(),
        _ => "chat".to_string(),
    }
}

fn normalize_chat_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "direct" | "private" | "dm" => "dm".to_string(),
        "group" | "supergroup" | "channel" | "room" => "group".to_string(),
        "" => "chat".to_string(),
        other => other.to_string(),
    }
}

fn wasm_emitted_chat_id(
    channel_name: &str,
    chat_type: &str,
    emitted: &EmittedMessage,
    metadata: &serde_json::Value,
) -> Option<String> {
    match channel_name {
        "telegram" => {
            let chat_id = metadata_string(metadata, "chat_id")?;
            if chat_type == "group"
                && let Some(thread_id) = metadata_string(metadata, "message_thread_id")
            {
                return Some(format!("{chat_id}:topic:{thread_id}"));
            }
            Some(chat_id)
        }
        "slack" => {
            let channel = metadata_string(metadata, "channel")
                .or_else(|| metadata_string(metadata, "channel_id"))?;
            metadata_string(metadata, "thread_ts")
                .filter(|thread_ts| !thread_ts.is_empty())
                .map(|thread_ts| format!("{channel}:thread:{thread_ts}"))
                .or(Some(channel))
        }
        "whatsapp" => metadata_string(metadata, "sender_phone")
            .or_else(|| metadata_string(metadata, "chat_id"))
            .or_else(|| metadata_string(metadata, "phone_number_id")),
        "discord" => metadata_string(metadata, "thread_id")
            .or_else(|| metadata_string(metadata, "channel_id"))
            .or_else(|| emitted.thread_id.clone()),
        _ => metadata_string(metadata, "chat_id")
            .or_else(|| metadata_string(metadata, "channel_id"))
            .or_else(|| metadata_string(metadata, "conversation_id"))
            .or_else(|| emitted.thread_id.clone()),
    }
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    let value = metadata.get(key)?;
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn metadata_bool(metadata: &serde_json::Value, key: &str) -> Option<bool> {
    match metadata.get(key)? {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn add_legacy_thread_aliases(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    channel_name: &str,
    legacy_thread_id: &str,
) {
    let legacy_thread_id = legacy_thread_id.trim();
    if legacy_thread_id.is_empty() {
        return;
    }

    let aliases = metadata
        .entry("legacy_session_key_aliases".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(alias_values) = aliases.as_array_mut() else {
        return;
    };

    for alias in [
        legacy_thread_id.to_string(),
        format!("{channel_name}:{legacy_thread_id}"),
        format!("agent:main:{channel_name}:{legacy_thread_id}"),
    ] {
        let value = serde_json::Value::String(alias);
        if !alias_values.contains(&value) {
            alias_values.push(value);
        }
    }
}

/// HTTP response from a WASM channel callback.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: Vec<u8>,
}

fn metadata_object(
    value: &serde_json::Value,
    fallback_key: &str,
) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert(fallback_key.to_string(), other.clone());
            map
        }
    }
}

fn serialize_response_attachments(
    attachments: &[thinclaw_media::MediaContent],
) -> Option<serde_json::Value> {
    if attachments.is_empty() {
        return None;
    }

    use base64::Engine;

    Some(serde_json::Value::Array(
        attachments
            .iter()
            .map(|attachment| {
                serde_json::json!({
                    "mime_type": attachment.mime_type,
                    "filename": attachment.filename,
                    "data": base64::engine::general_purpose::STANDARD.encode(&attachment.data),
                })
            })
            .collect(),
    ))
}

fn merged_response_metadata(
    original_metadata: &serde_json::Value,
    response: &OutgoingResponse,
) -> serde_json::Value {
    let mut merged = metadata_object(original_metadata, "original_metadata");

    for (key, value) in metadata_object(&response.metadata, "response_metadata") {
        merged.insert(key, value);
    }

    if let Some(serialized_attachments) = serialize_response_attachments(&response.attachments) {
        merged.insert("response_attachments".to_string(), serialized_attachments);
    }

    serde_json::Value::Object(merged)
}

fn response_content_for_wasm(channel_name: &str, response: &OutgoingResponse) -> String {
    if response.attachments.is_empty() || wasm_channel_has_media_delivery(channel_name) {
        return response.content.clone();
    }

    let mut content = response.content.clone();
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str("Generated media:");
    for attachment in &response.attachments {
        let filename = attachment.filename.as_deref().unwrap_or("generated-media");
        let source = attachment.source_url.as_deref().unwrap_or("stored locally");
        content.push_str(&format!(
            "\n- {} ({} bytes, {}): {}",
            filename,
            attachment.data.len(),
            attachment.mime_type,
            source
        ));
    }
    tracing::info!(
        channel = %channel_name,
        attachment_count = response.attachments.len(),
        "WASM channel using generated media text fallback"
    );
    content
}

fn wasm_channel_has_media_delivery(channel_name: &str) -> bool {
    matches!(channel_name, "telegram" | "whatsapp" | "slack" | "discord")
}

impl HttpResponse {
    /// Create an OK response.
    pub fn ok() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Create a JSON response.
    pub fn json(value: serde_json::Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_default();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            status: 200,
            headers,
            body,
        }
    }

    /// Create an error response.
    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: message.as_bytes().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::merged_response_metadata;
    use crate::pairing::PairingStore;
    use crate::wasm::capabilities::ChannelCapabilities;
    use crate::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::wasm::wrapper::{
        ChannelWorkspaceStore, HttpResponse, WasmChannel, default_wasm_channel_formatting_hints,
        response_content_for_wasm,
    };
    use thinclaw_channels_core::Channel;

    fn create_test_channel() -> WasmChannel {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing("test", "Test channel"));

        let capabilities = ChannelCapabilities::for_channel("test").with_path("/webhook/test");

        WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            None,
            Arc::new(PairingStore::new()),
        )
    }

    #[test]
    fn test_channel_name() {
        let channel = create_test_channel();
        assert_eq!(channel.name(), "test");
    }

    #[test]
    fn test_channel_uses_explicit_formatting_hints() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing(
            "custom",
            "Custom channel",
        ));

        let channel = WasmChannel::new(
            runtime,
            prepared,
            ChannelCapabilities::for_channel("custom"),
            "{}".to_string(),
            Some("Use plain text only.".to_string()),
            Arc::new(PairingStore::new()),
        );

        assert_eq!(
            channel.formatting_hints().as_deref(),
            Some("Use plain text only.")
        );
    }

    #[test]
    fn test_channel_falls_back_to_builtin_platform_hints() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing(
            "telegram",
            "Telegram channel",
        ));

        let channel = WasmChannel::new(
            runtime,
            prepared,
            ChannelCapabilities::for_channel("telegram"),
            "{}".to_string(),
            None,
            Arc::new(PairingStore::new()),
        );

        let hints = channel
            .formatting_hints()
            .expect("telegram should have default hints");
        assert!(hints.contains("Telegram"));
        assert!(hints.contains("HTML"));
    }

    #[test]
    fn test_builtin_platform_hint_mapping_covers_supported_wasm_channels() {
        let telegram = default_wasm_channel_formatting_hints("telegram")
            .expect("telegram fallback hints should exist");
        assert!(telegram.contains("Telegram"));

        let slack =
            default_wasm_channel_formatting_hints("slack").expect("slack fallback should exist");
        assert!(slack.contains("Slack"));

        let whatsapp = default_wasm_channel_formatting_hints("whatsapp")
            .expect("whatsapp fallback should exist");
        assert!(whatsapp.contains("WhatsApp"));

        let discord = default_wasm_channel_formatting_hints("discord")
            .expect("discord fallback should exist");
        assert!(discord.contains("Discord"));

        assert!(default_wasm_channel_formatting_hints("custom").is_none());
    }

    #[test]
    fn test_http_response_ok() {
        let response = HttpResponse::ok();
        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
    }

    #[test]
    fn test_http_response_json() {
        let response = HttpResponse::json(serde_json::json!({"key": "value"}));
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_http_response_error() {
        let response = HttpResponse::error(400, "Bad request");
        assert_eq!(response.status, 400);
        assert_eq!(response.body, b"Bad request");
    }

    #[tokio::test]
    async fn test_channel_start_and_shutdown() {
        let channel = create_test_channel();

        // Start should succeed
        let stream = channel.start().await;
        assert!(stream.is_ok());

        // Health check should pass
        assert!(channel.health_check().await.is_ok());

        // Shutdown should succeed
        assert!(channel.shutdown().await.is_ok());

        // Health check should fail after shutdown
        assert!(channel.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_execute_poll_no_wasm_returns_empty() {
        // When there's no WASM module (None component), execute_poll
        // should return an empty vector of messages
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing(
            "poll-test",
            "Test channel",
        ));

        let capabilities = ChannelCapabilities::for_channel("poll-test").with_polling(1000);
        let credentials = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let timeout = std::time::Duration::from_secs(5);

        let workspace_store = Arc::new(crate::wasm::host::ChannelWorkspaceStore::new());

        let result = WasmChannel::execute_poll(
            "poll-test",
            &runtime,
            &prepared,
            &capabilities,
            &credentials,
            Arc::new(PairingStore::new()),
            timeout,
            &workspace_store,
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_sends_to_channel() {
        use crate::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::wasm::host::ChannelEmitRateLimiter::new(
                crate::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![
            EmittedMessage::new("user1", "Hello from polling!"),
            EmittedMessage::new("user2", "Another message"),
        ];

        let result = WasmChannel::dispatch_emitted_messages(
            "test-channel",
            messages,
            &message_tx,
            &rate_limiter,
        )
        .await;

        assert!(result.is_ok());

        // Verify messages were sent
        let msg1 = rx.try_recv().expect("Should receive first message");
        assert_eq!(msg1.user_id, "user1");
        assert_eq!(msg1.content, "Hello from polling!");

        let msg2 = rx.try_recv().expect("Should receive second message");
        assert_eq!(msg2.user_id, "user2");
        assert_eq!(msg2.content, "Another message");

        // No more messages
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_wasm_emitted_whatsapp_message_uses_incoming_event_session_key() {
        use super::emitted_message_to_incoming_message;
        use crate::wasm::host::EmittedMessage;
        use thinclaw_identity::ConversationKind;

        let metadata = serde_json::json!({
            "sender_phone": "+15551234567",
            "phone_number_id": "biz-1",
            "conversation_kind": "direct",
            "conversation_scope_id": "whatsapp:direct:biz-1:+15551234567",
            "external_conversation_key": "whatsapp://direct/biz-1/+15551234567"
        });
        let emitted =
            EmittedMessage::new("+15551234567", "hello").with_metadata(metadata.to_string());

        let msg = emitted_message_to_incoming_message("whatsapp", emitted);

        assert_eq!(msg.channel, "whatsapp");
        assert_eq!(
            msg.thread_id.as_deref(),
            Some("agent:main:whatsapp:dm:+15551234567")
        );
        assert_eq!(msg.metadata["session_key"], msg.thread_id.clone().unwrap());
        assert_eq!(msg.metadata["raw"]["sender_phone"], "+15551234567");
        assert_eq!(msg.metadata["sender_phone"], "+15551234567");

        let identity = msg.resolved_identity();
        assert_eq!(identity.conversation_kind, ConversationKind::Direct);
        assert_eq!(identity.principal_id, "+15551234567");
    }

    #[test]
    fn test_wasm_emitted_telegram_group_topic_keeps_legacy_alias_and_slash_parse() {
        use super::emitted_message_to_incoming_message;
        use crate::wasm::host::EmittedMessage;
        use thinclaw_identity::ConversationKind;

        let metadata = serde_json::json!({
            "chat_id": -100123,
            "message_thread_id": 99,
            "is_private": false,
            "conversation_kind": "group",
            "conversation_scope_id": "telegram:group:-100123:topic:99",
            "external_conversation_key": "telegram://group/-100123/topic/99"
        });
        let emitted = EmittedMessage::new("42", "/Summarize   sprint notes")
            .with_thread_id("99")
            .with_metadata(metadata.to_string());

        let msg = emitted_message_to_incoming_message("telegram", emitted);

        assert_eq!(
            msg.thread_id.as_deref(),
            Some("agent:main:telegram:group:-100123_topic_99")
        );
        assert_eq!(msg.metadata["chat_id"], -100123);
        assert_eq!(msg.metadata["canonical_chat_id"], "-100123:topic:99");
        assert_eq!(msg.metadata["message_thread_id"], 99);
        assert_eq!(msg.metadata["slash_command"]["command"], "summarize");
        assert_eq!(msg.metadata["slash_command"]["args"], "sprint notes");

        let aliases = msg
            .metadata
            .get("legacy_session_key_aliases")
            .and_then(|value| value.as_array())
            .expect("legacy aliases should be present");
        assert!(aliases.contains(&serde_json::json!("telegram:group:-100123_topic_99")));
        assert!(aliases.contains(&serde_json::json!("99")));
        assert!(aliases.contains(&serde_json::json!("telegram:99")));

        let identity = msg.resolved_identity();
        assert_eq!(identity.conversation_kind, ConversationKind::Group);
        assert_eq!(
            identity.stable_external_conversation_key,
            "telegram://group/-100123/topic/99"
        );
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_no_sender_returns_ok() {
        use crate::wasm::host::EmittedMessage;

        // No sender available (channel not started)
        let message_tx = Arc::new(tokio::sync::RwLock::new(None));
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::wasm::host::ChannelEmitRateLimiter::new(
                crate::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![EmittedMessage::new("user1", "Hello!")];

        // Should return Ok even without a sender (logs warning but doesn't fail)
        let result = WasmChannel::dispatch_emitted_messages(
            "test-channel",
            messages,
            &message_tx,
            &rate_limiter,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_channel_with_polling_stores_shutdown_sender() {
        // Create a channel with polling capabilities
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing(
            "poll-channel",
            "Polling test channel",
        ));

        // Enable polling with a 1 second minimum interval
        let capabilities = ChannelCapabilities::for_channel("poll-channel")
            .with_path("/webhook/poll")
            .with_polling(1000);

        let channel = WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            None,
            Arc::new(PairingStore::new()),
        );

        // Start the channel
        let _stream = channel.start().await.expect("Channel should start");

        // Verify poll_shutdown_tx is set (polling was started)
        // Note: For testing channels without WASM, on_start returns no poll config,
        // so polling won't actually be started. This verifies the basic lifecycle.
        assert!(channel.health_check().await.is_ok());

        // Shutdown should clean up properly
        channel.shutdown().await.expect("Shutdown should succeed");
        assert!(channel.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_call_on_poll_no_wasm_succeeds() {
        // Verify call_on_poll returns Ok when there's no WASM module
        let channel = create_test_channel();

        // Start the channel first to set up message_tx
        let _stream = channel.start().await.expect("Channel should start");

        // call_on_poll should succeed (no-op for no WASM)
        let result = channel.call_on_poll().await;
        assert!(result.is_ok());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_starts_on_thinking() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Sending Thinking should succeed (no-op for no WASM)
        let result = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(result.is_ok());

        // A typing task should have been spawned
        assert!(channel.typing_task.read().await.is_some());

        // Shutdown should cancel the typing task
        channel.shutdown().await.expect("Shutdown should succeed");
        assert!(channel.typing_task.read().await.is_none());
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_done() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Send Done status
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Status("Done".into()),
                &metadata,
            )
            .await;

        // Typing task should be cancelled
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_persists_on_tool_started() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Intermediate tool status should not cancel typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::ToolStarted {
                    name: "http_request".into(),
                    parameters: None,
                },
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_some());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_approval_needed() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Approval-needed should stop typing while waiting for user action
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
                    request_id: "req-1".into(),
                    tool_name: "http_request".into(),
                    description: "Fetch weather".into(),
                    parameters: serde_json::json!({"url": "https://wttr.in"}),
                },
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_awaiting_approval_status() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Legacy terminal status string should also cancel typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Status("Awaiting approval".into()),
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_replaced_on_new_thinking() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("First...".into()),
                &metadata,
            )
            .await;

        // Get handle of first task
        let first_handle = {
            let guard = channel.typing_task.read().await;
            guard.as_ref().map(|h| h.id())
        };
        assert!(first_handle.is_some());

        // Start typing again (should replace the previous task)
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Second...".into()),
                &metadata,
            )
            .await;

        // Should still have a typing task, but it's a new one
        let second_handle = {
            let guard = channel.typing_task.read().await;
            guard.as_ref().map(|h| h.id())
        };
        assert!(second_handle.is_some());
        // The task IDs should differ (old one was aborted, new one spawned)
        assert_ne!(first_handle, second_handle);

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_respond_cancels_typing_task() {
        use thinclaw_channels_core::IncomingMessage;

        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Respond should cancel the typing task
        let msg = IncomingMessage::new("test", "user1", "hello").with_metadata(metadata);
        let _ = channel
            .respond(
                &msg,
                thinclaw_channels_core::OutgoingResponse::text("response"),
            )
            .await;

        // Typing task should be gone
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_stream_chunk_is_noop() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // StreamChunk should not start a typing task
        let result = channel
            .send_status(
                thinclaw_channels_core::StatusUpdate::StreamChunk("chunk".into()),
                &metadata,
            )
            .await;
        assert!(result.is_ok());
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[test]
    fn test_status_to_wit_thinking() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Thinking
        ));
        assert_eq!(wit.message, "Processing...");
        assert!(wit.metadata_json.contains("42"));
    }

    #[test]
    fn test_status_to_wit_done() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status("Done".into()),
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
    }

    #[test]
    fn test_status_to_wit_done_case_insensitive() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);

        // lowercase
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status("done".into()),
            &metadata,
        );
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));

        // with whitespace
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status(" Done ".into()),
            &metadata,
        );
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
    }

    #[test]
    fn test_status_to_wit_interrupted() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status("Interrupted".into()),
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));
    }

    #[test]
    fn test_status_to_wit_interrupted_case_insensitive() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);

        // lowercase
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status("interrupted".into()),
            &metadata,
        );
        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));

        // with whitespace
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status(" Interrupted ".into()),
            &metadata,
        );
        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));
    }

    #[test]
    fn test_status_to_wit_generic_status() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::Status("Awaiting approval".into()),
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
        assert_eq!(wit.message, "Awaiting approval");
    }

    #[test]
    fn test_status_to_wit_subagent_spawned_uses_structured_payload() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::SubagentSpawned {
                agent_id: "agent-1".to_string(),
                name: "Researcher".to_string(),
                task: "Check brave search".to_string(),
                task_packet: thinclaw_types::SubagentTaskPacket {
                    objective: "Check brave search".to_string(),
                    ..Default::default()
                },
                allowed_tools: vec![],
                allowed_skills: vec![],
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
        assert!(wit.message.starts_with("[subagent:spawned:agent-1] "));

        let payload = wit
            .message
            .split_once("] ")
            .map(|(_, payload)| payload)
            .expect("spawned message should include payload");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("spawned payload should be valid JSON");
        assert_eq!(payload["name"], "Researcher");
        assert_eq!(payload["task"], "Check brave search");
    }

    #[test]
    fn test_status_to_wit_subagent_progress_uses_structured_payload() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::SubagentProgress {
                agent_id: "agent-1".to_string(),
                message: "Running brave-search".to_string(),
                category: "tool".to_string(),
            },
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
        assert!(wit.message.starts_with("[subagent:progress:agent-1:tool] "));

        let payload = wit
            .message
            .split_once("] ")
            .map(|(_, payload)| payload)
            .expect("progress message should include payload");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("progress payload should be valid JSON");
        assert_eq!(payload["message"], "Running brave-search");
    }

    #[test]
    fn test_status_to_wit_subagent_completed_uses_structured_payload() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::SubagentCompleted {
                agent_id: "agent-1".to_string(),
                name: "Researcher".to_string(),
                success: true,
                response: "Done".to_string(),
                duration_ms: 1850,
                iterations: 3,
                task_packet: thinclaw_types::SubagentTaskPacket {
                    objective: "Check brave search".to_string(),
                    ..Default::default()
                },
                allowed_tools: vec![],
                allowed_skills: vec![],
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
        assert!(wit.message.starts_with("[subagent:completed:agent-1] "));

        let payload = wit
            .message
            .split_once("] ")
            .map(|(_, payload)| payload)
            .expect("completed message should include payload");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("completed payload should be valid JSON");
        assert_eq!(payload["name"], "Researcher");
        assert_eq!(payload["success"], true);
        assert_eq!(payload["response"], "Done");
        assert_eq!(payload["duration_ms"], 1850);
        assert_eq!(payload["iterations"], 3);
    }

    #[test]
    fn test_status_to_wit_auth_required() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::AuthRequired {
                extension_name: "weather".to_string(),
                instructions: Some("Paste your token".to_string()),
                auth_url: Some("https://example.com/auth".to_string()),
                setup_url: None,
                auth_mode: "manual_token".to_string(),
                auth_status: "awaiting_token".to_string(),
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
                thread_id: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthRequired
        ));
        assert!(wit.message.contains("Authentication required for weather"));
        assert!(wit.message.contains("Paste your token"));
    }

    #[test]
    fn test_status_to_wit_tool_started() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 7});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ToolStarted {
                name: "http_request".to_string(),
                parameters: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolStarted
        ));
        assert_eq!(wit.message, "Tool started: http_request");
    }

    #[test]
    fn test_status_to_wit_tool_completed_success() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ToolCompleted {
                name: "http_request".to_string(),
                success: true,
                result_preview: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolCompleted
        ));
        assert_eq!(wit.message, "Tool completed: http_request (ok)");
    }

    #[test]
    fn test_status_to_wit_tool_completed_failure() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ToolCompleted {
                name: "http_request".to_string(),
                success: false,
                result_preview: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolCompleted
        ));
        assert_eq!(wit.message, "Tool completed: http_request (failed)");
    }

    #[test]
    fn test_status_to_wit_tool_result() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ToolResult {
                name: "http_request".to_string(),
                preview: "{".to_string() + "\"temperature\": 22}",
                artifacts: Vec::new(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolResult
        ));
        assert!(wit.message.starts_with("Tool result: http_request\n"));
    }

    #[test]
    fn test_status_to_wit_tool_result_truncates_preview() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let long_preview = "x".repeat(400);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ToolResult {
                name: "big_tool".to_string(),
                preview: long_preview,
                artifacts: Vec::new(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolResult
        ));
        assert!(wit.message.ends_with("..."));
    }

    #[test]
    fn test_status_to_wit_job_started() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 1});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::JobStarted {
                job_id: "job-1".to_string(),
                title: "Daily sync".to_string(),
                browse_url: "https://example.com/jobs/job-1".to_string(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::JobStarted
        ));
        assert!(wit.message.contains("Daily sync"));
        assert!(wit.message.contains("https://example.com/jobs/job-1"));
    }

    #[test]
    fn test_status_to_wit_auth_completed_success() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::AuthCompleted {
                extension_name: "weather".to_string(),
                success: true,
                message: "Token saved".to_string(),
                auth_mode: Some("manual_token".to_string()),
                auth_status: Some("authenticated".to_string()),
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
                thread_id: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthCompleted
        ));
        assert!(wit.message.contains("Authentication completed"));
        assert!(wit.message.contains("Token saved"));
    }

    #[test]
    fn test_status_to_wit_auth_completed_failure() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::AuthCompleted {
                extension_name: "weather".to_string(),
                success: false,
                message: "Invalid token".to_string(),
                auth_mode: None,
                auth_status: None,
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
                thread_id: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthCompleted
        ));
        assert!(wit.message.contains("Authentication failed"));
        assert!(wit.message.contains("Invalid token"));
    }

    #[test]
    fn test_status_to_wit_approval_needed() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
                request_id: "req-123".to_string(),
                tool_name: "http_request".to_string(),
                description: "Fetch weather data".to_string(),
                parameters: serde_json::json!({"url": "https://api.weather.test"}),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ApprovalNeeded
        ));
        assert!(wit.message.contains("http_request"));
        assert!(wit.message.contains("/approve"));
    }

    #[test]
    fn test_approval_prompt_roundtrip_submission_aliases() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
                request_id: "req-321".to_string(),
                tool_name: "http_request".to_string(),
                description: "Fetch weather data".to_string(),
                parameters: serde_json::json!({"url": "https://api.weather.test"}),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ApprovalNeeded
        ));
        assert!(wit.message.contains("/approve"));
        assert!(wit.message.contains("/deny"));
        assert!(wit.message.contains("/always"));
    }

    #[test]
    fn test_clone_wit_status_update() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: "hello".to_string(),
            metadata_json: "{\"a\":1}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(cloned.status, wit_channel::StatusType::Thinking));
        assert_eq!(cloned.message, "hello");
        assert_eq!(cloned.metadata_json, "{\"a\":1}");
    }

    #[test]
    fn test_clone_wit_status_update_approval_needed() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ApprovalNeeded,
            message: "approval needed".to_string(),
            metadata_json: "{\"chat_id\":42}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(
            cloned.status,
            wit_channel::StatusType::ApprovalNeeded
        ));
        assert_eq!(cloned.message, "approval needed");
        assert_eq!(cloned.metadata_json, "{\"chat_id\":42}");
    }

    #[test]
    fn test_clone_wit_status_update_auth_completed() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthCompleted,
            message: "auth complete".to_string(),
            metadata_json: "{}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(
            cloned.status,
            wit_channel::StatusType::AuthCompleted
        ));
        assert_eq!(cloned.message, "auth complete");
    }

    #[test]
    fn test_clone_wit_status_update_all_variants() {
        use super::{clone_wit_status_update, wit_channel};

        let variants = vec![
            wit_channel::StatusType::Thinking,
            wit_channel::StatusType::Done,
            wit_channel::StatusType::Interrupted,
            wit_channel::StatusType::ToolStarted,
            wit_channel::StatusType::ToolCompleted,
            wit_channel::StatusType::ToolResult,
            wit_channel::StatusType::ApprovalNeeded,
            wit_channel::StatusType::Status,
            wit_channel::StatusType::JobStarted,
            wit_channel::StatusType::AuthRequired,
            wit_channel::StatusType::AuthCompleted,
        ];

        for status in variants {
            let original = wit_channel::StatusUpdate {
                status,
                message: "sample".to_string(),
                metadata_json: "{}".to_string(),
            };
            let cloned = clone_wit_status_update(&original);

            assert_eq!(
                std::mem::discriminant(&cloned.status),
                std::mem::discriminant(&original.status)
            );
            assert_eq!(cloned.message, "sample");
            assert_eq!(cloned.metadata_json, "{}");
        }
    }

    #[test]
    fn test_redact_credentials_replaces_values() {
        use super::ChannelStoreData;

        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "TELEGRAM_BOT_TOKEN".to_string(),
            "8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis".to_string(),
        );
        creds.insert("OTHER_SECRET".to_string(), "s3cret".to_string());

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            Arc::new(PairingStore::new()),
            Arc::new(ChannelWorkspaceStore::new()),
        );

        let error = "HTTP request failed: error sending request for url \
            (https://api.telegram.org/bot8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis/getUpdates)";

        let redacted = store.redact_credentials(error);

        assert!(
            !redacted.contains("8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis"),
            "credential value should be redacted"
        );
        assert!(
            redacted.contains("[REDACTED:TELEGRAM_BOT_TOKEN]"),
            "redacted text should contain placeholder name"
        );
        assert!(
            !redacted.contains("s3cret"),
            "other credentials should also be redacted"
        );
    }

    #[test]
    fn test_redact_credentials_no_op_without_credentials() {
        use super::ChannelStoreData;

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            std::collections::HashMap::new(),
            Arc::new(PairingStore::new()),
            Arc::new(ChannelWorkspaceStore::new()),
        );

        let input = "some error message";
        assert_eq!(store.redact_credentials(input), input);
    }

    #[test]
    fn test_redact_credentials_skips_empty_values() {
        use super::ChannelStoreData;

        let mut creds = std::collections::HashMap::new();
        creds.insert("EMPTY_TOKEN".to_string(), String::new());

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            Arc::new(PairingStore::new()),
            Arc::new(ChannelWorkspaceStore::new()),
        );

        let input = "should not match anything";
        assert_eq!(store.redact_credentials(input), input);
    }

    #[test]
    fn test_telegram_webhook_pending_updates_without_inbound_is_unhealthy() {
        let reason = WasmChannel::telegram_webhook_unhealthy_reason(
            200_000,
            Some("https://example.test/webhook/telegram"),
            Some("https://example.test/webhook/telegram"),
            None,
            None,
            Some(3),
            Some(0),
            None,
        );

        assert_eq!(
            reason.as_deref(),
            Some(
                "Telegram has 3 pending webhook update(s) but ThinClaw has not received any inbound webhook events"
            )
        );
    }

    #[test]
    fn test_telegram_webhook_recent_registration_allows_grace_period() {
        let reason = WasmChannel::telegram_webhook_unhealthy_reason(
            20_000,
            Some("https://example.test/webhook/telegram"),
            Some("https://example.test/webhook/telegram"),
            None,
            None,
            Some(2),
            Some(0),
            None,
        );

        assert!(reason.is_none());
    }

    #[test]
    fn test_telegram_webhook_recent_inbound_with_pending_updates_stays_healthy() {
        let reason = WasmChannel::telegram_webhook_unhealthy_reason(
            100_000,
            Some("https://example.test/webhook/telegram"),
            Some("https://example.test/webhook/telegram"),
            None,
            None,
            Some(1),
            Some(0),
            Some(40_000),
        );

        assert!(reason.is_none());
    }

    #[test]
    fn test_merged_response_metadata_overrides_and_includes_attachments() {
        let original = serde_json::json!({
            "chat_id": 42,
            "message_id": "orig",
            "keep": true,
        });
        let response = thinclaw_channels_core::OutgoingResponse {
            content: "hello".to_string(),
            thread_id: None,
            metadata: serde_json::json!({
                "message_id": "override",
                "extra": "value",
            }),
            attachments: vec![
                thinclaw_media::MediaContent::new(vec![1, 2, 3], "image/png")
                    .with_filename("reply.png"),
            ],
        };

        let merged = merged_response_metadata(&original, &response);

        assert_eq!(merged["chat_id"], 42);
        assert_eq!(merged["message_id"], "override");
        assert_eq!(merged["extra"], "value");
        assert_eq!(merged["response_attachments"][0]["mime_type"], "image/png");
        assert_eq!(merged["response_attachments"][0]["filename"], "reply.png");
        assert_eq!(merged["response_attachments"][0]["data"], "AQID");
    }

    #[test]
    fn test_wasm_response_content_falls_back_for_text_only_channels() {
        let response = thinclaw_channels_core::OutgoingResponse {
            content: "done".to_string(),
            thread_id: None,
            metadata: serde_json::Value::Null,
            attachments: vec![
                thinclaw_media::MediaContent::new(vec![1, 2, 3], "image/png")
                    .with_filename("reply.png")
                    .with_source_url("/tmp/reply.png"),
            ],
        };

        let fallback = response_content_for_wasm("twilio_sms", &response);
        assert!(fallback.contains("done"));
        assert!(fallback.contains("Generated media:"));
        assert!(fallback.contains("reply.png"));
        assert!(fallback.contains("/tmp/reply.png"));

        assert_eq!(response_content_for_wasm("slack", &response), "done");
    }

    /// Verify that WASM HTTP host functions work using a dedicated
    /// current-thread runtime inside spawn_blocking.
    #[tokio::test]
    async fn test_dedicated_runtime_inside_spawn_blocking() {
        let result = tokio::task::spawn_blocking(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime");
            rt.block_on(async { 42 })
        })
        .await
        .expect("spawn_blocking panicked");
        assert_eq!(result, 42);
    }

    /// Verify a real HTTP request works using the dedicated-runtime pattern.
    /// This catches DNS, TLS, and I/O driver issues that trivial tests miss.
    #[tokio::test]
    #[ignore] // requires network
    async fn test_dedicated_runtime_real_http() {
        let result = tokio::task::spawn_blocking(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime");
            rt.block_on(async {
                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("failed to build client");
                let resp = client
                    .get("https://api.telegram.org/bot000/getMe")
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;
                match resp {
                    Ok(r) => r.status().as_u16(),
                    Err(e) if e.is_timeout() => panic!("request timed out: {e}"),
                    Err(e) => panic!("unexpected error: {e}"),
                }
            })
        })
        .await
        .expect("spawn_blocking panicked");
        // 404 because "000" is not a valid bot token
        assert_eq!(result, 404);
    }
}

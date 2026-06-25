//! WASM channel store data and host-function bindings.
//!
//! Owns [`ChannelStoreData`] (the per-execution store payload), its
//! [`WasiView`] implementation, and the generated `channel-host`
//! [`Host`](super::near::agent::channel_host::Host) implementation that backs
//! every host call a WASM channel guest can make (logging, HTTP, workspace,
//! pairing, message emission).

use std::collections::HashMap;
use std::sync::Arc;

use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::pairing::PairingStore;
use crate::wasm::capabilities::ChannelCapabilities;
use crate::wasm::host::{ChannelHostState, ChannelWorkspaceStore, EmittedMessage};
use crate::wasm::host::{LogLevel, WorkspaceReader};
use crate::wasm::limits::WasmResourceLimiter;
use thinclaw_safety::LeakDetector;

use super::near;

/// A single tool lifecycle event accumulated during a turn.
///
/// Collected while processing and flushed as a single summary
/// message before the response is sent (debug mode only).
#[derive(Debug, Clone)]
pub(super) enum ToolEventEntry {
    /// Tool execution started.
    Started { name: String },
    /// Tool execution completed (success or failure).
    Completed { name: String, success: bool },
    /// Tool returned a result preview.
    Result { preview: String },
}

/// Escape HTML entities for safe embedding in Telegram HTML messages.
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Store data for WASM channel execution.
///
/// Contains the resource limiter, channel-specific host state, and WASI context.
pub(super) struct ChannelStoreData {
    pub(super) limiter: WasmResourceLimiter,
    pub(super) host_state: ChannelHostState,
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
    pub(super) fn new(
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
    pub(super) fn redact_credentials(&self, text: &str) -> String {
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
        crate::wasm::telegram_html::markdown_to_telegram_html(&markdown)
    }
}

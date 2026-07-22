//! WASM tool wrapper implementing the Tool trait.
//!
//! Uses wasmtime::component::bindgen! to generate typed bindings from the WIT
//! interface, ensuring all host functions are properly registered under the
//! correct `near:agent/host` namespace.
//!
//! Each execution creates a fresh instance (NEAR pattern) to ensure
//! isolation and deterministic behavior.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use thinclaw_secrets::{
    CreateSecretParams, CredentialLocation, SecretAccessContext, SecretError, SecretsStore,
};
use thinclaw_tools_core::{
    OutboundUrlGuardOptions, Tool, ToolError, ToolOutput, validate_outbound_url_pinned_async,
};
#[cfg(test)]
use thinclaw_tools_core::{is_public_outbound_ip, validate_outbound_url_pinned};
use thinclaw_types::JobContext;

use crate::wasm::capabilities::Capabilities;
use crate::wasm::credential_injector::{
    InjectedCredentials, host_matches_pattern, inject_credential,
};
use crate::wasm::error::WasmError;
use crate::wasm::host::{HostState, LogLevel};
use crate::wasm::limits::{ResourceLimits, WasmResourceLimiter};
pub use crate::wasm::oauth::OAuthRefreshConfig;
use crate::wasm::oauth::{
    bounded_oauth_error, decode_bounded_json, oauth_client_for, oauth_expiry_from_response,
    validate_bearer_token_type, validate_oauth_refresh_config, validate_oauth_secret_value,
};
use crate::wasm::ports::{ExactValueLeakScanner, HostToolInvoker, LeakScan, LeakScanner};
use crate::wasm::runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmToolRuntime};

const MAX_WASM_HTTP_METHOD_BYTES: usize = 16;
const MAX_WASM_HTTP_URL_BYTES: usize = 16 * 1024;
const MAX_WASM_HTTP_HEADERS_BYTES: usize = 64 * 1024;
const MAX_WASM_HTTP_HEADER_COUNT: usize = 128;
const MAX_WASM_HTTP_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_WASM_HTTP_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_WASM_HTTP_TIMEOUT: Duration = Duration::from_secs(300);

fn wasm_http_headers_are_valid(headers: &HashMap<String, String>) -> bool {
    if headers.len() > MAX_WASM_HTTP_HEADER_COUNT {
        return false;
    }
    let mut seen = HashSet::with_capacity(headers.len());
    let mut total_bytes = 0usize;
    headers.iter().all(|(name, value)| {
        let normalized = name.to_ascii_lowercase();
        let forbidden = matches!(
            normalized.as_str(),
            "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "proxy-authorization"
                | "proxy-authenticate"
                | "te"
                | "trailer"
                | "upgrade"
        );
        let Some(next_total) = total_bytes
            .checked_add(name.len())
            .and_then(|total| total.checked_add(value.len()))
        else {
            return false;
        };
        total_bytes = next_total;
        !name.is_empty()
            && name.len() <= 256
            && value.len() <= 16 * 1024
            && total_bytes <= MAX_WASM_HTTP_HEADERS_BYTES
            && !forbidden
            && seen.insert(normalized)
            && reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
            && reqwest::header::HeaderValue::from_str(value).is_ok()
    })
}

// Generate component model bindings from the WIT file.
//
// This creates:
// - `near::agent::host::Host` trait + `add_to_linker()` for the import interface
// - `SandboxedTool` struct with `instantiate()` for the world
// - `exports::near::agent::tool::*` types for the export interface
wasmtime::component::bindgen!({
    path: "../../wit/tool.wit",
    world: "sandboxed-tool",
    with: {},
});

// Alias the export interface types for convenience.
use exports::near::agent::tool as wit_tool;

/// Pre-resolved credential for host-based injection.
///
/// Built before each WASM execution by decrypting secrets from the store.
/// Applied per-request by matching the URL host against `host_patterns`.
/// WASM tools never see the raw secret values.
struct ResolvedHostCredential {
    /// Host patterns this credential applies to (e.g., "www.googleapis.com").
    host_patterns: Vec<String>,
    /// Headers to add to matching requests (e.g., "Authorization: Bearer ...").
    headers: HashMap<String, String>,
    /// Query parameters to add to matching requests.
    query_params: HashMap<String, String>,
    /// Raw secret value for redaction in error messages.
    secret_value: String,
}

#[derive(Debug, Clone)]
struct LeakBoundaryEvent {
    source: String,
    action_taken: String,
    content_hash: String,
    redacted_preview: Option<String>,
}

/// Store data for WASM tool execution.
///
/// Contains the resource limiter, host state, WASI context, and injected
/// credentials. Fresh instance created per execution (NEAR pattern).
struct StoreData {
    limiter: WasmResourceLimiter,
    host_state: HostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL/header placeholder substitution.
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    credentials: HashMap<String, String>,
    /// Pre-resolved credentials for automatic host-based injection.
    /// Applied by matching URL host against each credential's host_patterns.
    host_credentials: Vec<ResolvedHostCredential>,
    /// Leak detector events observed at host boundaries during this execution.
    leak_events: Vec<LeakBoundaryEvent>,
    /// Dedicated tokio runtime for HTTP requests, lazily initialized.
    /// Reused across multiple `http_request` calls within one execution.
    http_runtime: Option<tokio::runtime::Runtime>,
    /// Dedicated tokio runtime for host-mediated tool invocations.
    tool_runtime: Option<tokio::runtime::Runtime>,
    /// Optional policy-aware bridge for invoking host tools through aliases.
    tool_invoker: Option<Arc<dyn HostToolInvoker>>,
    /// Scanner used at host boundaries to catch leaked credentials.
    leak_scanner: Arc<dyn LeakScanner>,
    /// Parent job context used to preserve user/workspace scope.
    job_context: JobContext,
}

impl StoreData {
    #[allow(clippy::too_many_arguments)]
    fn new(
        memory_limit: u64,
        capabilities: Capabilities,
        credentials: HashMap<String, String>,
        host_credentials: Vec<ResolvedHostCredential>,
        available_secret_names: HashSet<String>,
        tool_invoker: Option<Arc<dyn HostToolInvoker>>,
        leak_scanner: Arc<dyn LeakScanner>,
        job_context: JobContext,
    ) -> Self {
        // Minimal WASI context: no filesystem, no env vars (security)
        let wasi = WasiCtxBuilder::new().build();

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: HostState::new(capabilities)
                .with_available_secret_names(available_secret_names),
            wasi,
            table: ResourceTable::new(),
            credentials,
            host_credentials,
            leak_events: Vec::new(),
            http_runtime: None,
            tool_runtime: None,
            tool_invoker,
            leak_scanner,
            job_context,
        }
    }

    /// Inject credentials into a string by replacing placeholders.
    ///
    /// Replaces patterns like `{GOOGLE_ACCESS_TOKEN}` with actual values.
    /// WASM tools reference credentials by placeholder, never seeing real values.
    fn inject_credentials(&self, input: &str, context: &str) -> String {
        let mut result = input.to_string();

        for (name, value) in &self.credentials {
            let placeholder = format!("{{{}}}", name);
            if result.contains(&placeholder) {
                tracing::debug!(
                    placeholder = %placeholder,
                    context = %context,
                    "Replacing credential placeholder in tool request"
                );
                result = result.replace(&placeholder, value);
            }
        }

        result
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages or logs.
    /// reqwest::Error includes the full URL in its Display output, so any
    /// error from an injected-URL request will contain the raw credential
    /// unless we scrub it.
    fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, value) in &self.credentials {
            if !value.is_empty() {
                result = result.replace(value, &format!("[REDACTED:{}]", name));
            }
        }
        for cred in &self.host_credentials {
            if !cred.secret_value.is_empty() {
                result = result.replace(&cred.secret_value, "[REDACTED:host_credential]");
            }
        }
        result
    }

    fn exact_leak_values(&self) -> Vec<String> {
        let mut values: Vec<String> = self.credentials.values().cloned().collect();
        values.extend(
            self.host_credentials
                .iter()
                .map(|credential| credential.secret_value.clone()),
        );
        values
    }

    fn record_leak_scan(&mut self, source: &str, content: &str, result: &LeakScan) {
        if result.matches.is_empty() {
            return;
        }
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        for leak_match in &result.matches {
            self.leak_events.push(LeakBoundaryEvent {
                source: source.to_string(),
                action_taken: leak_match.action_taken.clone(),
                content_hash: content_hash.clone(),
                redacted_preview: Some(leak_match.masked_preview.clone()),
            });
        }
    }

    /// Inject pre-resolved host credentials into the request.
    ///
    /// Matches the URL host against each resolved credential's host_patterns.
    /// Matching credentials have their headers merged and query params appended.
    fn inject_host_credentials(
        &self,
        url_host: &str,
        headers: &mut HashMap<String, String>,
        url: &mut String,
    ) {
        for cred in &self.host_credentials {
            let matches = cred
                .host_patterns
                .iter()
                .any(|pattern| host_matches_pattern(url_host, pattern));

            if !matches {
                continue;
            }

            // Merge injected headers (host credentials take precedence)
            for (key, value) in &cred.headers {
                headers.insert(key.clone(), value.clone());
            }

            // Append query parameters to URL (insert before fragment if present)
            if !cred.query_params.is_empty() {
                let (base, fragment) = match url.find('#') {
                    Some(i) => (url[..i].to_string(), Some(url[i..].to_string())),
                    None => (url.clone(), None),
                };
                *url = base;

                let separator = if url.contains('?') { '&' } else { '?' };
                for (i, (name, value)) in cred.query_params.iter().enumerate() {
                    if i == 0 {
                        url.push(separator);
                    } else {
                        url.push('&');
                    }
                    url.push_str(&urlencoding::encode(name));
                    url.push('=');
                    url.push_str(&urlencoding::encode(value));
                }

                if let Some(frag) = fragment {
                    url.push_str(&frag);
                }
            }
        }
    }
}

fn leak_boundary_events_from_scan(
    source: &str,
    content: &str,
    result: &LeakScan,
) -> Vec<LeakBoundaryEvent> {
    if result.matches.is_empty() {
        return Vec::new();
    }
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    result
        .matches
        .iter()
        .map(|leak_match| LeakBoundaryEvent {
            source: source.to_string(),
            action_taken: leak_match.action_taken.clone(),
            content_hash: content_hash.clone(),
            redacted_preview: Some(leak_match.masked_preview.clone()),
        })
        .collect()
}

async fn persist_wasm_leak_events(
    secrets_store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    events: &[LeakBoundaryEvent],
) {
    let Some(store) = secrets_store else {
        return;
    };
    for event in events {
        if let Err(error) = store
            .record_leak_detection_event(
                user_id,
                &event.source,
                &event.action_taken,
                &event.content_hash,
                event.redacted_preview.as_deref(),
            )
            .await
        {
            tracing::warn!(
                source = %event.source,
                action = %event.action_taken,
                error = %error,
                "Failed to persist leak detection event"
            );
        }
    }
}

// Provide WASI context for the WASM component.
// Required because tools are compiled with wasm32-wasip2 target.
impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Implement the generated Host trait from bindgen.
//
// This registers all 6 host functions under the `near:agent/host` namespace:
// log, now-millis, workspace-read, http-request, secret-exists, tool-invoke
impl near::agent::host::Host for StoreData {
    fn log(&mut self, level: near::agent::host::LogLevel, message: String) {
        let log_level = match level {
            near::agent::host::LogLevel::Trace => LogLevel::Trace,
            near::agent::host::LogLevel::Debug => LogLevel::Debug,
            near::agent::host::LogLevel::Info => LogLevel::Info,
            near::agent::host::LogLevel::Warn => LogLevel::Warn,
            near::agent::host::LogLevel::Error => LogLevel::Error,
        };
        let _ = self.host_state.log(log_level, message);
    }

    fn now_millis(&mut self) -> u64 {
        self.host_state.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host_state.workspace_read(&path).ok().flatten()
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<near::agent::host::HttpResponse, String> {
        let http_capability = self
            .host_state
            .capabilities()
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability is not granted".to_string())?;
        let max_request_bytes = http_capability
            .max_request_bytes
            .min(MAX_WASM_HTTP_REQUEST_BYTES);
        let max_response_bytes = http_capability
            .max_response_bytes
            .min(MAX_WASM_HTTP_RESPONSE_BYTES);
        let capability_timeout = http_capability.timeout.min(MAX_WASM_HTTP_TIMEOUT);
        if method.is_empty()
            || method.len() > MAX_WASM_HTTP_METHOD_BYTES
            || !method.bytes().all(|byte| byte.is_ascii_alphabetic())
            || url.is_empty()
            || url.len() > MAX_WASM_HTTP_URL_BYTES
            || headers_json.len() > MAX_WASM_HTTP_HEADERS_BYTES
            || body
                .as_ref()
                .is_some_and(|value| value.len() > max_request_bytes)
        {
            return Err(
                "HTTP request fields are malformed or exceed configured limits".to_string(),
            );
        }

        let leak_values = self.exact_leak_values();
        let raw_headers: HashMap<String, String> = serde_json::from_str(&headers_json)
            .map_err(|_| "HTTP headers must be a JSON object of strings".to_string())?;
        if !wasm_http_headers_are_valid(&raw_headers) {
            return Err("HTTP headers are malformed or exceed configured limits".to_string());
        }

        // Check HTTP allowlist
        self.host_state
            .check_http_allowed(&url, &method)
            .map_err(|e| format!("HTTP not allowed: {}", e))?;

        // Validate guest-provided request material before any host credential
        // injection so a WASM guest cannot use injected credentials to smuggle
        // its own secret-looking material through the boundary.
        let mut request_probe = format!("{method}\n{url}\n{headers_json}");
        if let Some(body) = body.as_deref() {
            request_probe.push('\n');
            request_probe.push_str(&String::from_utf8_lossy(body));
        }
        let request_scan = self.leak_scanner.scan(&request_probe, &leak_values);
        self.record_leak_scan("wasm_tool.request", &request_probe, &request_scan);
        if request_scan.should_block {
            let blocking_match = request_scan
                .matches
                .iter()
                .find(|m| m.action_taken == "block");
            return Err(format!(
                "Potential secret leak blocked: pattern '{}' matched '{}'",
                blocking_match
                    .map(|m| m.pattern_name.as_str())
                    .unwrap_or("unknown"),
                blocking_match
                    .map(|m| m.masked_preview.as_str())
                    .unwrap_or("")
            ));
        }

        // Record for rate limiting
        self.host_state
            .record_http_request()
            .map_err(|e| format!("Rate limit exceeded: {}", e))?;

        // Inject credentials into URL (e.g., replace {TELEGRAM_BOT_TOKEN})
        let injected_url = self.inject_credentials(&url, "url");

        let mut headers: HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let mut url = injected_url;

        // Inject pre-resolved host credentials (Bearer tokens, API keys, etc.)
        // based on the request's target host.
        if let Some(host) = extract_host_from_url(&url) {
            self.inject_host_credentials(&host, &mut headers, &mut url);
        }

        if url.len() > MAX_WASM_HTTP_URL_BYTES || !wasm_http_headers_are_valid(&headers) {
            return Err("Injected HTTP request fields exceed configured limits".to_string());
        }

        // Re-check the effective URL after all substitutions and credential
        // injection. Capability authorization must apply to the request that is
        // actually sent, not merely to the guest-provided template.
        self.host_state
            .check_http_allowed(&url, &method)
            .map_err(|e| format!("HTTP not allowed after credential injection: {e}"))?;

        // Make HTTP request using a dedicated single-threaded runtime.
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
        let rt = self
            .http_runtime
            .as_ref()
            .ok_or_else(|| "HTTP runtime initialization failed".to_string())?;
        let result = rt.block_on(async {
            // A request can shorten, but never extend, the declared capability
            // timeout or the hard process-wide ceiling. DNS resolution is part
            // of this same total deadline.
            let requested_timeout = Duration::from_millis(
                timeout_ms
                    .map(u64::from)
                    .unwrap_or(capability_timeout.as_millis().min(u128::from(u64::MAX)) as u64),
            );
            let timeout = requested_timeout
                .min(capability_timeout)
                .min(MAX_WASM_HTTP_TIMEOUT);
            let deadline = tokio::time::Instant::now() + timeout;
            let guarded = tokio::time::timeout_at(
                deadline,
                validate_outbound_url_pinned_async(
                    &url,
                    &OutboundUrlGuardOptions {
                        require_https: true,
                        upgrade_http_to_https: false,
                        allowlist: Vec::new(),
                    },
                ),
            )
            .await
            .map_err(|_| "HTTP request timed out during DNS validation".to_string())?
            .map_err(|error| error.to_string())?;
            let validated = ValidatedHost {
                host: guarded
                    .url
                    .host_str()
                    .ok_or_else(|| "Failed to parse host from URL".to_string())?
                    .to_string(),
                pinned_addrs: guarded.pinned_addrs,
            };
            let mut client_builder = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy();
            // Pin the connection to the validated addresses to close the
            // DNS-rebinding TOCTOU window. Empty for IP-literal hosts, where
            // reqwest connects to the literal directly and cannot rebind.
            if !validated.pinned_addrs.is_empty() {
                client_builder =
                    client_builder.resolve_to_addrs(&validated.host, &validated.pinned_addrs);
            }
            let client = client_builder
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

            for (key, value) in headers {
                request = request.header(&key, &value);
            }

            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            }

            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or_else(|| "HTTP request timed out before send".to_string())?;
            let response = request
                .timeout(remaining)
                .send()
                .await
                .map_err(|error| format!("HTTP request failed: {}", error.without_url()))?;

            let status = response.status().as_u16();
            let response_headers: HashMap<String, String> = response
                .headers()
                .iter()
                .take(MAX_WASM_HTTP_HEADER_COUNT)
                .filter_map(|(k, v)| {
                    v.to_str().ok().and_then(|value| {
                        (value.len() <= 16 * 1024)
                            .then(|| (k.as_str().to_string(), value.to_string()))
                    })
                })
                .collect();
            let headers_json = serde_json::to_string(&response_headers).unwrap_or_default();

            // Check Content-Length header for early rejection of oversized responses.
            let max_response = max_response_bytes;
            if let Some(cl) = response.content_length()
                && cl as usize > max_response
            {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    cl, max_response
                ));
            }

            // Stream with an in-flight cap. `Response::bytes()` would allocate
            // the complete chunked body before this limit could be enforced.
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    format!("Failed to read response body: {}", error.without_url())
                })?;
                if body.len().saturating_add(chunk.len()) > max_response {
                    return Err(format!(
                        "Response body exceeds limit of {} bytes",
                        max_response
                    ));
                }
                body.extend_from_slice(&chunk);
            }

            // Leak detection on response body
            let mut response_leak_events = Vec::new();
            if let Ok(body_str) = std::str::from_utf8(&body) {
                let response_scan = self.leak_scanner.scan(body_str, &leak_values);
                response_leak_events.extend(leak_boundary_events_from_scan(
                    "wasm_tool.response",
                    body_str,
                    &response_scan,
                ));
                if response_scan.should_block {
                    let blocking_match = response_scan
                        .matches
                        .iter()
                        .find(|m| m.action_taken == "block");
                    return Err(format!(
                        "Potential secret leak in response: pattern '{}' matched '{}'",
                        blocking_match
                            .map(|m| m.pattern_name.as_str())
                            .unwrap_or("unknown"),
                        blocking_match
                            .map(|m| m.masked_preview.as_str())
                            .unwrap_or("")
                    ));
                }
                if let Some(redacted) = response_scan.redacted_content {
                    body = redacted.into_bytes();
                }
            }

            Ok((
                near::agent::host::HttpResponse {
                    status,
                    headers_json,
                    body,
                },
                response_leak_events,
            ))
        });

        let result = result.map(|(response, events)| {
            self.leak_events.extend(events);
            response
        });

        // Redact credentials from error messages before returning to WASM
        result.map_err(|e| self.redact_credentials(&e))
    }

    fn tool_invoke(&mut self, alias: String, params_json: String) -> Result<String, String> {
        // Validate capability and resolve alias
        let real_name = self.host_state.check_tool_invoke_allowed(&alias)?;
        self.host_state.record_tool_invoke()?;

        let invoker = self
            .tool_invoker
            .clone()
            .ok_or_else(|| "Tool invocation from WASM tools is not configured".to_string())?;

        if self.tool_runtime.is_none() {
            self.tool_runtime = Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create tool invocation runtime: {e}"))?,
            );
        }
        let rt = self
            .tool_runtime
            .as_ref()
            .ok_or_else(|| "Tool invocation runtime was not initialized".to_string())?;
        rt.block_on(invoker.invoke_json(&self.job_context, &real_name, &params_json))
            .map_err(|err| err.to_string())
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host_state.secret_exists(&name)
    }
}

/// A Tool implementation backed by a WASM component.
///
/// Each call to `execute` creates a fresh instance for isolation.
pub struct WasmToolWrapper {
    /// Runtime for engine access.
    runtime: Arc<WasmToolRuntime>,
    /// Prepared module with compiled component.
    prepared: Arc<PreparedModule>,
    /// Capabilities to grant to this tool.
    capabilities: Capabilities,
    /// Cached description (from PreparedModule or override).
    description: String,
    /// Cached schema (from PreparedModule or override).
    schema: serde_json::Value,
    /// Injected credentials for HTTP requests (e.g., OAuth tokens).
    /// Keys are placeholder names like "GOOGLE_ACCESS_TOKEN".
    credentials: HashMap<String, String>,
    /// Secrets store for resolving host-based credential injection.
    /// Used in execute() to pre-decrypt secrets before WASM runs.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    oauth_refresh: Option<OAuthRefreshConfig>,
    /// Optional host-mediated bridge for tool_invoke aliases.
    tool_invoker: Option<Arc<dyn HostToolInvoker>>,
    /// Scanner used to inspect host-boundary inputs and outputs.
    leak_scanner: Arc<dyn LeakScanner>,
}

impl WasmToolWrapper {
    /// Create a new WASM tool wrapper.
    pub fn new<C>(
        runtime: Arc<WasmToolRuntime>,
        prepared: Arc<PreparedModule>,
        capabilities: C,
    ) -> Self
    where
        C: Into<Capabilities>,
    {
        Self {
            description: prepared.description.clone(),
            schema: prepared.schema.clone(),
            runtime,
            prepared,
            capabilities: capabilities.into(),
            credentials: HashMap::new(),
            secrets_store: None,
            oauth_refresh: None,
            tool_invoker: None,
            leak_scanner: Arc::new(ExactValueLeakScanner),
        }
    }

    /// Override the tool description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Override the parameter schema.
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = schema;
        self
    }

    /// Set credentials for HTTP request placeholder injection.
    pub fn with_credentials(mut self, credentials: HashMap<String, String>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Set the secrets store for host-based credential injection.
    ///
    /// When set, credentials declared in the tool's capabilities are
    /// automatically decrypted and injected into HTTP requests based
    /// on the target host (e.g., Bearer token for www.googleapis.com).
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Set OAuth refresh configuration for auto-refreshing expired tokens.
    ///
    /// When set, `execute()` checks the access token's `expires_at` before
    /// each call and silently refreshes it using the stored refresh token.
    pub fn with_oauth_refresh(mut self, config: OAuthRefreshConfig) -> Self {
        self.oauth_refresh = Some(config);
        self
    }

    /// Set the host-mediated tool invoker for WASM `tool_invoke`.
    pub fn with_tool_invoker<I>(mut self, invoker: Arc<I>) -> Self
    where
        I: HostToolInvoker + 'static,
    {
        self.tool_invoker = Some(invoker);
        self
    }

    /// Set a custom leak scanner for host-boundary requests and responses.
    pub fn with_leak_scanner(mut self, scanner: Arc<dyn LeakScanner>) -> Self {
        self.leak_scanner = scanner;
        self
    }

    /// Get the resource limits for this tool.
    pub fn limits(&self) -> &ResourceLimits {
        &self.prepared.limits
    }

    /// Add all host functions to the linker using generated bindings.
    ///
    /// Uses the bindgen-generated `add_to_linker` function to properly register
    /// all host functions with correct component model signatures under the
    /// `near:agent/host` namespace.
    fn add_host_functions(linker: &mut Linker<StoreData>) -> Result<(), WasmError> {
        // Add WASI support (required by components built with wasm32-wasip2)
        wasmtime_wasi::p2::add_to_linker_sync(linker)
            .map_err(|e| WasmError::ConfigError(format!("Failed to add WASI functions: {}", e)))?;

        // Add our custom host interface using the generated add_to_linker
        near::agent::host::add_to_linker::<StoreData, wasmtime::component::HasSelf<StoreData>>(
            linker,
            |state| state,
        )
        .map_err(|e| WasmError::ConfigError(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    /// Execute the WASM tool synchronously (called from spawn_blocking).
    fn execute_sync(
        &self,
        params: serde_json::Value,
        context_json: Option<String>,
        host_credentials: Vec<ResolvedHostCredential>,
        available_secret_names: HashSet<String>,
        job_context: JobContext,
    ) -> Result<
        (
            String,
            Vec<crate::wasm::host::LogEntry>,
            Vec<LeakBoundaryEvent>,
        ),
        WasmError,
    > {
        let engine = self.runtime.engine();
        let limits = &self.prepared.limits;

        // Create store with fresh state (NEAR pattern: fresh instance per call)
        let store_data = StoreData::new(
            limits.memory_bytes,
            self.capabilities.clone(),
            self.credentials.clone(),
            host_credentials,
            available_secret_names,
            self.tool_invoker.clone(),
            Arc::clone(&self.leak_scanner),
            job_context,
        );
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if self.runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmError::ConfigError(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline as a hard timeout backup.
        // The epoch ticker thread increments the engine epoch every EPOCH_TICK_INTERVAL.
        // Setting deadline to N means "trap after N ticks", so we compute the number
        // of ticks that fit in the tool's timeout. Minimum 1 to always have a backstop.
        store.epoch_deadline_trap();
        let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        // Use the pre-compiled component (no recompilation needed)
        let component = self.prepared.component().clone();

        // Create linker with all host functions properly namespaced
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance = SandboxedTool::instantiate(&mut store, &component, &linker)
            .map_err(|e| WasmError::InstantiationFailed(e.to_string()))?;

        // Prepare the request
        let params_json = serde_json::to_string(&params)
            .map_err(|e| WasmError::InvalidResponseJson(e.to_string()))?;

        let request = wit_tool::Request {
            params: params_json,
            context: context_json,
        };

        // Call execute using the generated typed interface
        let tool_iface = instance.near_agent_tool();
        let response = tool_iface.call_execute(&mut store, &request).map_err(|e| {
            let error_str = e.to_string();
            if error_str.contains("out of fuel") {
                WasmError::FuelExhausted { limit: limits.fuel }
            } else if error_str.contains("unreachable") {
                WasmError::Trapped("unreachable code executed".to_string())
            } else {
                WasmError::Trapped(error_str)
            }
        })?;

        // Get logs and boundary leak events from host state.
        let (logs, leak_events) = {
            let data = store.data_mut();
            (
                data.host_state.take_logs(),
                std::mem::take(&mut data.leak_events),
            )
        };

        // Check for tool-level error
        if let Some(err) = response.error {
            return Err(WasmError::ToolReturnedError(err));
        }

        // Return result (or empty string if none)
        Ok((response.output.unwrap_or_default(), logs, leak_events))
    }
}

#[async_trait]
impl Tool for WasmToolWrapper {
    fn name(&self) -> &str {
        &self.prepared.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let timeout = self.prepared.limits.timeout;

        // Pre-resolve host credentials from secrets store (async, before blocking task).
        // This decrypts the secrets once so the sync http_request() host function
        // can inject them without needing async access.
        let host_credentials = resolve_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &ctx.user_id,
            self.oauth_refresh.as_ref(),
        )
        .await;
        let available_secret_names = resolve_available_secret_names(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &ctx.user_id,
        )
        .await;

        // Serialize context for WASM
        let context_json = serde_json::to_string(ctx).ok();

        // Clone what we need for the blocking task
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let description = self.description.clone();
        let schema = self.schema.clone();
        let credentials = self.credentials.clone();
        let tool_invoker = self.tool_invoker.clone();
        let leak_scanner = Arc::clone(&self.leak_scanner);
        let job_context = ctx.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            let wrapper = WasmToolWrapper {
                runtime,
                prepared,
                capabilities,
                description,
                schema,
                credentials,
                secrets_store: None, // Not needed in blocking task
                oauth_refresh: None, // Already used above for pre-refresh
                tool_invoker,
                leak_scanner,
            };

            tokio::task::spawn_blocking(move || {
                wrapper.execute_sync(
                    params,
                    context_json,
                    host_credentials,
                    available_secret_names,
                    job_context,
                )
            })
            .await
            .map_err(|e| WasmError::ExecutionPanicked(e.to_string()))?
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((result_json, logs, leak_events))) => {
                persist_wasm_leak_events(self.secrets_store.as_deref(), &ctx.user_id, &leak_events)
                    .await;

                // Emit collected logs
                for log in logs {
                    match log.level {
                        LogLevel::Trace => {
                            tracing::trace!(target: "wasm_tool", message_bytes = log.message.len(), "WASM guest trace")
                        }
                        LogLevel::Debug => {
                            tracing::debug!(target: "wasm_tool", message_bytes = log.message.len(), "WASM guest debug log")
                        }
                        LogLevel::Info => {
                            tracing::info!(target: "wasm_tool", message_bytes = log.message.len(), "WASM guest info log")
                        }
                        LogLevel::Warn => {
                            tracing::warn!(target: "wasm_tool", message_bytes = log.message.len(), "WASM guest warning")
                        }
                        LogLevel::Error => {
                            tracing::error!(target: "wasm_tool", message_bytes = log.message.len(), "WASM guest error")
                        }
                    }
                }

                // Parse result JSON
                let result: serde_json::Value = serde_json::from_str(&result_json)
                    .unwrap_or(serde_json::Value::String(result_json));

                Ok(ToolOutput::success(result, duration))
            }
            Ok(Err(wasm_err)) => Err(wasm_err.into()),
            Err(_) => Err(WasmError::Timeout(timeout).into()),
        }
    }

    fn requires_sanitization(&self) -> bool {
        // WASM tools always require sanitization, they're untrusted by definition
        true
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        // Use the timeout as a conservative estimate
        Some(self.prepared.limits.timeout)
    }
}

impl std::fmt::Debug for WasmToolWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolWrapper")
            .field("name", &self.prepared.name)
            .field("description", &self.description)
            .field("limits", &self.prepared.limits)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tool_invoke_host_tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use base64::{Engine as _, engine::general_purpose};
    use thinclaw_tools_core::Tool;
    use thinclaw_types::JobContext;

    use super::*;
    use crate::wasm::capabilities::Capabilities;
    use crate::wasm::limits::ResourceLimits;
    use crate::wasm::ports::HostToolInvoker;
    use crate::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};

    struct RecordingInvoker {
        calls: Mutex<Vec<(String, String, serde_json::Value)>>,
        response: Mutex<Result<String, String>>,
    }

    impl Default for RecordingInvoker {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                response: Mutex::new(Ok("{}".to_string())),
            }
        }
    }

    impl RecordingInvoker {
        fn success(response: impl Into<String>) -> Self {
            Self {
                response: Mutex::new(Ok(response.into())),
                ..Self::default()
            }
        }

        fn failure(message: impl Into<String>) -> Self {
            Self {
                response: Mutex::new(Err(message.into())),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<(String, String, serde_json::Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl HostToolInvoker for RecordingInvoker {
        async fn invoke_json(
            &self,
            job_ctx: &JobContext,
            tool_name: &str,
            params_json: &str,
        ) -> Result<String, String> {
            self.calls.lock().unwrap().push((
                job_ctx.user_id.clone(),
                tool_name.to_string(),
                serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null),
            ));
            self.response.lock().unwrap().clone()
        }
    }

    fn store_with_tool_invoke(
        aliases: HashMap<String, String>,
        invoker: Option<Arc<dyn HostToolInvoker>>,
    ) -> StoreData {
        StoreData::new(
            1024 * 1024,
            Capabilities::default().with_tool_invoke(aliases),
            HashMap::new(),
            Vec::new(),
            HashSet::new(),
            invoker,
            Arc::new(ExactValueLeakScanner),
            JobContext {
                user_id: "wasm-user".to_string(),
                ..JobContext::default()
            },
        )
    }

    #[test]
    fn tool_invoke_calls_host_invoker_through_declared_alias_only() {
        let invoker = Arc::new(RecordingInvoker::success(r#"{"ok":true}"#));
        let mut store = store_with_tool_invoke(
            HashMap::from([("echo_alias".to_string(), "echo_tool".to_string())]),
            Some(invoker.clone()),
        );

        let output = <StoreData as near::agent::host::Host>::tool_invoke(
            &mut store,
            "echo_alias".to_string(),
            r#"{"message":"hello"}"#.to_string(),
        )
        .expect("declared alias should invoke host tool");

        assert_eq!(output, r#"{"ok":true}"#);
        assert_eq!(
            invoker.calls(),
            vec![(
                "wasm-user".to_string(),
                "echo_tool".to_string(),
                serde_json::json!({"message": "hello"})
            )]
        );
    }

    #[test]
    fn tool_invoke_rejects_undeclared_alias_before_host_invocation() {
        let invoker = Arc::new(RecordingInvoker::success("{}"));
        let mut store = store_with_tool_invoke(
            HashMap::from([("allowed".to_string(), "echo_tool".to_string())]),
            Some(invoker.clone()),
        );

        let error = <StoreData as near::agent::host::Host>::tool_invoke(
            &mut store,
            "not_declared".to_string(),
            "{}".to_string(),
        )
        .expect_err("undeclared alias should be denied");

        assert!(error.contains("Unknown tool alias"));
        assert!(invoker.calls().is_empty());
    }

    #[test]
    fn tool_invoke_reports_missing_host_bridge_after_capability_check() {
        let mut store = store_with_tool_invoke(
            HashMap::from([("echo_alias".to_string(), "echo_tool".to_string())]),
            None,
        );

        let error = <StoreData as near::agent::host::Host>::tool_invoke(
            &mut store,
            "echo_alias".to_string(),
            "{}".to_string(),
        )
        .expect_err("missing invoker should fail explicitly");

        assert!(error.contains("not configured"));
    }

    #[test]
    fn tool_invoke_propagates_policy_or_execution_denials_from_host_bridge() {
        let invoker = Arc::new(RecordingInvoker::failure("blocked by policy"));
        let mut store = store_with_tool_invoke(
            HashMap::from([("echo_alias".to_string(), "echo_tool".to_string())]),
            Some(invoker),
        );

        let error = <StoreData as near::agent::host::Host>::tool_invoke(
            &mut store,
            "echo_alias".to_string(),
            "{}".to_string(),
        )
        .expect_err("host denial should propagate to guest");

        assert!(error.contains("blocked by policy"));
    }

    #[test]
    fn tool_invoke_enforces_per_execution_rate_limit_before_invoking_host() {
        let invoker = Arc::new(RecordingInvoker::success("{}"));
        let mut store = store_with_tool_invoke(
            HashMap::from([("echo_alias".to_string(), "echo_tool".to_string())]),
            Some(invoker.clone()),
        );

        for _ in 0..20 {
            <StoreData as near::agent::host::Host>::tool_invoke(
                &mut store,
                "echo_alias".to_string(),
                "{}".to_string(),
            )
            .expect("first twenty invocations should pass");
        }
        let error = <StoreData as near::agent::host::Host>::tool_invoke(
            &mut store,
            "echo_alias".to_string(),
            "{}".to_string(),
        )
        .expect_err("twenty-first invocation should hit the rate limit");

        assert!(error.contains("Too many tool invocations"));
        assert_eq!(invoker.calls().len(), 20);
    }

    #[tokio::test]
    async fn prebuilt_component_invokes_host_tool_alias_end_to_end() {
        let encoded = include_str!(
            "../../../../tests/fixtures/wasm-tool-invoke-smoke/prebuilt/wasm_tool_invoke_smoke.wasm.base64"
        );
        let wasm_bytes = general_purpose::STANDARD
            .decode(encoded.trim())
            .expect("prebuilt smoke component should decode");
        let runtime = Arc::new(
            WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).expect("create runtime"),
        );
        let prepared = runtime
            .prepare(
                "wasm_tool_invoke_smoke",
                &wasm_bytes,
                Some(ResourceLimits::default().with_memory(4 * 1024 * 1024)),
            )
            .await
            .expect("prebuilt smoke component should prepare");
        let invoker = Arc::new(RecordingInvoker::success(r#"{"ok":true}"#));
        let wrapper = WasmToolWrapper::new(
            runtime,
            prepared,
            Capabilities::default().with_tool_invoke(HashMap::from([(
                "echo_alias".to_string(),
                "echo_tool".to_string(),
            )])),
        )
        .with_tool_invoker(invoker.clone());

        let output = wrapper
            .execute(serde_json::json!({}), &JobContext::default())
            .await
            .expect("prebuilt smoke component should invoke host tool");

        assert_eq!(
            output.result,
            serde_json::json!({
                "invoked": true,
                "output": { "ok": true }
            })
        );
        assert_eq!(invoker.calls().len(), 1);
    }

    // ── DNS-rebinding pin tests for the WASM HTTP host ──────────────────

    #[test]
    fn reject_private_ip_blocks_private_literal() {
        let err = super::reject_private_ip("https://192.168.1.1/x").unwrap_err();
        assert!(err.contains("not allowed"));
    }

    #[test]
    fn reject_private_ip_blocks_loopback_literal() {
        assert!(super::reject_private_ip("https://127.0.0.1/x").is_err());
    }

    #[test]
    fn reject_private_ip_public_literal_has_empty_pin() {
        // Public IP literal: validated directly, nothing to pin.
        let validated = super::reject_private_ip("https://8.8.8.8/x").unwrap();
        assert_eq!(validated.host, "8.8.8.8");
        assert!(
            validated.pinned_addrs.is_empty(),
            "IP-literal hosts must not be pinned"
        );
    }

    #[test]
    fn reject_private_ip_hostname_pin_is_consistent() {
        // For a real hostname, when resolution succeeds (it may not in a
        // sandboxed CI) the returned host is the bracket-stripped host and any
        // pinned address must have passed the private-IP check.
        if let Ok(validated) = super::reject_private_ip("https://example.com/") {
            assert_eq!(validated.host, "example.com");
            for addr in &validated.pinned_addrs {
                assert!(
                    !super::is_private_ip(addr.ip()),
                    "pinned address {} should have passed the private-IP check",
                    addr.ip()
                );
            }
        }
    }
}

/// Refresh an expired OAuth access token using the stored refresh token.
///
/// Posts to the provider's token endpoint with `grant_type=refresh_token`,
/// then stores the new access token (with expiry) and rotated refresh token
/// (if the provider returns one).
///
/// SSRF defense: `token_url` originates from a tool's capabilities JSON, so
/// a malicious tool could point it at an internal service to exfiltrate the
/// refresh token. We require HTTPS, reject private/loopback IPs (including
/// DNS-resolved), and disable redirects.
///
/// Returns `true` if the refresh succeeded, `false` otherwise.
async fn refresh_oauth_token(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    config: &OAuthRefreshConfig,
) -> bool {
    let endpoint = match validate_oauth_refresh_config(config).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            tracing::warn!(error = %error, "Refusing invalid OAuth refresh configuration");
            return false;
        }
    };

    let refresh_name = format!("{}_refresh_token", config.secret_name);
    let refresh_secret = match store
        .get_for_injection(
            user_id,
            &refresh_name,
            SecretAccessContext::new("wasm.oauth_refresh", "refresh_token").target(
                extract_host_from_url(&config.token_url).unwrap_or_else(|| "unknown".to_string()),
                url::Url::parse(&config.token_url)
                    .ok()
                    .map(|url| url.path().to_string())
                    .unwrap_or_default(),
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                secret_name = %refresh_name,
                error = %e,
                "No refresh token available, skipping token refresh"
            );
            return false;
        }
    };
    if let Err(error) = validate_oauth_secret_value(refresh_secret.expose(), "refresh token") {
        tracing::warn!(error = %error, "Refusing malformed stored OAuth refresh token");
        return false;
    }

    let client = match oauth_client_for(&endpoint, Duration::from_secs(15)) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build HTTP client for token refresh");
            return false;
        }
    };

    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_secret.expose().to_string()),
        ("client_id", config.client_id.clone()),
    ];
    if let Some(ref secret) = config.client_secret {
        params.push(("client_secret", secret.clone()));
    }

    let response = match client.post(endpoint.url).form(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "OAuth token refresh request failed");
            return false;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let error_code = bounded_oauth_error(response).await;
        tracing::warn!(
            status = %status,
            error_code = error_code.as_deref().unwrap_or("unspecified"),
            "OAuth token refresh returned non-success status"
        );
        return false;
    }

    let token_data: serde_json::Value = match decode_bounded_json(response).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse token refresh response");
            return false;
        }
    };

    let new_access_token = match token_data.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            tracing::warn!("Token refresh response missing access_token field");
            return false;
        }
    };
    if let Err(error) = validate_oauth_secret_value(new_access_token, "access token") {
        tracing::warn!(error = %error, "Refusing malformed OAuth refresh response");
        return false;
    }
    if let Err(error) = validate_bearer_token_type(&token_data) {
        tracing::warn!(error = %error, "Refusing unsupported OAuth refresh response");
        return false;
    }
    let expires_at = match oauth_expiry_from_response(&token_data) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, "Refusing malformed OAuth token expiry");
            return false;
        }
    };
    let rotated_refresh = token_data
        .get("refresh_token")
        .and_then(|value| value.as_str());
    if let Some(refresh) = rotated_refresh
        && let Err(error) = validate_oauth_secret_value(refresh, "refresh token")
    {
        tracing::warn!(error = %error, "Refusing malformed rotated OAuth refresh token");
        return false;
    }

    // A provider can invalidate the old refresh token as soon as it issues a
    // rotated one. Persist that continuity credential before publishing the
    // replacement access token; a failure then leaves the old access token as
    // the authoritative value.
    if let Some(new_refresh) = rotated_refresh {
        let mut refresh_params = CreateSecretParams::new(&refresh_name, new_refresh);
        if let Some(ref provider) = config.provider {
            refresh_params = refresh_params.with_provider(provider);
        }
        if let Err(e) = store.create(user_id, refresh_params).await {
            tracing::warn!(error = %e, "Failed to store rotated refresh token");
            return false;
        }
    }

    // Store the new access token with expiry
    let mut access_params = CreateSecretParams::new(&config.secret_name, new_access_token);
    if let Some(ref provider) = config.provider {
        access_params = access_params.with_provider(provider);
    }
    if let Some(expires_at) = expires_at {
        access_params = access_params.with_expiry(expires_at);
    }

    if let Err(e) = store.create(user_id, access_params).await {
        tracing::warn!(error = %e, "Failed to store refreshed access token");
        return false;
    }

    tracing::info!(
        secret_name = %config.secret_name,
        "OAuth access token refreshed successfully"
    );
    true
}

async fn resolve_available_secret_names(
    capabilities: &Capabilities,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
) -> HashSet<String> {
    let Some(secret_capability) = capabilities.secrets.as_ref() else {
        return HashSet::new();
    };
    let Some(store) = store else {
        return HashSet::new();
    };

    match store.list(user_id).await {
        Ok(secret_refs) => secret_refs
            .into_iter()
            .filter_map(|secret| {
                if secret_capability.is_allowed(&secret.name) {
                    Some(secret.name)
                } else {
                    None
                }
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                error = %error,
                "Failed to list secrets for WASM secret_exists checks"
            );
            HashSet::new()
        }
    }
}

/// Pre-resolve credentials for all HTTP capability mappings.
///
/// Called once per tool execution (in async context, before spawn_blocking)
/// so that the synchronous WASM host function can inject credentials
/// without needing async access to the secrets store.
///
/// If an `OAuthRefreshConfig` is provided and the access token is expired
/// (or within 5 minutes of expiry), attempts a transparent refresh first.
///
/// Silently skips credentials that can't be resolved (e.g., missing secrets).
/// The tool will get a 401/403 from the API, which is the expected UX when
/// auth hasn't been configured yet.
async fn resolve_host_credentials(
    capabilities: &Capabilities,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    oauth_refresh: Option<&OAuthRefreshConfig>,
) -> Vec<ResolvedHostCredential> {
    let store = match store {
        Some(s) => s,
        None => return Vec::new(),
    };

    // Check if the access token needs refreshing before resolving credentials.
    // This runs once per tool execution, keeping the hot path (credential injection
    // inside WASM) synchronous and allocation-free.
    if let Some(config) = oauth_refresh {
        let needs_refresh = match store.get(user_id, &config.secret_name).await {
            Ok(secret) => match secret.expires_at {
                Some(expires_at) => {
                    let buffer = chrono::Duration::minutes(5);
                    expires_at - buffer < chrono::Utc::now()
                }
                // No expires_at means legacy token, don't try to refresh
                None => false,
            },
            // Expired error from store means we definitely need to refresh
            Err(SecretError::Expired) => true,
            // Not found or other errors: skip refresh, let the normal flow handle it
            Err(_) => false,
        };

        if needs_refresh {
            tracing::debug!(
                secret_name = %config.secret_name,
                "Access token expired or near expiry, attempting refresh"
            );
            refresh_oauth_token(store, user_id, config).await;
        }
    }

    let http_cap = match &capabilities.http {
        Some(cap) => cap,
        None => return Vec::new(),
    };

    if http_cap.credentials.is_empty() {
        return Vec::new();
    }

    let mut resolved = Vec::new();

    for mapping in http_cap.credentials.values() {
        // Skip UrlPath credentials, they're handled by placeholder substitution
        if matches!(
            mapping.location,
            CredentialLocation::UrlPath { .. }
                | CredentialLocation::UrlBase { .. }
                | CredentialLocation::Body { .. }
        ) {
            continue;
        }

        let secret = match store
            .get_for_injection(
                user_id,
                &mapping.secret_name,
                SecretAccessContext::new("wasm.host_credentials", "http_credential_injection"),
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    secret_name = %mapping.secret_name,
                    error = %e,
                    "Could not resolve credential for WASM tool (auth may not be configured)"
                );
                continue;
            }
        };

        let mut injected = InjectedCredentials::empty();
        inject_credential(&mut injected, &mapping.location, &secret);

        if injected.is_empty() {
            continue;
        }

        resolved.push(ResolvedHostCredential {
            host_patterns: mapping.host_patterns.clone(),
            headers: injected.headers,
            query_params: injected.query_params,
            secret_value: secret.expose().to_string(),
        });
    }

    if !resolved.is_empty() {
        tracing::debug!(
            count = resolved.len(),
            "Pre-resolved host credentials for WASM tool execution"
        );
    }

    resolved
}

/// Extract the hostname from a URL string.
///
/// Handles `https://host:port/path`, stripping scheme, port, and path.
/// Also handles IPv6 bracket notation like `http://[::1]:8080/path`.
/// Returns None for malformed URLs.
fn extract_host_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|h| {
        h.strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(h)
            .to_lowercase()
    })
}

/// Outcome of validating a URL against the private-IP / DNS-rebinding policy.
///
/// `host` is the bracket-stripped host string from the URL (suitable for
/// `reqwest::ClientBuilder::resolve_to_addrs`). `pinned_addrs` holds the exact
/// addresses the host resolved to *at validation time*; it is empty for
/// IP-literal hosts (nothing to pin — `reqwest` connects to the literal
/// directly and cannot rebind it).
#[derive(Debug, Clone)]
struct ValidatedHost {
    host: String,
    pinned_addrs: Vec<std::net::SocketAddr>,
}

/// Resolve the URL's hostname and reject connections to private/internal IP addresses.
/// This prevents DNS rebinding attacks where an attacker's domain resolves to an
/// internal IP after passing the allowlist check.
///
/// On success it returns the validated host and the resolved addresses, so the
/// caller can pin the connection to exactly those addresses
/// (`resolve_to_addrs`) and close the time-of-check / time-of-use gap where the
/// host could rebind to a private address before `reqwest` performs its own
/// connect-time resolution.
#[cfg(test)]
fn reject_private_ip(url: &str) -> Result<ValidatedHost, String> {
    let guarded = validate_outbound_url_pinned(
        url,
        &OutboundUrlGuardOptions {
            // Credentials and tool data must never traverse a plaintext
            // network hop. Loopback/private HTTP is independently disallowed.
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: Vec::new(),
        },
    )
    .map_err(|error| error.to_string())?;
    let host = guarded
        .url
        .host_str()
        .ok_or_else(|| "Failed to parse host from URL".to_string())?
        .to_string();
    Ok(ValidatedHost {
        host,
        pinned_addrs: guarded.pinned_addrs,
    })
}

/// Check if an IP address belongs to a private/internal range.
#[cfg(test)]
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    !is_public_outbound_ip(ip)
}

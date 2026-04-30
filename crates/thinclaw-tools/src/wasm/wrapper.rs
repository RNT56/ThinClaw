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
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use thinclaw_secrets::{
    CreateSecretParams, CredentialLocation, SecretAccessContext, SecretError, SecretsStore,
};
use thinclaw_tools_core::{Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::wasm::capabilities::Capabilities;
use crate::wasm::credential_injector::{
    InjectedCredentials, host_matches_pattern, inject_credential,
};
use crate::wasm::error::WasmError;
use crate::wasm::host::{HostState, LogLevel};
use crate::wasm::limits::{ResourceLimits, WasmResourceLimiter};
pub use crate::wasm::oauth::OAuthRefreshConfig;
use crate::wasm::ports::{ExactValueLeakScanner, HostToolInvoker, LeakScan, LeakScanner};
use crate::wasm::runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmToolRuntime};

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
        let leak_values = self.exact_leak_values();
        let raw_headers: HashMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();

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

        // Get the max response size from capabilities (default 10MB).
        let max_response_bytes = self
            .host_state
            .capabilities()
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Resolve hostname and reject private/internal IPs to prevent DNS rebinding.
        reject_private_ip(&url)?;

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
        let rt = self.http_runtime.as_ref().expect("just initialized");
        let result = rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
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

            // Caller-specified timeout (default 30s, max 5min)
            let timeout_ms = timeout_ms.unwrap_or(30_000).min(300_000) as u64;
            let timeout = Duration::from_millis(timeout_ms);
            let response = request.timeout(timeout).send().await.map_err(|e| {
                // Walk the full error chain for the actual root cause
                let mut chain = format!("HTTP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                chain
            })?;

            let status = response.status().as_u16();
            let response_headers: HashMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
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

            // Read body with a size cap to prevent memory exhaustion.
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
        let rt = self.tool_runtime.as_ref().expect("just initialized");
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
                        LogLevel::Trace => tracing::trace!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Debug => tracing::debug!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Info => tracing::info!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Warn => tracing::warn!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Error => tracing::error!(target: "wasm_tool", "{}", log.message),
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
    // SSRF defense: token_url comes from the tool's capabilities file.
    if !config.token_url.starts_with("https://") {
        tracing::warn!(
            token_url = %config.token_url,
            "OAuth token_url must use HTTPS, refusing token refresh"
        );
        return false;
    }
    if let Err(reason) = reject_private_ip(&config.token_url) {
        tracing::warn!(
            token_url = %config.token_url,
            reason = %reason,
            "OAuth token_url points to a private/internal IP, refusing token refresh"
        );
        return false;
    }

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

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
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

    let response = match client.post(&config.token_url).form(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth token refresh request failed");
            return false;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %body,
            "OAuth token refresh returned non-success status"
        );
        return false;
    }

    let token_data: serde_json::Value = match response.json().await {
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

    // Store the new access token with expiry
    let mut access_params = CreateSecretParams::new(&config.secret_name, new_access_token);
    if let Some(ref provider) = config.provider {
        access_params = access_params.with_provider(provider);
    }
    if let Some(expires_in) = token_data.get("expires_in").and_then(|v| v.as_u64()) {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64);
        access_params = access_params.with_expiry(expires_at);
    }

    if let Err(e) = store.create(user_id, access_params).await {
        tracing::warn!(error = %e, "Failed to store refreshed access token");
        return false;
    }

    // Store rotated refresh token if the provider sent a new one
    if let Some(new_refresh) = token_data.get("refresh_token").and_then(|v| v.as_str()) {
        let mut refresh_params = CreateSecretParams::new(&refresh_name, new_refresh);
        if let Some(ref provider) = config.provider {
            refresh_params = refresh_params.with_provider(provider);
        }
        if let Err(e) = store.create(user_id, refresh_params).await {
            tracing::warn!(error = %e, "Failed to store rotated refresh token");
        }
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
        if matches!(mapping.location, CredentialLocation::UrlPath { .. }) {
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

/// Resolve the URL's hostname and reject connections to private/internal IP addresses.
/// This prevents DNS rebinding attacks where an attacker's domain resolves to an
/// internal IP after passing the allowlist check.
fn reject_private_ip(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Failed to parse URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Unsupported URL scheme: {}", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL contains userinfo (@) which is not allowed".to_string());
    }

    let host = parsed
        .host_str()
        .map(|h| {
            h.strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .unwrap_or(h)
        })
        .ok_or_else(|| "Failed to parse host from URL".to_string())?;

    // If the host is already an IP, check it directly
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if is_private_ip(ip) {
            Err(format!(
                "HTTP request to private/internal IP {} is not allowed",
                ip
            ))
        } else {
            Ok(())
        };
    }

    // Resolve DNS and check all addresses
    use std::net::ToSocketAddrs;
    // Port 0 is a placeholder; ToSocketAddrs needs host:port but the port
    // doesn't affect which IPs the hostname resolves to.
    let addrs: Vec<_> = format!("{}:0", host)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", host, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {}", host));
    }

    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(format!(
                "DNS rebinding detected: {} resolved to private IP {}",
                host,
                addr.ip()
            ));
        }
    }

    Ok(())
}

/// Check if an IP address belongs to a private/internal range.
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
            || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()      // 169.254.0.0/16
            || v4.is_unspecified()     // 0.0.0.0
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
            || v6.is_unspecified()     // ::
            // fc00::/7 (unique local)
            || (v6.segments()[0] & 0xFE00) == 0xFC00
            // fe80::/10 (link-local)
            || (v6.segments()[0] & 0xFFC0) == 0xFE80
        }
    }
}

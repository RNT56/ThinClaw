//! WASM channel store data and host-function bindings.
//!
//! Owns [`ChannelStoreData`] (the per-execution store payload), its
//! [`WasiView`] implementation, and the generated `channel-host`
//! [`Host`](super::near::agent::channel_host::Host) implementation that backs
//! every host call a WASM channel guest can make (logging, HTTP, workspace,
//! pairing, message emission).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::pairing::PairingStore;
use crate::wasm::capabilities::{
    ChannelCapabilities, CredentialLocation, CredentialMapping, credential_placeholder_name,
};
use crate::wasm::host::{ChannelHostState, ChannelWorkspaceStore, EmittedMessage};
use crate::wasm::host::{LogLevel, WorkspaceReader};
use crate::wasm::limits::WasmResourceLimiter;
use thinclaw_safety::LeakDetector;
use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned_async};

use super::near;

const MAX_WASM_HTTP_URL_BYTES: usize = 16 * 1024;
const MAX_WASM_HTTP_HEADERS_JSON_BYTES: usize = 128 * 1024;
const MAX_WASM_HTTP_HEADERS: usize = 128;
const MAX_WASM_HTTP_HEADER_NAME_BYTES: usize = 256;
const MAX_WASM_HTTP_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_WASM_HTTP_HEADER_TOTAL_BYTES: usize = 128 * 1024;
const MAX_WASM_HTTP_REQUEST_BYTES: usize = 20 * 1024 * 1024;
const MAX_WASM_HTTP_RESPONSE_BYTES: usize = 20 * 1024 * 1024;
const MAX_WASM_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_PAIRING_IDENTIFIER_BYTES: usize = 1024;
const MAX_PAIRING_USERNAME_BYTES: usize = 1024;
const MAX_PAIRING_METADATA_BYTES: usize = 256 * 1024;
const MAX_TELEGRAM_MARKDOWN_BYTES: usize = 64 * 1024;
const MAX_TELEGRAM_HTML_BYTES: usize = 256 * 1024;

type PreparedCredentialRequest = (String, HashMap<String, String>, Option<Vec<u8>>);

fn has_unresolved_credential_placeholder(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        let start = index + 1;
        index = start;
        while index < bytes.len()
            && (bytes[index].is_ascii_uppercase()
                || bytes[index].is_ascii_digit()
                || bytes[index] == b'_')
        {
            index += 1;
        }
        if index > start && index < bytes.len() && bytes[index] == b'}' {
            return true;
        }
    }
    false
}

#[derive(Clone)]
struct CredentialPolicyEntry {
    placeholder: String,
    marker: String,
    mapping: CredentialMapping,
}

fn credential_host_matches(host: &str, pattern: &str) -> bool {
    if host.eq_ignore_ascii_case(pattern) {
        return true;
    }
    let Some(suffix) = pattern.strip_prefix("*.") else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let suffix = suffix.to_ascii_lowercase();
    host.len() > suffix.len()
        && host.ends_with(&suffix)
        && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
}

fn parse_safe_credential_base(value: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(value)
        .map_err(|_| "Channel base URL credential is malformed".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Channel base URL credential is not a safe HTTPS base".to_string());
    }
    Ok(parsed)
}

fn url_is_within_base(url: &url::Url, base: &url::Url) -> bool {
    if url.origin() != base.origin() {
        return false;
    }
    let base_path = base.path().trim_end_matches('/');
    base_path.is_empty()
        || url.path() == base_path
        || url
            .path()
            .strip_prefix(base_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

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

    fn credential_policy_entries(&self) -> Result<Vec<CredentialPolicyEntry>, String> {
        let http = self
            .host_state
            .capabilities()
            .tool_capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;
        let mut entries = Vec::with_capacity(http.credentials.len());
        let mut declared = HashSet::with_capacity(http.credentials.len());
        for (index, mapping) in http.credentials.values().enumerate() {
            let name = credential_placeholder_name(&mapping.secret_name)
                .ok_or_else(|| "HTTP credential mapping has an invalid secret name".to_string())?;
            if !declared.insert(name.clone()) {
                return Err("HTTP credential mappings contain a duplicate secret".to_string());
            }
            entries.push(CredentialPolicyEntry {
                placeholder: format!("{{{name}}}"),
                marker: format!("tccredentialmarker{index}"),
                mapping: mapping.clone(),
            });
        }
        if self.credentials.keys().any(|name| !declared.contains(name)) {
            return Err("Injected credential is not declared by the channel manifest".to_string());
        }
        Ok(entries)
    }

    /// Validate every placeholder against its manifest-declared placement and
    /// destination before substituting any plaintext secret.
    pub(super) fn prepare_credential_request(
        &self,
        raw_url: &str,
        mut headers: HashMap<String, String>,
        body: Option<Vec<u8>>,
    ) -> Result<PreparedCredentialRequest, String> {
        let entries = self.credential_policy_entries()?;
        let body_text = body
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok());

        let mut normalized_header_names = HashSet::with_capacity(headers.len());
        if headers
            .keys()
            .any(|name| !normalized_header_names.insert(name.to_ascii_lowercase()))
        {
            return Err("WASM HTTP request contains duplicate header names".to_string());
        }

        let mut tokenized_url = raw_url.to_string();
        let mut used = vec![false; entries.len()];
        for (index, entry) in entries.iter().enumerate() {
            if !matches!(entry.mapping.location, CredentialLocation::UrlBase { .. }) {
                continue;
            }
            let count = raw_url.matches(&entry.placeholder).count();
            if count == 0 {
                continue;
            }
            let suffix = raw_url
                .strip_prefix(&entry.placeholder)
                .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                .ok_or_else(|| {
                    "Base URL credential placeholder is not in the URL base position".to_string()
                })?;
            if count != 1 {
                return Err("Base URL credential placeholder is ambiguous".to_string());
            }
            if headers
                .values()
                .any(|value| value.contains(&entry.placeholder))
                || body_text.is_some_and(|value| value.contains(&entry.placeholder))
            {
                return Err("Base URL credential appears outside its declared location".to_string());
            }
            tokenized_url = format!("https://tcbase{index}.invalid{suffix}");
            used[index] = true;
        }
        for entry in &entries {
            if !matches!(entry.mapping.location, CredentialLocation::UrlBase { .. }) {
                tokenized_url = tokenized_url.replace(&entry.placeholder, &entry.marker);
            }
        }
        let parsed_template = url::Url::parse(&tokenized_url)
            .map_err(|_| "WASM HTTP URL template is malformed".to_string())?;

        for (index, entry) in entries.iter().enumerate() {
            let url_count = raw_url.matches(&entry.placeholder).count();
            let header_matches = headers
                .iter()
                .filter(|(_, value)| value.contains(&entry.placeholder))
                .collect::<Vec<_>>();
            let body_count = body_text.map_or(0, |value| value.matches(&entry.placeholder).count());
            let occurrence_count = url_count
                .saturating_add(header_matches.len())
                .saturating_add(body_count);
            if occurrence_count == 0 {
                continue;
            }
            used[index] = true;

            match &entry.mapping.location {
                CredentialLocation::Bearer => {
                    if url_count != 0
                        || body_count != 0
                        || header_matches.len() != 1
                        || !header_matches[0].0.eq_ignore_ascii_case("authorization")
                        || header_matches[0].1 != &format!("Bearer {}", entry.placeholder)
                    {
                        return Err(
                            "Bearer credential appears outside its declared header".to_string()
                        );
                    }
                }
                CredentialLocation::Basic { username } => {
                    let mut expected_username = username.clone();
                    for candidate in &entries {
                        expected_username =
                            expected_username.replace(&candidate.placeholder, &candidate.marker);
                    }
                    if url_count != 1
                        || !header_matches.is_empty()
                        || body_count != 0
                        || parsed_template.password() != Some(entry.marker.as_str())
                        || parsed_template.username() != expected_username
                        || has_unresolved_credential_placeholder(username)
                            && has_unresolved_credential_placeholder(&expected_username)
                    {
                        return Err("Basic credential appears outside URL userinfo".to_string());
                    }
                }
                CredentialLocation::Header { name, prefix } => {
                    let expected = format!(
                        "{}{}",
                        prefix.as_deref().unwrap_or_default(),
                        entry.placeholder
                    );
                    if url_count != 0
                        || body_count != 0
                        || header_matches.len() != 1
                        || !header_matches[0].0.eq_ignore_ascii_case(name)
                        || header_matches[0].1 != &expected
                    {
                        return Err(
                            "Credential appears outside its declared HTTP header".to_string()
                        );
                    }
                }
                CredentialLocation::QueryParam { name } => {
                    let matching_query_values = parsed_template
                        .query_pairs()
                        .filter(|(query_name, value)| {
                            query_name == name.as_str() && value == entry.marker.as_str()
                        })
                        .count();
                    if url_count != 1
                        || !header_matches.is_empty()
                        || body_count != 0
                        || matching_query_values != 1
                    {
                        return Err(
                            "Credential appears outside its declared query parameter".to_string()
                        );
                    }
                }
                CredentialLocation::UrlPath { .. } => {
                    let path_count = parsed_template.path().matches(&entry.marker).count();
                    let username_count = usize::from(
                        parsed_template.username() == entry.marker
                            && entries.iter().any(|candidate| {
                                matches!(candidate.mapping.location, CredentialLocation::Basic { ref username } if username == &entry.placeholder)
                            }),
                    );
                    if url_count == 0
                        || !header_matches.is_empty()
                        || body_count != 0
                        || path_count.saturating_add(username_count) != url_count
                    {
                        return Err("URL credential appears outside its declared path".to_string());
                    }
                }
                CredentialLocation::UrlBase { .. } => {
                    if !used[index]
                        || url_count != 1
                        || !header_matches.is_empty()
                        || body_count != 0
                    {
                        return Err(
                            "Base URL credential appears outside its declared location".to_string()
                        );
                    }
                }
                CredentialLocation::Body { .. } => {
                    if url_count != 0 || !header_matches.is_empty() || body_count != 1 {
                        return Err(
                            "Credential appears outside its declared request body".to_string()
                        );
                    }
                }
            }
        }

        let mut injected_url = raw_url.to_string();
        for (index, entry) in entries.iter().enumerate() {
            if !used[index] {
                continue;
            }
            let name = credential_placeholder_name(&entry.mapping.secret_name)
                .ok_or_else(|| "HTTP credential mapping is invalid".to_string())?;
            let value = self
                .credentials
                .get(&name)
                .ok_or_else(|| "A required HTTP credential is unavailable".to_string())?;
            match &entry.mapping.location {
                CredentialLocation::UrlBase { .. } => {
                    parse_safe_credential_base(value)?;
                    let suffix = injected_url
                        .strip_prefix(&entry.placeholder)
                        .ok_or_else(|| "Base URL credential placement changed".to_string())?;
                    injected_url = format!("{}{suffix}", value.trim_end_matches('/'));
                }
                CredentialLocation::UrlPath { .. }
                | CredentialLocation::QueryParam { .. }
                | CredentialLocation::Basic { .. } => {
                    let encoded = urlencoding::encode(value);
                    injected_url = injected_url.replace(&entry.placeholder, encoded.as_ref());
                }
                CredentialLocation::Bearer | CredentialLocation::Header { .. } => {
                    for header_value in headers.values_mut() {
                        *header_value = header_value.replace(&entry.placeholder, value);
                    }
                }
                CredentialLocation::Body { .. } => {}
            }
        }

        let mut body = body;
        if let Some(body_bytes) = body.as_mut()
            && let Ok(text) = std::str::from_utf8(body_bytes)
        {
            let mut injected = text.to_string();
            for (index, entry) in entries.iter().enumerate() {
                if used[index] && matches!(entry.mapping.location, CredentialLocation::Body { .. })
                {
                    let name = credential_placeholder_name(&entry.mapping.secret_name)
                        .ok_or_else(|| "HTTP credential mapping is invalid".to_string())?;
                    let value = self
                        .credentials
                        .get(&name)
                        .ok_or_else(|| "A required HTTP credential is unavailable".to_string())?;
                    injected = injected.replace(&entry.placeholder, value);
                }
            }
            *body_bytes = injected.into_bytes();
        }

        if has_unresolved_credential_placeholder(&injected_url)
            || headers
                .values()
                .any(|value| has_unresolved_credential_placeholder(value))
            || body.as_deref().is_some_and(|bytes| {
                std::str::from_utf8(bytes)
                    .ok()
                    .is_some_and(has_unresolved_credential_placeholder)
            })
        {
            return Err("WASM HTTP request contains an unresolved credential".to_string());
        }

        let parsed_url = url::Url::parse(&injected_url)
            .map_err(|_| "WASM HTTP URL is malformed after credential injection".to_string())?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| "WASM HTTP URL has no destination host".to_string())?;
        let bases = entries
            .iter()
            .filter(|entry| matches!(entry.mapping.location, CredentialLocation::UrlBase { .. }))
            .filter_map(|entry| {
                let name = credential_placeholder_name(&entry.mapping.secret_name)?;
                self.credentials
                    .get(&name)
                    .and_then(|value| parse_safe_credential_base(value).ok())
            })
            .collect::<Vec<_>>();
        for (index, entry) in entries.iter().enumerate() {
            if !used[index] {
                continue;
            }
            let explicit_host_match = entry
                .mapping
                .host_patterns
                .iter()
                .filter(|pattern| pattern.as_str() != "*")
                .any(|pattern| credential_host_matches(host, pattern));
            let base_bound = bases
                .iter()
                .any(|base| url_is_within_base(&parsed_url, base));
            let wildcard_bound = entry
                .mapping
                .host_patterns
                .iter()
                .any(|pattern| pattern == "*")
                && base_bound;
            if !explicit_host_match && !wildcard_bound {
                return Err(
                    "Credential destination is not authorized by the channel manifest".to_string(),
                );
            }
            if let CredentialLocation::UrlBase { .. } = entry.mapping.location {
                let name = credential_placeholder_name(&entry.mapping.secret_name)
                    .ok_or_else(|| "HTTP credential mapping is invalid".to_string())?;
                let base = self
                    .credentials
                    .get(&name)
                    .ok_or_else(|| "A required HTTP credential is unavailable".to_string())
                    .and_then(|value| parse_safe_credential_base(value))?;
                if !url_is_within_base(&parsed_url, &base) {
                    return Err("Request escaped its configured channel base URL".to_string());
                }
            }
        }

        Ok((injected_url, headers, body))
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages, logs, or
    /// return values to WASM. reqwest::Error includes the full URL in its
    /// Display output, so any error from an injected-URL request will
    /// contain the raw credential unless we scrub it.
    pub(super) fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for value in self.credentials.values() {
            if !value.is_empty() {
                result = result.replace(value, "[REDACTED]");
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
        let method = method.to_ascii_uppercase();
        if !matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD"
        ) || url.is_empty()
            || url.len() > MAX_WASM_HTTP_URL_BYTES
            || headers_json.len() > MAX_WASM_HTTP_HEADERS_JSON_BYTES
        {
            return Err("Malformed or oversized WASM HTTP request".to_string());
        }

        let http_capability = self
            .host_state
            .capabilities()
            .tool_capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;
        let max_request_bytes = http_capability
            .max_request_bytes
            .min(MAX_WASM_HTTP_REQUEST_BYTES);
        let max_response_bytes = http_capability
            .max_response_bytes
            .min(MAX_WASM_HTTP_RESPONSE_BYTES);
        let capability_timeout = http_capability.timeout.min(MAX_WASM_HTTP_TIMEOUT);
        if body
            .as_ref()
            .is_some_and(|value| value.len() > max_request_bytes)
        {
            return Err("WASM HTTP request body exceeds the configured limit".to_string());
        }

        tracing::info!(
            method = %method,
            body_len = body.as_ref().map(|b| b.len()).unwrap_or(0),
            "WASM http_request called"
        );

        let leak_detector = self.leak_detector();
        let raw_headers: std::collections::HashMap<String, String> =
            serde_json::from_str(&headers_json)
                .map_err(|_| "WASM HTTP headers must be a JSON object of strings".to_string())?;
        if raw_headers.len() > MAX_WASM_HTTP_HEADERS
            || raw_headers.iter().any(|(name, value)| {
                name.is_empty()
                    || name.len() > MAX_WASM_HTTP_HEADER_NAME_BYTES
                    || value.len() > MAX_WASM_HTTP_HEADER_VALUE_BYTES
                    || name.chars().any(char::is_control)
                    || value.chars().any(|character| {
                        character == '\r' || character == '\n' || character == '\0'
                    })
            })
            || raw_headers.iter().fold(0usize, |total, (name, value)| {
                total.saturating_add(name.len()).saturating_add(value.len())
            }) > MAX_WASM_HTTP_HEADER_TOTAL_BYTES
        {
            return Err("WASM HTTP headers are malformed or oversized".to_string());
        }
        let raw_header_vec: Vec<(String, String)> = raw_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        leak_detector
            .scan_http_request(&url, &raw_header_vec, body.as_deref())
            .map_err(|e| format!("Potential secret leak blocked: {}", e))?;

        let (injected_url, mut headers, body) =
            self.prepare_credential_request(&url, raw_headers, body)?;
        if injected_url.len() > MAX_WASM_HTTP_URL_BYTES
            || headers.values().any(|value| {
                value.len() > MAX_WASM_HTTP_HEADER_VALUE_BYTES
                    || has_unresolved_credential_placeholder(value)
            })
            || body
                .as_ref()
                .is_some_and(|value| value.len() > max_request_bytes)
        {
            return Err("WASM HTTP request is oversized after credential injection".to_string());
        }

        let url_changed = injected_url != url;
        tracing::info!(url_changed = url_changed, "URL after credential injection");
        tracing::debug!(
            header_count = headers.len(),
            "Validated and injected request headers"
        );

        let mut parsed_url = url::Url::parse(&injected_url)
            .map_err(|_| "WASM HTTP URL is malformed after credential injection".to_string())?;
        if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
            if headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case(reqwest::header::AUTHORIZATION.as_str()))
            {
                return Err(
                    "WASM HTTP request contains conflicting authorization credentials".to_string(),
                );
            }
            let username = urlencoding::decode(parsed_url.username())
                .map_err(|_| "WASM HTTP basic-auth username is malformed".to_string())?;
            let password = parsed_url
                .password()
                .map(urlencoding::decode)
                .transpose()
                .map_err(|_| "WASM HTTP basic-auth password is malformed".to_string())?
                .unwrap_or_default();
            use base64::Engine;
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
            headers.insert("Authorization".to_string(), format!("Basic {encoded}"));
            parsed_url
                .set_username("")
                .map_err(|_| "Failed to normalize WASM HTTP URL".to_string())?;
            parsed_url
                .set_password(None)
                .map_err(|_| "Failed to normalize WASM HTTP URL".to_string())?;
        }
        if headers.len() > MAX_WASM_HTTP_HEADERS
            || headers.iter().fold(0usize, |total, (name, value)| {
                total.saturating_add(name.len()).saturating_add(value.len())
            }) > MAX_WASM_HTTP_HEADER_TOTAL_BYTES
            || headers.values().any(|value| {
                value.len() > MAX_WASM_HTTP_HEADER_VALUE_BYTES || value.contains(['\r', '\n', '\0'])
            })
        {
            return Err("WASM HTTP headers are oversized after credential injection".to_string());
        }
        let url = parsed_url.to_string();

        self.host_state
            .check_http_allowed(&url, &method)
            .map_err(|e| {
                let safe_error = self.redact_credentials(&e);
                tracing::error!(error = %safe_error, "HTTP not allowed");
                format!("HTTP not allowed: {safe_error}")
            })?;

        // Record the request for rate limiting
        self.host_state.record_http_request().map_err(|e| {
            tracing::error!(error = %e, "Rate limit exceeded");
            format!("Rate limit exceeded: {}", e)
        })?;
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
        let rt = self
            .http_runtime
            .as_ref()
            .ok_or_else(|| "WASM HTTP runtime initialization failed".to_string())?;
        let result = rt.block_on(async {
            let requested_timeout =
                std::time::Duration::from_millis(u64::from(timeout_ms.unwrap_or_else(|| {
                    u32::try_from(capability_timeout.as_millis()).unwrap_or(u32::MAX)
                })));
            let timeout = requested_timeout
                .min(capability_timeout)
                .min(MAX_WASM_HTTP_TIMEOUT)
                .max(std::time::Duration::from_millis(1));
            let deadline = tokio::time::Instant::now() + timeout;
            let guarded_url = tokio::time::timeout_at(
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
            .map_err(|_| "WASM HTTP request timed out during DNS validation".to_string())?
            .map_err(|_| {
                "WASM HTTP destination is not a trusted public HTTPS endpoint".to_string()
            })?;
            let mut client_builder = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy();
            if !guarded_url.pinned_addrs.is_empty()
                && let Some(host) = guarded_url.url.host_str()
            {
                client_builder = client_builder.resolve_to_addrs(host, &guarded_url.pinned_addrs);
            }
            let client = client_builder
                .build()
                .map_err(|_| "Failed to build WASM HTTP client".to_string())?;

            let mut request = match method.as_str() {
                "GET" => client.get(guarded_url.url.clone()),
                "POST" => client.post(guarded_url.url.clone()),
                "PUT" => client.put(guarded_url.url.clone()),
                "DELETE" => client.delete(guarded_url.url.clone()),
                "PATCH" => client.patch(guarded_url.url.clone()),
                "HEAD" => client.head(guarded_url.url.clone()),
                _ => return Err("Unsupported WASM HTTP method".to_string()),
            };

            // Add headers
            for (key, value) in headers {
                request = request.header(&key, &value);
            }

            // Add body if present
            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            }

            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or_else(|| "WASM HTTP request timed out before send".to_string())?;
            let response =
                request.timeout(remaining).send().await.map_err(|error| {
                    format!("WASM HTTP request failed: {}", error.without_url())
                })?;

            let status = response.status().as_u16();
            if response.headers().len() > MAX_WASM_HTTP_HEADERS {
                return Err("WASM HTTP response contains too many headers".to_string());
            }
            let mut response_headers = std::collections::HashMap::new();
            let mut response_header_bytes = 0usize;
            for (name, value) in response.headers() {
                let Ok(value) = value.to_str() else {
                    continue;
                };
                if name.as_str().len() > MAX_WASM_HTTP_HEADER_NAME_BYTES
                    || value.len() > MAX_WASM_HTTP_HEADER_VALUE_BYTES
                {
                    return Err(
                        "WASM HTTP response headers exceed the configured limit".to_string()
                    );
                }
                response_header_bytes = response_header_bytes
                    .saturating_add(name.as_str().len())
                    .saturating_add(value.len());
                if response_header_bytes > MAX_WASM_HTTP_HEADER_TOTAL_BYTES {
                    return Err(
                        "WASM HTTP response headers exceed the configured limit".to_string()
                    );
                }
                let cleaned = leak_detector
                    .scan_and_clean(value)
                    .map_err(|_| "Potential secret leak in HTTP response header".to_string())?;
                response_headers.insert(name.as_str().to_string(), cleaned);
            }
            let headers_json = serde_json::to_string(&response_headers)
                .map_err(|_| "Failed to encode WASM HTTP response headers".to_string())?;

            let mut body = crate::response::bounded_bytes(response, max_response_bytes)
                .await
                .map_err(|error| format!("Failed to read WASM HTTP response: {error}"))?;

            tracing::info!(
                status = status,
                body_len = body.len(),
                "HTTP response received"
            );

            // Leak detection on response body (best-effort)
            if let Ok(body_str) = std::str::from_utf8(&body) {
                let cleaned = leak_detector
                    .scan_and_clean(body_str)
                    .map_err(|e| format!("Potential secret leak in response: {}", e))?;
                if cleaned != body_str {
                    body = cleaned.into_bytes();
                    if body.len() > max_response_bytes {
                        return Err(
                            "Cleaned WASM HTTP response exceeds the configured limit".to_string()
                        );
                    }
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
            user_id_bytes = msg.user_id.len(),
            user_name_bytes = msg.user_name.as_ref().map_or(0, String::len),
            content_len = msg.content.len(),
            attachment_count = msg.attachments.len(),
            "WASM emit_message called"
        );

        let mut emitted = EmittedMessage::new(msg.user_id, msg.content);
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
        if channel.len() > 64 || channel != self.host_state.channel_name() {
            return Err("pairing namespace does not match the active channel".to_string());
        }
        if id.is_empty()
            || id.len() > MAX_PAIRING_IDENTIFIER_BYTES
            || id.chars().any(char::is_control)
            || meta_json.len() > MAX_PAIRING_METADATA_BYTES
        {
            return Err("pairing request is malformed or oversized".to_string());
        }
        let meta = if meta_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&meta_json)
                    .map_err(|_| "pairing metadata is not valid JSON".to_string())?,
            )
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
        if channel.len() > 64 || channel != self.host_state.channel_name() {
            return Err("pairing namespace does not match the active channel".to_string());
        }
        if id.is_empty()
            || id.len() > MAX_PAIRING_IDENTIFIER_BYTES
            || id.chars().any(char::is_control)
            || username.as_deref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > MAX_PAIRING_USERNAME_BYTES
                    || value.chars().any(char::is_control)
            })
        {
            return Err("pairing identity is malformed or oversized".to_string());
        }
        self.pairing_store
            .is_sender_allowed(&channel, &id, username.as_deref())
            .map_err(|e| e.to_string())
    }

    fn pairing_read_allow_from(&mut self, channel: String) -> Result<Vec<String>, String> {
        if channel.len() > 64 || channel != self.host_state.channel_name() {
            return Err("pairing namespace does not match the active channel".to_string());
        }
        self.pairing_store
            .read_allow_from(&channel)
            .map_err(|e| e.to_string())
    }

    fn markdown_to_telegram_html(&mut self, markdown: String) -> String {
        if markdown.len() > MAX_TELEGRAM_MARKDOWN_BYTES {
            tracing::warn!(
                markdown_bytes = markdown.len(),
                "Rejected oversized Telegram markdown conversion"
            );
            return String::new();
        }
        let html = crate::wasm::telegram_html::markdown_to_telegram_html(&markdown);
        if html.len() > MAX_TELEGRAM_HTML_BYTES {
            tracing::warn!(
                html_bytes = html.len(),
                "Rejected oversized Telegram HTML conversion output"
            );
            String::new()
        } else {
            html
        }
    }
}

//! HTTP request tool.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, StatusCode, Url};

use crate::wasm::{InjectedCredentials, SharedCredentialRegistry, inject_credential};
use thinclaw_safety::LeakDetector;
use thinclaw_secrets::SecretsStore;
#[cfg(test)]
use thinclaw_tools_core::validate_outbound_url_pinned;
use thinclaw_tools_core::{
    ApprovalRequirement, GuardedUrl, OutboundUrlGuardOptions, Tool, ToolError, ToolOutput,
    ToolRateLimitConfig, require_str, validate_outbound_url_pinned_async,
};
use thinclaw_types::JobContext;

#[cfg(feature = "html-to-markdown")]
use crate::builtin::convert_html_to_markdown;

/// Maximum response body size (5 MB).
///
/// 5 MB is large enough for typical JSON API responses and moderate HTML pages,
/// but small enough to prevent OOM from malicious or runaway servers.  The WASM
/// HTTP wrapper uses the same limit for consistency.
const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;
const MAX_REQUEST_BODY_SIZE: usize = 5 * 1024 * 1024;
const MAX_REQUEST_URL_BYTES: usize = 16 * 1024;
const MAX_REQUEST_HEADERS: usize = 64;
const MAX_REQUEST_HEADER_NAME_BYTES: usize = 256;
const MAX_REQUEST_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_REQUEST_HEADER_TOTAL_BYTES: usize = 64 * 1024;
const MAX_AUTOMATIC_CREDENTIALS: usize = 64;
const MAX_REDIRECTS: usize = 10;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_REQUEST_TIMEOUT_SECS: u64 = 120;

/// Tool for making HTTP requests.
pub struct HttpTool {
    client: Option<Client>,
    client_init_error: Option<String>,
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// Optional domain allowlist. When set (non-empty), only URLs whose host
    /// matches one of the glob patterns are permitted. Patterns are
    /// case-insensitive. Example: `*.openai.com, api.github.com`.
    url_allowlist: Vec<String>,
}

impl HttpTool {
    /// Create a new HTTP tool.
    pub fn new() -> Self {
        // Redirects are processed manually in `execute`: every hop is resolved,
        // checked, and pinned before a connection is attempted. Reqwest's
        // automatic redirect path cannot dynamically pin a newly selected host.
        let client_result = Client::builder()
            .timeout(Duration::from_secs(MAX_REQUEST_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            // A configured HTTP(S) proxy would receive the destination URL and
            // bypass the DNS address that passed the SSRF guard.
            .no_proxy()
            .build();
        let (client, client_init_error) = match client_result {
            Ok(client) => (Some(client), None),
            Err(error) => {
                tracing::error!(%error, "Failed to initialize the guarded HTTP tool client");
                (None, Some(error.to_string()))
            }
        };

        // Parse URL allowlist from environment
        let url_allowlist: Vec<String> = std::env::var("HTTP_URL_ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !url_allowlist.is_empty() {
            tracing::info!(patterns = ?url_allowlist, "HTTP URL allowlist active");
        }

        Self {
            client,
            client_init_error,
            credential_registry: None,
            secrets_store: None,
            url_allowlist,
        }
    }

    /// Attach a credential registry and secrets store for auto-injection.
    pub fn with_credentials(
        mut self,
        registry: Arc<SharedCredentialRegistry>,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.credential_registry = Some(registry);
        self.secrets_store = Some(secrets_store);
        self
    }
}

#[cfg(test)]
fn validate_url(url: &str, url_allowlist: &[String]) -> Result<GuardedUrl, ToolError> {
    if url.len() > MAX_REQUEST_URL_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "URL exceeds the {MAX_REQUEST_URL_BYTES}-byte limit"
        )));
    }
    validate_outbound_url_pinned(
        url,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: true,
            allowlist: url_allowlist.to_vec(),
        },
    )
}

async fn validate_url_async(url: &str, url_allowlist: &[String]) -> Result<GuardedUrl, ToolError> {
    if url.len() > MAX_REQUEST_URL_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "URL exceeds the {MAX_REQUEST_URL_BYTES}-byte limit"
        )));
    }
    validate_outbound_url_pinned_async(
        url,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: true,
            allowlist: url_allowlist.to_vec(),
        },
    )
    .await
}

/// Build a request-scoped client that pins the connection for `host` to the
/// exact socket addresses that passed SSRF validation, closing the
/// DNS-rebinding TOCTOU window. When `pinned_addrs` is empty (an IP-literal
/// host), the base client is returned unchanged.
///
/// reqwest only accepts a DNS override at client-build time, so a pinned client
/// is rebuilt with the same timeout and redirect policy as [`HttpTool::new`]
/// plus a `resolve_to_addrs` override for `host`. A client-build failure is
/// surfaced instead of falling back to a fresh DNS lookup.
fn pinned_client(
    base: &Client,
    host: &str,
    pinned_addrs: &[std::net::SocketAddr],
) -> Result<Client, ToolError> {
    if pinned_addrs.is_empty() {
        return Ok(base.clone());
    }
    // reqwest ignores the port carried by the override addresses and connects
    // using the port from the request URL, so passing the validated
    // SocketAddrs as-is is correct.
    match Client::builder()
        .timeout(Duration::from_secs(MAX_REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, pinned_addrs)
        .build()
    {
        Ok(client) => Ok(client),
        Err(error) => Err(ToolError::ExternalService(format!(
            "failed to create a DNS-pinned HTTP client: {error}"
        ))),
    }
}

fn redacted_url_for_log(url: &Url) -> String {
    let host = match url.host_str() {
        Some(host) if host.contains(':') => format!("[{host}]"),
        Some(host) => host.to_string(),
        None => return "<invalid-url>".to_string(),
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn sanitized_response_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name_text = name.as_str();
            if matches!(
                name_text,
                "set-cookie"
                    | "set-cookie2"
                    | "authorization"
                    | "proxy-authorization"
                    | "www-authenticate"
                    | "proxy-authenticate"
            ) {
                return None;
            }
            let value = value.to_str().ok()?;
            if name_text == "location" {
                let parsed = Url::parse(value).ok()?;
                return Some((name_text.to_string(), redacted_url_for_log(&parsed)));
            }
            Some((name_text.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(feature = "html-to-markdown")]
/// Heuristic: treat as HTML if the `Content-Type` header contains `text/html`.
fn is_html_response(headers: &HashMap<String, String>) -> bool {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_lowercase().contains("text/html"))
        .unwrap_or(false)
}

fn parse_headers_param(
    headers: Option<&serde_json::Value>,
) -> Result<Vec<(String, String)>, ToolError> {
    let parsed = match headers {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                let value = v.as_str().ok_or_else(|| {
                    ToolError::InvalidParameters(format!("header '{}' must have a string value", k))
                })?;
                out.push((k.clone(), value.to_string()));
            }
            Ok(out)
        }
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let obj = item.as_object().ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "headers[{}] must be an object with 'name' and 'value'",
                        idx
                    ))
                })?;
                let name = obj.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].name must be a string", idx))
                })?;
                let value = obj.get("value").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].value must be a string", idx))
                })?;
                out.push((name.to_string(), value.to_string()));
            }
            Ok(out)
        }
        Some(_) => Err(ToolError::InvalidParameters(
            "'headers' must be an object or an array of {name, value}".to_string(),
        )),
    }?;

    if parsed.len() > MAX_REQUEST_HEADERS {
        return Err(ToolError::InvalidParameters(format!(
            "request contains more than {MAX_REQUEST_HEADERS} headers"
        )));
    }
    let mut total_bytes = 0usize;
    let mut names = std::collections::HashSet::with_capacity(parsed.len());
    for (name, value) in &parsed {
        let normalized_name = name.to_ascii_lowercase();
        total_bytes = total_bytes
            .checked_add(name.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or_else(|| {
                ToolError::InvalidParameters("request header size overflow".to_string())
            })?;
        if name.is_empty()
            || name.len() > MAX_REQUEST_HEADER_NAME_BYTES
            || value.len() > MAX_REQUEST_HEADER_VALUE_BYTES
            || HeaderName::from_bytes(name.as_bytes()).is_err()
            || HeaderValue::from_str(value).is_err()
            || matches!(
                normalized_name.as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-connection"
                    | "upgrade"
                    | "te"
                    | "trailer"
            )
        {
            return Err(ToolError::InvalidParameters(
                "request contains an invalid or oversized header".to_string(),
            ));
        }
        if !names.insert(normalized_name) {
            return Err(ToolError::InvalidParameters(format!(
                "request contains duplicate header '{name}'"
            )));
        }
    }
    if total_bytes > MAX_REQUEST_HEADER_TOTAL_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "request headers exceed the {MAX_REQUEST_HEADER_TOTAL_BYTES}-byte limit"
        )));
    }

    Ok(parsed)
}

#[derive(Clone)]
struct RequestBody {
    bytes: Vec<u8>,
    is_json: bool,
}

fn parse_request_body(body: Option<&serde_json::Value>) -> Result<Option<RequestBody>, ToolError> {
    let Some(body) = body else {
        return Ok(None);
    };
    let (bytes, is_json) = if let Some(body_str) = body.as_str() {
        if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
            (
                serde_json::to_vec(&json_body).map_err(|error| {
                    ToolError::InvalidParameters(format!("invalid body JSON: {error}"))
                })?,
                true,
            )
        } else {
            (body_str.as_bytes().to_vec(), false)
        }
    } else {
        (
            serde_json::to_vec(body).map_err(|error| {
                ToolError::InvalidParameters(format!("invalid body JSON: {error}"))
            })?,
            true,
        )
    };
    if bytes.len() > MAX_REQUEST_BODY_SIZE {
        return Err(ToolError::InvalidParameters(format!(
            "request body exceeds the {MAX_REQUEST_BODY_SIZE}-byte limit"
        )));
    }
    Ok(Some(RequestBody { bytes, is_json }))
}

fn parse_request_timeout(params: &serde_json::Value) -> Result<Duration, ToolError> {
    let seconds = match params.get("timeout_secs") {
        None => DEFAULT_REQUEST_TIMEOUT_SECS,
        Some(value) => value.as_u64().ok_or_else(|| {
            ToolError::InvalidParameters("timeout_secs must be a positive integer".to_string())
        })?,
    };
    if !(1..=MAX_REQUEST_TIMEOUT_SECS).contains(&seconds) {
        return Err(ToolError::InvalidParameters(format!(
            "timeout_secs must be between 1 and {MAX_REQUEST_TIMEOUT_SECS}"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

fn request_origin(url: &Url) -> (&str, &str, Option<u16>) {
    (
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default(),
    )
}

fn redirect_target(
    response_url: &Url,
    response: &reqwest::Response,
) -> Result<Option<Url>, ToolError> {
    if !matches!(
        response.status(),
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    ) {
        return Ok(None);
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| {
            ToolError::ExternalService("redirect omitted its Location header".to_string())
        })?
        .to_str()
        .map_err(|_| {
            ToolError::ExternalService("redirect Location is not valid text".to_string())
        })?;
    response_url
        .join(location)
        .map(Some)
        .map_err(|_| ToolError::ExternalService("redirect Location is not a valid URL".to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CredentialDestination {
    Header(String),
    Query(String),
}

fn credential_destination(
    location: &thinclaw_secrets::CredentialLocation,
) -> Option<CredentialDestination> {
    match location {
        thinclaw_secrets::CredentialLocation::AuthorizationBearer
        | thinclaw_secrets::CredentialLocation::AuthorizationBasic { .. } => {
            Some(CredentialDestination::Header("authorization".to_string()))
        }
        thinclaw_secrets::CredentialLocation::Header { name, .. } => {
            Some(CredentialDestination::Header(name.to_ascii_lowercase()))
        }
        thinclaw_secrets::CredentialLocation::QueryParam { name } => {
            Some(CredentialDestination::Query(name.clone()))
        }
        thinclaw_secrets::CredentialLocation::UrlPath { .. }
        | thinclaw_secrets::CredentialLocation::UrlBase { .. }
        | thinclaw_secrets::CredentialLocation::Body { .. } => None,
    }
}

fn validate_automatic_credential_destinations(
    mappings: &[thinclaw_secrets::CredentialMapping],
    headers: &[(String, String)],
    url: &Url,
) -> Result<(), ToolError> {
    if mappings.len() > MAX_AUTOMATIC_CREDENTIALS {
        return Err(ToolError::NotAuthorized(format!(
            "more than {MAX_AUTOMATIC_CREDENTIALS} automatic credentials match this host"
        )));
    }
    let mut destinations = std::collections::HashSet::new();
    for mapping in mappings {
        let Some(destination) = credential_destination(&mapping.location) else {
            continue;
        };
        let destination_is_valid = match &destination {
            CredentialDestination::Header(name) => {
                !name.is_empty()
                    && name.len() <= MAX_REQUEST_HEADER_NAME_BYTES
                    && HeaderName::from_bytes(name.as_bytes()).is_ok()
                    && !headers
                        .iter()
                        .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
            }
            CredentialDestination::Query(name) => {
                !name.is_empty()
                    && name.len() <= MAX_REQUEST_HEADER_NAME_BYTES
                    && !name.chars().any(char::is_control)
                    && !url
                        .query_pairs()
                        .any(|(existing, _)| existing == name.as_str())
            }
        };
        if !destination_is_valid || !destinations.insert(destination.clone()) {
            let name = match destination {
                CredentialDestination::Header(name) | CredentialDestination::Query(name) => name,
            };
            return Err(ToolError::NotAuthorized(format!(
                "automatic credential destination '{name}' is invalid, conflicting, or ambiguous"
            )));
        }
    }
    Ok(())
}

/// Extract host from URL in params (for approval checks).
fn extract_host_from_params(params: &serde_json::Value) -> Option<String> {
    params
        .get("url")
        .and_then(|u| u.as_str())
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

impl Default for HttpTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str {
        "http"
    }

    fn description(&self) -> &str {
        "Make raw HTTP requests to external URLs and APIs. Use this when you need a \
         network call and there is no more specific built-in or extension tool for the service."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "maxLength": MAX_REQUEST_URL_BYTES,
                    "description": "The URL to request"
                },
                "headers": {
                    "type": "array",
                    "maxItems": MAX_REQUEST_HEADERS,
                    "description": "Optional headers as a list of {name, value} objects",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "maxLength": MAX_REQUEST_HEADER_NAME_BYTES },
                            "value": { "type": "string", "maxLength": MAX_REQUEST_HEADER_VALUE_BYTES }
                        },
                        "required": ["name", "value"],
                        "additionalProperties": false
                    }
                },
                "body": {
                    "description": "Request body (for POST/PUT/PATCH). Can be a JSON object, array, string, or other value."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_REQUEST_TIMEOUT_SECS,
                    "description": "Total request timeout in seconds, including redirects (default: 30)"
                }
            },
            "required": ["method", "url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let base_client = self.client.as_ref().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "guarded HTTP client is unavailable: {}",
                self.client_init_error
                    .as_deref()
                    .unwrap_or("initialization failed")
            ))
        })?;
        let method_name = require_str(&params, "method")?.to_ascii_uppercase();
        let mut method = match method_name.as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported method: {method_name}"
                )));
            }
        };

        let mut headers_vec = parse_headers_param(params.get("headers"))?;
        let mut request_body = parse_request_body(params.get("body"))?;
        let request_timeout = parse_request_timeout(&params)?;
        let deadline = tokio::time::Instant::now() + request_timeout;
        let detector = LeakDetector::new();

        let mut guarded = tokio::time::timeout_at(
            deadline,
            validate_url_async(require_str(&params, "url")?, &self.url_allowlist),
        )
        .await
        .map_err(|_| ToolError::Timeout(request_timeout))??;
        let initial_url = guarded.url.clone();
        let mut redirect_count = 0usize;
        let (response, parsed_url) = loop {
            let GuardedUrl {
                url: validated_url,
                pinned_addrs,
            } = guarded;

            // Only same-origin redirects are followed. This prevents caller
            // headers and host-injected credentials from crossing an origin
            // boundary while still supporting ordinary path redirects.
            if request_origin(&validated_url) != request_origin(&initial_url) {
                return Err(ToolError::NotAuthorized(
                    "cross-origin HTTP redirects are not followed".to_string(),
                ));
            }

            detector
                .scan_http_request(
                    validated_url.as_str(),
                    &headers_vec,
                    request_body.as_ref().map(|body| body.bytes.as_slice()),
                )
                .map_err(|error| ToolError::NotAuthorized(error.to_string()))?;

            let host = validated_url.host_str().unwrap_or_default().to_string();
            let path = validated_url.path().to_string();
            let client = pinned_client(base_client, &host, &pinned_addrs)?;
            let mut request_url = validated_url.clone();
            let mut request_headers = headers_vec.clone();

            // Resolve every destination before loading any secret. Ambiguous or
            // caller-controlled destinations fail without partial secret access.
            if let (Some(registry), Some(store)) = (
                self.credential_registry.as_ref(),
                self.secrets_store.as_ref(),
            ) {
                let matched = registry.find_for_host(&host);
                validate_automatic_credential_destinations(
                    &matched,
                    &request_headers,
                    &request_url,
                )?;
                for mapping in &matched {
                    if credential_destination(&mapping.location).is_none() {
                        continue;
                    }
                    let secret = store
                        .get_for_injection(
                            &_ctx.user_id,
                            &mapping.secret_name,
                            thinclaw_secrets::SecretAccessContext::new(
                                "builtin.http",
                                "http_credential_injection",
                            )
                            .target(host.clone(), path.clone()),
                        )
                        .await
                        .map_err(|_| {
                            ToolError::NotAuthorized(
                                "a required automatic credential could not be loaded".to_string(),
                            )
                        })?;
                    let mut injected = InjectedCredentials::empty();
                    inject_credential(&mut injected, &mapping.location, &secret);
                    request_headers.extend(injected.headers);
                    for (name, value) in injected.query_params {
                        request_url.query_pairs_mut().append_pair(&name, &value);
                    }
                }
            }

            if request_url.as_str().len() > MAX_REQUEST_URL_BYTES
                || request_headers.len() > MAX_REQUEST_HEADERS + MAX_AUTOMATIC_CREDENTIALS
                || request_headers.iter().any(|(name, value)| {
                    HeaderName::from_bytes(name.as_bytes()).is_err()
                        || HeaderValue::from_str(value).is_err()
                        || value.len() > MAX_REQUEST_HEADER_VALUE_BYTES
                })
            {
                return Err(ToolError::NotAuthorized(
                    "request is invalid or oversized after credential injection".to_string(),
                ));
            }

            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or(ToolError::Timeout(request_timeout))?;
            let mut request = client
                .request(method.clone(), request_url)
                .timeout(remaining);
            for (name, value) in &request_headers {
                request = request.header(name, value);
            }
            if let Some(body) = request_body.as_ref() {
                if body.is_json
                    && !request_headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case(CONTENT_TYPE.as_str()))
                {
                    request = request.header(CONTENT_TYPE, "application/json");
                }
                request = request.body(body.bytes.clone());
            }

            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    ToolError::Timeout(request_timeout)
                } else {
                    ToolError::ExternalService(error.without_url().to_string())
                }
            })?;

            let Some(target) = redirect_target(&validated_url, &response)? else {
                break (response, validated_url);
            };
            if redirect_count >= MAX_REDIRECTS {
                return Err(ToolError::ExternalService(format!(
                    "HTTP request exceeded {MAX_REDIRECTS} redirects"
                )));
            }
            if request_origin(&target) != request_origin(&initial_url) {
                return Err(ToolError::NotAuthorized(
                    "cross-origin HTTP redirects are not followed".to_string(),
                ));
            }

            let status = response.status();
            if status == StatusCode::SEE_OTHER
                || matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
                    && method == Method::POST
            {
                method = Method::GET;
                request_body = None;
                headers_vec.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
                        && !name.eq_ignore_ascii_case(CONTENT_TYPE.as_str())
                });
            }
            guarded = tokio::time::timeout_at(
                deadline,
                validate_url_async(target.as_str(), &self.url_allowlist),
            )
            .await
            .map_err(|_| ToolError::Timeout(request_timeout))??;
            redirect_count += 1;
        };

        let status = response.status().as_u16();

        // Never place session cookies or authentication challenges into the
        // model-visible tool result. Redirect locations are reduced to their
        // origin so signed query strings and path tokens cannot escape either.
        let headers = sanitized_response_headers(response.headers());

        // Pre-check Content-Length header to reject obviously oversized responses
        // before downloading anything, preventing OOM from malicious servers.
        if let Some(content_length) = response.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = content_length.to_str()
            && let Ok(len) = s.parse::<usize>()
            && len > MAX_RESPONSE_SIZE
        {
            tracing::warn!(
                url = %redacted_url_for_log(&parsed_url),
                content_length = len,
                max = MAX_RESPONSE_SIZE,
                "Rejected HTTP response: Content-Length exceeds limit"
            );
            return Err(ToolError::ExecutionFailed(format!(
                "Response Content-Length ({} bytes) exceeds maximum allowed size ({} bytes)",
                len, MAX_RESPONSE_SIZE
            )));
        }

        // Stream the response body with a hard size cap. Even if Content-Length was
        // absent or lied about the size, we stop reading once we exceed the limit.
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| {
                ToolError::ExternalService(format!(
                    "failed to read response body: {}",
                    e.without_url()
                ))
            })?;
            if body.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Response body exceeds maximum allowed size ({} bytes)",
                    MAX_RESPONSE_SIZE
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let body_bytes = bytes::Bytes::from(body);

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        #[cfg(feature = "html-to-markdown")]
        let body_text = if is_html_response(&headers) {
            match convert_html_to_markdown(&body_text, parsed_url.as_str()) {
                Ok(md) => md,
                Err(e) => {
                    tracing::warn!(url = %redacted_url_for_log(&parsed_url), error = %e, "HTML-to-markdown conversion failed, returning raw HTML");
                    body_text
                }
            }
        } else {
            body_text
        };

        // Try to parse as JSON, fall back to string
        let body: serde_json::Value = serde_json::from_str(&body_text)
            .unwrap_or_else(|_| serde_json::Value::String(body_text.clone()));

        let result = serde_json::json!({
            "status": status,
            "headers": headers,
            "body": body
        });

        Ok(ToolOutput::success(result, start.elapsed()).with_raw(body_text))
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(5)) // Average HTTP request time
    }

    fn requires_sanitization(&self) -> bool {
        true // External data always needs sanitization
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        // 1. Manual auth headers/query params in LLM params
        if thinclaw_safety::params_contain_manual_credentials(params) {
            return ApprovalRequirement::Always;
        }
        // 2. Target host has credential mappings (will be auto-injected)
        if let Some(ref registry) = self.credential_registry
            && let Some(host) = extract_host_from_params(params)
            && registry.has_credentials_for_host(&host)
        {
            return ApprovalRequirement::Always;
        }
        // Default: outbound HTTP still needs approval unless auto-approved
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(30, 500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_tool_schema_headers_is_array() {
        let tool = HttpTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["properties"]["headers"]["type"], "array");
    }

    #[test]
    fn test_validate_url_upgrades_http_to_https() {
        // validate_url silently upgrades http:// → https:// since the LLM
        // frequently generates http:// URLs even for HTTPS-capable sites.
        let url = validate_url("http://8.8.8.8", &[]).unwrap().url;
        assert_eq!(url.scheme(), "https");
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        let err = validate_url("https://localhost:8080", &[]).unwrap_err();
        assert!(err.to_string().contains("localhost"));
    }

    #[test]
    fn test_validate_url_accepts_https_public() {
        let url = validate_url("https://8.8.8.8", &[]).unwrap().url;
        assert_eq!(url.host_str(), Some("8.8.8.8"));
    }

    #[test]
    fn test_validate_url_rejects_private_ip_literal() {
        let err = validate_url("https://192.168.1.1/api", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_validate_url_rejects_loopback_ip() {
        let err = validate_url("https://127.0.0.1/api", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_validate_url_rejects_link_local() {
        let err = validate_url("https://169.254.169.254/latest/meta-data/", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_is_disallowed_ip_covers_ranges() {
        // Private ranges
        assert!(validate_url("https://10.0.0.1/test", &[]).is_err());
        assert!(validate_url("https://172.16.0.1/test", &[]).is_err());
        assert!(validate_url("https://192.168.0.1/test", &[]).is_err());
        // Loopback
        assert!(validate_url("https://127.0.0.1/test", &[]).is_err());
        // Cloud metadata
        assert!(validate_url("https://169.254.169.254/latest/meta-data/", &[]).is_err());
        // Public
        assert!(validate_url("https://8.8.8.8/test", &[]).is_ok());
    }

    #[test]
    fn test_max_response_size_is_reasonable() {
        // MAX_RESPONSE_SIZE should be 5 MB to prevent OOM while allowing typical API responses.
        assert_eq!(MAX_RESPONSE_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn response_headers_hide_credentials_and_signed_locations() {
        let mut headers = HeaderMap::new();
        headers.insert("set-cookie", HeaderValue::from_static("session=top-secret"));
        headers.insert(
            "www-authenticate",
            HeaderValue::from_static("Bearer error=\"token-secret\""),
        );
        headers.insert(
            "location",
            HeaderValue::from_static("https://user:pass@example.com/private?token=secret#part"),
        );
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        let sanitized = sanitized_response_headers(&headers);
        assert!(!sanitized.contains_key("set-cookie"));
        assert!(!sanitized.contains_key("www-authenticate"));
        assert_eq!(
            sanitized.get("location").map(String::as_str),
            Some("https://example.com")
        );
        assert_eq!(
            sanitized.get("content-type").map(String::as_str),
            Some("application/json")
        );
        let serialized = format!("{sanitized:?}");
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("token-secret"));
        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn log_url_is_reduced_to_origin() {
        let url = Url::parse(
            "https://user:password@example.com:8443/private/token?api_key=secret#fragment",
        )
        .unwrap();
        assert_eq!(redacted_url_for_log(&url), "https://example.com:8443");
    }

    #[test]
    fn test_parse_headers_param_accepts_object_legacy_shape() {
        let headers = serde_json::json!({"Authorization": "Bearer token"});
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![("Authorization".to_string(), "Bearer token".to_string())]
        );
    }

    #[test]
    fn test_parse_headers_param_accepts_array_shape() {
        let headers = serde_json::json!([
            {"name": "Authorization", "value": "Bearer token"},
            {"name": "X-Test", "value": "1"}
        ]);
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![
                ("Authorization".to_string(), "Bearer token".to_string()),
                ("X-Test".to_string(), "1".to_string())
            ]
        );
    }

    #[test]
    fn test_http_tool_schema_body_is_freeform() {
        let schema = HttpTool::new().parameters_schema();
        let body = schema
            .get("properties")
            .and_then(|p| p.get("body"))
            .expect("body schema missing");

        // Body is intentionally freeform (no "type" constraint) for OpenAI
        // compatibility. OpenAI rejects union types containing "array" unless
        // "items" is also specified, and body accepts any JSON value.
        assert!(
            body.get("type").is_none(),
            "body schema should not have a 'type' to be freeform for OpenAI compatibility"
        );
    }

    // ── Approval requirement tests ──────────────────────────────────────

    #[test]
    fn test_no_auth_headers_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_auth_header_object_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_array_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": [{"name": "Authorization", "value": "Bearer token123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_case_insensitive() {
        let tool = HttpTool::new();

        // Object format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer x"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);

        // Array format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Api-Key", "value": "key123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_all_auth_header_names_detected() {
        let tool = HttpTool::new();
        for header_name in [
            "authorization",
            "x-api-key",
            "cookie",
            "proxy-authorization",
            "x-auth-token",
            "api-key",
            "x-token",
            "x-access-token",
            "x-session-token",
            "x-csrf-token",
            "x-secret",
            "x-api-secret",
        ] {
            let mut headers = serde_json::Map::new();
            headers.insert(header_name.to_string(), serde_json::json!("value"));
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": headers
            });
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "Header '{}' should trigger Always approval",
                header_name
            );
        }
    }

    #[test]
    fn test_non_auth_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_empty_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();

        // Empty object
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );

        // Empty array
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": []
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    // ── Credential registry approval tests ─────────────────────────────

    #[test]
    fn test_host_with_credential_mapping_returns_always() {
        use crate::wasm::SharedCredentialRegistry;
        use thinclaw_secrets::CredentialMapping;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry
            .add_mappings(vec![CredentialMapping::bearer(
                "openai_key",
                "api.openai.com",
            )])
            .unwrap();

        let tool = HttpTool::new().with_credentials(
            registry,
            // secrets_store is not used in requires_approval, just needs to be present
            Arc::new(thinclaw_secrets::InMemorySecretsStore::new(Arc::new(
                thinclaw_secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.openai.com/v1/models"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_host_without_credential_mapping_returns_unless_auto_approved() {
        use crate::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        // Empty registry - no credential mappings

        let tool = HttpTool::new().with_credentials(
            registry,
            Arc::new(thinclaw_secrets::InMemorySecretsStore::new(Arc::new(
                thinclaw_secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_url_query_param_credential_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=secret123"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_bearer_value_in_custom_header_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Bearer sk-test123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_extract_host_from_params_valid() {
        let params = serde_json::json!({
            "url": "https://api.example.com/path"
        });
        assert_eq!(
            extract_host_from_params(&params),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_from_params_missing_url() {
        let params = serde_json::json!({"method": "GET"});
        assert_eq!(extract_host_from_params(&params), None);
    }

    // ── URL Allowlist tests ────────────────────────────────────────────

    #[test]
    fn test_allowlist_blocks_non_listed_host() {
        let allowlist = vec!["api.openai.com".to_string()];
        let err = validate_url("https://evil.com/data", &allowlist).unwrap_err();
        assert!(err.to_string().contains("not in the URL allowlist"));
    }

    #[test]
    fn test_allowlist_allows_listed_host() {
        let allowlist = vec!["8.8.8.8".to_string()];
        let url = validate_url("https://8.8.8.8/v1/models", &allowlist)
            .unwrap()
            .url;
        assert_eq!(url.host_str(), Some("8.8.8.8"));
    }

    #[test]
    fn test_allowlist_glob_matches_subdomain() {
        let allowlist = vec!["*.example.com".to_string()];
        let subdomain = validate_url("https://api.example.com/path", &allowlist);
        assert!(
            !subdomain
                .as_ref()
                .is_err_and(|error| error.to_string().contains("not in the URL allowlist")),
            "glob should accept a matching subdomain before optional DNS resolution"
        );

        // Root domain also matches
        let root = validate_url("https://example.com/path", &allowlist);
        assert!(
            !root
                .as_ref()
                .is_err_and(|error| error.to_string().contains("not in the URL allowlist")),
            "glob should accept its root domain before optional DNS resolution"
        );
    }

    #[test]
    fn test_allowlist_glob_rejects_different_domain() {
        let allowlist = vec!["*.openai.com".to_string()];
        let err = validate_url("https://evil-openai.com/phish", &allowlist).unwrap_err();
        assert!(err.to_string().contains("not in the URL allowlist"));
    }

    #[test]
    fn test_empty_allowlist_allows_all() {
        let url = validate_url("https://8.8.8.8/path", &[]).unwrap().url;
        assert_eq!(url.host_str(), Some("8.8.8.8"));
    }

    // ── IPv4-mapped IPv6 bypass blocking tests ─────────────────────────

    #[test]
    fn test_ipv4_mapped_v6_loopback_blocked() {
        // ::ffff:127.0.0.1 — IPv4-mapped loopback
        assert!(
            validate_url("https://[::ffff:127.0.0.1]/data", &[]).is_err(),
            "IPv4-mapped loopback should be blocked"
        );
    }

    #[test]
    fn test_ipv4_mapped_v6_private_blocked() {
        // ::ffff:192.168.1.1 — IPv4-mapped private
        assert!(
            validate_url("https://[::ffff:192.168.1.1]/data", &[]).is_err(),
            "IPv4-mapped private should be blocked"
        );
    }

    #[test]
    fn test_ipv4_mapped_v6_public_allowed() {
        // ::ffff:8.8.8.8 — IPv4-mapped public
        assert!(
            validate_url("https://[::ffff:8.8.8.8]/data", &[]).is_ok(),
            "IPv4-mapped public should be allowed"
        );
    }

    // ── DNS-rebinding pin tests ─────────────────────────────────────────

    #[test]
    fn test_validate_url_exposes_pinned_addrs_field() {
        // A hostname (non-literal) carries pinned_addrs from validation so the
        // request client can be pinned to the validated IPs. We can't assert a
        // specific resolved set without network access, but the field must be
        // present and, when populated, every address must have passed the
        // disallowed-IP check (resolution to a private IP would have errored).
        let guarded = validate_url("https://example.com/path", &[]);
        if let Ok(guarded) = guarded {
            // Hostname resolution may or may not be available in CI; either way
            // a non-literal host never produces a literal-only empty-by-design
            // result, and any pinned addr is a real validated SocketAddr.
            for addr in &guarded.pinned_addrs {
                assert!(addr.port() == 443 || addr.port() == 80);
            }
        }
    }

    #[test]
    fn test_validate_url_ip_literal_has_empty_pin() {
        // IP literals are validated directly; reqwest connects to the literal
        // so there is nothing to pin.
        let guarded = validate_url("https://8.8.8.8/", &[]).unwrap();
        assert!(
            guarded.pinned_addrs.is_empty(),
            "IP-literal hosts must not be pinned"
        );
    }

    #[test]
    fn test_pinned_client_passthrough_when_no_addrs() {
        // With no pinned addresses (IP-literal path), pinned_client returns a
        // usable client cloned from the base. This exercises the empty branch
        // and confirms the consumer of the pin field compiles and runs.
        let base = HttpTool::new().client;
        let _client = pinned_client(base.as_ref().unwrap(), "8.8.8.8", &[]).unwrap();
    }

    #[test]
    fn test_pinned_client_builds_with_addrs() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        // A non-empty pin produces a request-scoped client carrying the DNS
        // override. We only assert it builds successfully; the actual override
        // is exercised at connect time.
        let base = HttpTool::new().client;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443);
        let _client = pinned_client(base.as_ref().unwrap(), "example.com", &[addr]).unwrap();
    }

    #[test]
    fn request_headers_reject_duplicates_and_transport_overrides() {
        let duplicate = serde_json::json!([
            {"name": "Authorization", "value": "one"},
            {"name": "authorization", "value": "two"}
        ]);
        assert!(parse_headers_param(Some(&duplicate)).is_err());

        let host = serde_json::json!([{"name": "Host", "value": "internal.example"}]);
        assert!(parse_headers_param(Some(&host)).is_err());

        let transfer_encoding =
            serde_json::json!([{"name": "Transfer-Encoding", "value": "chunked"}]);
        assert!(parse_headers_param(Some(&transfer_encoding)).is_err());
    }

    #[test]
    fn request_timeout_is_bounded_and_honored_by_parser() {
        assert_eq!(
            parse_request_timeout(&serde_json::json!({})).unwrap(),
            Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS)
        );
        assert_eq!(
            parse_request_timeout(&serde_json::json!({"timeout_secs": 7})).unwrap(),
            Duration::from_secs(7)
        );
        assert!(parse_request_timeout(&serde_json::json!({"timeout_secs": 0})).is_err());
        assert!(
            parse_request_timeout(
                &serde_json::json!({"timeout_secs": MAX_REQUEST_TIMEOUT_SECS + 1})
            )
            .is_err()
        );
    }

    #[test]
    fn automatic_credentials_reject_ambiguous_or_manual_destinations() {
        use thinclaw_secrets::CredentialMapping;

        let url = Url::parse("https://8.8.8.8/resource").unwrap();
        let duplicate = vec![
            CredentialMapping::bearer("first", "8.8.8.8"),
            CredentialMapping::bearer("second", "8.8.8.8"),
        ];
        assert!(validate_automatic_credential_destinations(&duplicate, &[], &url).is_err());

        let manual = vec![("Authorization".to_string(), "manual".to_string())];
        assert!(
            validate_automatic_credential_destinations(&duplicate[..1], &manual, &url).is_err()
        );
    }

    #[tokio::test]
    async fn automatic_credential_lookup_fails_before_network_send() {
        use crate::wasm::SharedCredentialRegistry;
        use thinclaw_secrets::CredentialMapping;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry
            .add_mappings([CredentialMapping::bearer("missing", "8.8.8.8")])
            .unwrap();
        let store = Arc::new(thinclaw_secrets::InMemorySecretsStore::new(Arc::new(
            thinclaw_secrets::SecretsCrypto::new(secrecy::SecretString::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .unwrap(),
        )));
        let tool = HttpTool::new().with_credentials(registry, store);
        let error = tool
            .execute(
                serde_json::json!({
                    "method": "GET",
                    "url": "https://8.8.8.8/",
                    "timeout_secs": 1
                }),
                &JobContext::with_user("user", "test", "http test"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::NotAuthorized(_)));
        assert!(error.to_string().contains("required automatic credential"));
    }

    #[test]
    fn redirect_origin_comparison_includes_scheme_host_and_port() {
        let origin = Url::parse("https://example.com/a").unwrap();
        let same = Url::parse("https://example.com/b").unwrap();
        let other_host = Url::parse("https://www.example.com/b").unwrap();
        let other_port = Url::parse("https://example.com:8443/b").unwrap();
        assert_eq!(request_origin(&origin), request_origin(&same));
        assert_ne!(request_origin(&origin), request_origin(&other_host));
        assert_ne!(request_origin(&origin), request_origin(&other_port));
    }

    #[test]
    fn test_ipv4_mapped_v6_metadata_blocked() {
        // ::ffff:169.254.169.254 — IPv4-mapped cloud metadata
        assert!(
            validate_url("https://[::ffff:169.254.169.254]/data", &[]).is_err(),
            "IPv4-mapped cloud metadata should be blocked"
        );
    }
}

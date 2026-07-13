//! Core proxy type, connection state, construction, low-level HTTP request
//! primitives, and health checks.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, RETRY_AFTER},
    redirect::Policy,
    Method, Url,
};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;
use zeroize::Zeroize;

const MAX_JSON_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 4 * 1024;
const MAX_IDEMPOTENT_ATTEMPTS: u32 = 3;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Connection state for health monitoring
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}

/// HTTP/SSE proxy client for a remote ThinClaw gateway.
///
/// Cheaply cloneable — all state behind Arc.
#[derive(Clone)]
pub struct RemoteGatewayProxy {
    pub(super) inner: Arc<RemoteGatewayProxyInner>,
}

pub(super) struct RemoteGatewayProxyInner {
    /// Base URL of the remote gateway, e.g. "http://192.168.1.50:18789"
    pub(super) base_url: String,
    /// Sensitive Authorization header. `HeaderValue` redacts itself in Debug output.
    pub(super) auth_header: HeaderValue,
    /// Shared reqwest client (connection pool)
    pub(super) client: reqwest::Client,
    /// SSE subscription task handle (if started)
    pub(super) sse_handle: RwLock<Option<JoinHandle<()>>>,
    /// Current connection state
    pub(super) state: RwLock<ConnectionState>,
}

pub(super) fn remote_thread_id(session_key: &str) -> Option<String> {
    if session_key == "agent:main" || session_key.trim().is_empty() {
        None
    } else {
        Some(session_key.to_string())
    }
}

pub(super) fn required_remote_thread_id(
    session_key: &str,
    capability: &str,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    remote_thread_id(session_key).ok_or_else(|| {
        RemoteGatewayProxy::unavailable(
            capability,
            "the pinned assistant thread must be addressed through a concrete remote thread id",
        )
    })
}

impl RemoteGatewayProxy {
    /// Create a new proxy. Does NOT connect — call `health_check` or
    /// `start_sse_subscription` to establish the connection.
    pub fn new(
        base_url: &str,
        auth_token: &str,
    ) -> Result<Self, crate::thinclaw::bridge::BridgeError> {
        let base_url = validate_base_url(base_url)?;
        let token = auth_token.trim();
        if token.is_empty() {
            return Err(("Remote gateway token is required".to_string()).into());
        }

        let mut bearer = format!("Bearer {token}");
        let mut auth_header = HeaderValue::from_str(&bearer)
            .map_err(|_| "Remote gateway token contains invalid characters".to_string())?;
        auth_header.set_sensitive(true);
        bearer.zeroize();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            // Never forward a bearer credential to a redirect target.
            .redirect(Policy::none())
            .user_agent("ThinClawDesktop/0.14 (ThinClaw remote proxy)")
            .build()
            .map_err(|error| format!("Failed to initialize remote gateway client: {error}"))?;

        Ok(Self {
            inner: Arc::new(RemoteGatewayProxyInner {
                base_url,
                auth_header,
                client,
                sse_handle: RwLock::new(None),
                state: RwLock::new(ConnectionState::Disconnected),
            }),
        })
    }

    /// Base URL accessor.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    pub(super) fn url(&self, path: &str) -> String {
        format!("{}{}", self.inner.base_url, path)
    }

    pub(super) fn auth_header(&self) -> HeaderValue {
        self.inner.auth_header.clone()
    }

    pub fn unavailable(
        capability: &str,
        reason: impl AsRef<str>,
    ) -> crate::thinclaw::bridge::BridgeError {
        crate::thinclaw::bridge::gated(
            capability,
            format!(
                "remote ThinClaw gateway does not support this capability: {}",
                reason.as_ref()
            ),
            "switch to embedded mode or upgrade the remote gateway",
            crate::thinclaw::bridge::RouteMode::LocalOnly,
        )
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        headers: HeaderMap,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] {} {}", method, url);
        let max_attempts = if method == Method::GET {
            MAX_IDEMPOTENT_ATTEMPTS
        } else {
            1
        };

        for attempt in 1..=max_attempts {
            let mut req = self
                .inner
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, self.auth_header());
            if !headers.is_empty() {
                req = req.headers(headers.clone());
            }
            if let Some(body) = body {
                req = req.json(body);
            }

            let resp = match req.send().await {
                Ok(response) => response,
                Err(error) => {
                    let error = remote_transport_error(&method.to_string(), &url, error);
                    if attempt < max_attempts && bridge_error_is_retryable(&error) {
                        sleep_before_retry(path, attempt, None).await;
                        continue;
                    }
                    return Err(error);
                }
            };
            let status = resp.status();
            let retry_after = retry_after_delay(resp.headers());
            let max_bytes = if status.is_success() {
                MAX_JSON_RESPONSE_BYTES
            } else {
                MAX_ERROR_RESPONSE_BYTES
            };
            let response_body = match read_bounded_body(resp, max_bytes).await {
                Ok(body) => body,
                Err(error) => {
                    if attempt < max_attempts && bridge_error_is_retryable(&error) {
                        sleep_before_retry(path, attempt, retry_after).await;
                        continue;
                    }
                    return Err(error);
                }
            };

            if !status.is_success() {
                let error = remote_http_error(status, &response_body, path);
                if attempt < max_attempts && bridge_error_is_retryable(&error) {
                    sleep_before_retry(path, attempt, retry_after).await;
                    continue;
                }
                return Err(error);
            }

            if response_body.is_empty() {
                return Ok(serde_json::json!({ "ok": true }));
            }

            return serde_json::from_slice(&response_body).map_err(|error| {
                crate::thinclaw::bridge::BridgeError::Runtime {
                    message: format!("Failed to parse JSON response from {url}: {error}"),
                }
            });
        }

        unreachable!("request attempt loop always returns")
    }

    pub async fn get_json(
        &self,
        path: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.request_json(Method::GET, path, None, HeaderMap::new())
            .await
    }

    pub(super) async fn get_text(
        &self,
        path: &str,
    ) -> Result<String, crate::thinclaw::bridge::BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] GET {}", url);

        for attempt in 1..=MAX_IDEMPOTENT_ATTEMPTS {
            let resp = match self
                .inner
                .client
                .get(&url)
                .header(AUTHORIZATION, self.auth_header())
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = remote_transport_error("GET", &url, error);
                    if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                        sleep_before_retry(path, attempt, None).await;
                        continue;
                    }
                    return Err(error);
                }
            };

            let status = resp.status();
            let retry_after = retry_after_delay(resp.headers());
            let max_bytes = if status.is_success() {
                MAX_TEXT_RESPONSE_BYTES
            } else {
                MAX_ERROR_RESPONSE_BYTES
            };
            let body = match read_bounded_body(resp, max_bytes).await {
                Ok(body) => body,
                Err(error) => {
                    if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                        sleep_before_retry(path, attempt, retry_after).await;
                        continue;
                    }
                    return Err(error);
                }
            };
            if status.is_success() {
                return Ok(String::from_utf8_lossy(&body).into_owned());
            }

            let error = remote_http_error(status, &body, path);
            if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                sleep_before_retry(path, attempt, retry_after).await;
                continue;
            }
            return Err(error);
        }

        unreachable!("request attempt loop always returns")
    }

    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.request_json(Method::POST, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn post_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::POST, path, Some(body), headers)
            .await
    }

    pub async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.request_json(Method::PUT, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn put_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::PUT, path, Some(body), headers)
            .await
    }

    #[allow(dead_code)]
    async fn put_text(
        &self,
        path: &str,
        content: &str,
    ) -> Result<(), crate::thinclaw::bridge::BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] PUT {}", url);

        let resp = self
            .inner
            .client
            .put(&url)
            .header(AUTHORIZATION, self.auth_header())
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(content.to_string())
            .send()
            .await
            .map_err(|error| remote_transport_error("PUT", &url, error))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = read_bounded_body(resp, MAX_ERROR_RESPONSE_BYTES).await?;
            return Err(remote_http_error(status, &body, path));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_json(
        &self,
        path: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.request_json(Method::DELETE, path, None, HeaderMap::new())
            .await
    }

    pub async fn delete_json_body(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.request_json(Method::DELETE, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn delete_json_confirm(
        &self,
        path: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::DELETE, path, None, headers).await
    }

    // ── Health ───────────────────────────────────────────────────────────────

    /// Test connectivity to the remote gateway.
    ///
    /// Returns Ok(true) if the server accepts this credential on an authenticated endpoint.
    /// Returns Ok(false) if the server is reachable but auth failed.
    /// Returns Err if connection could not be established.
    pub async fn health_check(&self) -> Result<bool, crate::thinclaw::bridge::BridgeError> {
        let url = self.url("/api/gateway/status");
        for attempt in 1..=MAX_IDEMPOTENT_ATTEMPTS {
            let resp = match self
                .inner
                .client
                .get(&url)
                .header(AUTHORIZATION, self.auth_header())
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = remote_transport_error("health check", &url, error);
                    if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                        sleep_before_retry("/api/gateway/status", attempt, None).await;
                        continue;
                    }
                    *self.inner.state.write().await = ConnectionState::Disconnected;
                    return Err(error);
                }
            };

            if resp.status().is_success() {
                *self.inner.state.write().await = ConnectionState::Connected;
                return Ok(true);
            }

            if matches!(resp.status().as_u16(), 401 | 403) {
                *self.inner.state.write().await = ConnectionState::Disconnected;
                return Ok(false);
            }

            let status = resp.status();
            let retry_after = retry_after_delay(resp.headers());
            let body = match read_bounded_body(resp, MAX_ERROR_RESPONSE_BYTES).await {
                Ok(body) => body,
                Err(error) => {
                    if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                        sleep_before_retry("/api/gateway/status", attempt, retry_after).await;
                        continue;
                    }
                    *self.inner.state.write().await = ConnectionState::Disconnected;
                    return Err(error);
                }
            };
            let error = remote_http_error(status, &body, "/api/gateway/status");
            if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                sleep_before_retry("/api/gateway/status", attempt, retry_after).await;
                continue;
            }
            *self.inner.state.write().await = ConnectionState::Disconnected;
            return Err(error);
        }

        unreachable!("health-check attempt loop always returns")
    }

    /// Get full gateway status including agent info.
    pub async fn get_status(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/gateway/status").await
    }
}

fn validate_base_url(raw: &str) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    let url = Url::parse(raw.trim()).map_err(|_| {
        "Remote gateway URL must be an absolute http:// or https:// URL".to_string()
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(("Remote gateway URL must use http:// or https://".to_string()).into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(("Remote gateway URL must not contain credentials".to_string()).into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(("Remote gateway URL must not contain a query or fragment".to_string()).into());
    }
    if !matches!(url.path(), "" | "/") {
        return Err(("Remote gateway URL must not contain a path".to_string()).into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "Remote gateway URL must contain a host".to_string())?;
    if url.scheme() == "http" && !is_private_transport_host(host) {
        return Err((
            "Public remote gateways require HTTPS; plaintext HTTP is limited to private, loopback, .local, or Tailscale hosts"
                .to_string()).into());
    }

    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn is_private_transport_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".ts.net")
    {
        return true;
    }

    normalized.parse::<IpAddr>().is_ok_and(|ip| match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || is_cgnat(ip),
        IpAddr::V6(ip) => ip.is_loopback() || is_unique_local(ip) || is_ipv6_link_local(ip),
    })
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.octets()[0] & 0xfe == 0xfc
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 0xfe && octets[1] & 0xc0 == 0x80
}

async fn read_bounded_body(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, crate::thinclaw::bridge::BridgeError> {
    use futures_util::StreamExt as _;

    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err((format!("Remote response exceeded the {max_bytes}-byte safety limit")).into());
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| crate::thinclaw::bridge::BridgeError::Network {
            message: format!("Failed to read remote response body: {error}"),
            retryable: true,
        })?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(
                (format!("Remote response exceeded the {max_bytes}-byte safety limit")).into(),
            );
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn remote_http_error(
    status: reqwest::StatusCode,
    body: &[u8],
    resource: &str,
) -> crate::thinclaw::bridge::BridgeError {
    use crate::thinclaw::bridge::BridgeError;

    let detail: String = String::from_utf8_lossy(body)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let message = if detail.is_empty() {
        format!("Remote returned HTTP {status}")
    } else {
        format!("Remote returned HTTP {status}: {detail}")
    };

    match status.as_u16() {
        401 | 403 => BridgeError::Unauthorized {
            message,
            remediation: Some("verify the remote gateway token and its permissions".to_string()),
        },
        404 => BridgeError::NotFound {
            resource: resource.to_string(),
            message,
        },
        409 => BridgeError::Conflict {
            message,
            remediation: Some("refresh remote state and retry the action".to_string()),
        },
        408 => BridgeError::Timeout {
            operation: format!("remote request {resource}"),
            message,
            retryable: true,
        },
        425 | 429 | 500 | 502 | 503 | 504 => BridgeError::Network {
            message,
            retryable: true,
        },
        _ => BridgeError::Runtime { message },
    }
}

fn remote_transport_error(
    operation: &str,
    url: &str,
    error: reqwest::Error,
) -> crate::thinclaw::bridge::BridgeError {
    use crate::thinclaw::bridge::BridgeError;

    let message = format!("Remote {operation} failed ({url}): {error}");
    if error.is_timeout() {
        BridgeError::Timeout {
            operation: operation.to_string(),
            message,
            retryable: true,
        }
    } else {
        BridgeError::Network {
            message,
            retryable: error.is_connect() || error.is_request() || error.is_body(),
        }
    }
}

fn bridge_error_is_retryable(error: &crate::thinclaw::bridge::BridgeError) -> bool {
    matches!(
        error,
        crate::thinclaw::bridge::BridgeError::Timeout {
            retryable: true,
            ..
        } | crate::thinclaw::bridge::BridgeError::Network {
            retryable: true,
            ..
        }
    )
}

fn retry_after_delay(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|duration| duration.min(MAX_RETRY_AFTER))
}

async fn sleep_before_retry(path: &str, attempt: u32, retry_after: Option<Duration>) {
    let path_jitter_ms = path.bytes().fold(0_u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u64)
    }) % 75;
    let exponential_ms = 150_u64.saturating_mul(1_u64 << attempt.saturating_sub(1).min(5));
    let fallback = Duration::from_millis(exponential_ms.saturating_add(path_jitter_ms));
    let delay = retry_after.unwrap_or(fallback).min(MAX_RETRY_AFTER);
    debug!(
        "[remote_proxy] transient failure for {} (attempt {}); retrying in {:?}",
        path, attempt, delay
    );
    tokio::time::sleep(delay).await;
}

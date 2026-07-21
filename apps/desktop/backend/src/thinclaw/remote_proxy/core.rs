//! Core proxy type, connection state, construction, low-level HTTP request
//! primitives, and health checks.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, RETRY_AFTER},
    Method,
};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::thinclaw::bridge::{gated, BridgeError, RouteMode};

const MAX_JSON_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_AUTH_TOKEN_BYTES: usize = 8 * 1024;
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
    /// Sensitive bearer header. `HeaderValue`'s sensitive bit prevents debug
    /// formatting from exposing the credential.
    pub(super) authorization: HeaderValue,
    /// Shared reqwest client (connection pool)
    pub(super) client: reqwest::Client,
    /// Long-lived streaming client. Unlike the request client this has no
    /// whole-request timeout; connect timeout and redirect policy still apply.
    pub(super) sse_client: reqwest::Client,
    /// SSE subscription task handle (if started)
    pub(super) sse_handle: Mutex<Option<JoinHandle<()>>>,
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
) -> Result<String, BridgeError> {
    remote_thread_id(session_key).ok_or_else(|| {
        RemoteGatewayProxy::unavailable(
            capability,
            "the pinned assistant thread must be addressed through a concrete remote thread id",
        )
    })
}

impl RemoteGatewayProxy {
    pub(crate) fn validate_base_url(base_url: &str) -> Result<(), BridgeError> {
        validate_and_normalize_base_url(base_url).map(|_| ())
    }

    /// Create a new proxy. Does NOT connect — call `health_check` or
    /// `start_sse_subscription` to establish the connection.
    pub fn new(base_url: &str, auth_token: &str) -> Result<Self, BridgeError> {
        let base_url = validate_and_normalize_base_url(base_url)?;
        if auth_token.is_empty() {
            return Err("remote gateway token must not be empty".into());
        }
        if auth_token.len() > MAX_AUTH_TOKEN_BYTES {
            return Err(format!(
                "remote gateway token exceeds the {MAX_AUTH_TOKEN_BYTES}-byte limit"
            )
            .into());
        }
        let bearer = zeroize::Zeroizing::new(format!("Bearer {auth_token}"));
        let mut authorization = HeaderValue::from_str(bearer.as_str())
            .map_err(|_| "remote gateway token contains invalid header characters".to_string())?;
        authorization.set_sensitive(true);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ThinClawDesktop/0.14 (ThinClaw remote proxy)")
            .build()
            .map_err(|error| format!("failed to build remote gateway client: {error}"))?;
        let sse_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ThinClawDesktop/0.14 (ThinClaw remote proxy)")
            .build()
            .map_err(|error| format!("failed to build remote gateway stream client: {error}"))?;

        Ok(Self {
            inner: Arc::new(RemoteGatewayProxyInner {
                base_url,
                authorization,
                client,
                sse_client,
                sse_handle: Mutex::new(None),
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
        self.inner.authorization.clone()
    }

    pub fn unavailable(capability: &str, reason: impl AsRef<str>) -> BridgeError {
        gated(
            capability,
            format!(
                "remote ThinClaw gateway does not support this capability: {}",
                reason.as_ref()
            ),
            "switch to embedded mode or upgrade the remote gateway",
            RouteMode::LocalOnly,
        )
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        headers: HeaderMap,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] {} {}", method, url);
        let max_attempts = if method == Method::GET {
            MAX_IDEMPOTENT_ATTEMPTS
        } else {
            1
        };

        for attempt in 1..=max_attempts {
            let mut request = self
                .inner
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, self.auth_header());
            if !headers.is_empty() {
                request = request.headers(headers.clone());
            }
            if let Some(body) = body {
                request = request.json(body);
            }

            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    let error = remote_transport_error(method.as_ref(), &url, error);
                    if attempt < max_attempts && bridge_error_is_retryable(&error) {
                        sleep_before_retry(path, attempt, None).await;
                        continue;
                    }
                    return Err(error);
                }
            };
            let status = response.status();
            let retry_after = retry_after_delay(response.headers());
            let max_bytes = if status.is_success() {
                MAX_JSON_BODY_BYTES
            } else {
                MAX_ERROR_BODY_BYTES
            };
            let response_body = match read_body_limited(response, max_bytes).await {
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
            return serde_json::from_slice(&response_body).map_err(|error| BridgeError::Runtime {
                message: format!("Failed to parse JSON response from {url}: {error}"),
            });
        }

        unreachable!("request attempt loop always returns")
    }

    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value, BridgeError> {
        self.request_json(Method::GET, path, None, HeaderMap::new())
            .await
    }

    pub(super) async fn get_text(&self, path: &str) -> Result<String, BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] GET {}", url);

        for attempt in 1..=MAX_IDEMPOTENT_ATTEMPTS {
            let response = match self
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
            let status = response.status();
            let retry_after = retry_after_delay(response.headers());
            let max_bytes = if status.is_success() {
                MAX_TEXT_BODY_BYTES
            } else {
                MAX_ERROR_BODY_BYTES
            };
            let body = read_body_limited(response, max_bytes).await?;
            if status.is_success() {
                return String::from_utf8(body).map_err(|_| BridgeError::Runtime {
                    message: "remote gateway returned non-UTF-8 text".to_string(),
                });
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
    ) -> Result<serde_json::Value, BridgeError> {
        self.request_json(Method::POST, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn post_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::POST, path, Some(body), headers)
            .await
    }

    pub async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.request_json(Method::PUT, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn put_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::PUT, path, Some(body), headers)
            .await
    }

    #[allow(dead_code)]
    async fn put_text(&self, path: &str, content: &str) -> Result<(), BridgeError> {
        let url = self.url(path);
        debug!("[remote_proxy] PUT {}", url);

        let resp = self
            .inner
            .client
            .put(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(content.to_string())
            .send()
            .await
            .map_err(|error| remote_transport_error("PUT", &url, error))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(remote_http_error(status, &[], path));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_json(&self, path: &str) -> Result<serde_json::Value, BridgeError> {
        self.request_json(Method::DELETE, path, None, HeaderMap::new())
            .await
    }

    pub async fn delete_json_body(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.request_json(Method::DELETE, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn delete_json_confirm(&self, path: &str) -> Result<serde_json::Value, BridgeError> {
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
    pub async fn health_check(&self) -> Result<bool, BridgeError> {
        let path = "/api/gateway/status";
        let url = self.url(path);
        for attempt in 1..=MAX_IDEMPOTENT_ATTEMPTS {
            let response = match self
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
                        sleep_before_retry(path, attempt, None).await;
                        continue;
                    }
                    *self.inner.state.write().await = ConnectionState::Disconnected;
                    return Err(error);
                }
            };

            if response.status().is_success() {
                *self.inner.state.write().await = ConnectionState::Connected;
                return Ok(true);
            }
            if matches!(response.status().as_u16(), 401 | 403) {
                *self.inner.state.write().await = ConnectionState::Disconnected;
                return Ok(false);
            }

            let status = response.status();
            let retry_after = retry_after_delay(response.headers());
            let body = read_body_limited(response, MAX_ERROR_BODY_BYTES).await?;
            let error = remote_http_error(status, &body, path);
            if attempt < MAX_IDEMPOTENT_ATTEMPTS && bridge_error_is_retryable(&error) {
                sleep_before_retry(path, attempt, retry_after).await;
                continue;
            }
            *self.inner.state.write().await = ConnectionState::Disconnected;
            return Err(error);
        }

        unreachable!("health-check attempt loop always returns")
    }

    /// Get full gateway status including agent info.
    pub async fn get_status(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_json("/api/gateway/status").await
    }
}

fn validate_and_normalize_base_url(raw: &str) -> Result<String, BridgeError> {
    let parsed = reqwest::Url::parse(raw.trim())
        .map_err(|error| format!("invalid remote gateway URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("remote gateway URL must use http or https".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("remote gateway URL must not embed credentials".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("remote gateway URL must not contain a query or fragment".into());
    }
    if !matches!(parsed.path(), "" | "/") {
        return Err("remote gateway URL must not contain a path".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "remote gateway URL has no host".to_string())?;
    if parsed.scheme() == "http" && !is_private_overlay_http_host(host) {
        return Err(
            "plaintext HTTP gateways are allowed only on loopback or numeric Tailscale addresses; use HTTPS elsewhere"
                .into(),
        );
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn is_private_overlay_http_host(host: &str) -> bool {
    // `url`/`reqwest` may expose an IPv6 host with its URI brackets intact.
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            if ip.is_loopback() {
                return true;
            }
            let value = u32::from(ip);
            let start = u32::from(std::net::Ipv4Addr::new(100, 64, 0, 0));
            value >= start && value < start + (1 << 22)
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            if ip.is_loopback() {
                return true;
            }
            let segments = ip.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
        Err(_) => false,
    }
}

async fn read_body_limited(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BridgeError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("remote response exceeds the {limit}-byte limit").into());
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| BridgeError::Network {
            message: format!("failed to read remote response: {error}"),
            retryable: true,
        })?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(format!("remote response exceeds the {limit}-byte limit").into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn remote_http_error(status: reqwest::StatusCode, body: &[u8], resource: &str) -> BridgeError {
    let detail = String::from_utf8_lossy(body)
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
        400 | 422 => BridgeError::InvalidInput {
            message,
            field: None,
        },
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

fn remote_transport_error(operation: &str, url: &str, error: reqwest::Error) -> BridgeError {
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

fn bridge_error_is_retryable(error: &BridgeError) -> bool {
    matches!(
        error,
        BridgeError::Timeout {
            retryable: true,
            ..
        } | BridgeError::Network {
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
        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
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

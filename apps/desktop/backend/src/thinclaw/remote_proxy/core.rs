//! Core proxy type, connection state, construction, low-level HTTP request
//! primitives, and health checks.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Method,
};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::debug;

const MAX_JSON_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_HEALTH_BODY_BYTES: usize = 64 * 1024;
const MAX_AUTH_TOKEN_BYTES: usize = 8 * 1024;

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
) -> Result<String, String> {
    remote_thread_id(session_key).ok_or_else(|| {
        RemoteGatewayProxy::unavailable(
            capability,
            "the pinned assistant thread must be addressed through a concrete remote thread id",
        )
    })
}

impl RemoteGatewayProxy {
    pub(crate) fn validate_base_url(base_url: &str) -> Result<(), String> {
        validate_and_normalize_base_url(base_url).map(|_| ())
    }

    /// Create a new proxy. Does NOT connect — call `health_check` or
    /// `start_sse_subscription` to establish the connection.
    pub fn new(base_url: &str, auth_token: &str) -> Result<Self, String> {
        let base_url = validate_and_normalize_base_url(base_url)?;
        if auth_token.is_empty() {
            return Err("remote gateway token must not be empty".to_string());
        }
        if auth_token.len() > MAX_AUTH_TOKEN_BYTES {
            return Err(format!(
                "remote gateway token exceeds the {MAX_AUTH_TOKEN_BYTES}-byte limit"
            ));
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

    pub fn unavailable(capability: &str, reason: impl AsRef<str>) -> String {
        format!(
            "unavailable: remote ThinClaw gateway does not support {}: {}",
            capability,
            reason.as_ref()
        )
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        headers: HeaderMap,
    ) -> Result<serde_json::Value, String> {
        let url = self.url(path);
        debug!("[remote_proxy] {} {}", method, url);

        let mut req = self
            .inner
            .client
            .request(method, &url)
            .header(AUTHORIZATION, self.auth_header());
        if !headers.is_empty() {
            req = req.headers(headers);
        }
        if let Some(body) = body {
            req = req.json(body);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;
        let status = resp.status();
        let body = read_body_limited(resp, MAX_JSON_BODY_BYTES).await?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {status}"));
        }

        if body.is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }

        serde_json::from_slice(&body)
            .map_err(|e| format!("Failed to parse JSON response from {}: {}", url, e))
    }

    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value, String> {
        self.request_json(Method::GET, path, None, HeaderMap::new())
            .await
    }

    pub(super) async fn get_text(&self, path: &str) -> Result<String, String> {
        let url = self.url(path);
        debug!("[remote_proxy] GET {}", url);

        let resp = self
            .inner
            .client
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        let status = resp.status();
        let body = read_body_limited(resp, MAX_TEXT_BODY_BYTES).await?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {status}"));
        }

        String::from_utf8(body).map_err(|_| "remote gateway returned non-UTF-8 text".to_string())
    }

    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.request_json(Method::POST, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn post_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::POST, path, Some(body), headers)
            .await
    }

    pub async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.request_json(Method::PUT, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn put_json_confirm(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::PUT, path, Some(body), headers)
            .await
    }

    #[allow(dead_code)]
    async fn put_text(&self, path: &str, content: &str) -> Result<(), String> {
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
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(format!("Remote returned HTTP {status}"));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_json(&self, path: &str) -> Result<serde_json::Value, String> {
        self.request_json(Method::DELETE, path, None, HeaderMap::new())
            .await
    }

    pub async fn delete_json_body(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.request_json(Method::DELETE, path, Some(body), HeaderMap::new())
            .await
    }

    pub async fn delete_json_confirm(&self, path: &str) -> Result<serde_json::Value, String> {
        let mut headers = HeaderMap::new();
        headers.insert("x-confirm-action", "true".parse().expect("valid header"));
        self.request_json(Method::DELETE, path, None, headers).await
    }

    // ── Health ───────────────────────────────────────────────────────────────

    /// Test connectivity to the remote gateway.
    ///
    /// Returns Ok(true) if the server is reachable and responds to /api/health.
    /// Returns Ok(false) if the server is reachable but auth failed.
    /// Returns Err if connection could not be established.
    pub async fn health_check(&self) -> Result<bool, String> {
        let result = self.health_check_inner().await;
        *self.inner.state.write().await = if matches!(result, Ok(true)) {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };
        result
    }

    async fn health_check_inner(&self) -> Result<bool, String> {
        let url = self.url("/api/health");
        let resp = self
            .inner
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Cannot connect to remote gateway at {}: {}",
                    self.inner.base_url, e
                )
            })?;

        if !resp.status().is_success() {
            return Ok(false);
        }

        // A public health response proves only that *something* is listening.
        // Verify the configured bearer against an authenticated endpoint before
        // treating the proxy as connected.
        let status_url = self.url("/api/gateway/status");
        let response = self
            .inner
            .client
            .get(&status_url)
            .header(AUTHORIZATION, self.auth_header())
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|error| format!("Remote gateway authentication check failed: {error}"))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Ok(false);
        }
        if !status.is_success() {
            return Err(format!(
                "Remote gateway authentication check returned HTTP {status}"
            ));
        }
        let body = read_body_limited(response, MAX_HEALTH_BODY_BYTES).await?;
        serde_json::from_slice::<serde_json::Value>(&body)
            .map_err(|_| "remote gateway status returned invalid JSON".to_string())?;
        Ok(true)
    }

    /// Get full gateway status including agent info.
    pub async fn get_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }
}

fn validate_and_normalize_base_url(raw: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(raw.trim())
        .map_err(|error| format!("invalid remote gateway URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("remote gateway URL must use http or https".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("remote gateway URL must not embed credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("remote gateway URL must not contain a query or fragment".to_string());
    }
    if !matches!(parsed.path(), "" | "/") {
        return Err("remote gateway URL must not contain a path".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "remote gateway URL has no host".to_string())?;
    if parsed.scheme() == "http" && !is_private_overlay_http_host(host) {
        return Err(
            "plaintext HTTP gateways are allowed only on loopback or numeric Tailscale addresses; use HTTPS elsewhere"
                .to_string(),
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

async fn read_body_limited(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("remote response exceeds the {limit}-byte limit"));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("failed to read remote response: {error}"))?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(format!("remote response exceeds the {limit}-byte limit"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

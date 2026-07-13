//! Core proxy type, connection state, construction, low-level HTTP request
//! primitives, and health checks.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
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
) -> Result<String, String> {
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
    pub fn new(base_url: &str, auth_token: &str) -> Result<Self, String> {
        let base_url = validate_base_url(base_url)?;
        let token = auth_token.trim();
        if token.is_empty() {
            return Err("Remote gateway token is required".to_string());
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
        let max_bytes = if status.is_success() {
            MAX_JSON_RESPONSE_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let body = read_bounded_body(resp, max_bytes).await?;

        if !status.is_success() {
            return Err(remote_http_error(status, &body));
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
        let max_bytes = if status.is_success() {
            MAX_TEXT_RESPONSE_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let body = read_bounded_body(resp, max_bytes).await?;

        if !status.is_success() {
            return Err(remote_http_error(status, &body));
        }

        Ok(String::from_utf8_lossy(&body).into_owned())
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
            .header(AUTHORIZATION, self.auth_header())
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(content.to_string())
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = read_bounded_body(resp, MAX_ERROR_RESPONSE_BYTES).await?;
            return Err(remote_http_error(status, &body));
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
    /// Returns Ok(true) if the server accepts this credential on an authenticated endpoint.
    /// Returns Ok(false) if the server is reachable but auth failed.
    /// Returns Err if connection could not be established.
    pub async fn health_check(&self) -> Result<bool, String> {
        let url = self.url("/api/gateway/status");
        let resp = self
            .inner
            .client
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Cannot connect to remote gateway at {}: {}",
                    self.inner.base_url, e
                )
            })?;

        if resp.status().is_success() {
            *self.inner.state.write().await = ConnectionState::Connected;
            return Ok(true);
        }

        if matches!(resp.status().as_u16(), 401 | 403) {
            *self.inner.state.write().await = ConnectionState::Disconnected;
            return Ok(false);
        }

        let status = resp.status();
        let body = read_bounded_body(resp, MAX_ERROR_RESPONSE_BYTES).await?;
        Err(remote_http_error(status, &body))
    }

    /// Get full gateway status including agent info.
    pub async fn get_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }
}

fn validate_base_url(raw: &str) -> Result<String, String> {
    let url = Url::parse(raw.trim()).map_err(|_| {
        "Remote gateway URL must be an absolute http:// or https:// URL".to_string()
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Remote gateway URL must use http:// or https://".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Remote gateway URL must not contain credentials".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Remote gateway URL must not contain a query or fragment".to_string());
    }
    if !matches!(url.path(), "" | "/") {
        return Err("Remote gateway URL must not contain a path".to_string());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "Remote gateway URL must contain a host".to_string())?;
    if url.scheme() == "http" && !is_private_transport_host(host) {
        return Err(
            "Public remote gateways require HTTPS; plaintext HTTP is limited to private, loopback, .local, or Tailscale hosts"
                .to_string(),
        );
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
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || is_cgnat(ip)
        }
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
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt as _;

    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!(
            "Remote response exceeded the {max_bytes}-byte safety limit"
        ));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Failed to read response body: {error}"))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!(
                "Remote response exceeded the {max_bytes}-byte safety limit"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn remote_http_error(status: reqwest::StatusCode, body: &[u8]) -> String {
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

    if detail.is_empty() {
        format!("Remote returned HTTP {status}")
    } else {
        format!("Remote returned HTTP {status}: {detail}")
    }
}

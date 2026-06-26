//! Core proxy type, connection state, construction, low-level HTTP request
//! primitives, and health checks.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{header::HeaderMap, Method};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;

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
    /// Bearer auth token
    pub(super) auth_token: String,
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
    pub fn new(base_url: &str, auth_token: &str) -> Self {
        // Normalize URL (strip trailing slash)
        let base_url = base_url.trim_end_matches('/').to_string();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("ThinClawDesktop/0.14 (ThinClaw remote proxy)")
            .build()
            .expect("reqwest Client::build should not fail with valid config");

        Self {
            inner: Arc::new(RemoteGatewayProxyInner {
                base_url,
                auth_token: auth_token.to_string(),
                client,
                sse_handle: RwLock::new(None),
                state: RwLock::new(ConnectionState::Disconnected),
            }),
        }
    }

    /// Base URL accessor.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Auth token accessor.
    pub fn auth_token(&self) -> &str {
        &self.inner.auth_token
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    pub(super) fn url(&self, path: &str) -> String {
        format!("{}{}", self.inner.base_url, path)
    }

    pub(super) fn auth_header(&self) -> String {
        format!("Bearer {}", self.inner.auth_token)
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
            .header("Authorization", self.auth_header());
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
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {}: {}", status, body));
        }

        if body.is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }

        serde_json::from_str(&body)
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
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {}: {}", status, body));
        }

        Ok(body)
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

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Remote returned HTTP {}: {}", status, body));
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

        // /api/health is public (no auth) — 200 = online
        if resp.status().is_success() {
            *self.inner.state.write().await = ConnectionState::Connected;
            return Ok(true);
        }

        Ok(false)
    }

    /// Get full gateway status including agent info.
    pub async fn get_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }
}

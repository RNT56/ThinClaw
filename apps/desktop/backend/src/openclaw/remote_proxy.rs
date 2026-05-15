//! RemoteGatewayProxy — HTTP/SSE client for remote IronClaw gateway.
//!
//! When ThinClaw Desktop is in "remote" mode, all agent interactions are forwarded
//! to a remote IronClaw HTTP server instead of the embedded in-process engine.
//!
//! Architecture:
//!   Frontend → Tauri IPC → Command handler → RemoteGatewayProxy → HTTP API
//!   Remote SSE stream → RemoteGatewayProxy → Tauri emit("openclaw-event")
//!
//! The proxy is intentionally thin: it does not transform data but passes
//! raw JSON responses back to the command handlers who already know the
//! expected shape (same as local mode, since the remote IronClaw server
//! and the local embedded engine share the same API definitions).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Connection state for health monitoring
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}

/// HTTP/SSE proxy client for a remote IronClaw gateway.
///
/// Cheaply cloneable — all state behind Arc.
#[derive(Clone)]
pub struct RemoteGatewayProxy {
    inner: Arc<RemoteGatewayProxyInner>,
}

struct RemoteGatewayProxyInner {
    /// Base URL of the remote gateway, e.g. "http://192.168.1.50:18789"
    base_url: String,
    /// Bearer auth token
    auth_token: String,
    /// Shared reqwest client (connection pool)
    client: reqwest::Client,
    /// SSE subscription task handle (if started)
    sse_handle: RwLock<Option<JoinHandle<()>>>,
    /// Current connection state
    state: RwLock<ConnectionState>,
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

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.inner.base_url, path)
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.inner.auth_token)
    }

    async fn get_json(&self, path: &str) -> Result<serde_json::Value, String> {
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

        serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse JSON response from {}: {}", url, e))
    }

    async fn get_text(&self, path: &str) -> Result<String, String> {
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

    async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let url = self.url(path);
        debug!("[remote_proxy] POST {}", url);

        let resp = self
            .inner
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {}: {}", status, body_text));
        }

        // Some endpoints return empty body on success
        if body_text.is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }

        serde_json::from_str(&body_text)
            .map_err(|e| format!("Failed to parse JSON response from {}: {}", url, e))
    }

    async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let url = self.url(path);
        debug!("[remote_proxy] PUT {}", url);

        let resp = self
            .inner
            .client
            .put(&url)
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Request failed ({}): {}", url, e))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if !status.is_success() {
            return Err(format!("Remote returned HTTP {}: {}", status, body_text));
        }

        if body_text.is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }

        serde_json::from_str(&body_text)
            .map_err(|e| format!("Failed to parse JSON response from {}: {}", url, e))
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
    async fn delete(&self, path: &str) -> Result<(), String> {
        let url = self.url(path);
        debug!("[remote_proxy] DELETE {}", url);

        let resp = self
            .inner
            .client
            .delete(&url)
            .header("Authorization", self.auth_header())
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

    // ── Chat ─────────────────────────────────────────────────────────────────

    /// Send a chat message.
    ///
    /// Remote gateway endpoint: POST /api/chat/send
    /// Body: { session_key, message, stream }
    pub async fn send_message(
        &self,
        session_key: &str,
        text: &str,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/chat/send",
            &serde_json::json!({
                "session_key": session_key,
                "message": text,
                "stream": true,
            }),
        )
        .await
    }

    /// Abort a running chat turn.
    pub async fn abort_chat(&self, session_key: &str) -> Result<(), String> {
        self.post_json(
            "/api/chat/send",
            &serde_json::json!({
                "session_key": session_key,
                "abort": true,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Delete a chat session/thread.
    pub async fn delete_session(&self, session_key: &str) -> Result<(), String> {
        self.post_json(
            "/api/chat/threads/delete",
            &serde_json::json!({
                "session_key": session_key,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Reset (clear history of) a chat session.
    pub async fn reset_session(&self, session_key: &str) -> Result<(), String> {
        self.post_json(
            "/api/chat/threads/reset",
            &serde_json::json!({
                "session_key": session_key,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Get all chat sessions/threads.
    pub async fn get_sessions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/chat/threads").await
    }

    /// Get chat history for a session.
    pub async fn get_history(
        &self,
        session_key: &str,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/chat/history?session_key={}&limit={}",
            urlencoding::encode(session_key),
            limit
        ))
        .await
    }

    /// Resolve a tool approval request.
    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        approved: bool,
        allow_session: bool,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/chat/approval",
            &serde_json::json!({
                "approval_id": approval_id,
                "approved": approved,
                "allow_session": allow_session,
            }),
        )
        .await
    }

    // ── Memory / Workspace ───────────────────────────────────────────────────

    /// Read a workspace file.
    ///
    /// Remote endpoint: GET /api/memory/read?path={path}
    pub async fn get_file(&self, path: &str) -> Result<String, String> {
        let resp = self
            .get_json(&format!(
                "/api/memory/read?path={}",
                urlencoding::encode(path)
            ))
            .await?;

        // Gateway returns: { path, content, created_at, ... }
        Ok(resp
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Write a workspace file.
    ///
    /// Remote endpoint: POST /api/memory/write
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.post_json(
            "/api/memory/write",
            &serde_json::json!({
                "path": path,
                "content": content,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Delete a workspace file.
    ///
    /// Remote endpoint: POST /api/memory/delete
    pub async fn delete_file(&self, path: &str) -> Result<(), String> {
        self.post_json(
            "/api/memory/delete",
            &serde_json::json!({
                "path": path,
            }),
        )
        .await
        .map(|_| ())
    }

    /// List all workspace files.
    ///
    /// Remote endpoint: GET /api/memory/list
    pub async fn list_files(&self) -> Result<Vec<String>, String> {
        let resp = self.get_json("/api/memory/list").await?;
        let paths: Vec<String> = resp
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(paths)
    }

    /// Search workspace memory.
    ///
    /// Remote endpoint: POST /api/memory/search
    pub async fn search_memory(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/memory/search",
            &serde_json::json!({
                "query": query,
                "limit": limit,
            }),
        )
        .await
    }

    // ── Routines ─────────────────────────────────────────────────────────────

    /// List all routines.
    pub async fn list_routines(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/routines").await
    }

    /// Trigger a routine manually.
    pub async fn trigger_routine(&self, routine_id: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/routines/{}/trigger", urlencoding::encode(routine_id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// Get routine run history.
    pub async fn get_routine_history(
        &self,
        routine_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/routines/{}/runs?limit={}",
            urlencoding::encode(routine_id),
            limit
        ))
        .await
    }

    /// Create a new routine.
    pub async fn create_routine(
        &self,
        name: &str,
        description: &str,
        schedule: &str,
        task: &str,
    ) -> Result<serde_json::Value, String> {
        // The remote IronClaw settings API can store config
        self.post_json(
            "/api/settings/routines.create",
            &serde_json::json!({
                "name": name,
                "description": description,
                "schedule": schedule,
                "task": task,
            }),
        )
        .await
    }

    // ── Skills ───────────────────────────────────────────────────────────────

    /// List all installed skills.
    pub async fn list_skills(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/skills").await
    }

    // ── Providers / Routing ─────────────────────────────────────────────────

    /// Get remote provider and routing configuration.
    pub async fn get_providers_config(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/providers/config").await
    }

    /// Replace remote provider and routing configuration.
    pub async fn set_providers_config(&self, config: &serde_json::Value) -> Result<(), String> {
        self.put_json("/api/providers/config", config)
            .await
            .map(|_| ())
    }

    /// Get remote model options for one provider.
    pub async fn get_provider_models(&self, slug: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/providers/{}/models",
            urlencoding::encode(slug)
        ))
        .await
    }

    /// Save a remote provider API key through the provider vault endpoint.
    pub async fn save_provider_key(
        &self,
        slug: &str,
        api_key: &str,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/providers/{}/key", urlencoding::encode(slug)),
            &serde_json::json!({ "api_key": api_key }),
        )
        .await
    }

    /// Delete a remote provider API key through the provider vault endpoint.
    pub async fn delete_provider_key(&self, slug: &str) -> Result<(), String> {
        self.delete(&format!("/api/providers/{}/key", urlencoding::encode(slug)))
            .await
    }

    // ── Costs ────────────────────────────────────────────────────────────────

    /// Get remote LLM cost summary.
    pub async fn get_cost_summary(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/costs/summary").await
    }

    /// Export remote cost data as CSV.
    pub async fn export_cost_csv(&self) -> Result<String, String> {
        self.get_text("/api/costs/export").await
    }

    /// Reset remote cost tracking data.
    pub async fn reset_costs(&self) -> Result<(), String> {
        self.post_json("/api/costs/reset", &serde_json::json!({}))
            .await
            .map(|_| ())
    }

    // ── Export ───────────────────────────────────────────────────────────────

    /// Export a session as a formatted transcript.
    ///
    /// Remote endpoint: GET /api/chat/export?session_key=...&format=...
    pub async fn export_session(
        &self,
        session_key: &str,
        format: &str,
    ) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/chat/export?session_key={}&format={}",
            urlencoding::encode(session_key),
            urlencoding::encode(format)
        ))
        .await
    }

    // ── Extensions ───────────────────────────────────────────────────────────

    /// List all extensions.
    pub async fn list_extensions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/extensions").await
    }

    // ── Settings / Config ────────────────────────────────────────────────────

    /// Get a config setting from the remote agent.
    pub async fn get_setting(&self, key: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/settings/{}", urlencoding::encode(key)))
            .await
    }

    /// Set a config setting on the remote agent.
    pub async fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        let url = format!("/api/settings/{}", urlencoding::encode(key));
        let body = serde_json::json!({ "value": value });
        self.put_json(&url, &body).await.map(|_| ())
    }

    /// Inject API secrets into the remote agent.
    ///
    /// This uses the IronClaw settings API to push secrets individually.
    /// Each key is sent as a separate settings write so the agent can
    /// store them in its own SecretStore.
    ///
    /// Accepts a map of { "ANTHROPIC_API_KEY": "sk-ant-..." } etc.
    pub async fn inject_secrets(
        &self,
        secrets: &std::collections::HashMap<String, String>,
    ) -> Result<u32, String> {
        let mut count = 0u32;
        for (key, value) in secrets {
            match self
                .set_setting(key, &serde_json::Value::String(value.clone()))
                .await
            {
                Ok(()) => count += 1,
                Err(e) => {
                    warn!("[remote_proxy] Failed to inject secret {}: {}", key, e);
                }
            }
        }
        info!(
            "[remote_proxy] Injected {}/{} secrets",
            count,
            secrets.len()
        );
        Ok(count)
    }

    // ── Diagnostics / Logs ───────────────────────────────────────────────────

    /// Get full diagnostics from the remote gateway.
    pub async fn get_diagnostics(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }

    // ── SSE Event Subscription ───────────────────────────────────────────────

    /// Subscribe to the remote gateway's SSE event stream and re-emit
    /// all events as Tauri `openclaw-event` emissions.
    ///
    /// This runs as a background task. Events from the remote IronClaw
    /// are forwarded directly to the frontend — the UI sees no difference
    /// between local and remote agent events.
    ///
    /// Auto-reconnects on disconnect (exponential backoff, max 30s).
    pub async fn start_sse_subscription(&self, app_handle: tauri::AppHandle) -> Result<(), String> {
        // Stop existing subscription first
        self.stop_sse_subscription().await;

        *self.inner.state.write().await = ConnectionState::Connected;

        let proxy = self.clone();
        let handle = tokio::spawn(async move {
            proxy.sse_loop(app_handle).await;
        });

        *self.inner.sse_handle.write().await = Some(handle);
        info!(
            "[remote_proxy] SSE subscription started for {}",
            self.inner.base_url
        );
        Ok(())
    }

    /// Stop the background SSE subscription task.
    pub async fn stop_sse_subscription(&self) {
        if let Some(handle) = self.inner.sse_handle.write().await.take() {
            handle.abort();
            info!("[remote_proxy] SSE subscription stopped");
        }
        *self.inner.state.write().await = ConnectionState::Disconnected;
    }

    /// Current connection state.
    pub async fn connection_state(&self) -> ConnectionState {
        self.inner.state.read().await.clone()
    }

    /// Background SSE loop with auto-reconnect.
    async fn sse_loop(&self, app_handle: tauri::AppHandle) {
        use tauri::Emitter;

        let mut backoff_secs: u64 = 1;
        const MAX_BACKOFF: u64 = 30;

        loop {
            *self.inner.state.write().await = ConnectionState::Reconnecting;

            let url = self.url("/api/chat/events");
            info!("[remote_proxy] Connecting to SSE: {}", url);

            let result = self
                .inner
                .client
                .get(&url)
                .header("Authorization", self.auth_header())
                .header("Accept", "text/event-stream")
                .header("Cache-Control", "no-cache")
                // No global timeout — SSE is a long-lived connection
                .timeout(Duration::MAX)
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(
                        "[remote_proxy] SSE connection failed: {}. Retrying in {}s",
                        e, backoff_secs
                    );
                }
                Ok(response) if !response.status().is_success() => {
                    let status = response.status();
                    error!(
                        "[remote_proxy] SSE endpoint returned HTTP {}. Retrying in {}s",
                        status, backoff_secs
                    );
                }
                Ok(response) => {
                    *self.inner.state.write().await = ConnectionState::Connected;
                    backoff_secs = 1; // Reset backoff on successful connect

                    info!("[remote_proxy] SSE stream connected");

                    // Emit Connected event to frontend
                    let _ = app_handle.emit(
                        "openclaw-event",
                        &crate::openclaw::ui_types::UiEvent::Connected { protocol: 1 },
                    );

                    // Stream SSE events
                    let stream_result = self.consume_sse_stream(response, &app_handle).await;

                    match stream_result {
                        Ok(()) => {
                            info!("[remote_proxy] SSE stream ended cleanly. Reconnecting...");
                        }
                        Err(e) => {
                            warn!("[remote_proxy] SSE stream error: {}. Reconnecting...", e);
                        }
                    }

                    // Emit Disconnected to frontend on stream end
                    let _ = app_handle.emit(
                        "openclaw-event",
                        &crate::openclaw::ui_types::UiEvent::Disconnected {
                            reason: "Remote stream ended — reconnecting".to_string(),
                        },
                    );
                }
            }

            *self.inner.state.write().await = ConnectionState::Reconnecting;
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
        }
    }

    /// Consume a live SSE response stream and forward events to Tauri.
    ///
    /// The IronClaw gateway sends events in SSE format:
    ///   data: {"kind":"AssistantDelta","session_key":"...","delta":"..."}\n\n
    ///
    /// We parse each `data:` line as a `UiEvent` (same JSON schema as the
    /// local TauriChannel uses) and re-emit in identical format.
    async fn consume_sse_stream(
        &self,
        response: reqwest::Response,
        app_handle: &tauri::AppHandle,
    ) -> Result<(), String> {
        use futures_util::StreamExt;
        use tauri::Emitter;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("SSE stream read error: {}", e))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            // Process complete SSE lines (terminated by \n)
            loop {
                if let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim_end_matches('\r').to_string();
                    buffer.drain(..=pos);

                    if line.starts_with("data: ") {
                        let data = line.trim_start_matches("data: ").trim();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }

                        // Try to parse as UiEvent for typed re-emit
                        match serde_json::from_str::<crate::openclaw::ui_types::UiEvent>(data) {
                            Ok(event) => {
                                debug!("[remote_proxy] SSE event: {:?}", event);
                                if let Err(e) = app_handle.emit("openclaw-event", &event) {
                                    warn!("[remote_proxy] Failed to emit Tauri event: {}", e);
                                }
                            }
                            Err(_) => {
                                // Not a UiEvent — emit as raw JSON so frontend
                                // can inspect it (gracefully degraded)
                                if let Ok(raw_json) =
                                    serde_json::from_str::<serde_json::Value>(data)
                                {
                                    debug!("[remote_proxy] Unknown SSE event (raw): {}", data);
                                    let _ = app_handle.emit("openclaw-raw-event", &raw_json);
                                }
                            }
                        }
                    }
                } else {
                    break;
                }
            }
        }

        Ok(())
    }
}

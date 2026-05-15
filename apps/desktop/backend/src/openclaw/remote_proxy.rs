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

use reqwest::{header::HeaderMap, Method};
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
        let thread_id = if session_key == "agent:main" || session_key.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(session_key.to_string())
        };
        self.post_json(
            "/api/chat/send",
            &serde_json::json!({
                "thread_id": thread_id,
                "content": text,
            }),
        )
        .await
    }

    /// Abort a running chat turn.
    pub async fn abort_chat(&self, _session_key: &str) -> Result<(), String> {
        Err(Self::unavailable(
            "chat abort",
            "the gateway has no abort endpoint for active chat turns",
        ))
    }

    /// Delete a chat session/thread.
    pub async fn delete_session(&self, session_key: &str) -> Result<(), String> {
        if session_key == "agent:main" {
            return Err(Self::unavailable(
                "session delete",
                "the gateway assistant thread is pinned and cannot be deleted",
            ));
        }
        self.delete_json(&format!(
            "/api/chat/thread/{}",
            urlencoding::encode(session_key)
        ))
        .await
        .map(|_| ())
    }

    /// Reset (clear history of) a chat session.
    pub async fn reset_session(&self, _session_key: &str) -> Result<(), String> {
        Err(Self::unavailable(
            "session reset",
            "the gateway exposes delete/create thread but no clear-history endpoint",
        ))
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
        let mut query = format!("/api/chat/history?limit={}", limit);
        if session_key != "agent:main" && !session_key.trim().is_empty() {
            query.push_str("&thread_id=");
            query.push_str(&urlencoding::encode(session_key));
        }
        self.get_json(&query).await
    }

    /// Resolve a tool approval request.
    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        approved: bool,
        allow_session: bool,
    ) -> Result<serde_json::Value, String> {
        let action = if approved && allow_session {
            "always"
        } else if approved {
            "approve"
        } else {
            "deny"
        };
        self.post_json(
            "/api/chat/approval",
            &serde_json::json!({
                "request_id": approval_id,
                "action": action,
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
    /// The root gateway currently has read/write/list/search routes but no
    /// delete route. Return explicit unavailable instead of posting to a
    /// non-existent endpoint.
    pub async fn delete_file(&self, path: &str) -> Result<(), String> {
        let _ = path;
        Err(Self::unavailable(
            "workspace file delete",
            "the gateway memory API has no delete endpoint",
        ))
    }

    /// List all workspace files.
    ///
    /// Remote endpoint: GET /api/memory/list
    pub async fn list_files(&self) -> Result<Vec<String>, String> {
        let resp = self.get_json("/api/memory/tree").await?;
        let paths: Vec<String> = resp
            .get("entries")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|v| !v.get("is_dir").and_then(|d| d.as_bool()).unwrap_or(false))
                    .filter_map(|v| {
                        v.get("path")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string())
                    })
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
        self.post_json(
            "/api/routines",
            &serde_json::json!({
                "name": name,
                "description": description,
                "schedule": schedule,
                "task": task,
            }),
        )
        .await
    }

    /// Toggle a routine enabled/disabled.
    pub async fn toggle_routine(
        &self,
        routine_id: &str,
        enabled: bool,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/routines/{}/toggle", urlencoding::encode(routine_id)),
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
    }

    /// Delete a routine.
    pub async fn delete_routine(&self, routine_id: &str) -> Result<serde_json::Value, String> {
        self.delete_json(&format!(
            "/api/routines/{}",
            urlencoding::encode(routine_id)
        ))
        .await?;
        Ok(serde_json::json!({ "ok": true, "deleted_id": routine_id }))
    }

    /// Clear routine run history. If `routine_id` is absent, clears runs for
    /// all routines visible to the authenticated remote principal.
    pub async fn clear_routine_runs(
        &self,
        routine_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.delete_json_body(
            "/api/routines/runs",
            &serde_json::json!({ "routine_id": routine_id }),
        )
        .await
    }

    // ── Channels / Pairing ─────────────────────────────────────────────────

    /// List pending and approved channel pairings.
    pub async fn list_pairings(&self, channel: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/pairing/{}", urlencoding::encode(channel)))
            .await
    }

    /// Approve a channel pairing code.
    pub async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/pairing/{}/approve", urlencoding::encode(channel)),
            &serde_json::json!({ "code": code }),
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

    /// List remote providers with sanitized credential status only.
    pub async fn list_provider_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/providers").await
    }

    /// Simulate a remote route decision through ThinClaw's provider planner.
    pub async fn simulate_route(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.post_json("/api/providers/route/simulate", request)
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
        self.delete_json(&format!("/api/providers/{}/key", urlencoding::encode(slug)))
            .await
            .map(|_| ())
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
    /// The root gateway has history retrieval but no transcript export endpoint.
    pub async fn export_session(
        &self,
        session_key: &str,
        format: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = (session_key, format);
        Err(Self::unavailable(
            "session export",
            "the gateway has no transcript export endpoint",
        ))
    }

    // ── Extensions ───────────────────────────────────────────────────────────

    /// List all extensions.
    pub async fn list_extensions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/extensions").await
    }

    pub async fn activate_extension(&self, name: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/extensions/{}/activate", urlencoding::encode(name)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn remove_extension(&self, name: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/extensions/{}/remove", urlencoding::encode(name)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn list_tools(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/extensions/tools").await
    }

    // ── Settings / Config ────────────────────────────────────────────────────

    /// Get a config setting from the remote agent.
    pub async fn get_setting(&self, key: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/settings/{}", urlencoding::encode(key)))
            .await
    }

    /// List all non-sensitive config settings from the remote agent.
    pub async fn list_settings(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/settings").await
    }

    /// Set a config setting on the remote agent.
    pub async fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        let url = format!("/api/settings/{}", urlencoding::encode(key));
        let body = serde_json::json!({ "value": value });
        self.put_json(&url, &body).await.map(|_| ())
    }

    /// Legacy raw-secret injection is intentionally unavailable in remote mode.
    ///
    /// Remote credentials must move through the Provider Vault save/delete
    /// endpoints so the gateway stores them in its own secrets backend and only
    /// returns sanitized status metadata to Desktop.
    pub async fn inject_secrets(
        &self,
        _secrets: &std::collections::HashMap<String, String>,
    ) -> Result<u32, String> {
        Err(
            "unavailable: remote raw secret injection is disabled; use provider vault save/delete"
                .to_string(),
        )
    }

    // ── Diagnostics / Logs ───────────────────────────────────────────────────

    /// Get full diagnostics from the remote gateway.
    pub async fn get_diagnostics(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }

    // ── Jobs / Autonomy / Experiments / Learning / MCP ─────────────────────

    pub async fn get_jobs(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/jobs").await
    }

    pub async fn get_jobs_summary(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/jobs/summary").await
    }

    pub async fn get_job_detail(&self, job_id: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/jobs/{}", urlencoding::encode(job_id)))
            .await
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/jobs/{}/cancel", urlencoding::encode(job_id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn restart_job(&self, job_id: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/jobs/{}/restart", urlencoding::encode(job_id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn prompt_job(
        &self,
        job_id: &str,
        content: Option<String>,
        done: bool,
    ) -> Result<serde_json::Value, String> {
        let mut body = serde_json::Map::new();
        if let Some(content) = content {
            body.insert("content".to_string(), serde_json::Value::String(content));
        }
        body.insert("done".to_string(), serde_json::Value::Bool(done));
        self.post_json(
            &format!("/api/jobs/{}/prompt", urlencoding::encode(job_id)),
            &serde_json::Value::Object(body),
        )
        .await
    }

    pub async fn get_job_events(&self, job_id: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/jobs/{}/events", urlencoding::encode(job_id)))
            .await
    }

    pub async fn list_job_files(
        &self,
        job_id: &str,
        path: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let suffix = path
            .filter(|p| !p.is_empty())
            .map(|p| format!("?path={}", urlencoding::encode(p)))
            .unwrap_or_default();
        self.get_json(&format!(
            "/api/jobs/{}/files/list{}",
            urlencoding::encode(job_id),
            suffix
        ))
        .await
    }

    pub async fn read_job_file(
        &self,
        job_id: &str,
        path: &str,
    ) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/jobs/{}/files/read?path={}",
            urlencoding::encode(job_id),
            urlencoding::encode(path)
        ))
        .await
    }

    pub async fn get_autonomy_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/autonomy/status").await
    }

    pub async fn bootstrap_autonomy(&self) -> Result<serde_json::Value, String> {
        self.post_json("/api/autonomy/bootstrap", &serde_json::json!({}))
            .await
    }

    pub async fn pause_autonomy(
        &self,
        reason: Option<String>,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/autonomy/pause",
            &serde_json::json!({ "reason": reason }),
        )
        .await
    }

    pub async fn resume_autonomy(&self) -> Result<serde_json::Value, String> {
        self.post_json("/api/autonomy/resume", &serde_json::json!({}))
            .await
    }

    pub async fn get_autonomy_permissions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/autonomy/permissions").await
    }

    pub async fn rollback_autonomy(&self) -> Result<serde_json::Value, String> {
        self.post_json("/api/autonomy/rollback", &serde_json::json!({}))
            .await
    }

    pub async fn get_autonomy_rollouts(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/autonomy/rollouts").await
    }

    pub async fn get_autonomy_checks(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/autonomy/checks").await
    }

    pub async fn get_autonomy_evidence(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/autonomy/evidence").await
    }

    pub async fn get_learning_status(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/learning/status").await
    }

    pub async fn get_experiment_projects(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/experiments/projects").await
    }

    pub async fn get_mcp_servers(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/mcp/servers").await
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
    /// The ThinClaw gateway sends events in SSE format:
    ///   data: {"type":"stream_chunk","thread_id":"...","content":"..."}\n\n
    ///
    /// We normalize each `data:` line onto the desktop `UiEvent` contract and
    /// re-emit it on the same Tauri bus as local mode.
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

                        // Prefer UiEvent for remote gateways that already speak
                        // the desktop contract. Otherwise normalize ThinClaw
                        // gateway SSE (`type`) events into the same bus.
                        match serde_json::from_str::<crate::openclaw::ui_types::UiEvent>(data) {
                            Ok(event) => {
                                debug!("[remote_proxy] SSE event: {:?}", event);
                                if let Err(e) = app_handle.emit("openclaw-event", &event) {
                                    warn!("[remote_proxy] Failed to emit Tauri event: {}", e);
                                }
                            }
                            Err(_) => match serde_json::from_str::<serde_json::Value>(data) {
                                Ok(raw_json) => {
                                    for event in
                                        crate::openclaw::ironclaw_types::gateway_sse_to_ui_events(
                                            raw_json,
                                        )
                                    {
                                        debug!("[remote_proxy] normalized SSE event: {:?}", event);
                                        if let Err(e) = app_handle.emit("openclaw-event", &event) {
                                            warn!(
                                                "[remote_proxy] Failed to emit mapped gateway event: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("[remote_proxy] Failed to parse SSE data as JSON: {}", e)
                                }
                            },
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

#[cfg(test)]
mod tests {
    use super::RemoteGatewayProxy;

    #[test]
    fn unavailable_errors_are_explicitly_typed_by_prefix() {
        let message = RemoteGatewayProxy::unavailable("chat abort", "no endpoint");
        assert!(message.starts_with("unavailable:"));
        assert!(message.contains("chat abort"));
        assert!(message.contains("no endpoint"));
    }

    #[test]
    fn constructor_normalizes_trailing_slash() {
        let proxy = RemoteGatewayProxy::new("http://127.0.0.1:18789/", "token");
        assert_eq!(proxy.base_url(), "http://127.0.0.1:18789");
    }

    #[tokio::test]
    async fn raw_secret_injection_is_unavailable_in_remote_mode() {
        let proxy = RemoteGatewayProxy::new("http://127.0.0.1:18789", "token");
        let error = proxy
            .inject_secrets(&std::collections::HashMap::new())
            .await
            .expect_err("remote raw secret injection should stay disabled");

        assert!(error.starts_with("unavailable:"));
        assert!(error.contains("raw secret injection is disabled"));
        assert!(error.contains("provider vault save/delete"));
    }
}

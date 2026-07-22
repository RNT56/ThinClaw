//! MCP server config (DB with disk fallback), client lifecycle, roots-grant
//! sync watchers, pending-interaction surfacing, and MCP auth/activation.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

use crate::extensions::{ActivateResult, AuthResult, ExtensionError, ExtensionKind};
use crate::secrets::CreateSecretParams;
use crate::tools::mcp::auth::{is_authenticated, prepare_mcp_authorization};
use crate::tools::mcp::config::{McpConfigStore, McpServerConfig};
use crate::tools::mcp::{McpClient, McpPendingInteraction};

use super::ExtensionManager;
use super::core::{AuthRequestContext, PendingAuth};

const MCP_LOOP_STOP_TIMEOUT: Duration = Duration::from_secs(2);

impl ExtensionManager {
    // ── MCP config helpers (DB with disk fallback) ─────────────────────

    pub(super) async fn load_mcp_servers(
        &self,
    ) -> Result<crate::tools::mcp::config::McpServersFile, crate::tools::mcp::config::ConfigError>
    {
        self.mcp_config_store().load_servers().await
    }

    pub(super) async fn get_mcp_server(
        &self,
        name: &str,
    ) -> Result<McpServerConfig, crate::tools::mcp::config::ConfigError> {
        let servers = self.load_mcp_servers().await?;
        servers.get(name).cloned().ok_or_else(|| {
            crate::tools::mcp::config::ConfigError::ServerNotFound {
                name: name.to_string(),
            }
        })
    }

    pub(super) async fn add_mcp_server(
        &self,
        config: McpServerConfig,
    ) -> Result<(), crate::tools::mcp::config::ConfigError> {
        self.mcp_config_store().upsert_server(config).await
    }

    pub async fn list_mcp_server_configs(
        &self,
    ) -> Result<Vec<McpServerConfig>, crate::tools::mcp::config::ConfigError> {
        let mut servers = self.load_mcp_servers().await?.servers;
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn get_mcp_server_config(
        &self,
        name: &str,
    ) -> Result<McpServerConfig, crate::tools::mcp::config::ConfigError> {
        self.get_mcp_server(name).await
    }

    pub async fn upsert_mcp_server_config(
        &self,
        config: McpServerConfig,
    ) -> Result<(), crate::tools::mcp::config::ConfigError> {
        let name = config.name.clone();
        let _operation = self.mcp_operation_lock.lock().await;
        self.add_mcp_server(config).await?;
        self.discard_mcp_client_unlocked(&name).await;
        self.unregister_mcp_tools_unlocked(&name).await;
        Ok(())
    }

    pub async fn get_active_mcp_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.mcp_clients.read().await.get(name).cloned()
    }

    fn mcp_config_store(&self) -> McpConfigStore {
        McpConfigStore::new(self.store.clone(), self.user_id.clone())
    }

    async fn ensure_mcp_watcher(&self, name: &str, client: &Arc<McpClient>) {
        let mut watchers = self.mcp_watchers.write().await;
        if watchers.contains_key(name) {
            return;
        }

        let server_name = name.to_string();
        let config_store = self.mcp_config_store();
        let weak_client = Arc::downgrade(client);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::debug!(server = %server_name, "MCP roots-grant watcher stopped");
                        break;
                    }
                    _ = interval.tick() => {}
                }
                let Some(client) = weak_client.upgrade() else {
                    break;
                };
                let Ok(Some(config)) = config_store.get_server(&server_name).await else {
                    continue;
                };
                if client.update_roots_grants(config.roots_grants).await
                    && let Err(error) = client.notify_roots_list_changed().await
                {
                    tracing::debug!(
                        server = %server_name,
                        error = %error,
                        "Failed to notify MCP server about updated roots grants"
                    );
                }
            }
        });
        watchers.insert(name.to_string(), (handle, shutdown_tx));
    }

    pub(super) async fn stop_mcp_watcher(&self, name: &str) {
        if let Some((handle, shutdown_tx)) = self.mcp_watchers.write().await.remove(name) {
            let _ = shutdown_tx.send(());
            drain_mcp_task(handle, "mcp_roots_grant_watcher").await;
        }
    }

    /// Remove and stop a published MCP client so changed credentials or config
    /// cannot be masked by a stale authenticated transport.
    pub(super) async fn discard_mcp_client_unlocked(&self, name: &str) {
        self.stop_mcp_watcher(name).await;
        let client = self.mcp_clients.write().await.remove(name);
        if let Some(client) = client {
            client.shutdown().await;
        }
    }

    pub(super) async fn unregister_mcp_tools_unlocked(&self, name: &str) -> Vec<String> {
        let prefix = McpClient::registered_tool_prefix(name);
        let names = self
            .tool_registry
            .list()
            .await
            .into_iter()
            .filter(|tool_name| tool_name.starts_with(&prefix))
            .collect::<Vec<_>>();
        for tool_name in &names {
            self.tool_registry.unregister(tool_name).await;
        }
        names
    }

    pub async fn stop_mcp_background_tasks(&self) {
        self.stop_mcp_health_monitor().await;
        let _operation = self.mcp_operation_lock.lock().await;

        let watchers = {
            let mut guard = self.mcp_watchers.write().await;
            guard.drain().collect::<Vec<_>>()
        };
        for (_name, (handle, shutdown_tx)) in watchers {
            let _ = shutdown_tx.send(());
            drain_mcp_task(handle, "mcp_roots_grant_watcher").await;
        }

        let clients = {
            let mut clients = self.mcp_clients.write().await;
            clients
                .drain()
                .map(|(_, client)| client)
                .collect::<Vec<_>>()
        };
        let mut shutdowns = tokio::task::JoinSet::new();
        for client in clients {
            shutdowns.spawn(async move {
                client.shutdown().await;
            });
        }
        while let Some(result) = shutdowns.join_next().await {
            if let Err(error) = result {
                tracing::warn!(%error, "MCP client shutdown task failed");
            }
        }
    }

    /// Persist a health snapshot for a server, best-effort.
    ///
    /// This is what makes [`McpRuntimeHealth`] a live value instead of a
    /// write-only schema: every probe updates `connected`, `last_error`, and
    /// (on success) `last_connected_at`, which operator surfaces read back to
    /// show whether a configured server is actually reachable.
    async fn record_mcp_health(&self, name: &str, error: Option<String>) {
        let store = self.mcp_config_store();
        let Ok(Some(mut config)) = store.get_server(name).await else {
            return;
        };
        let connected = error.is_none();
        let last_connected_at = if connected {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            config
                .runtime_health
                .as_ref()
                .and_then(|h| h.last_connected_at.clone())
        };
        config.runtime_health = Some(crate::tools::mcp::config::McpRuntimeHealth {
            last_error: error,
            last_connected_at,
            connected,
        });
        if let Err(e) = store.upsert_server(config).await {
            tracing::debug!(server = %name, error = %e, "Failed to persist MCP runtime health");
        }
    }

    /// Probe every active MCP client once and persist a health snapshot.
    /// Returns the names of servers that failed their probe (does not reconnect).
    pub async fn probe_mcp_health(&self) -> Vec<String> {
        let clients: Vec<(String, Arc<McpClient>)> = self
            .mcp_clients
            .read()
            .await
            .iter()
            .map(|(name, client)| (name.clone(), Arc::clone(client)))
            .collect();

        let mut unhealthy = Vec::new();
        for (name, client) in clients {
            match client.health_check().await {
                Ok(()) => self.record_mcp_health(&name, None).await,
                Err(error) => {
                    self.record_mcp_health(&name, Some(error.to_string())).await;
                    unhealthy.push(name);
                }
            }
        }
        unhealthy
    }

    /// Rebuild a crashed MCP client and re-register its tools through the normal
    /// activation path. Returns `true` on success. Drops the old (dead) client
    /// first so `activate_mcp` builds a fresh one rather than reusing the crashed
    /// handle.
    pub async fn reconnect_mcp_server(&self, name: &str) -> bool {
        let _operation = self.mcp_operation_lock.lock().await;
        self.discard_mcp_client_unlocked(name).await;
        match self.activate_mcp_unlocked(name).await {
            Ok(_) => {
                tracing::info!(server = %name, "Reconnected MCP server after health failure");
                self.record_mcp_health(name, None).await;
                true
            }
            Err(error) => {
                tracing::warn!(server = %name, error = %error, "Failed to reconnect MCP server");
                self.record_mcp_health(name, Some(error.to_string())).await;
                false
            }
        }
    }

    /// Probe all active servers and immediately reconnect any that failed
    /// (no backoff). Operator-triggered convenience; the background monitor uses
    /// [`ExtensionManager::probe_mcp_health`] + [`ExtensionManager::reconnect_mcp_server`]
    /// with per-server backoff instead. Returns the names reconnected.
    pub async fn refresh_mcp_health(&self) -> Vec<String> {
        let mut reconnected = Vec::new();
        for name in self.probe_mcp_health().await {
            tracing::warn!(server = %name, "MCP server health check failed; attempting reconnect");
            if self.reconnect_mcp_server(&name).await {
                reconnected.push(name);
            }
        }
        reconnected
    }

    /// Spawn the background MCP health monitor. Probes all active servers on a
    /// fixed interval, persists [`McpRuntimeHealth`], and reconnects crashed
    /// servers with **per-server exponential backoff** so a permanently-broken
    /// server does not spawn a reconnect (and subprocess/connection) every tick
    /// forever. Crucially, a server whose reconnect fails stays in the retry set
    /// even though it is no longer in `mcp_clients`, so it is not silently
    /// dropped from the rotation. Idempotent: a second call is a no-op while a
    /// monitor is already running.
    pub async fn start_mcp_health_monitor(self: &Arc<Self>, interval: Duration) {
        let mut guard = self.mcp_health_monitor.write().await;
        if guard.is_some() {
            return;
        }
        let manager = Arc::downgrade(self);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            /// Backoff ceiling: never wait longer than this between attempts.
            const MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);

            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            ticker.tick().await; // consume the immediate first tick

            // Servers awaiting reconnect: name -> (consecutive_failures, next_attempt).
            let mut pending: std::collections::HashMap<String, (u32, std::time::Instant)> =
                std::collections::HashMap::new();

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::debug!("MCP health monitor stopped");
                        break;
                    }
                    _ = ticker.tick() => {}
                }
                let Some(manager) = manager.upgrade() else {
                    break;
                };

                // Newly-unhealthy active servers enter the pending set, due now.
                let now = std::time::Instant::now();
                for name in manager.probe_mcp_health().await {
                    pending.entry(name).or_insert((0, now));
                }

                // Attempt reconnects whose backoff has elapsed.
                let due: Vec<String> = pending
                    .iter()
                    .filter(|(_, (_, next))| std::time::Instant::now() >= *next)
                    .map(|(name, _)| name.clone())
                    .collect();
                for name in due {
                    if manager.reconnect_mcp_server(&name).await {
                        pending.remove(&name);
                    } else {
                        let (failures, next) = pending.entry(name).or_insert((0, now));
                        *failures = failures.saturating_add(1);
                        let backoff = interval
                            .saturating_mul(1u32 << (*failures).min(6))
                            .min(MAX_BACKOFF);
                        *next = std::time::Instant::now() + backoff;
                    }
                }
            }
        });
        *self.mcp_health_monitor_shutdown.write().await = Some(shutdown_tx);
        *guard = Some(handle);
    }

    pub async fn stop_mcp_health_monitor(&self) {
        let handle = self.mcp_health_monitor.write().await.take();
        if let Some(tx) = self.mcp_health_monitor_shutdown.write().await.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = handle {
            drain_mcp_task(handle, "mcp_health_monitor").await;
        }
    }

    async fn build_mcp_client(
        &self,
        server: &McpServerConfig,
    ) -> Result<Arc<McpClient>, ExtensionError> {
        let config_store = Some(self.mcp_config_store().into_inner());
        let client = if server.is_stdio() {
            McpClient::new_stdio_with_store(server, config_store)
                .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?
        } else {
            let has_tokens = is_authenticated(server, &self.secrets, &self.user_id).await;
            if has_tokens {
                McpClient::new_authenticated_with_store(
                    server.clone(),
                    Arc::clone(&self.mcp_session_manager),
                    Arc::clone(&self.secrets),
                    &self.user_id,
                    config_store,
                )
            } else {
                McpClient::new_configured_with_store(server.clone(), config_store)
            }
        };

        Ok(Arc::new(client))
    }

    pub async fn connect_mcp_client(&self, name: &str) -> Result<Arc<McpClient>, ExtensionError> {
        let _operation = self.mcp_operation_lock.lock().await;
        self.connect_mcp_client_unlocked(name).await
    }

    async fn connect_mcp_client_unlocked(
        &self,
        name: &str,
    ) -> Result<Arc<McpClient>, ExtensionError> {
        if let Some(client) = self.get_active_mcp_client(name).await {
            return Ok(client);
        }

        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;
        let candidate = self.build_mcp_client(&server).await?;
        let client = {
            let mut clients = self.mcp_clients.write().await;
            clients
                .entry(name.to_string())
                .or_insert_with(|| Arc::clone(&candidate))
                .clone()
        };
        if !Arc::ptr_eq(&client, &candidate) {
            // Concurrent callers may race through the initial read/build. Keep
            // the atomically published client and deterministically stop the
            // losing candidate so its stdio child cannot leak.
            candidate.shutdown().await;
        }
        self.ensure_mcp_watcher(name, &client).await;
        Ok(client)
    }

    pub async fn list_pending_mcp_interactions(&self) -> Vec<McpPendingInteraction> {
        let clients = self
            .mcp_clients
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut interactions = Vec::new();
        for client in clients {
            interactions.extend(client.pending_interactions().await);
        }
        interactions.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        interactions
    }

    pub async fn resolve_pending_mcp_interaction(
        &self,
        interaction_id: &str,
        approved: bool,
        result: Option<serde_json::Value>,
        message: Option<String>,
    ) -> Result<(), ExtensionError> {
        let clients = self
            .mcp_clients
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();

        for client in clients {
            if client
                .pending_interactions()
                .await
                .iter()
                .any(|pending| pending.id == interaction_id)
            {
                return client
                    .resolve_pending_interaction(interaction_id, approved, result, message)
                    .await
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()));
            }
        }

        Err(ExtensionError::Other(format!(
            "No active MCP interaction with id '{}'",
            interaction_id
        )))
    }

    pub(super) async fn remove_mcp_server(
        &self,
        name: &str,
    ) -> Result<(), crate::tools::mcp::config::ConfigError> {
        self.mcp_config_store().remove_server(name).await
    }

    // ── MCP auth/activation ───────────────────────────────────────────

    pub(super) async fn auth_mcp(
        &self,
        name: &str,
        token: Option<&str>,
        context: AuthRequestContext,
    ) -> Result<AuthResult, ExtensionError> {
        let _operation = self.mcp_operation_lock.lock().await;
        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;

        // If a token was provided directly, store it and we're done.
        if let Some(token_value) = token {
            if token_value.is_empty()
                || token_value.len() > 64 * 1024
                || token_value.chars().any(char::is_control)
            {
                return Err(ExtensionError::AuthFailed(
                    "MCP bearer token is empty, malformed, or oversized".to_string(),
                ));
            }
            let secret_name = server.token_secret_name();
            let params =
                CreateSecretParams::new(&secret_name, token_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            self.discard_mcp_client_unlocked(name).await;
            self.unregister_mcp_tools_unlocked(name).await;

            tracing::info!("MCP server '{}' authenticated via manual token", name);
            return Ok(self.auth_result(
                name,
                ExtensionKind::McpServer,
                "manual_token",
                "authenticated",
            ));
        }

        // Check if already authenticated
        if is_authenticated(&server, &self.secrets, &self.user_id).await {
            return Ok(self.auth_result(name, ExtensionKind::McpServer, "oauth", "authenticated"));
        }

        // Build a non-blocking transaction for the gateway callback route. The
        // CLI has its own loopback listener flow; extension-management requests
        // must return an authorization URL instead of occupying a request task
        // for up to five minutes.
        match self.auth_mcp_build_url(name, &server, context).await {
            Ok(result) => Ok(result),
            Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
                let mut result = self.auth_result(
                    name,
                    ExtensionKind::McpServer,
                    "manual_token",
                    "awaiting_token",
                );
                result.instructions = Some(format!(
                    "Server '{}' does not support OAuth. \
                     Please provide an API token/key for this server.",
                    name
                ));
                result.awaiting_token = true;
                Ok(result)
            }
            Err(e) => {
                // OAuth failed for some other reason, fall back to manual token
                let mut result = self.auth_result(
                    name,
                    ExtensionKind::McpServer,
                    "manual_token",
                    "awaiting_token",
                );
                result.instructions = Some(format!(
                    "OAuth failed for '{}': {}. \
                     Please provide an API token/key manually.",
                    name, e
                ));
                result.awaiting_token = true;
                Ok(result)
            }
        }
    }

    /// Build an auth URL for cases where non-interactive auth is needed
    /// (e.g., running via Telegram where we can't open a browser).
    async fn auth_mcp_build_url(
        &self,
        name: &str,
        server: &McpServerConfig,
        context: AuthRequestContext,
    ) -> Result<AuthResult, crate::tools::mcp::auth::AuthError> {
        let callback_base = self
            .resolve_callback_base_url(context.callback_base_url.as_deref())
            .await
            .ok_or_else(|| {
                crate::tools::mcp::auth::AuthError::DiscoveryFailed(
                    "No HTTPS (or loopback) OAuth callback URL is configured for this gateway"
                        .to_string(),
                )
            })?;
        let redirect_uri = format!("{}/oauth/callback", callback_base.trim_end_matches('/'));
        let state_nonce = uuid::Uuid::new_v4().to_string();
        let prepared = prepare_mcp_authorization(server, &redirect_uri, &state_nonce).await?;
        let auth_url = prepared.authorization_url().to_string();

        // Store pending auth for later callback handling
        self.insert_pending_auth(
            state_nonce.clone(),
            PendingAuth {
                name: name.to_string(),
                kind: ExtensionKind::McpServer,
                code_verifier: None,
                mcp_authorization: Some(prepared),
                redirect_uri: Some(redirect_uri.clone()),
                thread_id: context.thread_id.clone(),
                created_at: std::time::Instant::now(),
            },
        )
        .await;

        let mut result = self.auth_result(
            name,
            ExtensionKind::McpServer,
            "oauth",
            "awaiting_authorization",
        );
        result.auth_url = Some(auth_url);
        result.callback_type = Some(context.callback_type.unwrap_or_else(|| "web".to_string()));
        result.instructions = Some(format!("Connect {} to continue.", server.display_label()));
        Ok(result)
    }

    pub(super) async fn activate_mcp(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        let _operation = self.mcp_operation_lock.lock().await;
        self.activate_mcp_unlocked(name).await
    }

    pub(super) async fn activate_mcp_unlocked(
        &self,
        name: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        let client = self.connect_mcp_client_unlocked(name).await?;

        // Try to list and create tools
        let mcp_tools = match client.list_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                self.discard_mcp_client_unlocked(name).await;
                self.unregister_mcp_tools_unlocked(name).await;
                return Err(ExtensionError::ActivationFailed(error.to_string()));
            }
        };

        let tool_impls = match client.create_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                self.discard_mcp_client_unlocked(name).await;
                self.unregister_mcp_tools_unlocked(name).await;
                return Err(ExtensionError::ActivationFailed(error.to_string()));
            }
        };

        let tool_names: Vec<String> = mcp_tools
            .iter()
            .map(|t| McpClient::registered_tool_name(name, &t.name))
            .collect();

        let prefix = McpClient::registered_tool_prefix(name);
        let desired = tool_names
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let stale_tools = self
            .tool_registry
            .list()
            .await
            .into_iter()
            .filter(|tool_name| tool_name.starts_with(&prefix) && !desired.contains(tool_name))
            .collect::<Vec<_>>();

        for tool in tool_impls {
            let tool_name = tool.name().to_string();
            if !self.tool_registry.register(tool).await {
                self.discard_mcp_client_unlocked(name).await;
                self.unregister_mcp_tools_unlocked(name).await;
                return Err(ExtensionError::ActivationFailed(format!(
                    "MCP tool '{tool_name}' conflicts with a protected built-in tool name"
                )));
            }
        }
        for tool_name in stale_tools {
            self.tool_registry.unregister(&tool_name).await;
        }

        tracing::info!(
            "Activated MCP server '{}' with {} tools",
            name,
            tool_names.len()
        );

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            tools_loaded: tool_names,
            message: format!("Connected to '{}' and loaded tools", name),
        })
    }
}

async fn drain_mcp_task(mut handle: tokio::task::JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(task = name, error = %error, "MCP background task exited with error");
            }
        }
        _ = tokio::time::sleep(MCP_LOOP_STOP_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(task = name, "MCP background task did not drain before timeout; aborted");
        }
    }
}

//! MCP server config (DB with disk fallback), client lifecycle, roots-grant
//! sync watchers, pending-interaction surfacing, and MCP auth/activation.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::extensions::{ActivateResult, AuthResult, ExtensionError, ExtensionKind};
use crate::secrets::CreateSecretParams;
use crate::tools::mcp::auth::{
    PkceChallenge, authorize_mcp_server, build_authorization_url, discover_oauth_bundle,
    find_available_port, is_authenticated, register_client,
};
use crate::tools::mcp::config::{McpConfigStore, McpServerConfig};
use crate::tools::mcp::{McpClient, McpPendingInteraction};

use super::ExtensionManager;
use super::core::PendingAuth;

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
        self.add_mcp_server(config).await
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
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
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
        watchers.insert(name.to_string(), handle);
    }

    pub(super) async fn stop_mcp_watcher(&self, name: &str) {
        if let Some(handle) = self.mcp_watchers.write().await.remove(name) {
            handle.abort();
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
        if let Some(client) = self.get_active_mcp_client(name).await {
            return Ok(client);
        }

        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;
        let client = self.build_mcp_client(&server).await?;
        self.mcp_clients
            .write()
            .await
            .insert(name.to_string(), Arc::clone(&client));
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
    ) -> Result<AuthResult, ExtensionError> {
        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;

        // If a token was provided directly, store it and we're done.
        if let Some(token_value) = token {
            let secret_name = server.token_secret_name();
            let params =
                CreateSecretParams::new(&secret_name, token_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

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

        // Run the full OAuth flow (opens browser, waits for callback)
        match authorize_mcp_server(&server, &self.secrets, &self.user_id).await {
            Ok(_token) => {
                tracing::info!("MCP server '{}' authenticated via OAuth", name);
                Ok(self.auth_result(name, ExtensionKind::McpServer, "oauth", "authenticated"))
            }
            Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
                // Server doesn't support OAuth, try building a URL first
                match self.auth_mcp_build_url(name, &server).await {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        // No OAuth, no DCR: fall back to manual token entry
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
                }
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
    ) -> Result<AuthResult, ExtensionError> {
        // Try to discover OAuth metadata and build a URL the user can open manually
        let bundle = discover_oauth_bundle(&server.url)
            .await
            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        let metadata = bundle.authorization_server;

        // Try DCR if no client_id configured
        let (client_id, redirect_uri) = if let Some(ref oauth) = server.oauth {
            let port = find_available_port()
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            let redirect = format!("http://localhost:{}/callback", port.1);
            (oauth.client_id.clone(), redirect)
        } else if let Some(ref reg_endpoint) = metadata.registration_endpoint {
            let port = find_available_port()
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            let redirect = format!("http://localhost:{}/callback", port.1);

            let registration = register_client(reg_endpoint, &redirect)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            (registration.client_id, redirect)
        } else {
            return Err(ExtensionError::AuthFailed(
                "Server doesn't support OAuth or Dynamic Client Registration".to_string(),
            ));
        };

        // Generate a state nonce for CSRF protection
        let state_nonce = uuid::Uuid::new_v4().to_string();
        let pkce = PkceChallenge::generate();
        let auth_url = build_authorization_url(
            &metadata.authorization_endpoint,
            &client_id,
            &redirect_uri,
            &metadata.scopes_supported,
            Some(&pkce),
            Some(&state_nonce),
            Some(
                server
                    .oauth
                    .as_ref()
                    .and_then(|oauth| oauth.resource.as_deref())
                    .unwrap_or(&bundle.protected_resource.resource),
            ),
            &std::collections::HashMap::new(),
        );

        // Store pending auth for later callback handling
        self.pending_auth.write().await.insert(
            state_nonce.clone(),
            PendingAuth {
                name: name.to_string(),
                kind: ExtensionKind::McpServer,
                code_verifier: None,
                redirect_uri: Some(redirect_uri.clone()),
                thread_id: None,
                created_at: std::time::Instant::now(),
            },
        );

        let mut result = self.auth_result(
            name,
            ExtensionKind::McpServer,
            "oauth",
            "awaiting_authorization",
        );
        result.auth_url = Some(auth_url);
        result.callback_type = Some("local".to_string());
        Ok(result)
    }

    pub(super) async fn activate_mcp(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        let client = if let Some(existing) = self.get_active_mcp_client(name).await {
            existing
        } else {
            let server = self
                .get_mcp_server(name)
                .await
                .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;
            let client = self.build_mcp_client(&server).await?;
            self.mcp_clients
                .write()
                .await
                .insert(name.to_string(), Arc::clone(&client));
            self.ensure_mcp_watcher(name, &client).await;
            client
        };

        // Try to list and create tools
        let mcp_tools = client
            .list_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let tool_impls = client
            .create_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let tool_names: Vec<String> = mcp_tools
            .iter()
            .map(|t| McpClient::registered_tool_name(name, &t.name))
            .collect();

        for tool in tool_impls {
            self.tool_registry.register(tool).await;
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

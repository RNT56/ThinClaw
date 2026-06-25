//! Top-level lifecycle dispatchers that fan an operation out by
//! [`ExtensionKind`]: search, install, auth, activate, list, and remove.

use crate::extensions::{
    ActivateResult, AuthResult, ExtensionError, ExtensionKind, InstallResult, InstalledExtension,
    ResultSource, SearchResult,
};
use crate::tools::mcp::McpClient;
use crate::tools::mcp::auth::is_authenticated;
use crate::tools::wasm::{WasmToolAuthCheck, WasmToolAuthMode, WasmToolAuthStatus, discover_tools};
use thinclaw_tools::builtin::extension_tools as extension_tool_policy;

use super::ExtensionManager;
use super::core::{AuthRequestContext, extension_kind_from_tool_kind};

impl ExtensionManager {
    /// Search for extensions. If `discover` is true, also searches online.
    pub async fn search(
        &self,
        query: &str,
        discover: bool,
    ) -> Result<Vec<SearchResult>, ExtensionError> {
        let mut results = self.registry.search(query).await;

        if discover && results.is_empty() {
            tracing::info!("No built-in results for '{}', searching online...", query);
            let discovered = self.discovery.discover(query).await;

            if !discovered.is_empty() {
                // Cache for future lookups
                self.registry.cache_discovered(discovered.clone()).await;

                // Add to results
                for entry in discovered {
                    results.push(SearchResult {
                        entry,
                        source: ResultSource::Discovered,
                        validated: true,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Install an extension by name (from registry) or by explicit URL.
    pub async fn install(
        &self,
        name: &str,
        url: Option<&str>,
        kind_hint: Option<ExtensionKind>,
    ) -> Result<InstallResult, ExtensionError> {
        tracing::info!(extension = %name, url = ?url, kind = ?kind_hint, "Installing extension");
        Self::validate_extension_name(name)?;

        let kind_label = kind_hint
            .map(|k| format!("{:?}", k))
            .unwrap_or_else(|| "unknown".into());
        self.fire_lifecycle_event(
            crate::extensions::lifecycle_hooks::LifecycleEvent::Installing {
                name: name.to_string(),
                kind: kind_label,
            },
        )
        .await;

        // If we have a registry entry, use it (prefer kind_hint to resolve collisions)
        if let Some(entry) = self.registry.get_with_kind(name, kind_hint).await {
            let result = self.install_from_entry(&entry).await.map_err(|e| {
                tracing::error!(extension = %name, error = %e, "Extension install failed");
                e
            });
            match &result {
                Ok(_) => {
                    self.fire_lifecycle_event(
                        crate::extensions::lifecycle_hooks::LifecycleEvent::Installed {
                            name: name.to_string(),
                        },
                    )
                    .await;
                }
                Err(e) => {
                    self.fire_lifecycle_event(
                        crate::extensions::lifecycle_hooks::LifecycleEvent::Failed {
                            name: name.to_string(),
                            event: "install".into(),
                            reason: e.to_string(),
                        },
                    )
                    .await;
                }
            }
            return result;
        }

        // If a URL was provided, determine kind and install
        if let Some(url) = url {
            let kind = kind_hint.unwrap_or_else(|| {
                extension_kind_from_tool_kind(extension_tool_policy::infer_kind_from_url(url))
            });
            let result = match kind {
                ExtensionKind::McpServer => self.install_mcp_from_url(name, url).await,
                ExtensionKind::WasmTool => self.install_wasm_tool_from_url(name, url).await,
                ExtensionKind::WasmChannel => {
                    self.install_wasm_channel_from_url(name, url, None).await
                }
                // Native plugins are never installed from a URL. They are
                // operator-side artifacts: a signed manifest placed by hand into
                // an allowlisted directory, then discovered via a manifest scan.
                // We deliberately refuse a network download for native binaries.
                ExtensionKind::NativePlugin => Err(ExtensionError::InstallFailed(
                    "native plugins are not installed from a URL; place a signed manifest in an \
                     allowlisted directory (extensions.native_plugin_allowlist_dirs) and rescan"
                        .to_string(),
                )),
            }
            .map_err(|e| {
                tracing::error!(extension = %name, url = %url, error = %e, "Extension install from URL failed");
                e
            });
            match &result {
                Ok(_) => {
                    self.fire_lifecycle_event(
                        crate::extensions::lifecycle_hooks::LifecycleEvent::Installed {
                            name: name.to_string(),
                        },
                    )
                    .await;
                }
                Err(e) => {
                    self.fire_lifecycle_event(
                        crate::extensions::lifecycle_hooks::LifecycleEvent::Failed {
                            name: name.to_string(),
                            event: "install".into(),
                            reason: e.to_string(),
                        },
                    )
                    .await;
                }
            }
            return result;
        }

        let err = ExtensionError::NotFound(format!(
            "'{}' not found in registry. Try searching with discover:true or provide a URL.",
            name
        ));
        tracing::warn!(extension = %name, "Extension not found in registry");
        Err(err)
    }

    /// Authenticate an installed extension.
    pub async fn auth(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<AuthResult, ExtensionError> {
        self.auth_with_context(name, token, AuthRequestContext::default())
            .await
    }

    /// Authenticate an installed extension with callback/UI context.
    pub async fn auth_with_context(
        &self,
        name: &str,
        token: Option<&str>,
        context: AuthRequestContext,
    ) -> Result<AuthResult, ExtensionError> {
        // Clean up expired pending auths
        self.cleanup_expired_auths().await;

        // Determine what kind of extension this is
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::McpServer => self.auth_mcp(name, token).await,
            ExtensionKind::WasmTool => self.auth_wasm_tool(name, token, context).await,
            ExtensionKind::WasmChannel => self.auth_wasm_channel(name, token).await,
            // Native plugins authenticate via manifest signature verification at
            // load time, not via OAuth/token. There is no interactive auth step.
            ExtensionKind::NativePlugin => Ok(self.auth_result(
                name,
                ExtensionKind::NativePlugin,
                "none",
                "no_auth_required",
            )),
        }
    }

    /// Activate an installed (and optionally authenticated) extension.
    pub async fn activate(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        Self::validate_extension_name(name)?;
        let kind = self.determine_installed_kind(name).await?;

        self.fire_lifecycle_event(
            crate::extensions::lifecycle_hooks::LifecycleEvent::Activating {
                name: name.to_string(),
            },
        )
        .await;

        let result = match kind {
            ExtensionKind::McpServer => self.activate_mcp(name).await,
            ExtensionKind::WasmTool => self.activate_wasm_tool(name).await,
            ExtensionKind::WasmChannel => self.activate_wasm_channel(name).await,
            ExtensionKind::NativePlugin => self.activate_native_plugin(name).await,
        };

        match &result {
            Ok(r) => {
                self.fire_lifecycle_event(
                    crate::extensions::lifecycle_hooks::LifecycleEvent::Activated {
                        name: name.to_string(),
                        tools: r.tools_loaded.clone(),
                    },
                )
                .await;
            }
            Err(e) => {
                self.fire_lifecycle_event(
                    crate::extensions::lifecycle_hooks::LifecycleEvent::Failed {
                        name: name.to_string(),
                        event: "activate".into(),
                        reason: e.to_string(),
                    },
                )
                .await;
            }
        }

        result
    }

    /// List extensions with their status.
    ///
    /// When `include_available` is `true`, registry entries that are not yet
    /// installed are appended with `installed: false`.
    pub async fn list(
        &self,
        kind_filter: Option<ExtensionKind>,
        include_available: bool,
    ) -> Result<Vec<InstalledExtension>, ExtensionError> {
        let mut extensions = Vec::new();

        // List MCP servers
        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::McpServer) {
            match self.load_mcp_servers().await {
                Ok(servers) => {
                    for server in &servers.servers {
                        let authenticated =
                            is_authenticated(server, &self.secrets, &self.user_id).await;
                        let clients = self.mcp_clients.read().await;
                        let active = clients.contains_key(&server.name);

                        // Get tool names if active
                        let tools = if active {
                            let prefix = McpClient::registered_tool_prefix(&server.name);
                            self.tool_registry
                                .list()
                                .await
                                .into_iter()
                                .filter(|t| t.starts_with(&prefix))
                                .collect()
                        } else {
                            Vec::new()
                        };

                        extensions.push(InstalledExtension {
                            name: server.name.clone(),
                            kind: ExtensionKind::McpServer,
                            description: server.description.clone(),
                            url: Some(server.url.clone()),
                            authenticated,
                            auth_mode: "oauth".to_string(),
                            auth_status: if authenticated {
                                "authenticated".to_string()
                            } else {
                                "awaiting_authorization".to_string()
                            },
                            active,
                            tools,
                            needs_setup: false,
                            shared_auth_provider: None,
                            missing_scopes: Vec::new(),
                            installed: true,
                            activation_error: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to load MCP servers for listing: {}", e);
                }
            }
        }

        // List WASM tools
        if (kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmTool))
            && self.wasm_tools_dir.exists()
        {
            match discover_tools(&self.wasm_tools_dir).await {
                Ok(tools) => {
                    for (name, _discovered) in tools {
                        let active = self.tool_registry.has(&name).await;
                        let auth_state = self
                            .check_wasm_tool_auth_status(&name)
                            .await
                            .unwrap_or_else(|_| WasmToolAuthCheck::no_auth_required());

                        extensions.push(InstalledExtension {
                            name: name.clone(),
                            kind: ExtensionKind::WasmTool,
                            description: None,
                            url: None,
                            authenticated: matches!(
                                auth_state.auth_status,
                                WasmToolAuthStatus::Authenticated
                                    | WasmToolAuthStatus::NoAuthRequired
                            ),
                            auth_mode: auth_state.auth_mode.as_str().to_string(),
                            auth_status: auth_state.auth_status.as_str().to_string(),
                            active,
                            tools: if active { vec![name] } else { Vec::new() },
                            needs_setup: auth_state.auth_mode == WasmToolAuthMode::ManualToken,
                            shared_auth_provider: auth_state.shared_auth_provider,
                            missing_scopes: auth_state.missing_scopes,
                            installed: true,
                            activation_error: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to discover WASM tools for listing: {}", e);
                }
            }
        }

        // List WASM channels
        if (kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmChannel))
            && self.wasm_channels_dir.exists()
        {
            match crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await {
                Ok(channels) => {
                    let active_names = self.active_channel_names.read().await;
                    let errors = self.activation_errors.read().await;
                    for (name, _discovered) in channels {
                        let active = active_names.contains(&name);
                        let (authenticated, needs_setup) =
                            self.check_channel_auth_status(&name).await;
                        let activation_error = errors.get(&name).cloned();
                        extensions.push(InstalledExtension {
                            name,
                            kind: ExtensionKind::WasmChannel,
                            description: None,
                            url: None,
                            authenticated,
                            auth_mode: "secrets".to_string(),
                            auth_status: if authenticated {
                                "authenticated".to_string()
                            } else {
                                "awaiting_token".to_string()
                            },
                            active,
                            tools: Vec::new(),
                            needs_setup,
                            shared_auth_provider: None,
                            missing_scopes: Vec::new(),
                            installed: true,
                            activation_error,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to discover WASM channels for listing: {}", e);
                }
            }
        }

        // List native plugins (registered from a signed manifest scan; only
        // present when the operator opted in via allow_native_plugins).
        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::NativePlugin) {
            let native = self.native_plugins.read().await;
            for id in native.registered_ids() {
                let active = native.is_loaded(&id);
                extensions.push(InstalledExtension {
                    name: id.clone(),
                    kind: ExtensionKind::NativePlugin,
                    description: Some(
                        "Native dynamic-library plugin (in-process, full host privileges)"
                            .to_string(),
                    ),
                    url: None,
                    // Native plugins authenticate via manifest signature, which
                    // was verified before registration.
                    authenticated: true,
                    auth_mode: "none".to_string(),
                    auth_status: "no_auth_required".to_string(),
                    active,
                    tools: if active { vec![id.clone()] } else { Vec::new() },
                    needs_setup: false,
                    shared_auth_provider: None,
                    missing_scopes: Vec::new(),
                    installed: true,
                    activation_error: None,
                });
            }
        }

        // Append available-but-not-installed registry entries
        if include_available {
            let installed_names: std::collections::HashSet<(String, ExtensionKind)> = extensions
                .iter()
                .map(|e| (e.name.clone(), e.kind))
                .collect();

            for entry in self.registry.all_entries().await {
                if let Some(filter) = kind_filter
                    && entry.kind != filter
                {
                    continue;
                }
                if installed_names.contains(&(entry.name.clone(), entry.kind)) {
                    continue;
                }
                extensions.push(InstalledExtension {
                    name: entry.name,
                    kind: entry.kind,
                    description: Some(entry.description),
                    url: None,
                    authenticated: false,
                    auth_mode: "none".to_string(),
                    auth_status: "no_auth_required".to_string(),
                    active: false,
                    tools: Vec::new(),
                    needs_setup: false,
                    shared_auth_provider: None,
                    missing_scopes: Vec::new(),
                    installed: false,
                    activation_error: None,
                });
            }
        }

        Ok(extensions)
    }

    /// Remove an installed extension.
    pub async fn remove(&self, name: &str) -> Result<String, ExtensionError> {
        Self::validate_extension_name(name)?;
        let kind = self.determine_installed_kind(name).await?;

        self.fire_lifecycle_event(
            crate::extensions::lifecycle_hooks::LifecycleEvent::Uninstalling {
                name: name.to_string(),
            },
        )
        .await;

        let result: Result<String, ExtensionError> = match kind {
            ExtensionKind::McpServer => {
                // Unregister tools with this server's prefix
                let prefix = McpClient::registered_tool_prefix(name);
                let tool_names: Vec<String> = self
                    .tool_registry
                    .list()
                    .await
                    .into_iter()
                    .filter(|t| t.starts_with(&prefix))
                    .collect();

                for tool_name in &tool_names {
                    self.tool_registry.unregister(tool_name).await;
                }

                // Remove MCP client
                self.mcp_clients.write().await.remove(name);
                self.stop_mcp_watcher(name).await;

                // Remove from config
                self.remove_mcp_server(name)
                    .await
                    .map_err(|e| ExtensionError::Config(e.to_string()))?;

                Ok(format!(
                    "Removed MCP server '{}' and {} tool(s)",
                    name,
                    tool_names.len()
                ))
            }
            ExtensionKind::WasmTool => {
                // Unregister from tool registry
                self.tool_registry.unregister(name).await;

                // Unregister hooks registered from this plugin source.
                let removed_hooks = self
                    .unregister_hook_prefix(&format!("plugin.tool:{}::", name))
                    .await
                    + self
                        .unregister_hook_prefix(&format!("plugin.dev_tool:{}::", name))
                        .await;
                if removed_hooks > 0 {
                    tracing::info!(
                        extension = name,
                        removed_hooks = removed_hooks,
                        "Removed plugin hooks for WASM tool"
                    );
                }

                // Delete files
                let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));

                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }

                Ok(format!("Removed WASM tool '{}'", name))
            }
            ExtensionKind::WasmChannel => {
                // Delete channel files
                let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));

                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }

                Ok(format!(
                    "Removed channel '{}'. The hot-reload watcher will unload it automatically.",
                    name
                ))
            }
            ExtensionKind::NativePlugin => {
                // Unload the in-process runtime if loaded. We intentionally do
                // NOT delete the operator-placed manifest/artifact from disk —
                // removal here means "stop running it", not "delete operator
                // files". To stop it permanently, the operator removes the
                // manifest from the allowlisted directory.
                let unloaded = self.native_plugins.write().await.deactivate(name);
                if unloaded {
                    Ok(format!("Unloaded native plugin '{}'", name))
                } else {
                    Ok(format!("Native plugin '{}' was not loaded", name))
                }
            }
        };

        match &result {
            Ok(_) => {
                self.fire_lifecycle_event(
                    crate::extensions::lifecycle_hooks::LifecycleEvent::Uninstalled {
                        name: name.to_string(),
                    },
                )
                .await;
            }
            Err(e) => {
                self.fire_lifecycle_event(
                    crate::extensions::lifecycle_hooks::LifecycleEvent::Failed {
                        name: name.to_string(),
                        event: "remove".into(),
                        reason: e.to_string(),
                    },
                )
                .await;
            }
        }

        result
    }
}

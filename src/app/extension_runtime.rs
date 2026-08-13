//! Extension, MCP, and host-tool initialization for AppBuilder.

use super::*;
#[cfg(feature = "wasm-runtime")]
use crate::config::WasmConfigExt as _;

impl AppBuilder {
    /// Phase 5: Load WASM tools, MCP servers, and create extension manager.
    pub async fn init_extensions(
        &self,
        tools: &Arc<ToolRegistry>,
        safety: &Arc<SafetyLayer>,
        hooks: &Arc<HookRegistry>,
    ) -> Result<
        (
            Arc<McpSessionManager>,
            Option<Arc<WasmToolRuntime>>,
            Option<Arc<ExtensionManager>>,
            Vec<crate::extensions::RegistryEntry>,
            Vec<String>,
            Option<Arc<crate::desktop_autonomy::DesktopAutonomyManager>>,
        ),
        anyhow::Error,
    > {
        use crate::tools::mcp::{McpClient, config::load_mcp_servers_from_db, is_authenticated};
        #[cfg(feature = "wasm-runtime")]
        use crate::tools::wasm::{WasmToolLoader, load_dev_tools};

        let mcp_session_manager = Arc::new(McpSessionManager::new());

        // Create WASM tool runtime
        #[cfg(feature = "wasm-runtime")]
        let wasm_tool_runtime: Option<Arc<WasmToolRuntime>> =
            if self.config.wasm.enabled && self.config.wasm.tools_dir.exists() {
                match WasmToolRuntime::new(self.config.wasm.to_runtime_config()) {
                    Ok(runtime) => Some(Arc::new(runtime)),
                    Err(e) => {
                        tracing::warn!("Failed to initialize WASM runtime: {}", e);
                        None
                    }
                }
            } else {
                None
            };
        #[cfg(not(feature = "wasm-runtime"))]
        let wasm_tool_runtime: Option<Arc<WasmToolRuntime>> = None;

        let wasm_tool_invoker = Arc::new(crate::tools::execution::HostMediatedToolInvoker::new(
            Arc::clone(tools),
            Arc::clone(safety),
            crate::tools::ToolExecutionLane::WorkerRuntime,
            crate::tools::ToolProfile::ExplicitOnly,
        ));

        // Load WASM tools and MCP servers concurrently
        #[cfg(feature = "wasm-runtime")]
        let wasm_tools_future = {
            let wasm_tool_runtime = wasm_tool_runtime.clone();
            let secrets_store = self.secrets_store.clone();
            let tools = Arc::clone(tools);
            let tool_invoker = Arc::clone(&wasm_tool_invoker);
            let wasm_config = self.config.wasm.clone();
            async move {
                let mut dev_loaded_tool_names: Vec<String> = Vec::new();

                if let Some(ref runtime) = wasm_tool_runtime {
                    let mut loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&tools));
                    loader = loader.with_tool_invoker(Arc::clone(&tool_invoker));
                    if let Some(ref secrets) = secrets_store {
                        loader = loader.with_secrets_store(Arc::clone(secrets));
                    }

                    match loader.load_from_dir(&wasm_config.tools_dir).await {
                        Ok(results) => {
                            if !results.loaded.is_empty() {
                                tracing::info!(
                                    "Loaded {} WASM tools from {}",
                                    results.loaded.len(),
                                    wasm_config.tools_dir.display()
                                );
                            }
                            for (path, err) in &results.errors {
                                tracing::warn!(
                                    "Failed to load WASM tool {}: {}",
                                    path.display(),
                                    err
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to scan WASM tools directory: {}", e);
                        }
                    }

                    match load_dev_tools(&loader, &wasm_config.tools_dir).await {
                        Ok(results) => {
                            dev_loaded_tool_names.extend(results.loaded.iter().cloned());
                            if !dev_loaded_tool_names.is_empty() {
                                tracing::info!(
                                    "Loaded {} dev WASM tools from build artifacts",
                                    dev_loaded_tool_names.len()
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!("No dev WASM tools found: {}", e);
                        }
                    }
                }

                dev_loaded_tool_names
            }
        };
        #[cfg(not(feature = "wasm-runtime"))]
        let wasm_tools_future = async { Vec::<String>::new() };

        let mcp_servers_future = {
            let secrets_store = self.secrets_store.clone();
            let db = self.db.clone();
            let tools = Arc::clone(tools);
            let mcp_sm = Arc::clone(&mcp_session_manager);
            async move {
                let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> = if let Some(
                    ref secrets,
                ) =
                    secrets_store
                {
                    Arc::clone(secrets)
                } else {
                    use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
                    let ephemeral_key = secrecy::SecretString::from(
                        crate::platform::secure_store::generate_master_key_hex(),
                    );
                    let crypto = match SecretsCrypto::new(ephemeral_key) {
                        Ok(crypto) => Arc::new(crypto),
                        Err(error) => {
                            tracing::error!(%error, "failed to initialize ephemeral MCP secrets crypto");
                            return;
                        }
                    };
                    tracing::debug!(
                        "Using ephemeral in-memory secrets store for startup MCP loading"
                    );
                    Arc::new(InMemorySecretsStore::new(crypto))
                };

                let servers_result = if let Some(ref d) = db {
                    load_mcp_servers_from_db(d.as_ref(), "default").await
                } else {
                    crate::tools::mcp::config::load_mcp_servers().await
                };
                match servers_result {
                    Ok(servers) => {
                        let enabled: Vec<_> = servers.enabled_servers().cloned().collect();
                        if !enabled.is_empty() {
                            tracing::info!("Loading {} configured MCP server(s)...", enabled.len());
                        }

                        let mut join_set = tokio::task::JoinSet::new();
                        for server in enabled {
                            let mcp_sm = Arc::clone(&mcp_sm);
                            let secrets = Arc::clone(&secrets);
                            let tools = Arc::clone(&tools);
                            let config_store = crate::tools::mcp::config::McpConfigStore::new(
                                db.clone(),
                                "default",
                            )
                            .into_inner();

                            join_set.spawn(async move {
                                let server_name = server.name.clone();

                                let client = if server.is_stdio() {
                                    let secret_env = match crate::tools::mcp::config::resolve_mcp_secret_environment(
                                        &server,
                                        &secrets,
                                        "default",
                                    )
                                    .await
                                    {
                                        Ok(secret_env) => secret_env,
                                        Err(error) => {
                                            tracing::warn!(
                                                "Failed to resolve stdio MCP credentials for '{}': {}",
                                                server_name,
                                                error
                                            );
                                            return;
                                        }
                                    };
                                    match McpClient::new_stdio_with_store_and_secret_env(
                                        &server,
                                        Some(config_store.clone()),
                                        &secret_env,
                                    ) {
                                        Ok(c) => c,
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to spawn stdio MCP server '{}': {}",
                                                server_name,
                                                e
                                            );
                                            return;
                                        }
                                    }
                                } else {
                                    let has_tokens =
                                        is_authenticated(&server, &secrets, "default").await;

                                    if has_tokens || server.requires_auth() {
                                        McpClient::new_authenticated_with_store(
                                            server,
                                            mcp_sm,
                                            secrets,
                                            "default",
                                            Some(config_store.clone()),
                                        )
                                    } else {
                                        McpClient::new_configured_with_store(
                                            server.clone(),
                                            Some(config_store.clone()),
                                        )
                                    }
                                };

                                match client.list_tools().await {
                                    Ok(mcp_tools) => {
                                        let tool_count = mcp_tools.len();
                                        match client.create_tools().await {
                                            Ok(tool_impls) => {
                                                let source_id = format!("mcp/{server_name}");
                                                let requests = tool_impls
                                                    .into_iter()
                                                    .map(|tool| {
                                                        crate::tools::RegistrationRequest::new(
                                                            tool,
                                                            crate::tools::ToolOrigin::Mcp,
                                                            source_id.clone(),
                                                        )
                                                    })
                                                    .collect();
                                                if let Err(conflict) =
                                                    tools.register_batch(requests)
                                                {
                                                    tracing::warn!(
                                                        server = %server_name,
                                                        tool = %conflict.name,
                                                        reason = %conflict.reason,
                                                        "Rejected entire MCP startup activation"
                                                    );
                                                    return;
                                                }
                                                tracing::info!(
                                                    "Loaded {} tools from MCP server '{}'",
                                                    tool_count,
                                                    server_name
                                                );
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    "Failed to create tools from MCP server '{}': {}",
                                                    server_name,
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let err_str = e.to_string();
                                        if err_str.contains("401")
                                            || err_str.contains("authentication")
                                        {
                                            tracing::warn!(
                                                "MCP server '{}' requires authentication. \
                                                 Run: thinclaw extensions mcp server auth {}",
                                                server_name,
                                                server_name
                                            );
                                        } else {
                                            tracing::warn!(
                                                "Failed to connect to MCP server '{}': {}",
                                                server_name,
                                                e
                                            );
                                        }
                                    }
                                }
                            });
                        }

                        while let Some(result) = join_set.join_next().await {
                            if let Err(e) = result {
                                tracing::warn!("MCP server loading task panicked: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("No MCP servers configured ({})", e);
                    }
                }
            }
        };

        let (dev_loaded_tool_names, _) = tokio::join!(wasm_tools_future, mcp_servers_future);

        // Load registry catalog entries for extension discovery
        let catalog_entries = match crate::registry::RegistryCatalog::load_or_embedded() {
            Ok(catalog) => {
                let entries: Vec<_> = catalog
                    .all()
                    .iter()
                    .map(|m| m.to_registry_entry())
                    .collect();
                tracing::info!(
                    count = entries.len(),
                    "Loaded registry catalog entries for extension discovery"
                );
                entries
            }
            Err(e) => {
                tracing::warn!("Failed to load registry catalog: {}", e);
                Vec::new()
            }
        };

        // Create extension manager. Use ephemeral in-memory secrets if no
        // persistent store is configured (listing/install/activate still work).
        let ext_secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> = if let Some(ref s) =
            self.secrets_store
        {
            Arc::clone(s)
        } else {
            use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
            let ephemeral_key = secrecy::SecretString::from(
                crate::platform::secure_store::generate_master_key_hex(),
            );
            let crypto = Arc::new(SecretsCrypto::new(ephemeral_key).map_err(|error| {
                anyhow::anyhow!("failed to initialize ephemeral extension secrets crypto: {error}")
            })?);
            tracing::debug!("Using ephemeral in-memory secrets store for extension manager");
            Arc::new(InMemorySecretsStore::new(crypto))
        };
        let extension_manager = {
            let manager = Arc::new(ExtensionManager::new(
                Arc::clone(&mcp_session_manager),
                ext_secrets,
                Arc::clone(tools),
                Some(Arc::clone(&wasm_tool_invoker)),
                Some(Arc::clone(hooks)),
                wasm_tool_runtime.clone(),
                self.config.wasm.tools_dir.clone(),
                self.config.channels.wasm_channels_dir.clone(),
                "default".to_string(),
                self.db.clone(),
                catalog_entries.clone(),
            ));
            tools.register_extension_tools(Arc::clone(&manager));
            tracing::info!("Extension manager initialized with in-chat discovery tools");

            // Native dynamic-library plugins are default-off and signature-gated.
            // Register any signed manifests from operator-configured allowlist dirs
            // (no-op unless `allow_native_plugins` is enabled; registration loads no
            // code — the signature-checked dlopen only happens on explicit activation).
            let _ = manager.register_native_plugins_from_allowlist().await;

            // Background MCP health monitor: probes active servers, persists
            // McpRuntimeHealth, and auto-reconnects crashed stdio servers.
            manager
                .start_mcp_health_monitor(std::time::Duration::from_secs(30))
                .await;

            Some(manager)
        };

        // register_builder_tool() now registers dev tools with the correct workspace dirs
        // internally (sandbox/project/unrestricted). Only register here when builder is off.
        let tool_plan = self.tool_runtime_assembly_plan();
        if let Some(dev_workspace) = tool_plan.dev_tools_workspace.clone() {
            if let Some(dir) = dev_workspace.create_dir.as_ref() {
                let _ = std::fs::create_dir_all(dir);
            }
            tracing::info!(
                workspace_mode = tool_plan.workspace_mode.as_config_value(),
                base_dir = dev_workspace
                    .base_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".to_string()),
                working_dir = dev_workspace
                    .working_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".to_string()),
                "[app] Development tool workspace resolved"
            );
            let sandbox =
                matches!(tool_plan.workspace_mode, RuntimeWorkspaceMode::Sandboxed).then(|| {
                    Arc::new(self.build_sandbox_manager(self.config.sandbox.to_sandbox_config()))
                });
            let sandbox_policy = sandbox
                .as_ref()
                .map(|_| crate::sandbox::SandboxPolicy::WorkspaceWrite);
            tools.register_dev_tools_with_runtime(
                dev_workspace.base_dir,
                dev_workspace.working_dir,
                Some(&self.config.safety),
                sandbox,
                sandbox_policy,
            );
        }

        // Register host device tools only after explicit user opt-in.
        let desktop_autonomy_blocker = desktop_autonomy_headless_blocker();
        let screen_capture_enabled = crate::platform::env_flag_enabled("SCREEN_CAPTURE_ENABLED");
        let reckless_desktop_capture = self.config.desktop_autonomy.is_reckless_enabled()
            && self.config.desktop_autonomy.capture_evidence
            && desktop_autonomy_blocker.is_none();
        if self.config.agent.allow_local_tools
            && desktop_autonomy_blocker.is_none()
            && (screen_capture_enabled || reckless_desktop_capture)
        {
            use crate::tools::builtin::ScreenCaptureTool;
            tools.register_sync(Arc::new(ScreenCaptureTool::new()));
            tracing::info!("Registered screen capture tool (enabled via user toggle)");
        } else if self.config.agent.allow_local_tools
            && screen_capture_enabled
            && desktop_autonomy_blocker.is_some()
        {
            tracing::warn!(
                runtime_profile = desktop_autonomy_blocker.unwrap_or("unknown"),
                "Screen capture requested but blocked by headless runtime profile"
            );
        }
        if self.config.agent.allow_local_tools
            && crate::platform::env_flag_enabled("CAMERA_CAPTURE_ENABLED")
        {
            use crate::tools::builtin::CameraCaptureTool;
            tools.register_sync(Arc::new(CameraCaptureTool::new()));
            tracing::info!("Registered camera capture tool (enabled via user toggle)");
        }
        if self.config.agent.allow_local_tools
            && crate::platform::env_flag_enabled("TALK_MODE_ENABLED")
        {
            tools.register_sync(Arc::new(crate::talk_mode::TalkModeTool::new()));
            tracing::info!("Registered talk mode tool (enabled via user toggle)");
        }
        if self.config.agent.allow_local_tools
            && crate::platform::env_flag_enabled("LOCATION_ENABLED")
        {
            use crate::tools::builtin::LocationTool;
            tools.register_sync(Arc::new(LocationTool::new()));
            tracing::info!("Registered location tool (enabled via user toggle)");
        }

        let desktop_autonomy_manager = if self.config.desktop_autonomy.is_reckless_enabled()
            && desktop_autonomy_blocker.is_none()
        {
            let manager = Arc::new(crate::desktop_autonomy::DesktopAutonomyManager::new(
                self.config.desktop_autonomy.clone(),
                Some(self.config.database.clone()),
                self.db.clone(),
            ));
            crate::desktop_autonomy::install_global_manager(Some(Arc::clone(&manager)));
            tools.register_desktop_autonomy_tools(Arc::clone(&manager));
            tracing::info!(
                deployment_mode = manager.config().deployment_mode.as_str(),
                "Reckless desktop autonomy manager initialized"
            );
            Some(manager)
        } else {
            if self.config.desktop_autonomy.is_reckless_enabled() {
                tracing::warn!(
                    runtime_profile = desktop_autonomy_blocker.unwrap_or("unknown"),
                    "Desktop autonomy requested but blocked by headless runtime profile"
                );
            }
            crate::desktop_autonomy::install_global_manager(None);
            None
        };

        // Hermes-parity runtime tools.
        tools.register_todo_tool(crate::tools::builtin::new_shared_todo_store());

        let tool_plan = self.tool_runtime_assembly_plan();
        if tool_plan.local_tools_enabled {
            let process_registry: crate::tools::builtin::SharedProcessRegistry =
                Arc::new(tokio::sync::RwLock::new(Default::default()));
            let sandbox_backend =
                Arc::new(self.build_sandbox_manager(self.config.sandbox.to_sandbox_config()));

            match tool_plan.process_registration {
                RuntimeExecRegistrationMode::LocalHost => {
                    crate::tools::builtin::start_reaper(Arc::clone(&process_registry));
                    tools.register_process_tool(process_registry);
                }
                RuntimeExecRegistrationMode::Disabled => {
                    tracing::info!(
                        workspace_mode = tool_plan.workspace_mode.as_config_value(),
                        "Background process tool disabled in restricted workspace mode"
                    );
                }
                RuntimeExecRegistrationMode::DockerSandbox => {
                    tracing::warn!(
                        workspace_mode = tool_plan.workspace_mode.as_config_value(),
                        "Background process tool is unavailable for Docker sandbox mode"
                    );
                }
            }

            let workspace_dir = tool_plan
                .search_files_workspace
                .as_ref()
                .and_then(|plan| plan.working_dir.clone().or_else(|| plan.base_dir.clone()));
            if let Some(plan) = tool_plan.search_files_workspace.as_ref()
                && let Some(dir) = plan.create_dir.as_ref()
            {
                let _ = std::fs::create_dir_all(dir);
            }

            match tool_plan.execute_code_registration {
                RuntimeExecRegistrationMode::DockerSandbox => {
                    let backend =
                        crate::tools::execution_backend::DockerSandboxExecutionBackend::from_sandbox(
                            Arc::clone(&sandbox_backend),
                            crate::sandbox::SandboxPolicy::WorkspaceWrite,
                        );
                    tools.register_execute_code_tool_with_backend(
                        workspace_dir.clone(),
                        false,
                        Some(backend),
                    );
                }
                RuntimeExecRegistrationMode::Disabled => {
                    tracing::info!(
                        workspace_mode = tool_plan.workspace_mode.as_config_value(),
                        sandbox_enabled = self.config.sandbox.enabled,
                        "execute_code disabled by runtime assembly policy"
                    );
                }
                RuntimeExecRegistrationMode::LocalHost => {
                    tools.register_execute_code_tool(workspace_dir.clone(), false);
                }
            }
            tools.register_search_files_tool(workspace_dir);
        }

        // Register TTS tool (always available — uses OpenAI TTS API)
        let tts_output_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("thinclaw")
            .join("tts");
        let tts_secrets = self.secrets_store.clone();
        tools.register_tts_tool(tts_secrets, tts_output_dir);

        tools.register_comfyui_tools(self.config.comfyui.clone(), self.secrets_store.clone());
        if !self.config.comfyui.enabled {
            tracing::info!(
                "ComfyUI generation starts disabled; image_generate will report setup guidance until enabled"
            );
        }

        // Register Apple Mail tool if on macOS and Apple Mail channel is configured
        #[cfg(target_os = "macos")]
        if self.config.channels.apple_mail.is_some() {
            tools.register_apple_mail_tool(None); // auto-detect Envelope Index path
        }

        Ok((
            mcp_session_manager,
            wasm_tool_runtime,
            extension_manager,
            catalog_entries,
            dev_loaded_tool_names,
            desktop_autonomy_manager,
        ))
    }
}

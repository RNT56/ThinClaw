//! Application builder for initializing core ThinClaw components.
//!
//! Extracts the mechanical initialization phases from `main.rs` into a
//! reusable builder so that:
//!
//! - Tests can construct a full `AppComponents` without wiring channels
//! - Main stays focused on CLI dispatch and channel setup
//! - Each init phase is independently testable

use std::sync::Arc;

use crate::channels::web::log_layer::LogBroadcaster;
use crate::config::Config;
use crate::context::ContextManager;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::extensions::lifecycle_hooks::AuditLogHook;
use crate::hardware_bridge::{SessionApprovals, ToolBridge};
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::llm::cost_tracker::{BudgetConfig, CostTracker};
use crate::llm::response_cache_ext::{CacheConfig, CachedResponseStore};
use crate::llm::usage_tracking::UsageTrackingProvider;
use crate::llm::{LlmRuntimeManager, normalize_providers_settings};
use crate::safety::SafetyLayer;
use crate::secrets::SecretsStore;
use crate::skills::SkillRegistry;
use crate::skills::catalog::SkillCatalog;
use crate::tools::ToolRegistry;
use crate::tools::mcp::McpSessionManager;
use crate::tools::wasm::SharedCredentialRegistry;
use crate::tools::wasm::WasmToolRuntime;
use crate::workspace::{EmbeddingProvider, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeExecRegistrationMode {
    Disabled,
    LocalHost,
    DockerSandbox,
}

fn process_registration_mode(workspace_mode: &str) -> RuntimeExecRegistrationMode {
    match workspace_mode {
        "sandboxed" | "project" => RuntimeExecRegistrationMode::Disabled,
        _ => RuntimeExecRegistrationMode::LocalHost,
    }
}

fn execute_code_registration_mode(
    workspace_mode: &str,
    sandbox_enabled: bool,
) -> RuntimeExecRegistrationMode {
    match workspace_mode {
        "sandboxed" if sandbox_enabled => RuntimeExecRegistrationMode::DockerSandbox,
        "sandboxed" | "project" => RuntimeExecRegistrationMode::Disabled,
        _ => RuntimeExecRegistrationMode::LocalHost,
    }
}

fn desktop_autonomy_headless_blocker() -> Option<&'static str> {
    let runtime_profile = std::env::var("THINCLAW_RUNTIME_PROFILE").unwrap_or_default();
    desktop_autonomy_headless_blocker_for(
        runtime_profile.trim(),
        crate::platform::env_flag_enabled("THINCLAW_HEADLESS"),
    )
}

fn desktop_autonomy_headless_blocker_for(
    runtime_profile: &str,
    headless_enabled: bool,
) -> Option<&'static str> {
    let normalized_profile = runtime_profile
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    match normalized_profile.as_str() {
        "pi" | "pi-os-lite" | "pi-os-lite-64" | "raspberry-pi-os-lite" => Some("pi-os-lite-64"),
        _ if headless_enabled => Some("headless"),
        _ => None,
    }
}

/// Fully initialized application components, ready for channel wiring
/// and agent construction.
pub struct AppComponents {
    /// The (potentially mutated) config after DB reload and secret injection.
    pub config: Config,
    pub db: Option<Arc<dyn Database>>,
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    pub llm_runtime: Arc<LlmRuntimeManager>,
    pub oauth_credential_sync: Option<crate::llm::OAuthCredentialSyncHandle>,
    pub llm: Arc<dyn LlmProvider>,
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub embeddings: Option<Arc<dyn EmbeddingProvider>>,
    pub workspace: Option<Arc<Workspace>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub mcp_session_manager: Arc<McpSessionManager>,
    pub wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    pub log_broadcaster: Arc<LogBroadcaster>,
    pub context_manager: Arc<ContextManager>,
    pub hooks: Arc<HookRegistry>,
    pub skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<SkillCatalog>>,
    pub skill_remote_hub: Option<crate::skills::SharedRemoteSkillHub>,
    pub skill_quarantine: Option<Arc<crate::skills::quarantine::QuarantineManager>>,
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    pub catalog_entries: Vec<crate::extensions::RegistryEntry>,
    pub dev_loaded_tool_names: Vec<String>,
    /// Hardware bridge for sensor access (camera, mic, screen).
    /// Present when running inside a host (Scrappy) that provides sensor capture.
    pub tool_bridge: Option<Arc<dyn ToolBridge>>,
    /// Session-level sensor approvals (cleared on restart).
    pub session_approvals: Arc<SessionApprovals>,
    /// Shared cost tracker — populated by every LLM call, read by Tauri command.
    pub cost_tracker: Arc<tokio::sync::Mutex<CostTracker>>,
    /// Audit log hook — receives real plugin lifecycle events from HookRegistry.
    pub audit_hook: Arc<AuditLogHook>,
    /// Shared response cache — populated by Reasoning, read by `openclaw_cache_stats`.
    pub response_cache: Arc<tokio::sync::RwLock<CachedResponseStore>>,
    /// Live smart routing policy owned by the runtime manager.
    pub routing_policy: Arc<std::sync::RwLock<crate::llm::routing_policy::RoutingPolicy>>,
}

/// Options that control optional init phases.
#[derive(Default)]
pub struct AppBuilderFlags {
    pub no_db: bool,
}

/// Builder that orchestrates the 5 mechanical init phases.
pub struct AppBuilder {
    config: Config,
    flags: AppBuilderFlags,
    toml_path: Option<std::path::PathBuf>,
    log_broadcaster: Arc<LogBroadcaster>,

    // Accumulated state
    db: Option<Arc<dyn Database>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,

    // Hardware bridge for sensor access (injected by Scrappy)
    tool_bridge: Option<Arc<dyn ToolBridge>>,

    // Backend-specific handles needed by secrets store
    #[cfg(feature = "postgres")]
    pg_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    libsql_db: Option<Arc<libsql::Database>>,

    // Multi-provider cloud intelligence settings (from Settings)
    providers_settings: Option<crate::settings::ProvidersSettings>,
}

impl AppBuilder {
    /// Create a new builder.
    ///
    /// The `log_broadcaster` is created before the builder because tracing
    /// must be initialized before any init phase runs, and the log broadcaster
    /// is part of the tracing layer.
    pub fn new(
        config: Config,
        flags: AppBuilderFlags,
        toml_path: Option<std::path::PathBuf>,
        log_broadcaster: Arc<LogBroadcaster>,
    ) -> Self {
        Self {
            config,
            flags,
            toml_path,
            log_broadcaster,
            db: None,
            secrets_store: None,
            tool_bridge: None,
            #[cfg(feature = "postgres")]
            pg_pool: None,
            #[cfg(feature = "libsql")]
            libsql_db: None,
            providers_settings: None,
        }
    }

    /// Inject a pre-built secrets store (e.g. from Scrappy's Keychain backend).
    ///
    /// When set, [`init_secrets()`] will use this store directly instead of
    /// creating one from the master key + database handles. Keys will still
    /// be injected into the config overlay.
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Inject multi-provider cloud intelligence settings.
    ///
    /// When set, `init_llm()` will create a multi-provider chain with
    /// failover and smart routing based on these settings.
    /// In Scrappy mode, these come from the Cloud Intelligence UI.
    /// In headless mode, they come from config.toml / DB settings.
    pub fn with_providers_settings(mut self, settings: crate::settings::ProvidersSettings) -> Self {
        self.providers_settings = Some(settings);
        self
    }

    /// Inject a hardware bridge for sensor access (camera, mic, screen).
    ///
    /// When set, `build_all()` will register bridged sensor tools in the
    /// `ToolRegistry`. In desktop mode, Scrappy implements the `ToolBridge`
    /// trait and passes it here at startup.
    pub fn with_tool_bridge(mut self, bridge: Arc<dyn ToolBridge>) -> Self {
        self.tool_bridge = Some(bridge);
        self
    }

    /// Inspect the currently resolved config while using the builder incrementally.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Inspect the initialized database handle while using the builder incrementally.
    pub fn db(&self) -> Option<&Arc<dyn Database>> {
        self.db.as_ref()
    }

    /// Inspect the initialized secrets store while using the builder incrementally.
    pub fn secrets_store(&self) -> Option<&Arc<dyn SecretsStore + Send + Sync>> {
        self.secrets_store.as_ref()
    }

    /// Phase 1: Initialize database backend.
    ///
    /// Creates the database connection, runs migrations, reloads config
    /// from DB, attaches DB to session manager, and cleans up stale jobs.
    pub async fn init_database(&mut self) -> Result<(), anyhow::Error> {
        if self.flags.no_db {
            tracing::warn!("Running without database connection");
            return Ok(());
        }

        let db: Arc<dyn Database> = match self.config.database.backend {
            #[cfg(feature = "libsql")]
            crate::config::DatabaseBackend::LibSql => {
                use crate::db::Database as _;
                use crate::db::libsql::LibSqlBackend;
                use secrecy::ExposeSecret as _;

                let default_path = crate::config::default_libsql_path();
                let db_path = self
                    .config
                    .database
                    .libsql_path
                    .as_deref()
                    .unwrap_or(&default_path);

                let backend = if let Some(ref url) = self.config.database.libsql_url {
                    let token =
                        self.config
                            .database
                            .libsql_auth_token
                            .as_ref()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set"
                                )
                            })?;
                    LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret()).await?
                } else {
                    LibSqlBackend::new_local(db_path).await?
                };
                backend.run_migrations().await?;
                tracing::info!("libSQL database connected and migrations applied");

                #[cfg(feature = "libsql")]
                {
                    self.libsql_db = Some(backend.shared_db());
                }

                Arc::new(backend) as Arc<dyn Database>
            }
            #[cfg(feature = "postgres")]
            _ => {
                use crate::db::Database as _;
                let pg = crate::db::postgres::PgBackend::new(&self.config.database)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                pg.run_migrations()
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                tracing::info!("PostgreSQL database connected and migrations applied");

                #[cfg(feature = "postgres")]
                {
                    self.pg_pool = Some(pg.pool());
                }

                Arc::new(pg) as Arc<dyn Database>
            }
            #[cfg(not(feature = "postgres"))]
            _ => {
                anyhow::bail!(
                    "No database backend available. Enable 'postgres' or 'libsql' feature."
                );
            }
        };

        // Post-init: migrate disk config, reload config from DB, attach session, cleanup
        if let Err(e) = crate::bootstrap::migrate_disk_to_db(db.as_ref(), "default").await {
            tracing::warn!("Disk-to-DB settings migration failed: {}", e);
        }

        let toml_path = self.toml_path.as_deref();
        match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
            Ok(db_config) => {
                self.config = db_config;
                tracing::info!("Configuration reloaded from database");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to reload config from DB, keeping env-based config: {}",
                    e
                );
            }
        }

        // Extract ProvidersSettings from DB settings if not already injected.
        // This enables headless deployments to configure multi-provider failover
        // via config.toml or DB settings without needing Scrappy's UI.
        if self.providers_settings.is_none() {
            match db.get_all_settings("default").await {
                Ok(map) => {
                    let settings = crate::settings::Settings::from_db_map(&map);
                    let providers = normalize_providers_settings(&settings);
                    if !providers.enabled.is_empty() || providers.primary.is_some() {
                        tracing::info!(
                            "Multi-provider settings loaded from DB: {} provider(s) enabled",
                            providers.enabled.len()
                        );
                        self.providers_settings = Some(providers);
                    }
                }
                Err(e) => {
                    tracing::debug!("Could not load providers settings from DB: {}", e);
                }
            }
        }

        // Housekeeping — run cleanup BEFORE returning so stale routine_runs
        // from previous sessions are cleared before the routine engine starts.
        // Previously this was fire-and-forget (tokio::spawn), which caused a
        // race: the routine engine's check_concurrent() would see stale
        // 'running' records and skip routines until the reaper eventually ran.
        {
            let db_cleanup = db.clone();
            if let Err(e) = db_cleanup.cleanup_stale_sandbox_jobs().await {
                tracing::warn!("Failed to cleanup stale sandbox jobs: {}", e);
            }
            match db_cleanup.cleanup_stale_routine_runs().await {
                Ok(0) => {}
                Ok(n) => tracing::info!("Cleaned up {} orphaned RUNNING routine runs", n),
                Err(e) => tracing::warn!("Failed to cleanup stale routine runs: {}", e),
            }
        }

        self.db = Some(db);
        Ok(())
    }

    /// Phase 2: Create secrets store.
    ///
    /// Requires a master key and a backend-specific DB handle. After creating
    /// the store, injects any encrypted LLM API keys into the config overlay
    /// and re-resolves config.
    pub async fn init_secrets(&mut self) -> Result<(), anyhow::Error> {
        // If a secrets store was pre-injected (e.g. Scrappy's Keychain backend),
        // skip creation but still inject keys and re-resolve config.
        if self.secrets_store.is_some() {
            if let Some(ref secrets) = self.secrets_store {
                crate::config::inject_all_secrets_from_store(secrets.as_ref(), "default").await;
                if let Some(ref db) = self.db {
                    let toml_path = self.toml_path.as_deref();
                    match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
                        Ok(refreshed) => {
                            self.config = refreshed;
                            tracing::debug!(
                                "LlmConfig re-resolved after external secret injection"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to re-resolve config after secret injection: {}",
                                e
                            );
                        }
                    }
                }
            }
            return Ok(());
        }

        let master_key = match self.config.secrets.master_key() {
            Some(k) => k,
            None => {
                // Consume unused handles
                #[cfg(feature = "libsql")]
                {
                    self.libsql_db.take();
                }
                return Ok(());
            }
        };

        let crypto = match crate::secrets::SecretsCrypto::new(master_key.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!("Failed to initialize secrets crypto: {}", e);
                #[cfg(feature = "libsql")]
                {
                    self.libsql_db.take();
                }
                return Ok(());
            }
        };

        let store: Option<Arc<dyn SecretsStore + Send + Sync>> = None;

        #[cfg(feature = "libsql")]
        let store = store.or_else(|| {
            self.libsql_db.take().map(|db| {
                Arc::new(crate::secrets::LibSqlSecretsStore::new(
                    db,
                    Arc::clone(&crypto),
                )) as Arc<dyn SecretsStore + Send + Sync>
            })
        });

        #[cfg(feature = "postgres")]
        let store = store.or_else(|| {
            self.pg_pool.as_ref().map(|pool| {
                Arc::new(crate::secrets::PostgresSecretsStore::new(
                    pool.clone(),
                    Arc::clone(&crypto),
                )) as Arc<dyn SecretsStore + Send + Sync>
            })
        });

        if let Some(ref secrets) = store {
            // Inject LLM API keys from encrypted storage
            crate::config::inject_all_secrets_from_store(secrets.as_ref(), "default").await;

            // Re-resolve config with newly available keys
            if let Some(ref db) = self.db {
                let toml_path = self.toml_path.as_deref();
                match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
                    Ok(refreshed) => {
                        self.config = refreshed;
                        tracing::debug!("LlmConfig re-resolved after secret injection");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to re-resolve config after secret injection: {}", e);
                    }
                }
            }
        }

        self.secrets_store = store;
        Ok(())
    }

    /// Phase 3: Initialize LLM provider chain.
    ///
    /// Delegates to `build_provider_chain` which applies all decorators
    /// (retry, smart routing, failover, circuit breaker, response cache).
    /// When `providers_settings` is available, enables multi-provider failover.
    #[allow(clippy::type_complexity)]
    pub fn init_llm(
        &self,
    ) -> Result<(Arc<dyn LlmProvider>, Option<Arc<dyn LlmProvider>>), anyhow::Error> {
        let (llm, cheap_llm) =
            crate::llm::build_provider_chain(&self.config.llm, self.providers_settings.as_ref())?;
        Ok((llm, cheap_llm))
    }

    /// Phase 4: Initialize safety, tools, embeddings, and workspace.
    pub async fn init_tools(
        &self,
        llm: &Arc<dyn LlmProvider>,
        cheap_llm: Option<&Arc<dyn LlmProvider>>,
        cost_tracker: Option<Arc<tokio::sync::Mutex<CostTracker>>>,
    ) -> Result<
        (
            Arc<SafetyLayer>,
            Arc<ToolRegistry>,
            Option<Arc<dyn EmbeddingProvider>>,
            Option<Arc<Workspace>>,
        ),
        anyhow::Error,
    > {
        let safety = Arc::new(SafetyLayer::new(&self.config.safety));
        tracing::info!("Safety layer initialized");

        // Initialize tool registry with credential injection support
        let credential_registry = Arc::new(SharedCredentialRegistry::new());
        let tools = if let Some(ref ss) = self.secrets_store {
            Arc::new(
                ToolRegistry::new()
                    .with_credentials(Arc::clone(&credential_registry), Arc::clone(ss)),
            )
        } else {
            Arc::new(ToolRegistry::new())
        };
        tools.register_builtin_tools_with_browser_backend(
            &self.config.agent.browser_backend,
            self.config.agent.cloud_browser_provider.as_deref(),
        );
        tools.register_vision_tool(Arc::clone(llm));
        tools.register_moa_tool(
            Arc::clone(llm),
            cheap_llm.cloned(),
            self.config.llm.reliability.moa_reference_models.clone(),
            self.config.llm.reliability.moa_aggregator_model.clone(),
            self.config.llm.reliability.moa_min_successful,
        );

        // Create embeddings provider using the unified method
        let embeddings = self.config.embeddings.create_provider().await;

        // Warn if libSQL backend is used with non-1536 embedding dimension.
        if self.config.database.backend == crate::config::DatabaseBackend::LibSql
            && self.config.embeddings.enabled
            && self.config.embeddings.dimension != 1536
        {
            tracing::warn!(
                configured_dimension = self.config.embeddings.dimension,
                "Embedding dimension {} is not 1536. libSQL currently uses a fixed \
                 1536-dim vector index, so ThinClaw will keep storing documents but \
                 skip vector embeddings/search for that backend and fall back to FTS.",
                self.config.embeddings.dimension
            );
        }

        // Register memory tools if database is available
        let workspace = if let Some(ref db) = self.db {
            let mut ws = Workspace::new_with_db("default", db.clone());
            if let Some(ref emb) = embeddings {
                ws = ws.with_embeddings(emb.clone());
            }
            let ws = Arc::new(ws);
            tools.register_memory_tools(
                Arc::clone(&ws),
                Some(Arc::clone(db)),
                cheap_llm.cloned(),
                None,
            );

            Some(ws)
        } else {
            None
        };

        // Register builder tool if enabled
        if self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled)
        {
            // Resolve workspace directories — same logic as the non-builder path below.
            // The builder must respect sandboxed/project/unrestricted just like raw dev tools.
            let mode = self.config.agent.workspace_mode.as_str();
            let root = self.config.agent.workspace_root.clone();

            let (builder_base_dir, builder_working_dir) = match mode {
                "sandboxed" => {
                    let dir = root.unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("Library")
                            .join("Application Support")
                            .join("OpenClaw")
                            .join("agent_workspace")
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    tracing::info!("[app] Builder workspace: sandboxed → {}", dir.display());
                    (Some(dir.clone()), Some(dir))
                }
                "project" => {
                    let dir = root.unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    tracing::info!("[app] Builder workspace: project → {}", dir.display());
                    (None, Some(dir))
                }
                _ => {
                    // Unrestricted: full filesystem access — no restrictions at all.
                    // Intended for remote/server deployments or power users.
                    tracing::info!(
                        "[app] Builder workspace: unrestricted (full filesystem access)"
                    );
                    (None, None)
                }
            };

            tools
                .register_builder_tool(
                    llm.clone(),
                    safety.clone(),
                    Some(self.config.builder.to_builder_config()),
                    builder_base_dir,
                    builder_working_dir,
                    (self.config.agent.workspace_mode == "sandboxed"
                        && self.config.sandbox.enabled)
                        .then(|| {
                            Arc::new(crate::sandbox::SandboxManager::new(
                                self.config.sandbox.to_sandbox_config(),
                            ))
                        }),
                    Some(crate::sandbox::SandboxPolicy::WorkspaceWrite),
                    cost_tracker.clone(),
                )
                .await;
            tracing::info!("Builder mode enabled");
        }

        Ok((safety, tools, embeddings, workspace))
    }

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
        ),
        anyhow::Error,
    > {
        use crate::tools::mcp::{McpClient, config::load_mcp_servers_from_db, is_authenticated};
        use crate::tools::wasm::{WasmToolLoader, load_dev_tools};

        let mcp_session_manager = Arc::new(McpSessionManager::new());

        // Create WASM tool runtime
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

        let wasm_tool_invoker = Arc::new(crate::tools::execution::HostMediatedToolInvoker::new(
            Arc::clone(tools),
            Arc::clone(safety),
            crate::tools::ToolExecutionLane::WorkerRuntime,
            crate::tools::ToolProfile::ExplicitOnly,
        ));

        // Load WASM tools and MCP servers concurrently
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

        let mcp_servers_future = {
            let secrets_store = self.secrets_store.clone();
            let db = self.db.clone();
            let tools = Arc::clone(tools);
            let mcp_sm = Arc::clone(&mcp_session_manager);
            async move {
                let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
                    if let Some(ref secrets) = secrets_store {
                        Arc::clone(secrets)
                    } else {
                        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
                        let ephemeral_key = secrecy::SecretString::from(
                            crate::platform::secure_store::generate_master_key_hex(),
                        );
                        let crypto =
                            Arc::new(SecretsCrypto::new(ephemeral_key).expect("ephemeral crypto"));
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
                            );

                            join_set.spawn(async move {
                                let server_name = server.name.clone();

                                let client = if server.is_stdio() {
                                    match McpClient::new_stdio_with_store(
                                        &server,
                                        Some(config_store.clone()),
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
                                                for tool in tool_impls {
                                                    tools.register(tool).await;
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
                                                 Run: thinclaw mcp auth {}",
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
        let ext_secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            if let Some(ref s) = self.secrets_store {
                Arc::clone(s)
            } else {
                use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
                let ephemeral_key = secrecy::SecretString::from(
                    crate::platform::secure_store::generate_master_key_hex(),
                );
                let crypto = Arc::new(SecretsCrypto::new(ephemeral_key).expect("ephemeral crypto"));
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
            Some(manager)
        };

        // register_builder_tool() now registers dev tools with the correct workspace dirs
        // internally (sandbox/project/unrestricted). Only register here when builder is off.
        let builder_registered_dev_tools = self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled);
        if self.config.agent.allow_local_tools && !builder_registered_dev_tools {
            // Resolve workspace mode → (base_dir, working_dir) for tool registration
            let mode = self.config.agent.workspace_mode.as_str();
            let root = self.config.agent.workspace_root.clone();

            match mode {
                "sandboxed" => {
                    // Full sandbox: file tools restricted + shell cwd set
                    let dir = root.unwrap_or_else(|| {
                        // Resolve from TAURI app data dir via env — set during bridge init
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("Library")
                            .join("Application Support")
                            .join("OpenClaw")
                            .join("agent_workspace")
                    });
                    // Ensure directory exists
                    let _ = std::fs::create_dir_all(&dir);
                    tracing::info!("[app] Workspace mode: sandboxed → {}", dir.display());
                    tools.register_dev_tools_with_runtime(
                        Some(dir.clone()),
                        Some(dir),
                        Some(&self.config.safety),
                        Some(Arc::new(crate::sandbox::SandboxManager::new(
                            self.config.sandbox.to_sandbox_config(),
                        ))),
                        Some(crate::sandbox::SandboxPolicy::WorkspaceWrite),
                    );
                }
                "project" => {
                    // Project mode: working dir set, no file sandbox
                    let dir = root.unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    tracing::info!("[app] Workspace mode: project → {}", dir.display());
                    tools.register_dev_tools_with_runtime(
                        None,
                        Some(dir),
                        Some(&self.config.safety),
                        None,
                        None,
                    );
                }
                _ => {
                    // Unrestricted: full filesystem access — no base_dir, no working_dir forced.
                    // The agent can write to any absolute path the user/LLM specifies.
                    // (This mode is intended for remote/server deployments or power users.)
                    tracing::info!("[app] Workspace mode: unrestricted (full filesystem access)");
                    tools.register_dev_tools_with_runtime(
                        None,
                        None,
                        Some(&self.config.safety),
                        None,
                        None,
                    );
                }
            }
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

        let _desktop_autonomy_manager = if self.config.desktop_autonomy.is_reckless_enabled()
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

        if self.config.agent.allow_local_tools {
            let process_registry: crate::tools::builtin::SharedProcessRegistry =
                Arc::new(tokio::sync::RwLock::new(Default::default()));
            let mode = self.config.agent.workspace_mode.as_str();
            let root = self.config.agent.workspace_root.clone();
            let sandbox_backend = Arc::new(crate::sandbox::SandboxManager::new(
                self.config.sandbox.to_sandbox_config(),
            ));

            match process_registration_mode(mode) {
                RuntimeExecRegistrationMode::LocalHost => {
                    crate::tools::builtin::start_reaper(Arc::clone(&process_registry));
                    tools.register_process_tool(process_registry);
                }
                RuntimeExecRegistrationMode::Disabled => {
                    tracing::info!(
                        workspace_mode = mode,
                        "Background process tool disabled in restricted workspace mode"
                    );
                }
                RuntimeExecRegistrationMode::DockerSandbox => {
                    tracing::warn!(
                        workspace_mode = mode,
                        "Background process tool is unavailable for Docker sandbox mode"
                    );
                }
            }

            match mode {
                "sandboxed" => {
                    let dir = root.unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("Library")
                            .join("Application Support")
                            .join("OpenClaw")
                            .join("agent_workspace")
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    match execute_code_registration_mode(mode, self.config.sandbox.enabled) {
                        RuntimeExecRegistrationMode::DockerSandbox => {
                            let backend =
                                crate::tools::execution_backend::DockerSandboxExecutionBackend::from_sandbox(
                                    Arc::clone(&sandbox_backend),
                                    crate::sandbox::SandboxPolicy::WorkspaceWrite,
                                );
                            tools.register_execute_code_tool_with_backend(
                                Some(dir.clone()),
                                false,
                                Some(backend),
                            );
                        }
                        RuntimeExecRegistrationMode::Disabled => {
                            tracing::warn!(
                                workspace_mode = mode,
                                sandbox_enabled = self.config.sandbox.enabled,
                                "execute_code disabled because no isolated execution backend is available"
                            );
                        }
                        RuntimeExecRegistrationMode::LocalHost => {
                            tools.register_execute_code_tool(Some(dir.clone()), false);
                        }
                    }
                    tools.register_search_files_tool(Some(dir));
                }
                "project" => {
                    let dir = root.unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    match execute_code_registration_mode(mode, self.config.sandbox.enabled) {
                        RuntimeExecRegistrationMode::Disabled => {
                            tracing::info!(
                                workspace_mode = mode,
                                "execute_code disabled in project mode because it has no hard execution isolation there"
                            );
                        }
                        RuntimeExecRegistrationMode::DockerSandbox => {
                            let backend =
                                crate::tools::execution_backend::DockerSandboxExecutionBackend::from_sandbox(
                                    Arc::clone(&sandbox_backend),
                                    crate::sandbox::SandboxPolicy::WorkspaceWrite,
                                );
                            tools.register_execute_code_tool_with_backend(
                                Some(dir.clone()),
                                false,
                                Some(backend),
                            );
                        }
                        RuntimeExecRegistrationMode::LocalHost => {
                            tools.register_execute_code_tool(Some(dir.clone()), false);
                        }
                    }
                    tools.register_search_files_tool(Some(dir));
                }
                _ => {
                    tools.register_execute_code_tool(None, false);
                    tools.register_search_files_tool(None);
                }
            }
        }

        // Register TTS tool (always available — uses OpenAI TTS API)
        let tts_output_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("thinclaw")
            .join("tts");
        let tts_secrets = self.secrets_store.clone();
        tools.register_tts_tool(tts_secrets, tts_output_dir);

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
        ))
    }

    /// Run all init phases in order and return the assembled components.
    pub async fn build_all(mut self) -> Result<AppComponents, anyhow::Error> {
        let build_all_start = std::time::Instant::now();

        let phase_start = std::time::Instant::now();
        self.init_database().await?;
        tracing::info!(
            elapsed_ms = phase_start.elapsed().as_millis(),
            "Startup phase: database"
        );

        let phase_start = std::time::Instant::now();
        self.init_secrets().await?;
        tracing::info!(
            elapsed_ms = phase_start.elapsed().as_millis(),
            "Startup phase: secrets"
        );

        let mut providers_settings = self.providers_settings.clone().unwrap_or_default();
        let primed_oauth_credentials =
            crate::llm::prime_runtime_oauth_credentials(&providers_settings);
        if primed_oauth_credentials > 0 {
            let refreshed = if let Some(ref db) = self.db {
                Config::from_db_with_toml(db.as_ref(), "default", self.toml_path.as_deref()).await
            } else {
                Config::from_env_with_toml(self.toml_path.as_deref()).await
            };

            match refreshed {
                Ok(config) => {
                    self.config = config;
                    tracing::info!(
                        primed_oauth_credentials,
                        "Primed external OAuth credentials into the runtime overlay"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Failed to re-resolve config after priming external OAuth credentials"
                    );
                }
            }
        }
        crate::llm::hydrate_runtime_credentials_from_secrets(
            &mut self.config,
            &mut providers_settings,
            self.secrets_store.as_ref(),
            "default",
        )
        .await;
        let llm_runtime = LlmRuntimeManager::new(
            self.config.clone(),
            providers_settings.clone(),
            self.db.clone(),
            self.secrets_store.clone(),
            "default",
            self.toml_path.clone(),
        )?;
        let oauth_credential_sync = crate::llm::OAuthCredentialSyncHandle::start(
            Arc::clone(&llm_runtime),
            &providers_settings,
        );
        let runtime_llm = llm_runtime.primary_handle();
        let runtime_cheap_llm = Some(llm_runtime.cheap_handle());

        // Create the shared cost tracker early (before init_tools) so the
        // builder tool's iterative LLM loop can be cost-tracked from the start.
        let cost_tracker = {
            let budget = BudgetConfig {
                daily_limit_usd: self
                    .config
                    .agent
                    .max_cost_per_day_cents
                    .map(|c| c as f64 / 100.0),
                ..BudgetConfig::default()
            };
            let mut tracker = CostTracker::new(budget);

            // Restore persisted entries from the ThinClaw DB.
            if let Some(ref db) = self.db {
                match db.get_setting("default", "cost_entries").await {
                    Ok(Some(json)) => tracker.from_json(&json),
                    Ok(None) => tracing::debug!("[cost] No persisted cost entries in DB"),
                    Err(e) => tracing::warn!("[cost] Failed to load cost entries from DB: {}", e),
                }
            }

            Arc::new(tokio::sync::Mutex::new(tracker))
        };

        if let Err(err) = crate::timezone::set_user_timezone_override(
            "default",
            self.config.heartbeat.user_timezone.as_deref(),
        ) {
            tracing::warn!("Failed to initialize live timezone override: {}", err);
        }

        let cost_guard = Arc::new(crate::agent::cost_guard::CostGuard::new(
            crate::agent::cost_guard::CostGuardConfig {
                max_cost_per_day_cents: self.config.agent.max_cost_per_day_cents,
                max_actions_per_hour: self.config.agent.max_actions_per_hour,
            },
        ));
        {
            use rust_decimal::prelude::FromPrimitive;

            let now = chrono::Utc::now();
            let today = now.format("%Y-%m-%d").to_string();
            let this_month = now.format("%Y-%m").to_string();
            let (daily_spend, actions_last_hour, model_usage) = {
                let tracker = cost_tracker.lock().await;
                let model_usage = tracker
                    .summary(&today, &this_month)
                    .model_details
                    .into_iter()
                    .map(|entry| {
                        (
                            entry.model,
                            crate::agent::cost_guard::ModelTokens {
                                input_tokens: entry.input_tokens,
                                output_tokens: entry.output_tokens,
                                cost: rust_decimal::Decimal::from_f64(entry.cost_usd)
                                    .unwrap_or_default(),
                            },
                        )
                    })
                    .collect();
                (
                    rust_decimal::Decimal::from_f64(tracker.cost_for_date(&today))
                        .unwrap_or_default(),
                    tracker.recent_action_count(now, chrono::Duration::hours(1)),
                    model_usage,
                )
            };
            cost_guard
                .hydrate(daily_spend, actions_last_hour, model_usage)
                .await;
        }

        let llm: Arc<dyn LlmProvider> = Arc::new(UsageTrackingProvider::new(
            runtime_llm,
            Arc::clone(&cost_tracker),
            self.db.clone(),
            Some(Arc::clone(&cost_guard)),
        ));
        let cheap_llm = runtime_cheap_llm.map(|cheap| {
            Arc::new(UsageTrackingProvider::new(
                cheap,
                Arc::clone(&cost_tracker),
                self.db.clone(),
                Some(Arc::clone(&cost_guard)),
            )) as Arc<dyn LlmProvider>
        });

        let phase_start = std::time::Instant::now();
        let (safety, tools, embeddings, workspace) = self
            .init_tools(&llm, cheap_llm.as_ref(), Some(Arc::clone(&cost_tracker)))
            .await?;
        tracing::info!(
            elapsed_ms = phase_start.elapsed().as_millis(),
            "Startup phase: tools"
        );

        // Create hook registry early so runtime extension activation can register hooks.
        let hooks = Arc::new(HookRegistry::new());

        // Create the audit log hook standalone — it will receive extension lifecycle events
        // via ExtensionManager::set_lifecycle_audit_hook() wired below.
        let audit_hook = Arc::new(AuditLogHook::new());

        let phase_start = std::time::Instant::now();
        let (
            mcp_session_manager,
            wasm_tool_runtime,
            extension_manager,
            catalog_entries,
            dev_loaded_tool_names,
        ) = self.init_extensions(&tools, &safety, &hooks).await?;
        tracing::info!(
            elapsed_ms = phase_start.elapsed().as_millis(),
            "Startup phase: extensions"
        );

        let mut user_tools_dir =
            crate::platform::expand_home_dir(&self.config.extensions.user_tools_dir);
        let legacy_user_tools_dir = crate::platform::resolve_data_dir("user_tools");
        if !user_tools_dir.exists() && legacy_user_tools_dir.exists() {
            if let Some(parent) = user_tools_dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::rename(&legacy_user_tools_dir, &user_tools_dir) {
                Ok(()) => {
                    tracing::info!(
                        from = %legacy_user_tools_dir.display(),
                        to = %user_tools_dir.display(),
                        "Migrated legacy user tools directory to canonical hyphenated path"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        from = %legacy_user_tools_dir.display(),
                        to = %user_tools_dir.display(),
                        error = %error,
                        "Could not migrate legacy user tools directory; using legacy path for this run"
                    );
                    user_tools_dir = legacy_user_tools_dir;
                }
            }
        }
        let (user_tool_base_dir, user_tool_working_dir) =
            match self.config.agent.workspace_mode.as_str() {
                "sandboxed" => {
                    let dir = self.config.agent.workspace_root.clone().unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("Library")
                            .join("Application Support")
                            .join("OpenClaw")
                            .join("agent_workspace")
                    });
                    let _ = std::fs::create_dir_all(&dir);
                    (Some(dir.clone()), Some(dir))
                }
                "project" => (
                    None,
                    Some(self.config.agent.workspace_root.clone().unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                    })),
                ),
                _ => (None, None),
            };
        let user_tool_results = tools
            .auto_discover_user_tools(
                &user_tools_dir,
                user_tool_base_dir,
                user_tool_working_dir,
                Some(&self.config.safety),
                wasm_tool_runtime.clone(),
                self.secrets_store.clone(),
                Some(Arc::new(
                    crate::tools::execution::HostMediatedToolInvoker::new(
                        Arc::clone(&tools),
                        Arc::clone(&safety),
                        crate::tools::ToolExecutionLane::WorkerRuntime,
                        crate::tools::ToolProfile::ExplicitOnly,
                    ),
                )),
            )
            .await;
        if !user_tool_results.loaded.is_empty() {
            tracing::info!(
                dir = %user_tools_dir.display(),
                count = user_tool_results.loaded.len(),
                tools = ?user_tool_results.loaded,
                "Loaded user-defined tools"
            );
        }
        for (path, error) in &user_tool_results.errors {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "Failed to load user-defined tool"
            );
        }

        // Wire the lifecycle audit hook into the extension manager so
        // install/activate/remove events are recorded for the UI.
        if let Some(ref ext_mgr) = extension_manager {
            ext_mgr
                .set_lifecycle_audit_hook(Arc::clone(&audit_hook))
                .await;
        }

        // Register lifecycle hooks: bundled (AuditLogHook) + plugin + workspace.
        // Without this, the HookRegistry remains empty in Scrappy/Tauri mode.
        let active_tool_names = tools.list().await;
        let hook_bootstrap = crate::hooks::bootstrap_hooks(
            &hooks,
            workspace.as_ref(),
            &self.config.wasm.tools_dir,
            &self.config.channels.wasm_channels_dir,
            &active_tool_names,
            &[], // WASM channel names are loaded separately in the bridge
            &dev_loaded_tool_names,
        )
        .await;
        tracing::info!(
            bundled = hook_bootstrap.bundled_hooks,
            plugin = hook_bootstrap.plugin_hooks,
            workspace = hook_bootstrap.workspace_hooks,
            outbound_webhooks = hook_bootstrap.outbound_webhooks,
            errors = hook_bootstrap.errors,
            "Lifecycle hooks initialized"
        );

        // Seed workspace and backfill embeddings
        if let Some(ref ws) = workspace {
            match ws
                .seed_if_empty(
                    Some(&self.config.agent.name),
                    Some(&self.config.agent.personality_pack),
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Failed to seed workspace: {}", e);
                }
            }

            // ── Timezone sync: Settings <-> USER.md ──────────────────────
            // Shared USER.md is the durable prompt-facing source, while the DB
            // setting drives runtime config. On startup we reconcile them, then
            // refresh future routine fire times in the effective timezone.
            let configured_tz = self.config.heartbeat.user_timezone.clone();
            let user_md_tz = ws.extract_user_timezone().await;
            let effective_tz = user_md_tz.clone().or(configured_tz.clone());

            if let Err(err) =
                crate::timezone::set_user_timezone_override("default", effective_tz.as_deref())
            {
                tracing::warn!("Failed to refresh live timezone override: {}", err);
            }

            if let Err(err) = ws.sync_user_timezone(effective_tz.as_deref()).await {
                tracing::warn!("Failed to sync workspace timezone documents: {}", err);
            }

            if let Some(ref db) = self.db {
                if effective_tz != configured_tz {
                    match effective_tz.as_deref() {
                        Some(tz) => {
                            if let Err(err) = db
                                .set_setting("default", "user_timezone", &serde_json::json!(tz))
                                .await
                            {
                                tracing::warn!(
                                    "Failed to sync timezone setting from USER.md: {}",
                                    err
                                );
                            }
                        }
                        None => {
                            if let Err(err) = db.delete_setting("default", "user_timezone").await {
                                tracing::warn!("Failed to clear timezone setting: {}", err);
                            }
                        }
                    }
                }

                let preserve_due = user_md_tz.as_deref() == configured_tz.as_deref();
                if let Err(err) = crate::timezone::refresh_user_routine_timezones(
                    db,
                    "default",
                    effective_tz.as_deref(),
                    preserve_due,
                )
                .await
                {
                    tracing::warn!("Failed to refresh routine schedules for timezone: {}", err);
                }
            }

            if embeddings.is_some() {
                let ws_bg = Arc::clone(ws);
                tokio::spawn(async move {
                    match ws_bg.backfill_embeddings().await {
                        Ok(count) if count > 0 => {
                            tracing::info!("Backfilled embeddings for {} chunks", count);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("Failed to backfill embeddings: {}", e);
                        }
                    }
                });
            }
        }

        // Skills system
        let (skill_registry, skill_catalog, skill_remote_hub, skill_quarantine) =
            if self.config.skills.enabled {
                let mut registry = SkillRegistry::new(self.config.skills.local_dir.clone())
                    .with_installed_dir(self.config.skills.installed_dir.clone());

                // Wire workspace skills dir: <workspace_root>/skills/
                //
                // Workspace skills have highest priority (see discover_all ordering) and
                // are loaded with Trusted trust level. They are intended for repo-scoped
                // or project-scoped skills placed by developers alongside their code.
                //
                // Derivation priority:
                //   1. SKILLS_WORKSPACE_DIR env var (explicit override)
                //   2. <workspace_root>/skills/ when workspace_root is configured
                //   3. No workspace skills directory (workspace skills disabled)
                let workspace_skills_dir =
                    if let Ok(explicit) = std::env::var("SKILLS_WORKSPACE_DIR") {
                        if !explicit.is_empty() {
                            Some(std::path::PathBuf::from(explicit))
                        } else {
                            None
                        }
                    } else {
                        self.config
                            .agent
                            .workspace_root
                            .as_ref()
                            .map(|root| root.join("skills"))
                    };

                if let Some(ws_dir) = workspace_skills_dir {
                    tracing::info!("Skills: workspace dir → {}", ws_dir.display());
                    registry = registry.with_workspace_dir(ws_dir);
                }

                let loaded = registry.discover_all().await;
                if !loaded.is_empty() {
                    tracing::info!("Loaded {} skill(s): {}", loaded.len(), loaded.join(", "));
                }
                let registry = Arc::new(tokio::sync::RwLock::new(registry));
                let catalog = crate::skills::catalog::shared_catalog();
                let remote_hub = crate::skills::build_remote_skill_hub(
                    self.config.skills.skill_taps.clone(),
                    self.config.skills.well_known_skill_registries.clone(),
                );
                let shared_remote_hub = crate::skills::SharedRemoteSkillHub::new(remote_hub);
                let quarantine = Arc::new(crate::skills::quarantine::QuarantineManager::new(
                    self.config.skills.quarantine_dir.clone(),
                ));
                tools.register_skill_tools(
                    Arc::clone(&registry),
                    Arc::clone(&catalog),
                    Some(shared_remote_hub.clone()),
                    Arc::clone(&quarantine),
                    self.db.as_ref().map(Arc::clone),
                );
                (
                    Some(registry),
                    Some(catalog),
                    Some(shared_remote_hub),
                    Some(quarantine),
                )
            } else {
                (None, None, None, None)
            };

        if let Some(db) = self.db.as_ref() {
            tools.register_learning_tools(
                Arc::clone(db),
                workspace.as_ref().cloned(),
                skill_registry.clone(),
            );
        }

        let context_manager = Arc::new(ContextManager::new(self.config.agent.max_parallel_jobs));

        // Register hardware bridge tools if a bridge was injected
        let session_approvals = Arc::new(SessionApprovals::new());
        if let Some(ref bridge) = self.tool_bridge {
            let bridged = crate::hardware_bridge::create_bridged_tools(
                Arc::clone(bridge),
                Arc::clone(&session_approvals),
            );
            let count = bridged.len();
            for bt in bridged {
                tracing::debug!(
                    sensor = %bt.sensor(),
                    action = %bt.action(),
                    "Registering bridged sensor tool"
                );
                tools.register_sync(Arc::new(bt));
            }
            tracing::info!("Hardware bridge active: {} sensor tools registered", count);
        }

        // Background prefetch ClawHub catalog so the plugin browser is non-empty on first open.
        if let Some(ref ext_mgr) = extension_manager {
            let catalog = ext_mgr.catalog_cache();
            tokio::spawn(async move {
                let mut guard = catalog.lock().await;
                match guard.prefetch_into().await {
                    Ok(count) => tracing::info!("ClawHub catalog prefetched: {} entries", count),
                    Err(e) => tracing::debug!("ClawHub prefetch skipped ({})", e),
                }
            });
        }

        tracing::info!(
            "Tool registry initialized with {} total tools",
            tools.count()
        );

        tracing::info!(
            elapsed_ms = build_all_start.elapsed().as_millis(),
            "Startup phase: build_all total"
        );

        Ok(AppComponents {
            config: self.config,
            db: self.db,
            secrets_store: self.secrets_store,
            llm_runtime: Arc::clone(&llm_runtime),
            oauth_credential_sync,
            llm,
            cheap_llm,
            safety,
            tools,
            embeddings,
            workspace,
            extension_manager,
            mcp_session_manager,
            wasm_tool_runtime,
            log_broadcaster: self.log_broadcaster,
            context_manager,
            hooks,
            skill_registry,
            skill_catalog,
            skill_remote_hub,
            skill_quarantine,
            cost_guard,
            catalog_entries,
            dev_loaded_tool_names,
            tool_bridge: self.tool_bridge,
            session_approvals,
            cost_tracker,
            audit_hook,
            response_cache: Arc::new(tokio::sync::RwLock::new(CachedResponseStore::new(
                CacheConfig::default(),
            ))),
            routing_policy: Arc::clone(&llm_runtime.routing_policy),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_modes_disable_background_processes() {
        assert_eq!(
            process_registration_mode("sandboxed"),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            process_registration_mode("project"),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            process_registration_mode("unrestricted"),
            RuntimeExecRegistrationMode::LocalHost
        );
    }

    #[test]
    fn execute_code_requires_real_isolation_in_restricted_modes() {
        assert_eq!(
            execute_code_registration_mode("sandboxed", true),
            RuntimeExecRegistrationMode::DockerSandbox
        );
        assert_eq!(
            execute_code_registration_mode("sandboxed", false),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            execute_code_registration_mode("project", true),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            execute_code_registration_mode("unrestricted", false),
            RuntimeExecRegistrationMode::LocalHost
        );
    }

    #[test]
    fn pi_os_lite_runtime_blocks_desktop_autonomy_registration() {
        assert_eq!(
            desktop_autonomy_headless_blocker_for("pi-os-lite-64", false),
            Some("pi-os-lite-64")
        );
        assert_eq!(
            desktop_autonomy_headless_blocker_for("raspberry-pi-os-lite", false),
            Some("pi-os-lite-64")
        );
        assert_eq!(
            desktop_autonomy_headless_blocker_for("remote", true),
            Some("headless")
        );
        assert_eq!(desktop_autonomy_headless_blocker_for("remote", false), None);
    }
}

//! Helper functions for the main entry point.
//!
//! Extracted from main.rs to keep the entry point focused on CLI dispatch
//! and agent startup orchestration.

use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "docker-sandbox")]
use tracing_subscriber::EnvFilter;

use thinclaw::channels::wasm::{
    RegisteredWebhookAuth, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter,
    WasmChannelRuntime, WasmChannelRuntimeConfig, create_wasm_channel_router,
};
use thinclaw::config::Config;
use thinclaw::config::EmbeddingsConfigProviderExt as _;
use thinclaw::pairing::PairingStore;
#[cfg(all(feature = "docker-sandbox", target_os = "macos"))]
use thinclaw::secrets::CreateSecretParams;
use thinclaw::secrets::SecretsStore;

#[cfg(any(feature = "docker-sandbox", feature = "tunnel"))]
fn redacted_url_for_log(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

#[cfg(feature = "docker-sandbox")]
/// Initialize tracing for worker/bridge processes (info level).
pub(crate) fn init_worker_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("thinclaw=info")),
        )
        .init();
}

/// Run the Memory CLI subcommand.
pub(crate) async fn run_memory_command(
    mem_cmd: &thinclaw::cli::MemoryCommand,
) -> anyhow::Result<()> {
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let embeddings = config.embeddings.create_provider().await;

    // Warn if libSQL backend is used with non-1536 embedding dimension.
    if config.database.backend == thinclaw::config::DatabaseBackend::LibSql
        && config.embeddings.enabled
        && config.embeddings.dimension != 1536
    {
        tracing::warn!(
            configured_dimension = config.embeddings.dimension,
            "Embedding dimension {} is not 1536. libSQL currently uses a fixed \
             1536-dim vector index, so ThinClaw will keep storing documents but \
             skip vector embeddings/search for that backend and fall back to FTS.",
            config.embeddings.dimension
        );
    }

    let db: Arc<dyn thinclaw::db::Database> = thinclaw::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    thinclaw::cli::run_memory_command_with_db(mem_cmd.clone(), db, embeddings).await
}

#[cfg(feature = "docker-sandbox")]
/// Run the Worker subcommand (inside Docker containers).
pub(crate) async fn run_worker(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_iterations: u32,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting worker for job {} (orchestrator: {})",
        job_id,
        redacted_url_for_log(orchestrator_url)
    );

    let config = thinclaw::worker::runtime::WorkerConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_iterations,
        timeout: std::time::Duration::from_secs(600),
    };

    let runtime = thinclaw::worker::WorkerRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Worker init failed: {}", e))?;

    runtime
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Worker failed: {}", e))
}

#[cfg(feature = "docker-sandbox")]
/// Run the Claude Code bridge subcommand (inside Docker containers).
pub(crate) async fn run_claude_bridge(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_turns: u32,
    model: &str,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting Claude Code bridge for job {} (orchestrator: {}, model: {})",
        job_id,
        redacted_url_for_log(orchestrator_url),
        model
    );

    let config = thinclaw::worker::claude_bridge::ClaudeBridgeConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_turns,
        model: model.to_string(),
        timeout: std::time::Duration::from_secs(1800),
        allowed_tools: thinclaw::config::ClaudeCodeConfig::from_env().allowed_tools,
    };

    let runtime = thinclaw::worker::ClaudeBridgeRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Claude bridge init failed: {}", e))?;

    runtime
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Claude bridge failed: {}", e))
}

#[cfg(feature = "docker-sandbox")]
/// Run the Codex bridge subcommand (inside Docker containers).
pub(crate) async fn run_codex_bridge(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    model: &str,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting Codex bridge for job {} (orchestrator: {}, model: {})",
        job_id,
        redacted_url_for_log(orchestrator_url),
        model
    );

    let config = thinclaw::worker::codex_bridge::CodexBridgeConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        model: model.to_string(),
        timeout: std::time::Duration::from_secs(1800),
    };

    let runtime = thinclaw::worker::CodexBridgeRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Codex bridge init failed: {}", e))?;

    runtime
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Codex bridge failed: {}", e))
}

#[cfg(feature = "docker-sandbox")]
/// Run the fixed-target relay used to keep sandbox containers off the Docker
/// host gateway while preserving access to authenticated ThinClaw endpoints.
pub(crate) async fn run_network_relay(forwards: &[String]) -> anyhow::Result<()> {
    tracing::info!(count = forwards.len(), "Starting sandbox network relay");
    thinclaw::sandbox::relay::run_network_relay(forwards)
        .await
        .map_err(|error| anyhow::anyhow!("Network relay failed: {error}"))
}

#[cfg(feature = "docker-sandbox")]
pub(crate) async fn resolve_container_provider_api_key(
    user_id: &str,
    env_key: &str,
    provider_secret_name: &str,
    provider_slug: &str,
    legacy_keychain_account: &str,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> Option<String> {
    if let Ok(value) = std::env::var(env_key)
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    if let Some(store) = secrets_store
        && let Ok(secret) = store
            .get_for_injection(
                user_id,
                provider_secret_name,
                thinclaw::secrets::SecretAccessContext::new(
                    "main_helpers",
                    "provider_credential_resolution",
                ),
            )
            .await
    {
        let value = secret.expose().trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    #[cfg(not(target_os = "macos"))]
    let _ = provider_slug;

    if let Some(value) = thinclaw::platform::secure_store::get_api_key(legacy_keychain_account)
        .await
        .filter(|value| !value.trim().is_empty())
    {
        #[cfg(target_os = "macos")]
        if let Some(store) = secrets_store {
            let params = CreateSecretParams::new(provider_secret_name, value.clone())
                .with_provider(provider_slug.to_string());
            match store.create(user_id, params).await {
                Ok(_) => {
                    tracing::info!(
                        legacy_keychain_account,
                        provider_secret_name,
                        "Migrated legacy macOS sandbox API key into the encrypted secrets store"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        legacy_keychain_account,
                        provider_secret_name,
                        error = %error,
                        "Failed to migrate legacy macOS sandbox API key into the encrypted secrets store"
                    );
                }
            }
        }

        return Some(value);
    }

    None
}

/// Start managed tunnel if configured and no static URL is already set.
#[cfg(feature = "tunnel")]
pub(crate) async fn start_tunnel(
    mut config: thinclaw::config::Config,
) -> (
    thinclaw::config::Config,
    Option<Box<dyn thinclaw::tunnel::Tunnel>>,
) {
    if config.tunnel.public_url.is_some() {
        tracing::info!(
            "Static tunnel URL in use: {}",
            redacted_url_for_log(config.tunnel.public_url.as_deref().unwrap_or("?"))
        );
        return (config, None);
    }

    let Some(ref provider_config) = config.tunnel.provider else {
        return (config, None);
    };

    let gateway_port = config
        .channels
        .gateway
        .as_ref()
        .map(|g| g.port)
        .unwrap_or(3000);
    let gateway_host = config
        .channels
        .gateway
        .as_ref()
        .map(|g| g.host.as_str())
        .unwrap_or("127.0.0.1");

    match thinclaw::tunnel::create_tunnel(provider_config) {
        Ok(Some(tunnel)) => {
            tracing::info!(
                "Starting {} tunnel on {}:{}...",
                tunnel.name(),
                gateway_host,
                gateway_port
            );
            match tunnel.start(gateway_host, gateway_port).await {
                Ok(url) => {
                    tracing::info!("Tunnel started: {}", redacted_url_for_log(&url));
                    config.tunnel.public_url = Some(url);
                    (config, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!("Failed to start tunnel: {}", e);
                    (config, None)
                }
            }
        }
        Ok(None) => (config, None),
        Err(e) => {
            tracing::error!("Failed to create tunnel: {}", e);
            (config, None)
        }
    }
}

/// Result of WASM channel setup.
pub(crate) struct WasmChannelSetup {
    pub(crate) channels: Vec<(String, Box<dyn thinclaw::channels::Channel>)>,
    pub(crate) channel_names: Vec<String>,
    pub(crate) webhook_routes: Option<axum::Router>,
    /// Runtime objects needed for hot-activation via ExtensionManager.
    pub(crate) wasm_channel_runtime: Arc<WasmChannelRuntime>,
    pub(crate) pairing_store: Arc<PairingStore>,
    pub(crate) wasm_channel_router: Arc<WasmChannelRouter>,
    /// Loader for hot-reload (shared with channel watcher).
    pub(crate) wasm_channel_loader: Arc<WasmChannelLoader>,
    /// Directory being watched for WASM channels.
    pub(crate) channels_dir: std::path::PathBuf,
}

/// Load WASM channels and register their webhook routes.
pub(crate) async fn setup_wasm_channels(
    config: &thinclaw::config::Config,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    extension_manager: Option<&Arc<thinclaw::extensions::ExtensionManager>>,
) -> Option<WasmChannelSetup> {
    #[cfg(feature = "wasm-runtime")]
    let runtime_config = WasmChannelRuntimeConfig::default();
    #[cfg(not(feature = "wasm-runtime"))]
    let runtime_config = WasmChannelRuntimeConfig;

    let runtime = match WasmChannelRuntime::new(runtime_config) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            tracing::warn!("Failed to initialize WASM channel runtime: {}", e);
            return None;
        }
    };

    let pairing_store = Arc::new(PairingStore::new());
    let loader = Arc::new(WasmChannelLoader::new(
        Arc::clone(&runtime),
        Arc::clone(&pairing_store),
    ));
    let channels_dir = config.channels.wasm_channels_dir.clone();

    let results = match loader
        .load_from_dir(&config.channels.wasm_channels_dir)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to scan WASM channels directory: {}", e);
            return None;
        }
    };

    let wasm_router = Arc::new(WasmChannelRouter::new());
    let mut channels: Vec<(String, Box<dyn thinclaw::channels::Channel>)> = Vec::new();
    let mut channel_names: Vec<String> = Vec::new();
    let host_config = thinclaw::channels::wasm::WasmChannelHostConfig::from_config(config);

    for loaded in results.loaded {
        let channel_name = loaded.name().to_string();
        tracing::info!("Loaded WASM channel: {}", channel_name);

        let signature_secret_name = loaded.webhook_secret_name();
        let verify_token_secret_name = loaded.webhook_verify_token_secret_name();
        let secret_header = loaded.webhook_secret_header().map(str::to_string);
        let secret_validation = loaded.webhook_secret_validation();
        let verify_token_param = loaded.webhook_verify_token_param().map(str::to_string);

        let signature_secret = if let Some(secrets) = secrets_store {
            secrets
                .get_for_injection(
                    "default",
                    &signature_secret_name,
                    thinclaw::secrets::SecretAccessContext::new(
                        "main_helpers",
                        "webhook_signature_validation",
                    ),
                )
                .await
                .ok()
                .map(|s| s.expose().to_string())
        } else {
            None
        };

        let verify_token_secret = if let Some(secret_name) = verify_token_secret_name.as_ref() {
            if signature_secret_name == *secret_name {
                signature_secret.clone()
            } else if let Some(secrets) = secrets_store {
                secrets
                    .get_for_injection(
                        "default",
                        secret_name,
                        thinclaw::secrets::SecretAccessContext::new(
                            "main_helpers",
                            "webhook_verify_token",
                        ),
                    )
                    .await
                    .ok()
                    .map(|s| s.expose().to_string())
            } else {
                None
            }
        } else {
            None
        };

        let webhook_auth = RegisteredWebhookAuth {
            secret_header: secret_header.clone(),
            secret_validation,
            signature_secret: signature_secret.clone(),
            verify_token_param,
            verify_token_secret,
        };

        let channel_arc = Arc::new(loaded.channel);

        let runtime_update_count = thinclaw::channels::wasm::apply_channel_host_config(
            &channel_arc,
            &channel_name,
            &host_config,
            signature_secret.as_deref(),
        )
        .await;
        if runtime_update_count > 0 {
            tracing::info!(
                channel = %channel_name,
                runtime_updates = runtime_update_count,
                "Injected runtime config into channel"
            );
        }

        if let Some(secrets) = secrets_store {
            match thinclaw::channels::wasm::inject_channel_credentials_from_secrets(
                &channel_arc,
                secrets.as_ref(),
                &channel_name,
                "default",
            )
            .await
            {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            channel = %channel_name,
                            credentials_injected = count,
                            "Channel credentials injected"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        channel = %channel_name,
                        error = %e,
                        "Failed to inject channel credentials"
                    );
                    continue;
                }
            }
        } else if channel_arc
            .capabilities()
            .tool_capabilities
            .http
            .as_ref()
            .is_some_and(|http| !http.credentials.is_empty())
        {
            tracing::error!(
                channel = %channel_name,
                "Cannot activate channel credentials without a secrets store"
            );
            continue;
        }

        if let Err(error) = channel_arc.prime_on_start_config().await {
            tracing::error!(
                channel = %channel_name,
                error = %error,
                "Failed to prime channel on_start config before router registration"
            );
            continue;
        }

        tracing::info!(
            channel = %channel_name,
            has_signature_secret = webhook_auth.signature_secret.is_some(),
            has_verify_token_secret = webhook_auth.verify_token_secret.is_some(),
            secret_header = ?secret_header,
            "Registering channel with router"
        );

        if let Err(error) = wasm_router
            .register(
                Arc::clone(&channel_arc),
                channel_arc.endpoints().await,
                webhook_auth,
            )
            .await
        {
            tracing::error!(
                channel = %channel_name,
                error = %error,
                "Failed to register WASM channel webhook routes"
            );
            continue;
        }

        channel_names.push(channel_name.clone());
        channels.push((channel_name, Box::new(SharedWasmChannel::new(channel_arc))));
    }

    for (path, err) in &results.errors {
        tracing::warn!("Failed to load WASM channel {}: {}", path.display(), err);
    }

    // Always create webhook routes (even with no channels loaded) so that
    // channels hot-added at runtime can receive webhooks without a restart.
    let webhook_routes = {
        Some(create_wasm_channel_router(
            Arc::clone(&wasm_router),
            extension_manager.map(Arc::clone),
        ))
    };

    Some(WasmChannelSetup {
        channels,
        channel_names,
        webhook_routes,
        wasm_channel_runtime: runtime,
        pairing_store,
        wasm_channel_router: wasm_router,
        wasm_channel_loader: loader,
        channels_dir,
    })
}

/// Check if onboarding is needed and return the reason.
///
/// Delegates to the canonical implementation in [`thinclaw::setup`] so that
/// both the binary entry point and the library crate share the same logic.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn check_onboard_needed(toml_path: Option<&Path>, no_db: bool) -> Option<String> {
    thinclaw::setup::check_onboard_needed(toml_path, no_db)
}

/// Spawn the Unix `SIGHUP` hot-reload handler for the HTTP webhook server.
///
/// On `SIGHUP` it refreshes the secrets overlay, reloads config (DB or env
/// fallback), and performs a two-phase listener swap if the HTTP bind address
/// changed. The returned handle must be drained during runtime shutdown.
#[cfg(unix)]
pub(crate) fn spawn_sighup_reload_handler(
    webhook_server: Option<Arc<tokio::sync::Mutex<thinclaw::channels::WebhookServer>>>,
    store: Option<Arc<dyn thinclaw::db::Database>>,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    owner_id: String,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to register SIGHUP handler: {}", e);
                return;
            }
        };

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::debug!("SIGHUP hot-reload handler stopped");
                    break;
                }
                received = sighup.recv() => {
                    if received.is_none() {
                        tracing::debug!("SIGHUP stream closed; hot-reload handler exiting");
                        break;
                    }
                }
            }
            tracing::info!("SIGHUP received — reloading HTTP webhook config");

            // 1. Refresh secrets overlay (thread-safe, no unsafe set_var)
            if let Some(ref secrets) = secrets {
                thinclaw::config::refresh_secrets(secrets.as_ref(), &owner_id).await;
            }

            // 2. Reload config from DB (or env fallback)
            let new_config = match &store {
                Some(store) => thinclaw::config::Config::from_db(store.as_ref(), &owner_id).await,
                None => thinclaw::config::Config::from_env().await,
            };
            let new_config = match new_config {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("SIGHUP config reload failed: {}", e);
                    continue;
                }
            };

            // 3. Check HTTP channel config
            let new_http = match new_config.channels.http {
                Some(c) => c,
                None => {
                    tracing::warn!("SIGHUP: HTTP channel no longer configured, skipping");
                    continue;
                }
            };

            // 4. Two-phase listener swap if address changed
            let new_addr: std::net::SocketAddr =
                match format!("{}:{}", new_http.host, new_http.port).parse() {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!("SIGHUP: invalid addr: {}", e);
                        continue;
                    }
                };

            if let Some(ref ws_arc) = webhook_server {
                let (old_addr, router) = {
                    let ws = ws_arc.lock().await;
                    (ws.current_addr(), ws.merged_router_clone())
                };

                if old_addr != new_addr {
                    tracing::info!(
                        "SIGHUP: HTTP addr {} -> {}, restarting listener",
                        old_addr,
                        new_addr
                    );
                    if let Some(app) = router {
                        // Phase 1: bind new listener outside the lock
                        match tokio::net::TcpListener::bind(new_addr).await {
                            Ok(listener) => {
                                // Phase 2: swap under lock
                                let (old_tx, old_handle) = {
                                    let mut ws = ws_arc.lock().await;
                                    ws.install_listener(new_addr, listener, app)
                                };
                                // Phase 3: shut down old listener outside lock
                                if let Some(tx) = old_tx {
                                    let _ = tx.send(());
                                }
                                if let Some(handle) = old_handle {
                                    let _ = handle.await;
                                }
                                tracing::info!("SIGHUP: webhook server restarted on {}", new_addr);
                            }
                            Err(e) => {
                                tracing::error!("SIGHUP: bind failed on {}: {}", new_addr, e);
                            }
                        }
                    }
                } else {
                    tracing::info!(
                        "SIGHUP: HTTP addr unchanged ({}), config refreshed",
                        old_addr
                    );
                }
            }
        }
    });
    tracing::info!(
        "SIGHUP hot-reload handler registered (send `kill -HUP` to reload HTTP webhook config)"
    );
    (shutdown_tx, handle)
}

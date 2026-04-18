//! Helper functions for the main entry point.
//!
//! Extracted from main.rs to keep the entry point focused on CLI dispatch
//! and agent startup orchestration.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(feature = "docker-sandbox")]
use tracing_subscriber::EnvFilter;

use thinclaw::channels::wasm::{
    RegisteredEndpoint, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter,
    WasmChannelRuntime, WasmChannelRuntimeConfig, create_wasm_channel_router,
};
use thinclaw::config::Config;
use thinclaw::pairing::PairingStore;
use thinclaw::secrets::SecretsStore;

const STARTUP_SPINNER_FRAMES: &[char] = &['|', '/', '-', '\\'];

/// Minimal terminal spinner shown during quiet interactive startup.
pub(crate) struct QuietStartupSpinner {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl QuietStartupSpinner {
    pub(crate) fn start() -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_for_thread = Arc::clone(&running);

        let handle = std::thread::spawn(move || {
            let mut frame_idx = 0usize;
            let mut stdout = std::io::stdout();

            while running_for_thread.load(Ordering::Relaxed) {
                let frame = STARTUP_SPINNER_FRAMES[frame_idx % STARTUP_SPINNER_FRAMES.len()];
                let _ = write!(stdout, "\r\x1b[2K  Starting ThinClaw... {frame}");
                let _ = stdout.flush();
                frame_idx += 1;
                std::thread::sleep(Duration::from_millis(120));
            }

            let _ = write!(stdout, "\r\x1b[2K");
            let _ = stdout.flush();
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    pub(crate) fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for QuietStartupSpinner {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) fn should_show_quiet_startup_spinner(
    should_run_agent: bool,
    debug: bool,
    has_single_message: bool,
    cli_enabled: bool,
    has_rust_log_override: bool,
    stdin_is_tty: bool,
    stdout_is_tty: bool,
) -> bool {
    should_run_agent
        && !debug
        && !has_single_message
        && cli_enabled
        && !has_rust_log_override
        && stdin_is_tty
        && stdout_is_tty
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

    let embeddings = config.embeddings.create_provider();

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
        orchestrator_url
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
        orchestrator_url,
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
        orchestrator_url,
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
            config.tunnel.public_url.as_deref().unwrap_or("?")
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
                    tracing::info!("Tunnel started: {}", url);
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
    let runtime = match WasmChannelRuntime::new(WasmChannelRuntimeConfig::default()) {
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
        channel_names.push(channel_name.clone());
        tracing::info!("Loaded WASM channel: {}", channel_name);

        let secret_name = loaded.webhook_secret_name();

        let webhook_secret = if let Some(secrets) = secrets_store {
            secrets
                .get_decrypted("default", &secret_name)
                .await
                .ok()
                .map(|s| s.expose().to_string())
        } else {
            None
        };

        let secret_header = loaded.webhook_secret_header().map(|s| s.to_string());

        let webhook_path = format!("/webhook/{}", channel_name);
        let endpoints = vec![RegisteredEndpoint {
            channel_name: channel_name.clone(),
            path: webhook_path,
            methods: vec!["POST".to_string()],
            require_secret: webhook_secret.is_some(),
        }];

        let channel_arc = Arc::new(loaded.channel);

        let runtime_update_count = thinclaw::channels::wasm::apply_channel_host_config(
            &channel_arc,
            &channel_name,
            &host_config,
            webhook_secret.as_deref(),
        )
        .await;
        if runtime_update_count > 0 {
            tracing::info!(
                channel = %channel_name,
                runtime_updates = runtime_update_count,
                "Injected runtime config into channel"
            );
        }

        tracing::info!(
            channel = %channel_name,
            has_webhook_secret = webhook_secret.is_some(),
            secret_header = ?secret_header,
            "Registering channel with router"
        );

        wasm_router
            .register(
                Arc::clone(&channel_arc),
                endpoints,
                webhook_secret.clone(),
                secret_header,
            )
            .await;
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
                }
            }
        }

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

#[cfg(test)]
mod tests {
    use super::should_show_quiet_startup_spinner;

    #[test]
    fn quiet_spinner_shows_for_interactive_quiet_agent_runs() {
        assert!(should_show_quiet_startup_spinner(
            true, false, false, true, false, true, true
        ));
    }

    #[test]
    fn quiet_spinner_stays_off_for_debug_runs() {
        assert!(!should_show_quiet_startup_spinner(
            true, true, false, true, false, true, true
        ));
    }

    #[test]
    fn quiet_spinner_stays_off_for_non_tty_or_message_runs() {
        assert!(!should_show_quiet_startup_spinner(
            true, false, true, true, false, true, true
        ));
        assert!(!should_show_quiet_startup_spinner(
            true, false, false, true, false, false, true
        ));
        assert!(!should_show_quiet_startup_spinner(
            true, false, false, true, false, true, false
        ));
    }
}

/// Check if onboarding is needed and return the reason.
///
/// Delegates to the canonical implementation in [`thinclaw::setup`] so that
/// both the binary entry point and the library crate share the same logic.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn check_onboard_needed(toml_path: Option<&Path>, no_db: bool) -> Option<String> {
    thinclaw::setup::check_onboard_needed(toml_path, no_db)
}

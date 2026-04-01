//! ThinClaw - Main entry point.

mod main_helpers;

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use thinclaw::{
    agent::{Agent, AgentDeps},
    app::{AppBuilder, AppBuilderFlags},
    channels::{
        ChannelManager, DiscordChannel, GatewayChannel, HttpChannel, NostrChannel, ReplChannel,
        SignalChannel, WebhookServer, WebhookServerConfig,
        wasm::{WasmChannelRouter, WasmChannelRuntime},
        web::log_layer::LogBroadcaster,
    },
    cli::{
        Cli, Command, run_channels_command, run_gateway_command, run_mcp_command,
        run_pairing_command, run_status_command, run_tool_command,
    },
    config::Config,
    hooks::bootstrap_hooks,
    orchestrator::{
        ContainerJobConfig, ContainerJobManager, OrchestratorApi, TokenStore,
        api::OrchestratorState,
    },
    pairing::PairingStore,
};

use thinclaw::channels::GmailChannel;
#[cfg(target_os = "macos")]
use thinclaw::channels::IMessageChannel;

#[cfg(any(feature = "postgres", feature = "libsql"))]
use thinclaw::setup::{SetupConfig, SetupWizard};

use main_helpers::*;

/// Initialize tracing for simple CLI commands (warn level, no fancy layers).
fn init_cli_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing();
            return run_tool_command(tool_cmd.clone()).await;
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing();
            return thinclaw::cli::run_config_command(config_cmd.clone()).await;
        }
        Some(Command::Registry(registry_cmd)) => {
            init_cli_tracing();
            return thinclaw::cli::run_registry_command(registry_cmd.clone()).await;
        }
        Some(Command::Mcp(mcp_cmd)) => {
            init_cli_tracing();
            return run_mcp_command(mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing();
            return run_memory_command(mem_cmd).await;
        }
        Some(Command::Pairing(pairing_cmd)) => {
            init_cli_tracing();
            return run_pairing_command(pairing_cmd.clone()).map_err(|e| anyhow::anyhow!("{}", e));
        }
        #[cfg(feature = "repl")]
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing();
            return thinclaw::cli::run_service_command(service_cmd);
        }
        Some(Command::Doctor) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_doctor_command().await;
        }
        Some(Command::Status) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_status_command().await;
        }
        Some(Command::Cron(cron_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_cron_command(cron_cmd.clone()).await;
        }
        Some(Command::Gateway(gw_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_gateway_command(gw_cmd.clone()).await;
        }
        Some(Command::Channels(ch_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_channels_command(ch_cmd.clone()).await;
        }
        Some(Command::Message(msg_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_message_command(msg_cmd.clone()).await;
        }
        Some(Command::Models(model_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_model_command(model_cmd.clone()).await;
        }
        Some(Command::Completion(completion)) => {
            init_cli_tracing();
            return completion.run();
        }
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            init_worker_tracing();
            return run_worker(*job_id, orchestrator_url, *max_iterations).await;
        }
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            return run_claude_bridge(*job_id, orchestrator_url, *max_turns, model).await;
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
        }) => {
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();

            #[cfg(any(feature = "postgres", feature = "libsql"))]
            {
                let config = SetupConfig {
                    skip_auth: *skip_auth,
                    channels_only: *channels_only,
                };
                let mut wizard = SetupWizard::with_config(config);
                wizard.run().await?;
            }
            #[cfg(not(any(feature = "postgres", feature = "libsql")))]
            {
                let _ = (skip_auth, channels_only);
                eprintln!("Onboarding wizard requires the 'postgres' or 'libsql' feature.");
            }
            return Ok(());
        }
        Some(Command::Agents(agent_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            // In standalone CLI mode, create a fresh router.
            // Runtime agent routing state is in-memory only.
            let router = thinclaw::agent::AgentRouter::new();
            thinclaw::cli::run_agents_command(agent_cmd.clone(), &router).await;
            return Ok(());
        }
        Some(Command::Sessions(session_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            // In standalone CLI mode, create a fresh session manager.
            // Runtime session state is in-memory only.
            let mgr = std::sync::Arc::new(thinclaw::agent::SessionManager::new());
            thinclaw::cli::run_sessions_command(session_cmd.clone(), &mgr).await;
            return Ok(());
        }
        Some(Command::Logs(log_cmd)) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_log_command(log_cmd.clone()).await;
        }
        Some(Command::Browser(browser_cmd)) => {
            init_cli_tracing();
            return thinclaw::cli::run_browser_command(browser_cmd.clone()).await;
        }
        Some(Command::Update(update_cmd)) => {
            init_cli_tracing();
            return thinclaw::cli::run_update_command(update_cmd.clone()).await;
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // ── Agent startup ──────────────────────────────────────────────────

    // Load .env files early so DATABASE_URL (and any other vars) are
    // available to all subsequent env-based config resolution.
    let _ = dotenvy::dotenv();
    thinclaw::bootstrap::load_thinclaw_env();

    // Enhanced first-run detection
    #[cfg(any(feature = "postgres", feature = "libsql"))]
    if !cli.no_onboard
        && let Some(reason) = check_onboard_needed()
    {
        println!("Onboarding needed: {}", reason);
        println!();
        let mut wizard = SetupWizard::new();
        wizard.run().await?;
    }

    // Load initial config from env + disk + optional TOML (before DB is available)
    let toml_path = cli.config.as_deref();
    let config = match Config::from_env_with_toml(toml_path).await {
        Ok(c) => c,
        Err(thinclaw::error::ConfigError::MissingRequired { key, hint }) => {
            eprintln!("Configuration error: Missing required setting '{}'", key);
            eprintln!("  {}", hint);
            eprintln!();
            eprintln!(
                "Run 'thinclaw onboard' to configure, or set the required environment variables."
            );
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };

    // Create log broadcaster before tracing init so the WebLogLayer can capture all events.
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    // Initialize tracing with a reloadable EnvFilter so the gateway can switch
    // log levels at runtime without restarting.
    let log_level_handle =
        thinclaw::channels::web::log_layer::init_tracing(Arc::clone(&log_broadcaster));

    tracing::info!("Starting ThinClaw...");
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);
    tracing::info!("LLM backend: {}", config.llm.backend);

    // ── Phase 1-5: Build all core components via AppBuilder ────────────

    let flags = AppBuilderFlags { no_db: cli.no_db };
    let components = AppBuilder::new(
        config,
        flags,
        toml_path.map(std::path::PathBuf::from),
        Arc::clone(&log_broadcaster),
    )
    .build_all()
    .await?;

    let config = components.config;

    // ── Tunnel setup ───────────────────────────────────────────────────

    let (config, active_tunnel) = start_tunnel(config).await;

    // ── Orchestrator / container job manager ────────────────────────────

    // Proactive Docker detection
    let docker_status = if config.sandbox.enabled {
        let detection = thinclaw::sandbox::check_docker().await;
        match detection.status {
            thinclaw::sandbox::DockerStatus::Available => {
                tracing::info!("Docker is available");
            }
            thinclaw::sandbox::DockerStatus::NotInstalled => {
                tracing::warn!(
                    "Docker is not installed -- sandbox disabled for this session. {}",
                    detection.platform.install_hint()
                );
            }
            thinclaw::sandbox::DockerStatus::NotRunning => {
                tracing::warn!(
                    "Docker is installed but not running -- sandbox disabled for this session. {}",
                    detection.platform.start_hint()
                );
            }
            thinclaw::sandbox::DockerStatus::Disabled => {}
        }
        detection.status
    } else {
        thinclaw::sandbox::DockerStatus::Disabled
    };

    let job_event_tx: Option<
        tokio::sync::broadcast::Sender<(uuid::Uuid, thinclaw::channels::web::types::SseEvent)>,
    > = if config.sandbox.enabled && docker_status.is_ok() {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Some(tx)
    } else {
        None
    };
    let prompt_queue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        uuid::Uuid,
        std::collections::VecDeque<thinclaw::orchestrator::api::PendingPrompt>,
    >::new()));

    let container_job_manager: Option<Arc<ContainerJobManager>> =
        if config.sandbox.enabled && docker_status.is_ok() {
            let token_store = TokenStore::new();

            // Resolve Claude Code API key: env var > OS keychain > (OAuth fallback in config)
            let claude_code_api_key = match std::env::var("ANTHROPIC_API_KEY").ok() {
                Some(key) => Some(key),
                None => {
                    // Check OS keychain for API key stored by the wizard
                    thinclaw::secrets::keychain::get_api_key(
                        thinclaw::secrets::keychain::CLAUDE_CODE_API_KEY_ACCOUNT,
                    )
                    .await
                }
            };

            let job_config = ContainerJobConfig {
                image: config.sandbox.image.clone(),
                memory_limit_mb: config.sandbox.memory_limit_mb,
                cpu_shares: config.sandbox.cpu_shares,
                orchestrator_port: 50051,
                claude_code_api_key,
                claude_code_oauth_token: thinclaw::config::ClaudeCodeConfig::extract_oauth_token(),
                claude_code_model: config.claude_code.model.clone(),
                claude_code_max_turns: config.claude_code.max_turns,
                claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
                claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
            };
            let jm = Arc::new(ContainerJobManager::new(job_config, token_store.clone()));

            // Clean up orphan containers from a previous process crash
            // (fire-and-forget — never blocks startup)
            {
                let jm_cleanup = Arc::clone(&jm);
                tokio::spawn(async move {
                    jm_cleanup.cleanup_orphan_containers().await;
                });
            }

            // Start the orchestrator internal API in the background
            let orchestrator_state = OrchestratorState {
                llm: components.llm.clone(),
                job_manager: Arc::clone(&jm),
                token_store,
                job_event_tx: job_event_tx.clone(),
                prompt_queue: Arc::clone(&prompt_queue),
                store: components.db.clone(),
                secrets_store: components.secrets_store.clone(),
                user_id: "default".to_string(),
            };

            tokio::spawn(async move {
                if let Err(e) = OrchestratorApi::start(orchestrator_state, 50051).await {
                    tracing::error!("Orchestrator API failed: {}", e);
                }
            });

            if config.claude_code.enabled {
                tracing::info!(
                    "Claude Code sandbox mode available (model: {}, max_turns: {})",
                    config.claude_code.model,
                    config.claude_code.max_turns
                );
            }
            Some(jm)
        } else {
            None
        };

    // ── Channel setup ──────────────────────────────────────────────────

    let channels = Arc::new(ChannelManager::new());
    let mut channel_names: Vec<String> = Vec::new();
    let mut loaded_wasm_channel_names: Vec<String> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut wasm_channel_runtime_state: Option<(
        Arc<WasmChannelRuntime>,
        Arc<PairingStore>,
        Arc<WasmChannelRouter>,
        Arc<thinclaw::channels::wasm::WasmChannelLoader>,
        std::path::PathBuf,
    )> = None;

    // Create CLI channel
    let repl_channel = if let Some(ref msg) = cli.message {
        Some(ReplChannel::with_message(msg.clone()))
    } else if config.channels.cli.enabled {
        let repl = ReplChannel::new();
        repl.suppress_banner();
        Some(repl)
    } else {
        None
    };

    if let Some(repl) = repl_channel {
        channels.add(Box::new(repl)).await;
        if cli.message.is_some() {
            tracing::info!("Single message mode");
        } else {
            channel_names.push("repl".to_string());
            tracing::info!("REPL mode enabled");
        }
    }

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();

    // Load WASM channels and register their webhook routes.
    if config.channels.wasm_channels_enabled && config.channels.wasm_channels_dir.exists() {
        let wasm_result = setup_wasm_channels(
            &config,
            &components.secrets_store,
            components.extension_manager.as_ref(),
        )
        .await;

        if let Some(result) = wasm_result {
            loaded_wasm_channel_names = result.channel_names;
            wasm_channel_runtime_state = Some((
                result.wasm_channel_runtime,
                result.pairing_store,
                result.wasm_channel_router,
                result.wasm_channel_loader,
                result.channels_dir,
            ));
            for (name, channel) in result.channels {
                channel_names.push(name);
                channels.add(channel).await;
            }
            if let Some(routes) = result.webhook_routes {
                webhook_routes.push(routes);
            }
        }
    }

    // Add Signal channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref signal_config) = config.channels.signal
    {
        let signal_channel = SignalChannel::new(signal_config.clone())?;
        channel_names.push("signal".to_string());
        channels.add(Box::new(signal_channel)).await;
        let safe_url = SignalChannel::redact_url(&signal_config.http_url);
        tracing::info!(
            url = %safe_url,
            "Signal channel enabled"
        );
        if signal_config.allow_from.is_empty() {
            tracing::warn!(
                "Signal channel has empty allow_from list - ALL messages will be DENIED."
            );
        }
    }

    // Add Nostr channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref nostr_config) = config.channels.nostr
    {
        match NostrChannel::new(nostr_config.clone()) {
            Ok(nostr_channel) => {
                channel_names.push("nostr".to_string());
                channels.add(Box::new(nostr_channel)).await;
                tracing::info!(relays = nostr_config.relays.len(), "Nostr channel enabled");
                if nostr_config.allow_from.is_empty() {
                    tracing::info!(
                        "Nostr channel allow_from is empty — accepting messages from all senders."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Nostr channel");
            }
        }
    }

    // Add Discord channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref discord_config) = config.channels.discord
    {
        match DiscordChannel::new(discord_config.clone().into()) {
            Ok(discord_channel) => {
                channel_names.push("discord".to_string());
                channels.add(Box::new(discord_channel)).await;
                tracing::info!(
                    guild_id = discord_config.guild_id.as_deref().unwrap_or("all"),
                    "Discord channel enabled (Gateway WS)"
                );
                if discord_config.allow_from.is_empty() {
                    tracing::info!(
                        "Discord channel allow_from is empty — accepting messages from all channels."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Discord channel");
            }
        }
    }

    // Add iMessage channel if configured (macOS only) and not CLI-only mode.
    #[cfg(target_os = "macos")]
    if !cli.cli_only
        && let Some(ref imessage_config) = config.channels.imessage
    {
        use thinclaw::channels::IMessageConfig;

        // Auto-start Messages.app if not running
        thinclaw::channels::ensure_app_running("Messages").await;

        let channel_config = IMessageConfig {
            allow_from: imessage_config.allow_from.clone(),
            poll_interval_secs: imessage_config.poll_interval_secs,
            ..IMessageConfig::default()
        };
        match IMessageChannel::new(channel_config) {
            Ok(imessage_channel) => {
                channel_names.push("imessage".to_string());
                channels.add(Box::new(imessage_channel)).await;
                tracing::info!("iMessage channel enabled (chat.db polling)");
                if imessage_config.allow_from.is_empty() {
                    tracing::warn!(
                        "iMessage channel has empty allow_from list — ALL messages will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize iMessage channel");
            }
        }
    }

    // Add Apple Mail channel if configured (macOS only) and not CLI-only mode.
    #[cfg(target_os = "macos")]
    if !cli.cli_only
        && let Some(ref mail_config) = config.channels.apple_mail
    {
        use thinclaw::channels::{AppleMailChannel, AppleMailConfig};

        // Auto-start Mail.app if not running
        thinclaw::channels::ensure_app_running("Mail").await;

        let channel_config = AppleMailConfig {
            allow_from: mail_config.allow_from.clone(),
            poll_interval_secs: mail_config.poll_interval_secs,
            unread_only: mail_config.unread_only,
            mark_as_read: mail_config.mark_as_read,
            ..AppleMailConfig::default()
        };
        match AppleMailChannel::new(channel_config) {
            Ok(mail_channel) => {
                channel_names.push("apple_mail".to_string());
                channels.add(Box::new(mail_channel)).await;
                tracing::info!("Apple Mail channel enabled (Envelope Index polling)");
                if mail_config.allow_from.is_empty() {
                    tracing::warn!(
                        "Apple Mail channel has empty allow_from list — ALL emails will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Apple Mail channel");
            }
        }
    }


    // Add Gmail channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref gmail_config) = config.channels.gmail
    {
        use thinclaw::channels::gmail_wiring::GmailConfig;

        let gmail_wiring_config = GmailConfig {
            enabled: true,
            project_id: gmail_config.project_id.clone(),
            subscription_id: gmail_config.subscription_id.clone(),
            topic_id: gmail_config.topic_id.clone(),
            oauth_token: gmail_config.oauth_token.clone(),
            allowed_senders: gmail_config.allowed_senders.clone(),
            label_filters: gmail_config.label_filters.clone(),
            max_message_size_bytes: gmail_config.max_message_size_bytes,
            ..GmailConfig::default()
        };

        match GmailChannel::new(gmail_wiring_config) {
            Ok(gmail_channel) => {
                channel_names.push("gmail".to_string());
                channels.add(Box::new(gmail_channel)).await;
                tracing::info!(
                    project = %gmail_config.project_id,
                    subscription = %gmail_config.subscription_id,
                    "Gmail channel enabled (Pub/Sub pull)"
                );
                if gmail_config.allowed_senders.is_empty() {
                    tracing::warn!(
                        "Gmail channel has empty allowed_senders list — ALL incoming emails will be processed."
                    );
                }
                if gmail_config.oauth_token.is_none() {
                    tracing::warn!(
                        "Gmail channel has no OAuth token. Run `thinclaw auth gmail` to authenticate."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Gmail channel");
            }
        }
    }

    // Add HTTP channel if configured and not CLI-only mode.
    let mut webhook_server_addr: Option<std::net::SocketAddr> = None;
    if !cli.cli_only
        && let Some(ref http_config) = config.channels.http
    {
        let http_channel = HttpChannel::new(http_config.clone());
        webhook_routes.push(http_channel.routes());
        let (host, port) = http_channel.addr();
        webhook_server_addr = Some(
            format!("{}:{}", host, port)
                .parse()
                .expect("HttpConfig host:port must be a valid SocketAddr"),
        );
        channel_names.push("http".to_string());
        channels.add(Box::new(http_channel)).await;
        tracing::info!(
            "HTTP channel enabled on {}:{}",
            http_config.host,
            http_config.port
        );
    }

    // Create the shared canvas store and mount HTTP routes.
    let canvas_store = thinclaw::channels::canvas_gateway::CanvasStore::default();
    webhook_routes.push(thinclaw::channels::canvas_gateway::canvas_routes(
        canvas_store.clone(),
    ));

    // Start the unified webhook server if any routes were registered.
    let webhook_server: Option<Arc<tokio::sync::Mutex<WebhookServer>>> =
        if !webhook_routes.is_empty() {
            let addr = webhook_server_addr
                .unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], 8080)));
            if addr.ip().is_unspecified() {
                tracing::warn!(
                    "Webhook server is binding to {} — it will be reachable from all network interfaces. \
                     Set HTTP_HOST=127.0.0.1 to restrict to localhost.",
                    addr.ip()
                );
            }
            let mut server = WebhookServer::new(WebhookServerConfig { addr });
            for routes in webhook_routes {
                server.add_routes(routes);
            }
            server.start().await?;
            Some(Arc::new(tokio::sync::Mutex::new(server)))
        } else {
            None
        };

    // Register lifecycle hooks.
    let active_tool_names = components.tools.list().await;

    let hook_bootstrap = bootstrap_hooks(
        &components.hooks,
        components.workspace.as_ref(),
        &config.wasm.tools_dir,
        &config.channels.wasm_channels_dir,
        &active_tool_names,
        &loaded_wasm_channel_names,
        &components.dev_loaded_tool_names,
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

    // Create session manager (shared between agent and web gateway)
    let session_manager =
        Arc::new(thinclaw::agent::SessionManager::new().with_hooks(components.hooks.clone()));

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
    components.tools.register_job_tools(
        Arc::clone(&components.context_manager),
        container_job_manager.clone(),
        components.db.clone(),
        job_event_tx.clone(),
        Some(channels.inject_sender()),
        if config.sandbox.enabled {
            Some(Arc::clone(&prompt_queue))
        } else {
            None
        },
        components.secrets_store.clone(),
    );

    // ── Gateway channel ────────────────────────────────────────────────

    #[cfg(feature = "repl")]
    let mut gateway_url: Option<String> = None;
    let mut sse_sender: Option<
        tokio::sync::broadcast::Sender<thinclaw::channels::web::types::SseEvent>,
    > = None;
    let mut gateway_state: Option<std::sync::Arc<thinclaw::channels::web::server::GatewayState>> =
        None;
    if let Some(ref gw_config) = config.channels.gateway {
        let mut gw =
            GatewayChannel::new(gw_config.clone()).with_llm_provider(Arc::clone(&components.llm));
        if let Some(ref ws) = components.workspace {
            gw = gw.with_workspace(Arc::clone(ws));
        }
        gw = gw.with_session_manager(Arc::clone(&session_manager));
        gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
        gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
        gw = gw.with_tool_registry(Arc::clone(&components.tools));
        if let Some(ref ext_mgr) = components.extension_manager {
            gw = gw.with_extension_manager(Arc::clone(ext_mgr));
        }
        if !components.catalog_entries.is_empty() {
            gw = gw.with_registry_entries(components.catalog_entries.clone());
        }
        if let Some(ref d) = components.db {
            gw = gw.with_store(Arc::clone(d));
        }
        if let Some(ref jm) = container_job_manager {
            gw = gw.with_job_manager(Arc::clone(jm));
        }
        if let Some(ref sr) = components.skill_registry {
            gw = gw.with_skill_registry(Arc::clone(sr));
        }
        if let Some(ref sc) = components.skill_catalog {
            gw = gw.with_skill_catalog(Arc::clone(sc));
        }
        gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));
        if let Some(ref ss) = components.secrets_store {
            gw = gw.with_secrets_store(Arc::clone(ss));
        }
        gw = gw.with_channel_manager(Arc::clone(&channels));
        // Mount WASM channel webhook routes on the gateway so they are
        // reachable through the public tunnel URL. We create a second
        // Router instance since axum::Router is not Clone.
        if let Some((_, _, ref wasm_router, _, _)) = wasm_channel_runtime_state {
            let gateway_webhook_routes = thinclaw::channels::wasm::router::create_wasm_channel_router(
                Arc::clone(wasm_router),
                components.extension_manager.as_ref().map(Arc::clone),
            );
            gw = gw.with_webhook_routes(vec![gateway_webhook_routes]);
        }
        if config.sandbox.enabled {
            gw = gw.with_prompt_queue(Arc::clone(&prompt_queue));

            if let Some(ref tx) = job_event_tx {
                let mut rx = tx.subscribe();
                let gw_state = Arc::clone(gw.state());
                tokio::spawn(async move {
                    while let Ok((_job_id, event)) = rx.recv().await {
                        gw_state.sse.broadcast(event);
                    }
                });
            }
        }

        #[cfg(feature = "repl")]
        {
            gateway_url = Some(format!(
                "http://{}:{}/?token={}",
                gw_config.host,
                gw_config.port,
                gw.auth_token()
            ));
        }

        tracing::info!("Web UI: http://{}:{}/", gw_config.host, gw_config.port);

        // Capture SSE sender before moving gw into channels.
        // IMPORTANT: This must come after all `with_*` calls since `rebuild_state`
        // creates a new SseManager, which would orphan this sender.
        sse_sender = Some(gw.state().sse.sender());
        gateway_state = Some(Arc::clone(gw.state()));

        channel_names.push("gateway".to_string());
        channels.add(Box::new(gw)).await;
    }

    // ── Boot screen ────────────────────────────────────────────────────

    #[cfg(feature = "repl")]
    if config.channels.cli.enabled && cli.message.is_none() {
        let boot_tool_count = components.tools.count();
        let boot_llm_model = components.llm.model_name().to_string();
        let boot_cheap_model = components
            .cheap_llm
            .as_ref()
            .map(|c| c.model_name().to_string());

        let boot_info = thinclaw::boot_screen::BootInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            agent_name: config.agent.name.clone(),
            llm_backend: config.llm.backend.to_string(),
            llm_model: boot_llm_model,
            cheap_model: boot_cheap_model,
            db_backend: if cli.no_db {
                "none".to_string()
            } else {
                config.database.backend.to_string()
            },
            db_connected: !cli.no_db,
            tool_count: boot_tool_count,
            gateway_url,
            embeddings_enabled: config.embeddings.enabled,
            embeddings_provider: if config.embeddings.enabled {
                Some(config.embeddings.provider.clone())
            } else {
                None
            },
            heartbeat_enabled: config.heartbeat.enabled,
            heartbeat_interval_secs: config.heartbeat.interval_secs,
            sandbox_enabled: config.sandbox.enabled,
            docker_status,
            claude_code_enabled: config.claude_code.enabled,
            routines_enabled: config.routines.enabled,
            skills_enabled: config.skills.enabled,
            channels: channel_names,
            tunnel_url: active_tunnel
                .as_ref()
                .and_then(|t| t.public_url())
                .or_else(|| config.tunnel.public_url.clone()),
            tunnel_provider: active_tunnel.as_ref().map(|t| t.name().to_string()),
        };
        thinclaw::boot_screen::print_boot_screen(&boot_info);
    }

    // ── Run the agent ──────────────────────────────────────────────────

    // Wire up channel runtime for hot-activation of WASM channels.
    // Also capture the loader & channels_dir for the hot-reload watcher.
    let mut wasm_watcher_state: Option<(
        Arc<thinclaw::channels::wasm::WasmChannelLoader>,
        std::path::PathBuf,
    )> = None;

    if let Some((rt, ps, router, loader, channels_dir)) = wasm_channel_runtime_state.take() {
        // Always capture for the watcher — it works without an extension manager.
        wasm_watcher_state = Some((Arc::clone(&loader), channels_dir));

        // Wire the runtime into the extension manager if available.
        if let Some(ref ext_mgr) = components.extension_manager {
            ext_mgr
                .set_channel_runtime(
                    Arc::clone(&channels),
                    rt,
                    ps,
                    router,
                    config.channels.telegram_owner_id,
                )
                .await;
            tracing::info!("Channel runtime wired into extension manager for hot-activation");

            // Auto-activate persisted WASM channels that weren't loaded from disk.
            let persisted = ext_mgr.load_persisted_active_channels().await;
            let active_at_startup: std::collections::HashSet<String> =
                loaded_wasm_channel_names.iter().cloned().collect();
            for name in &persisted {
                if active_at_startup.contains(name) {
                    continue;
                }
                match ext_mgr.activate(name).await {
                    Ok(_) => {
                        tracing::debug!(channel = %name, "Auto-activated persisted WASM channel");
                    }
                    Err(e) => {
                        tracing::warn!(
                            channel = %name,
                            error = %e,
                            "Failed to auto-activate persisted WASM channel"
                        );
                    }
                }
            }
        }
    }

    // Clone the SSE sender for the routine engine before the extension manager consumes it.
    let routine_sse_sender = sse_sender.clone();

    // ── SIGHUP hot-reload handler (Unix only) ──────────────────────────
    #[cfg(unix)]
    {
        let sighup_webhook_server = webhook_server.clone();
        let sighup_store: Option<Arc<dyn thinclaw::db::Database>> =
            components.db.as_ref().map(Arc::clone);
        let sighup_secrets = components.secrets_store.clone();
        let sighup_owner_id = "default".to_string();

        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sighup = match signal(SignalKind::hangup()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to register SIGHUP handler: {}", e);
                    return;
                }
            };

            loop {
                sighup.recv().await;
                tracing::info!("SIGHUP received — reloading HTTP webhook config");

                // 1. Refresh secrets overlay (thread-safe, no unsafe set_var)
                if let Some(ref secrets) = sighup_secrets {
                    thinclaw::config::refresh_secrets(secrets.as_ref(), &sighup_owner_id).await;
                }

                // 2. Reload config from DB (or env fallback)
                let new_config = match &sighup_store {
                    Some(store) => {
                        thinclaw::config::Config::from_db(store.as_ref(), &sighup_owner_id).await
                    }
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

                if let Some(ref ws_arc) = sighup_webhook_server {
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
                                    tracing::info!(
                                        "SIGHUP: webhook server restarted on {}",
                                        new_addr
                                    );
                                }
                                Err(e) => {
                                    tracing::error!("SIGHUP: bind failed on {}: {}", new_addr, e);
                                }
                            }
                        }
                    } else {
                        tracing::info!("SIGHUP: HTTP addr unchanged ({}), config refreshed", old_addr);
                    }
                }
            }
        });
        tracing::info!("SIGHUP hot-reload handler registered (send `kill -HUP` to reload HTTP webhook config)");
    }

    // ── WASM channel hot-reload watcher ─────────────────────────────────
    if let Some((loader, channels_dir)) = wasm_watcher_state {
        use thinclaw::channels::wasm::channel_watcher::ChannelWatcher;

        let watcher = ChannelWatcher::new(
            channels_dir,
            loader,
            Arc::clone(&channels),
        );
        watcher.seed_from_dir().await;
        watcher.start().await;
        tracing::info!("WASM channel hot-reload watcher started (new/modified/deleted .wasm files auto-detected)");
        // Watcher runs until the process exits (task is aborted on shutdown).
    }

    // Wire SSE sender into channel manager for ChannelStatusChange events.
    if let Some(ref sender) = sse_sender {
        channels.set_sse_sender(sender.clone()).await;
    }

    // Wire SSE sender into extension manager for broadcasting status events.
    if let Some(ref ext_mgr) = components.extension_manager
        && let Some(sender) = sse_sender
    {
        ext_mgr.set_sse_sender(sender).await;
    }

    // ── Sub-agent system ────────────────────────────────────────────────
    let subagent_executor = {
        let (executor, _result_rx) = thinclaw::agent::SubagentExecutor::new(
            components.llm.clone(),
            components.safety.clone(),
            components.tools.clone(),
            channels.clone(),
            thinclaw::agent::SubagentConfig::default(),
        );

        // Wire store + SSE + cost tracker for routine run finalization by subagents
        let mut executor = executor;
        if let Some(ref db) = components.db {
            executor = executor.with_store(Arc::clone(db));
        }
        if let Some(ref sender) = routine_sse_sender {
            executor = executor.with_sse_tx(sender.clone());
        }
        executor = executor.with_cost_tracker(Arc::clone(&components.cost_tracker));

        let executor = std::sync::Arc::new(executor);

        // Register sub-agent tools with the executor
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::SpawnSubagentTool::new(executor.clone()),
        ));
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::ListSubagentsTool::new(executor.clone()),
        ));
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::CancelSubagentTool::new(executor.clone()),
        ));

        tracing::info!("Sub-agent system initialized (with routine finalization support)");
        executor
    };

    // Register LLM management tools (llm_select, llm_list_models).
    // The shared model override connects the tool output to the dispatcher.
    let model_override = thinclaw::tools::builtin::new_shared_model_override();
    components.tools.register_llm_tools(model_override.clone());

    // ── Agent registry (persistent multi-agent management) ──────────────
    //
    // A single shared AgentRouter is used by both the registry (for CRUD sync)
    // and the agent loop (for message routing). The registry populates it from
    // the database at startup.
    let shared_agent_router = Arc::new(thinclaw::agent::AgentRouter::new());

    let agent_registry = {
        let registry = thinclaw::agent::agent_registry::AgentRegistry::new(
            Arc::clone(&shared_agent_router),
            components.db.clone(),
        );

        // Load persisted agent workspaces from DB → populate the shared router
        if components.db.is_some() {
            match registry.load_from_db().await {
                Ok(count) if count > 0 => {
                    tracing::info!("Loaded {} persisted agent workspace(s)", count);
                }
                Err(e) => {
                    tracing::warn!("Failed to load agent workspaces from DB: {}", e);
                }
                _ => {}
            }
        }

        let registry = Arc::new(registry);

        // Register agent management tools (create, list, update, remove, message)
        components
            .tools
            .register_agent_management_tools(Arc::clone(&registry));

        registry
    };

    let deps = AgentDeps {
        store: components.db,
        llm: components.llm,
        cheap_llm: components.cheap_llm,
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        cost_guard: components.cost_guard,
        sse_sender: routine_sse_sender,
        agent_router: Some(shared_agent_router),
        agent_registry: Some(agent_registry),
        canvas_store: Some(canvas_store),
        subagent_executor: Some(subagent_executor),
        cost_tracker: Some(components.cost_tracker),
        response_cache: Some(components.response_cache),
        routing_policy: Some(components.routing_policy),
        model_override: Some(model_override),
    };

    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    if let Some(ref server) = webhook_server {
        server.lock().await.shutdown().await;
    }

    if let Some(tunnel) = active_tunnel {
        tracing::info!("Stopping {} tunnel...", tunnel.name());
        if let Err(e) = tunnel.stop().await {
            tracing::warn!("Failed to stop tunnel cleanly: {}", e);
        }
    }

    tracing::info!("Agent shutdown complete");

    // Check if a restart was requested via the gateway API.
    if let Some(ref gw_state) = gateway_state
        && gw_state
            .restart_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        eprintln!("Restarting ThinClaw (exit code 75)...");
        std::process::exit(75);
    }

    Ok(())
}

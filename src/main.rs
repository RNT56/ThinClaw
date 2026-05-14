//! ThinClaw - Main entry point.

mod main_helpers;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;

#[cfg(feature = "docker-sandbox")]
use thinclaw::orchestrator::{
    ContainerJobConfig, ContainerJobManager, OrchestratorApi, TokenStore, api::OrchestratorState,
};
use thinclaw::{
    agent::{Agent, AgentDeps},
    app::{
        AppBuilder, AppBuilderFlags, LocalRuntimeChannel, NativeChannelActivationInput,
        NativeChannelActivationPlan, PeriodicPersistencePlan, QuietStartupSpinner,
        RuntimeCommandIntent, RuntimeEntryMode, RuntimeEntrypointPlan, RuntimeShutdownAction,
        RuntimeShutdownPlan, init_cli_tracing, relaunch_current_process,
        restart_is_managed_by_service, run_async_entrypoint, should_show_quiet_startup_spinner,
    },
    channels::{
        ChannelDescriptor, ChannelManager, DiscordChannel, GatewayChannel, HttpChannel,
        NativeEndpointRegistry, NativeHttpClient, NativeLifecycleChannel,
        NativeLifecycleChannelConfig, NativeLifecycleWebhookConfig, ReplChannel,
        ReqwestNativeHttpClient, SignalChannel, TuiChannel, WebhookServer, WebhookServerConfig,
        native_lifecycle_webhook_routes,
        wasm::{WasmChannelRouter, WasmChannelRuntime},
        web::log_layer::LogBroadcaster,
    },
    cli::{
        Cli, Command, run_channels_command, run_gateway_command, run_identity_command,
        run_mcp_command, run_pairing_command, run_reset_command, run_secrets_command,
        run_status_command, run_tool_command, run_trajectory_command,
    },
    config::Config,
    pairing::PairingStore,
};

use thinclaw::channels::GmailChannel;
#[cfg(target_os = "macos")]
use thinclaw::channels::IMessageChannel;
use thinclaw::channels::{
    ApnsNativeClient, ApnsNativeConfig, BlueBubblesChannel, BlueBubblesConfig,
    BrowserPushNativeClient, BrowserPushNativeConfig, DiscordConfig, MatrixNativeClient,
    MatrixNativeConfig, VoiceCallNativeClient, VoiceCallNativeConfig,
};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use thinclaw::setup::{SetupConfig, SetupWizard, UiMode};

use main_helpers::*;

fn main() -> anyhow::Result<()> {
    run_async_entrypoint(async_main())
}

fn runtime_command_intent(command: Option<&Command>) -> RuntimeCommandIntent {
    match command {
        None | Some(Command::Run) => RuntimeCommandIntent::AgentRuntime,
        Some(Command::Tui) => RuntimeCommandIntent::TuiRuntime,
        Some(Command::Onboard { .. }) => RuntimeCommandIntent::Onboarding,
        #[cfg(feature = "docker-sandbox")]
        Some(Command::Worker { .. })
        | Some(Command::ClaudeBridge { .. })
        | Some(Command::CodexBridge { .. }) => RuntimeCommandIntent::WorkerRuntime,
        #[cfg(all(feature = "repl", target_os = "windows"))]
        Some(Command::WindowsServiceRuntime { .. }) => RuntimeCommandIntent::ServiceRuntime,
        _ => RuntimeCommandIntent::ImmediateCli,
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn runtime_entry_mode_from_ui_mode(ui_mode: UiMode) -> RuntimeEntryMode {
    match ui_mode {
        UiMode::Tui => RuntimeEntryMode::Tui,
        UiMode::Cli | UiMode::Auto => RuntimeEntryMode::Cli,
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn setup_config_for_onboard_command(
    skip_auth: bool,
    channels_only: bool,
    guide_topic: Option<thinclaw::setup::GuideTopic>,
    ui_mode: UiMode,
    profile: Option<thinclaw::setup::OnboardingProfile>,
) -> SetupConfig {
    SetupConfig {
        skip_auth,
        channels_only,
        guide_topic,
        ui_mode,
        profile,
        pause_after_completion: false,
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn setup_config_for_startup_onboarding(runtime_entry_mode: RuntimeEntryMode) -> SetupConfig {
    let ui_mode = match runtime_entry_mode {
        RuntimeEntryMode::Tui => UiMode::Tui,
        RuntimeEntryMode::Cli => UiMode::Cli,
        RuntimeEntryMode::Default => UiMode::Auto,
    };

    SetupConfig {
        ui_mode,
        ..SetupConfig::default()
    }
}

async fn async_main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command_intent = runtime_command_intent(cli.command.as_ref());
    let mut runtime_entry_mode = command_intent.initial_entry_mode();

    // Handle non-agent commands first (they don't need full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing(cli.debug);
            return run_tool_command(tool_cmd.clone()).await;
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_config_command(config_cmd.clone()).await;
        }
        Some(Command::Registry(registry_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_registry_command(registry_cmd.clone()).await;
        }
        Some(Command::Mcp(mcp_cmd)) => {
            init_cli_tracing(cli.debug);
            return run_mcp_command(mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing(cli.debug);
            return run_memory_command(mem_cmd).await;
        }
        Some(Command::Pairing(pairing_cmd)) => {
            init_cli_tracing(cli.debug);
            return run_pairing_command(pairing_cmd.clone())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e));
        }
        #[cfg(feature = "repl")]
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_service_command(service_cmd);
        }
        #[cfg(all(feature = "repl", target_os = "windows"))]
        Some(Command::WindowsServiceRuntime { home }) => {
            return thinclaw::service::run_windows_service_dispatcher(home.clone());
        }
        Some(Command::Doctor { profile }) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_doctor_command((*profile).into()).await;
        }
        Some(Command::Status { profile }) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_status_command((*profile).into()).await;
        }
        Some(Command::Reset(reset_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_reset_command(reset_cmd.clone()).await;
        }
        Some(Command::Secrets(secrets_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_secrets_command(secrets_cmd.clone()).await;
        }
        Some(Command::Cron(cron_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_cron_command(cron_cmd.clone()).await;
        }
        Some(Command::Experiments(experiments_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_experiments_command(experiments_cmd.clone()).await;
        }
        Some(Command::Gateway(gw_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_gateway_command(gw_cmd.clone()).await;
        }
        Some(Command::Identity(identity_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_identity_command(identity_cmd.clone()).await;
        }
        Some(Command::Channels(ch_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return run_channels_command(ch_cmd.clone()).await;
        }
        Some(Command::Comfy(comfy_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_comfy_command(comfy_cmd.clone()).await;
        }
        Some(Command::Message(msg_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_message_command(msg_cmd.clone()).await;
        }
        Some(Command::Models(model_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_model_command(model_cmd.clone()).await;
        }
        Some(Command::Completion(completion)) => {
            init_cli_tracing(cli.debug);
            return completion.run();
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            init_worker_tracing();
            return run_worker(*job_id, orchestrator_url, *max_iterations).await;
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            return run_claude_bridge(*job_id, orchestrator_url, *max_turns, model).await;
        }
        #[cfg(feature = "docker-sandbox")]
        Some(Command::CodexBridge {
            job_id,
            orchestrator_url,
            model,
        }) => {
            init_worker_tracing();
            return run_codex_bridge(*job_id, orchestrator_url, model).await;
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
            guide,
            ui,
            profile,
        }) => {
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();

            #[cfg(any(feature = "postgres", feature = "libsql"))]
            {
                let config = setup_config_for_onboard_command(
                    *skip_auth,
                    *channels_only,
                    *guide,
                    *ui,
                    *profile,
                );
                let mut wizard = SetupWizard::with_config(config);
                wizard.run().await?;
                if wizard.should_continue_to_runtime() {
                    runtime_entry_mode = runtime_entry_mode_from_ui_mode(wizard.runtime_ui_mode());
                } else {
                    return Ok(());
                }
            }
            #[cfg(not(any(feature = "postgres", feature = "libsql")))]
            {
                let _ = (skip_auth, channels_only, guide, ui, profile);
                eprintln!("Onboarding wizard requires the 'postgres' or 'libsql' feature.");
                return Ok(());
            }
        }
        Some(Command::Agents(agent_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            // In standalone CLI mode, create a fresh router.
            // Runtime agent routing state is in-memory only.
            let router = thinclaw::agent::AgentRouter::new();
            thinclaw::cli::run_agents_command(agent_cmd.clone(), &router).await;
            return Ok(());
        }
        Some(Command::Sessions(session_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            // In standalone CLI mode, create a fresh session manager.
            // Runtime session state is in-memory only.
            let mgr = std::sync::Arc::new(thinclaw::agent::SessionManager::new());
            thinclaw::cli::run_sessions_command(session_cmd.clone(), &mgr).await;
            return Ok(());
        }
        Some(Command::Logs(log_cmd)) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            return thinclaw::cli::run_log_command(log_cmd.clone()).await;
        }
        Some(Command::Browser(browser_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_browser_command(browser_cmd.clone()).await;
        }
        Some(Command::Trajectory(trajectory_cmd)) => {
            init_cli_tracing(cli.debug);
            return run_trajectory_command(trajectory_cmd.clone()).await;
        }
        Some(Command::ExperimentRunner {
            lease_id,
            gateway_url,
            token,
            workspace_root,
        }) => {
            init_cli_tracing(cli.debug);
            return thinclaw::experiments::runner::run_remote_runner(
                gateway_url,
                *lease_id,
                token,
                workspace_root.clone(),
            )
            .await;
        }
        Some(Command::AutonomyShadowCanary { manifest }) => {
            init_cli_tracing(cli.debug);
            let _ = dotenvy::dotenv();
            thinclaw::bootstrap::load_thinclaw_env();
            let report = thinclaw::desktop_autonomy::run_shadow_canary_entrypoint(manifest)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!(
                "{}",
                serde_json::to_string(&report)
                    .map_err(|e| anyhow::anyhow!("failed to encode canary report: {}", e))?
            );
            return Ok(());
        }
        Some(Command::Update(update_cmd)) => {
            init_cli_tracing(cli.debug);
            return thinclaw::cli::run_update_command(update_cmd.clone()).await;
        }
        None | Some(Command::Run) | Some(Command::Tui) => {
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
        && let Some(reason) = check_onboard_needed(cli.config.as_deref(), cli.no_db)
    {
        println!("Onboarding needed: {}", reason);
        println!();
        let mut wizard =
            SetupWizard::with_config(setup_config_for_startup_onboarding(runtime_entry_mode));
        wizard.run().await?;
        runtime_entry_mode = runtime_entry_mode_from_ui_mode(wizard.runtime_ui_mode());
    }

    // Load initial config from env + disk + optional TOML (before DB is available)
    let toml_path = cli.config.as_deref();
    let config = match Config::from_env_with_toml_options(toml_path, !cli.no_db).await {
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

    let entrypoint_plan = RuntimeEntrypointPlan::new(
        runtime_entry_mode,
        config.channels.cli.enabled,
        cli.message.is_some(),
    );
    let local_runtime_requested = matches!(
        entrypoint_plan.local_channel,
        Some(LocalRuntimeChannel::Repl | LocalRuntimeChannel::SingleMessage)
    );

    #[cfg_attr(not(feature = "repl"), allow(unused_mut))]
    let mut quiet_startup_spinner = if should_show_quiet_startup_spinner(
        cli.should_run_agent(),
        cli.debug,
        cli.message.is_some(),
        local_runtime_requested,
        std::env::var_os("RUST_LOG").is_some(),
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    ) {
        Some(QuietStartupSpinner::start())
    } else {
        None
    };

    // Create log broadcaster before tracing init so the WebLogLayer can capture all events.
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    // Initialize tracing with a reloadable EnvFilter so the gateway can switch
    // log levels at runtime without restarting.
    let log_level_handle =
        thinclaw::channels::web::log_layer::init_tracing(Arc::clone(&log_broadcaster), cli.debug);

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

    if let Some(db) = components.db.clone() {
        thinclaw::tauri_commands::configure_routing_persistence(
            db,
            "default",
            Arc::clone(&components.llm_runtime),
        );
    }

    #[cfg(feature = "docker-sandbox")]
    let runtime_secrets_store = components.secrets_store.clone();
    let config = components.config;

    // ── Tunnel setup ───────────────────────────────────────────────────

    #[cfg(feature = "tunnel")]
    let (config, active_tunnel) = start_tunnel(config).await;
    #[cfg(not(feature = "tunnel"))]
    let _active_tunnel: Option<Box<dyn std::any::Any>> = None;

    // ── Orchestrator / container job manager ────────────────────────────

    // Proactive Docker detection
    let phase_start = std::time::Instant::now();
    // Docker status is used in feature-gated blocks (docker-sandbox, repl boot screen).
    #[allow(unused_variables)]
    let docker_status = if config.sandbox.enabled {
        let detection = thinclaw::sandbox::check_docker().await;
        match detection.status {
            thinclaw::sandbox::DockerStatus::Available => {
                tracing::info!("Docker is available");
            }
            thinclaw::sandbox::DockerStatus::NotInstalled => {
                tracing::warn!(
                    "Docker is not installed -- sandbox features pending. {}",
                    detection.platform.install_hint()
                );
            }
            thinclaw::sandbox::DockerStatus::NotRunning => {
                tracing::warn!(
                    "Docker is installed but not running -- sandbox features will activate when Docker starts. {}",
                    detection.platform.start_hint()
                );
            }
            thinclaw::sandbox::DockerStatus::Disabled => {}
        }
        detection.status
    } else {
        thinclaw::sandbox::DockerStatus::Disabled
    };
    tracing::info!(
        elapsed_ms = phase_start.elapsed().as_millis(),
        "Startup phase: docker detection"
    );

    let job_event_tx: Option<
        tokio::sync::broadcast::Sender<(uuid::Uuid, thinclaw::channels::web::types::SseEvent)>,
    > = if config.sandbox.enabled {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Some(tx)
    } else {
        None
    };
    #[cfg(feature = "docker-sandbox")]
    let prompt_queue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        uuid::Uuid,
        std::collections::VecDeque<thinclaw::orchestrator::api::PendingPrompt>,
    >::new()));
    #[cfg(feature = "docker-sandbox")]
    let mut orchestrator_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>> = None;

    #[cfg(feature = "docker-sandbox")]
    let container_job_manager: Option<Arc<ContainerJobManager>> = if config.sandbox.enabled {
        let token_store = TokenStore::new();

        // On macOS, prefer the encrypted secrets store and treat the OS keychain
        // as the root trust anchor (master key) plus a legacy migration fallback.
        let claude_code_api_key = resolve_container_provider_api_key(
            "default",
            "ANTHROPIC_API_KEY",
            "llm_anthropic_api_key",
            "anthropic",
            thinclaw::platform::secure_store::CLAUDE_CODE_API_KEY_ACCOUNT,
            &runtime_secrets_store,
        )
        .await;

        let codex_code_api_key = resolve_container_provider_api_key(
            "default",
            "OPENAI_API_KEY",
            "llm_openai_api_key",
            "openai",
            thinclaw::platform::secure_store::CODEX_CODE_API_KEY_ACCOUNT,
            &runtime_secrets_store,
        )
        .await;

        let job_config = ContainerJobConfig {
            image: config.sandbox.image.clone(),
            memory_limit_mb: config.sandbox.memory_limit_mb,
            cpu_shares: config.sandbox.cpu_shares,
            orchestrator_port: 50051,
            claude_code_api_key,
            claude_code_oauth_token: thinclaw::config::ClaudeCodeConfig::extract_oauth_token(),
            claude_code_enabled: config.claude_code.enabled,
            claude_code_model: config.claude_code.model.clone(),
            claude_code_max_turns: config.claude_code.max_turns,
            claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
            claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
            codex_code_api_key,
            codex_code_enabled: config.codex_code.enabled,
            codex_code_model: config.codex_code.model.clone(),
            codex_code_memory_limit_mb: config.codex_code.memory_limit_mb,
            codex_code_home_dir: config.codex_code.home_dir.clone(),
            interactive_idle_timeout_secs: config.sandbox.interactive_idle_timeout_secs,
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
            secrets_store: runtime_secrets_store.clone(),
        };

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        orchestrator_shutdown_tx = Some(shutdown_tx);
        tokio::spawn(async move {
            if let Err(e) =
                OrchestratorApi::start_with_shutdown(orchestrator_state, 50051, async move {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                tracing::error!("Orchestrator API failed: {}", e);
            }
        });

        if config.claude_code.enabled {
            if docker_status.is_ok() {
                tracing::info!(
                    "Claude Code sandbox mode available (model: {}, max_turns: {})",
                    config.claude_code.model,
                    config.claude_code.max_turns
                );
            } else {
                tracing::info!(
                    "Claude Code sandbox mode configured (model: {}, max_turns: {}) — will activate when Docker starts",
                    config.claude_code.model,
                    config.claude_code.max_turns
                );
            }
        }
        if config.codex_code.enabled {
            if docker_status.is_ok() {
                tracing::info!(
                    "Codex sandbox mode available (model: {})",
                    config.codex_code.model
                );
            } else {
                tracing::info!(
                    "Codex sandbox mode configured (model: {}) — will activate when Docker starts",
                    config.codex_code.model
                );
            }
        }
        Some(jm)
    } else {
        None
    };

    #[cfg(not(feature = "docker-sandbox"))]
    let _container_job_manager: Option<std::sync::Arc<std::convert::Infallible>> = None;

    // ── Channel setup ──────────────────────────────────────────────────

    let channels = Arc::new(ChannelManager::new());
    let mut channel_names: Vec<String> = Vec::new();
    for descriptor in native_lifecycle_channel_descriptors(&config) {
        channels.add_descriptor(descriptor).await;
    }
    let channel_plan = NativeChannelActivationPlan::from_input(NativeChannelActivationInput {
        cli_only: cli.cli_only,
        signal_configured: config.channels.signal.is_some(),
        nostr_configured: {
            #[cfg(feature = "nostr")]
            {
                config.channels.nostr.is_some()
            }
            #[cfg(not(feature = "nostr"))]
            {
                false
            }
        },
        discord_configured: config.channels.discord.is_some(),
        imessage_configured: {
            #[cfg(target_os = "macos")]
            {
                config.channels.imessage.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        },
        apple_mail_configured: {
            #[cfg(target_os = "macos")]
            {
                config.channels.apple_mail.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        },
        bluebubbles_configured: config.channels.bluebubbles.is_some(),
        gmail_configured: config.channels.gmail.is_some(),
        http_configured: config.channels.http.is_some(),
        gateway_configured: config.channels.gateway.is_some(),
        wasm_channels_enabled: config.channels.wasm_channels_enabled,
        wasm_channels_dir_exists: config.channels.wasm_channels_dir.exists(),
    });
    #[cfg(feature = "nostr")]
    let mut nostr_channel: Option<thinclaw::channels::NostrChannel> = None;
    #[cfg(feature = "nostr")]
    let mut nostr_runtime = None;

    #[cfg(feature = "nostr")]
    if let Some(ref nostr_config) = config.channels.nostr {
        let channel_config = thinclaw::channels::NostrConfig {
            private_key: nostr_config.private_key.clone(),
            relays: nostr_config.relays.clone(),
            owner_pubkey: nostr_config.owner_pubkey.clone(),
            social_dm_enabled: nostr_config.social_dm_enabled,
            allow_from: nostr_config.allow_from.clone(),
        };
        match thinclaw::channels::NostrChannel::new(channel_config) {
            Ok(channel) => {
                nostr_runtime = Some(channel.runtime());
                nostr_channel = Some(channel);
            }
            Err(error) => {
                tracing::error!(error = %error, "Failed to initialize Nostr runtime");
            }
        }
    }
    let mut loaded_wasm_channel_names: Vec<String> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut wasm_channel_runtime_state: Option<(
        Arc<WasmChannelRuntime>,
        Arc<PairingStore>,
        Arc<WasmChannelRouter>,
        Arc<thinclaw::channels::wasm::WasmChannelLoader>,
        std::path::PathBuf,
    )> = None;

    match entrypoint_plan.local_channel {
        Some(LocalRuntimeChannel::SingleMessage) => {
            if let Some(ref msg) = cli.message {
                channels
                    .add(Box::new(ReplChannel::with_message(msg.clone())))
                    .await;
                tracing::info!("Single message mode");
            }
        }
        Some(LocalRuntimeChannel::Tui) => {
            channels.add(Box::new(TuiChannel::new())).await;
            channel_names.push("tui".to_string());
            tracing::info!("Full-screen TUI mode enabled");
        }
        Some(LocalRuntimeChannel::Repl) => {
            let repl = ReplChannel::new();
            repl.suppress_banner();
            channels.add(Box::new(repl)).await;
            channel_names.push("repl".to_string());
            tracing::info!("REPL mode enabled");
        }
        None => {}
    }

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();
    if !cli.cli_only {
        webhook_routes.extend(
            register_native_lifecycle_channels(&config, Arc::clone(&channels), &mut channel_names)
                .await,
        );
    }

    // Load WASM channels and register their webhook routes.
    if channel_plan.wasm_channels {
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
    if channel_plan.signal
        && let Some(ref signal_config) = config.channels.signal
    {
        let channel_config = thinclaw::channels::SignalConfig {
            http_url: signal_config.http_url.clone(),
            account: signal_config.account.clone(),
            allow_from: signal_config.allow_from.clone(),
            allow_from_groups: signal_config.allow_from_groups.clone(),
            dm_policy: signal_config.dm_policy.clone(),
            group_policy: signal_config.group_policy.clone(),
            group_allow_from: signal_config.group_allow_from.clone(),
            ignore_attachments: signal_config.ignore_attachments,
            ignore_stories: signal_config.ignore_stories,
        };
        let signal_channel = SignalChannel::new(channel_config)?;
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
    #[cfg(feature = "nostr")]
    if channel_plan.nostr
        && let Some(nostr_channel) = nostr_channel.take()
        && let Some(ref nostr_config) = config.channels.nostr
    {
        channel_names.push("nostr".to_string());
        channels.add(Box::new(nostr_channel)).await;
        tracing::info!(
            relays = nostr_config.relays.len(),
            owner_pubkey = ?nostr_config.owner_pubkey,
            control_ready = nostr_config.owner_pubkey.is_some(),
            social_dm_enabled = nostr_config.social_dm_enabled,
            "Nostr channel enabled"
        );
        if nostr_config.owner_pubkey.is_none() {
            tracing::warn!(
                "Nostr channel has no owner pubkey configured — inbound commands are denied until NOSTR_OWNER_PUBKEY is set"
            );
        }
    }

    // Add Discord channel if configured and not CLI-only mode.
    if channel_plan.discord
        && let Some(ref discord_config) = config.channels.discord
    {
        let channel_config = DiscordConfig {
            bot_token: discord_config.bot_token.clone(),
            guild_id: discord_config.guild_id.clone(),
            allow_from: discord_config.allow_from.clone(),
            stream_mode: discord_config.stream_mode,
        };
        match DiscordChannel::new(channel_config) {
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
    if channel_plan.imessage
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
    if channel_plan.apple_mail
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

    // Add BlueBubbles iMessage bridge if configured and not CLI-only mode.
    // Cross-platform — works on any OS with a BlueBubbles server on a Mac.
    if channel_plan.bluebubbles
        && let Some(ref bb_config) = config.channels.bluebubbles
    {
        let channel_config = BlueBubblesConfig::new(
            bb_config.server_url.clone(),
            bb_config.password.clone(),
            bb_config.webhook_host.clone(),
            bb_config.webhook_port,
            bb_config.webhook_path.clone(),
            bb_config.allow_from.clone(),
            bb_config.send_read_receipts,
        );
        match BlueBubblesChannel::init(channel_config).await {
            Ok(bb_channel) => {
                channel_names.push("bluebubbles".to_string());
                channels.add(Box::new(bb_channel)).await;
                tracing::info!("BlueBubbles iMessage channel enabled (webhook mode)");
                if bb_config.allow_from.is_empty() {
                    tracing::warn!(
                        "BlueBubbles channel has empty allow_from list — ALL messages will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize BlueBubbles channel");
            }
        }
    }

    // Add Gmail channel if configured and not CLI-only mode.
    if channel_plan.gmail
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
    if channel_plan.http
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
    let webhook_server: Option<Arc<tokio::sync::Mutex<WebhookServer>>> = if !webhook_routes
        .is_empty()
    {
        let addr = webhook_server_addr
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));
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
    let send_message_channels = Arc::clone(&channels);
    let email_channel = {
        #[cfg(target_os = "macos")]
        {
            if config.channels.apple_mail.is_some() {
                Some("apple_mail".to_string())
            } else if config.channels.gmail.is_some() {
                Some("gmail".to_string())
            } else {
                None
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if config.channels.gmail.is_some() {
                Some("gmail".to_string())
            } else {
                None
            }
        }
    };
    components.tools.register_send_message_tool(Some(Arc::new(
        move |platform, recipient, text, thread_id| {
            let channels = Arc::clone(&send_message_channels);
            let email_channel = email_channel.clone();
            Box::pin(async move {
                let channel_name = match platform.as_str() {
                    "email" => email_channel
                        .as_deref()
                        .ok_or_else(|| "No email channel is configured.".to_string())?,
                    other => other,
                };

                channels
                    .broadcast(
                        channel_name,
                        &recipient,
                        thinclaw::channels::OutgoingResponse {
                            content: text,
                            thread_id,
                            metadata: serde_json::Value::Null,
                            attachments: Vec::new(),
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(uuid::Uuid::new_v4().to_string())
            })
        },
    )));

    #[cfg(feature = "nostr")]
    if let Some(runtime) = nostr_runtime {
        components
            .tools
            .register_sync(Arc::new(thinclaw::tools::builtin::NostrActionsTool::new(
                runtime,
            )));
        tracing::info!("Registered nostr_actions tool");
    }

    // NOTE: bootstrap_hooks() is already called inside AppBuilder::build_all()
    // (app.rs). Do NOT call it again here — that would double-register bundled
    // hooks and emit a spurious "Replacing existing hook" WARN.

    // Create session manager (shared between agent and web gateway)
    let session_manager =
        Arc::new(thinclaw::agent::SessionManager::new().with_hooks(components.hooks.clone()));

    #[cfg(feature = "docker-sandbox")]
    let sandbox_children = Some(Arc::new(thinclaw::sandbox_jobs::SandboxChildRegistry::new(
        thinclaw::sandbox_jobs::SandboxJobController::new(
            components.db.clone(),
            container_job_manager.clone(),
            job_event_tx.clone(),
            if config.sandbox.enabled {
                Some(Arc::clone(&prompt_queue))
            } else {
                None
            },
        ),
    )));
    #[cfg(not(feature = "docker-sandbox"))]
    let sandbox_children = None;
    let shared_context_manager = Arc::clone(&components.context_manager);
    let shared_db = components.db.clone();
    let shared_secrets_store = components.secrets_store.clone();
    let inject_sender = channels.inject_sender();
    #[cfg(feature = "docker-sandbox")]
    let shared_prompt_queue = if config.sandbox.enabled {
        Some(Arc::clone(&prompt_queue))
    } else {
        None
    };

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
    #[cfg(feature = "docker-sandbox")]
    components.tools.register_job_tools(
        Arc::clone(&shared_context_manager),
        container_job_manager.clone(),
        shared_db.clone(),
        None,
        job_event_tx.clone(),
        Some(inject_sender.clone()),
        shared_prompt_queue.clone(),
        sandbox_children.clone(),
        shared_secrets_store.clone(),
    );

    #[cfg(not(feature = "docker-sandbox"))]
    components.tools.register_job_tools(
        Arc::clone(&shared_context_manager),
        None,
        shared_db.clone(),
        None,
        job_event_tx.clone(),
        Some(inject_sender.clone()),
        None,
        None,
        shared_secrets_store.clone(),
    );

    // ── Gateway channel ────────────────────────────────────────────────

    #[cfg(feature = "repl")]
    let mut gateway_url: Option<String> = None;
    let mut sse_sender: Option<
        tokio::sync::broadcast::Sender<thinclaw::channels::web::types::SseEvent>,
    > = None;
    let mut gateway_state: Option<std::sync::Arc<thinclaw::channels::web::server::GatewayState>> =
        None;
    if channel_plan.gateway
        && let Some(ref gw_config) = config.channels.gateway
    {
        let mut gw = GatewayChannel::new(gw_config.clone())
            .with_llm_provider(Arc::clone(&components.llm))
            .with_llm_runtime(Arc::clone(&components.llm_runtime));
        if let Some(ref ws) = components.workspace {
            gw = gw.with_workspace(Arc::clone(ws));
        }
        gw = gw.with_session_manager(Arc::clone(&session_manager));
        gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
        gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
        gw = gw.with_tool_registry(Arc::clone(&components.tools));
        gw = gw.with_context_manager(Arc::clone(&shared_context_manager));
        if let Some(ref ext_mgr) = components.extension_manager {
            gw = gw.with_extension_manager(Arc::clone(ext_mgr));
        }
        if !components.catalog_entries.is_empty() {
            gw = gw.with_registry_entries(components.catalog_entries.clone());
        }
        if let Some(ref d) = components.db {
            gw = gw.with_store(Arc::clone(d));
        }
        #[cfg(feature = "docker-sandbox")]
        if let Some(ref jm) = container_job_manager {
            gw = gw.with_job_manager(Arc::clone(jm));
        }
        if let Some(ref sr) = components.skill_registry {
            gw = gw.with_skill_registry(Arc::clone(sr));
        }
        if let Some(ref sc) = components.skill_catalog {
            gw = gw.with_skill_catalog(Arc::clone(sc));
        }
        if let Some(ref hub) = components.skill_remote_hub {
            gw = gw.with_skill_remote_hub(hub.clone());
        }
        if let Some(ref quarantine) = components.skill_quarantine {
            gw = gw.with_skill_quarantine(Arc::clone(quarantine));
        }
        gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));
        gw = gw.with_cost_tracker(Arc::clone(&components.cost_tracker));
        if let Some(ref ss) = components.secrets_store {
            gw = gw.with_secrets_store(Arc::clone(ss));
        }
        gw = gw.with_channel_manager(Arc::clone(&channels));
        // Mount WASM channel webhook routes on the gateway so they are
        // reachable through the public tunnel URL. We create a second
        // Router instance since axum::Router is not Clone.
        if let Some((_, _, ref wasm_router, _, _)) = wasm_channel_runtime_state {
            let gateway_webhook_routes =
                thinclaw::channels::wasm::router::create_wasm_channel_router(
                    Arc::clone(wasm_router),
                    components.extension_manager.as_ref().map(Arc::clone),
                );
            gw = gw.with_webhook_routes(vec![gateway_webhook_routes]);
        }
        #[cfg(feature = "docker-sandbox")]
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

        let gateway_token_url = format!(
            "http://{}:{}/?token={}",
            gw_config.host,
            gw_config.port,
            gw.auth_token()
        );
        thinclaw::tui::set_runtime_gateway_url_override(Some(gateway_token_url.clone()));

        #[cfg(feature = "repl")]
        {
            gateway_url = Some(gateway_token_url);
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
    if matches!(
        entrypoint_plan.local_channel,
        Some(LocalRuntimeChannel::Repl)
    ) {
        if let Some(mut spinner) = quiet_startup_spinner.take() {
            spinner.stop();
        }

        let boot_tool_count = components.tools.count();
        let boot_llm_model = components.llm.active_model_name();
        let boot_cheap_model = components.cheap_llm.as_ref().map(|c| c.active_model_name());

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
            codex_code_enabled: config.codex_code.enabled,
            routines_enabled: config.routines.enabled,
            skills_enabled: config.skills.enabled,
            channels: channel_names,
            tunnel_url: {
                #[cfg(feature = "tunnel")]
                {
                    active_tunnel
                        .as_ref()
                        .and_then(|t| t.public_url())
                        .or_else(|| config.tunnel.public_url.clone())
                }
                #[cfg(not(feature = "tunnel"))]
                {
                    config.tunnel.public_url.clone()
                }
            },
            tunnel_provider: {
                #[cfg(feature = "tunnel")]
                {
                    active_tunnel.as_ref().map(|t| t.name().to_string())
                }
                #[cfg(not(feature = "tunnel"))]
                {
                    None
                }
            },
            cli_skin: config.agent.cli_skin.clone(),
        };
        thinclaw::boot_screen::print_boot_screen(&boot_info);
    }

    drop(quiet_startup_spinner);

    // ── Run the agent ──────────────────────────────────────────────────

    // Wire up channel runtime for hot-activation of WASM channels.
    // Also capture the loader & channels_dir for the hot-reload watcher.
    let mut wasm_watcher_state: Option<(
        Arc<thinclaw::channels::wasm::WasmChannelLoader>,
        std::path::PathBuf,
        Arc<thinclaw::channels::wasm::WasmChannelRouter>,
    )> = None;

    if let Some((rt, ps, router, loader, channels_dir)) = wasm_channel_runtime_state.take() {
        // Always capture for the watcher — it works without an extension manager.
        wasm_watcher_state = Some((Arc::clone(&loader), channels_dir, Arc::clone(&router)));

        // Wire the runtime into the extension manager if available.
        if let Some(ref ext_mgr) = components.extension_manager {
            ext_mgr
                .set_channel_runtime(
                    Arc::clone(&channels),
                    rt,
                    ps,
                    router,
                    thinclaw::channels::wasm::WasmChannelHostConfig::from_config(&config),
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

    if let Some(ref runtime) = components.wasm_tool_runtime {
        let mut loader = thinclaw::tools::wasm::WasmToolLoader::new(
            Arc::clone(runtime),
            Arc::clone(&components.tools),
        );
        loader = loader.with_tool_invoker(Arc::new(
            thinclaw::tools::execution::HostMediatedToolInvoker::new(
                Arc::clone(&components.tools),
                Arc::clone(&components.safety),
                thinclaw::tools::ToolExecutionLane::WorkerRuntime,
                thinclaw::tools::ToolProfile::ExplicitOnly,
            ),
        ));
        if let Some(ref secrets) = components.secrets_store {
            loader = loader.with_secrets_store(Arc::clone(secrets));
        }
        let tool_watcher = thinclaw::tools::wasm::ToolWatcher::new(
            config.wasm.tools_dir.clone(),
            Arc::new(loader),
            Arc::clone(&components.tools),
        );
        tool_watcher.seed_from_sources().await;
        tool_watcher.start().await;
        tracing::info!(
            "WASM tool hot-reload watcher started (new/modified/deleted tools auto-detected)"
        );
    }

    if let Some(ref skill_registry) = components.skill_registry {
        let skill_watcher = thinclaw::skills::SkillWatcher::new(Arc::clone(skill_registry));
        skill_watcher.seed_from_registry().await;
        skill_watcher.start().await;
        tracing::info!(
            "Skill hot-reload watcher started (new/modified/deleted SKILL.md files auto-detected)"
        );
    }

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
    }

    // ── WASM channel hot-reload watcher ─────────────────────────────────
    if let Some((loader, channels_dir, wasm_router)) = wasm_watcher_state {
        use thinclaw::channels::wasm::channel_watcher::ChannelWatcher;

        let mut watcher = ChannelWatcher::new(channels_dir, loader, Arc::clone(&channels))
            .with_webhook_router(wasm_router)
            .with_host_config(
                thinclaw::channels::wasm::WasmChannelHostConfig::from_config(&config),
            );
        if let Some(ref secrets_store) = components.secrets_store {
            watcher = watcher.with_secrets_store(Arc::clone(secrets_store), "default");
        }
        watcher.seed_from_dir().await;
        watcher.start().await;
        tracing::info!(
            "WASM channel hot-reload watcher started (new/modified/deleted .wasm files auto-detected)"
        );
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
        let (executor, mut result_rx) = thinclaw::agent::SubagentExecutor::new(
            components.llm.clone(),
            components.safety.clone(),
            components.tools.clone(),
            channels.clone(),
            thinclaw::agent::SubagentConfig {
                default_tool_profile: config.agent.subagent_tool_profile,
                ..thinclaw::agent::SubagentConfig::default()
            },
        );

        // Wire store + SSE + cost tracker for routine run finalization by subagents
        let mut executor = executor;
        if let Some(ref db) = components.db {
            executor = executor.with_store(Arc::clone(db));
        }
        if let Some(ref sender) = routine_sse_sender {
            executor = executor.with_sse_tx(sender.clone());
        }
        if let Some(ref workspace) = components.workspace {
            executor = executor.with_workspace(Arc::clone(workspace));
        }
        if let Some(ref skill_registry) = components.skill_registry {
            executor =
                executor.with_skill_registry(Arc::clone(skill_registry), config.skills.clone());
        }
        executor = executor.with_cost_tracker(Arc::clone(&components.cost_tracker));

        let executor = std::sync::Arc::new(executor);
        thinclaw::api::experiments::register_experiment_subagent_executor(std::sync::Arc::clone(
            &executor,
        ));
        if let Some(ref secrets_store) = components.secrets_store {
            thinclaw::api::experiments::register_experiment_secrets_store(std::sync::Arc::clone(
                secrets_store,
            ));
        }
        let inject_tx = channels.inject_sender();
        let db_for_subagent_results = components.db.as_ref().map(Arc::clone);

        tokio::spawn(async move {
            while let Some(msg) = result_rx.recv().await {
                let summary = if msg.result.success {
                    msg.result.response.clone()
                } else {
                    msg.result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Sub-agent failed without an error message.".to_string())
                };

                let mut metadata = msg.channel_metadata.clone();
                if !metadata.is_object() {
                    metadata = serde_json::json!({});
                }
                if let Some(map) = metadata.as_object_mut() {
                    map.insert(
                        "thread_id".to_string(),
                        serde_json::json!(msg.parent_thread_id.clone()),
                    );
                    map.insert(
                        "subagent_result".to_string(),
                        serde_json::to_value(&msg.result).unwrap_or_default(),
                    );
                }

                let content = if msg.result.success {
                    format!("[Sub-agent result from {}]\n\n{}", msg.result.name, summary)
                } else {
                    format!("[Sub-agent {} failed]\n\n{}", msg.result.name, summary)
                };

                let parent_thread_id = msg.parent_thread_id.clone();
                let mut injected = thinclaw::channels::IncomingMessage::new(
                    msg.channel_name,
                    msg.parent_user_id,
                    content,
                )
                .with_thread(parent_thread_id.clone())
                .with_metadata(metadata);
                if let Some(identity) = msg.parent_identity {
                    injected = injected.with_identity(identity);
                }

                if inject_tx.send(injected).await.is_err() {
                    tracing::warn!("Sub-agent result injection channel closed");
                    break;
                }
                if let Some(ref db) = db_for_subagent_results
                    && let Ok(parent_thread_id) = uuid::Uuid::parse_str(&parent_thread_id)
                {
                    let agent_id = msg.result.agent_id.to_string();
                    let _ =
                        thinclaw::agent::mutate_thread_runtime(db, parent_thread_id, |runtime| {
                            runtime
                                .active_subagents
                                .retain(|entry| entry.agent_id.to_string() != agent_id);
                        })
                        .await;
                }
            }
        });

        // Register sub-agent tools with the executor
        let subagent_port: std::sync::Arc<
            dyn thinclaw::tools::builtin::subagent::SubagentToolPort,
        > = executor.clone();
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::SpawnSubagentTool::new(std::sync::Arc::clone(&subagent_port)),
        ));
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::ListSubagentsTool::new(std::sync::Arc::clone(&subagent_port)),
        ));
        components.tools.register_sync(std::sync::Arc::new(
            thinclaw::tools::builtin::CancelSubagentTool::new(subagent_port),
        ));

        tracing::info!("Sub-agent system initialized (with routine finalization support)");
        executor
    };

    // Register LLM management tools (llm_select, llm_list_models).
    // The shared model override connects the tool output to the dispatcher.
    let model_override = thinclaw::tools::builtin::new_shared_model_override();
    components.tools.register_llm_tools(
        model_override.clone(),
        Arc::clone(&components.llm),
        components.cheap_llm.as_ref().map(Arc::clone),
    );
    components.llm_runtime.set_advisor_ready_callback({
        let tools = Arc::clone(&components.tools);
        move |advisor_ready| {
            if advisor_ready {
                tools.register_advisor_tool(true);
            } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let tools = Arc::clone(&tools);
                handle.spawn(async move {
                    tools.reconcile_advisor_tool_readiness(false).await;
                });
            }
        }
    });

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

    // ── Periodic cost persistence ────────────────────────────────────
    // Flush cost tracker entries to the DB every 60 seconds so cost
    // data survives restarts (fixes the data-loss-on-restart gap).
    if let Some(ref db) = components.db {
        let persistence_plan = PeriodicPersistencePlan::cost_entries();
        let persist_db = Arc::clone(db);
        let persist_tracker = Arc::clone(&components.cost_tracker);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(persistence_plan.interval);
            interval.tick().await; // skip the initial immediate tick
            let mut last_count: usize = 0;
            loop {
                interval.tick().await;
                let (snapshot, count) = {
                    let guard = persist_tracker.lock().await;
                    (guard.to_json(), guard.entry_count())
                };
                // Only write when new entries have been recorded.
                if count != last_count {
                    match persist_db
                        .set_setting("default", persistence_plan.setting_key, &snapshot)
                        .await
                    {
                        Ok(()) => {
                            tracing::debug!("[cost] Persisted {} cost entries to DB", count);
                            last_count = count;
                        }
                        Err(e) => {
                            tracing::warn!("[cost] Failed to persist cost entries: {}", e);
                        }
                    }
                }
            }
        });
        tracing::info!("Cost persistence background task started (60s interval)");
    }

    // ── Background pricing sync ──────────────────────────────────────
    // Fetch per-token pricing from OpenRouter's public API and update the
    // dynamic cost overlay in costs.rs. Runs at startup (loading DB cache
    // first for instant availability) then refreshes every 24 hours.
    {
        let pricing_db = components.db.as_ref().map(Arc::clone);
        thinclaw::llm::pricing_sync::spawn_pricing_sync(pricing_db);
        tracing::info!("Pricing sync background task started (24h interval)");
    }

    if config.experiments.enabled {
        if let Some(db) = components.db.as_ref().cloned() {
            let experiments_db = Arc::clone(&db);
            tokio::spawn(async move {
                thinclaw::api::experiments::start_experiment_controller_loop(experiments_db).await;
            });
            tracing::info!("Experiment controller reconciler started (periodic cadence)");
        }
    } else {
        tracing::info!("Experiment controller not started because experiments are disabled.");
    }

    // Clone handles for the shutdown flush (before components are moved into deps).
    let shutdown_db = components.db.as_ref().map(Arc::clone);
    let shutdown_tracker = Arc::clone(&components.cost_tracker);

    let restart_requested = Arc::new(AtomicBool::new(false));
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
        llm_runtime: Some(components.llm_runtime),
        routing_policy: Some(components.routing_policy),
        model_override: Some(model_override),
        restart_requested: Arc::clone(&restart_requested),
        sandbox_children: sandbox_children.clone(),
        runtime_ports: None,
    };

    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(Arc::clone(&shared_context_manager)),
        Some(session_manager),
    );

    #[cfg(feature = "docker-sandbox")]
    agent.scheduler().tools().register_job_tools(
        Arc::clone(&shared_context_manager),
        container_job_manager.clone(),
        shared_db.clone(),
        Some(Arc::clone(agent.scheduler())),
        job_event_tx.clone(),
        Some(inject_sender.clone()),
        shared_prompt_queue.clone(),
        sandbox_children.clone(),
        shared_secrets_store.clone(),
    );

    #[cfg(not(feature = "docker-sandbox"))]
    agent.scheduler().tools().register_job_tools(
        Arc::clone(&shared_context_manager),
        None,
        shared_db.clone(),
        Some(Arc::clone(agent.scheduler())),
        job_event_tx.clone(),
        Some(inject_sender.clone()),
        None,
        None,
        shared_secrets_store.clone(),
    );

    if let Some(ref gw_state) = gateway_state {
        *gw_state.scheduler.write().await = Some(Arc::clone(agent.scheduler()));
    }

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    // Final cost flush — captures any entries since the last periodic flush.
    if let Some(ref db) = shutdown_db {
        let persistence_plan = PeriodicPersistencePlan::cost_entries();
        let snapshot = shutdown_tracker.lock().await.to_json();
        match db
            .set_setting("default", persistence_plan.setting_key, &snapshot)
            .await
        {
            Ok(()) => tracing::info!("[cost] Final cost flush on shutdown"),
            Err(e) => tracing::warn!("[cost] Failed to persist cost entries on shutdown: {}", e),
        }
    }

    if let Some(ref server) = webhook_server {
        server.lock().await.shutdown().await;
    }

    #[cfg(feature = "docker-sandbox")]
    if let Some(tx) = orchestrator_shutdown_tx.take() {
        let _ = tx.send(());
    }

    #[cfg(feature = "tunnel")]
    if let Some(tunnel) = active_tunnel {
        tracing::info!("Stopping {} tunnel...", tunnel.name());
        if let Err(e) = tunnel.stop().await {
            tracing::warn!("Failed to stop tunnel cleanly: {}", e);
        }
    }

    tracing::info!("Agent shutdown complete");

    // Check if a restart was requested via the gateway API.
    let gateway_restart_requested = gateway_state.as_ref().is_some_and(|gw_state| {
        gw_state
            .restart_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    });
    let shutdown_plan = RuntimeShutdownPlan::from_restart_signals(
        restart_requested.load(Ordering::SeqCst),
        gateway_restart_requested,
        restart_is_managed_by_service(),
    );
    match shutdown_plan.action {
        RuntimeShutdownAction::Complete => {}
        RuntimeShutdownAction::ExitForSupervisor(code) => {
            eprintln!("Restarting ThinClaw (exit code 75 for service manager)...");
            std::process::exit(code);
        }
        RuntimeShutdownAction::Relaunch => {
            relaunch_current_process()?;
        }
    }

    Ok(())
}

fn native_lifecycle_channel_descriptors(config: &Config) -> Vec<ChannelDescriptor> {
    thinclaw::channels::native_lifecycle_channel_descriptors(&NativeLifecycleChannelConfig {
        matrix_enabled: config.channels.matrix_enabled,
        voice_call_enabled: config.channels.voice_call_enabled,
        voice_call_available: config.channels.voice_call_available,
        apns_enabled: config.channels.apns_enabled,
        browser_push_enabled: config.channels.browser_push_enabled,
        browser_push_available: config.channels.browser_push_available,
    })
}

async fn register_native_lifecycle_channels(
    config: &Config,
    channels: Arc<ChannelManager>,
    channel_names: &mut Vec<String>,
) -> Vec<axum::Router> {
    let http: Arc<dyn NativeHttpClient> = Arc::new(ReqwestNativeHttpClient::new());
    let mut webhook_config = NativeLifecycleWebhookConfig::default();

    if config.channels.matrix_enabled {
        match matrix_native_config_from_env() {
            Ok(Some(matrix_config)) => {
                let client = Arc::new(MatrixNativeClient::new(matrix_config, Arc::clone(&http)));
                let channel = NativeLifecycleChannel::matrix(client);
                webhook_config.matrix = Some(channel.ingress());
                channels.add(Box::new(channel)).await;
                channel_names.push("matrix".to_string());
                tracing::info!("Matrix native lifecycle channel enabled");
            }
            Ok(None) => {
                tracing::warn!(
                    "Matrix native lifecycle is enabled but MATRIX_HOMESERVER or MATRIX_ACCESS_TOKEN is missing"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "Matrix native lifecycle configuration is invalid")
            }
        }
    }

    if config.channels.voice_call_enabled {
        if !config.channels.voice_call_available {
            tracing::warn!(
                "Voice-call native lifecycle is enabled but the binary was built without the voice feature"
            );
        } else {
            match voice_call_native_config_from_env() {
                Ok(Some(voice_config)) => {
                    webhook_config.voice_call_secret = voice_config.webhook_secret.clone();
                    let client =
                        Arc::new(VoiceCallNativeClient::new(voice_config, Arc::clone(&http)));
                    let channel = NativeLifecycleChannel::voice_call(client);
                    webhook_config.voice_call = Some(channel.ingress());
                    channels.add(Box::new(channel)).await;
                    channel_names.push("voice-call".to_string());
                    tracing::info!("Voice-call native lifecycle channel enabled");
                }
                Ok(None) => {
                    tracing::warn!(
                        "Voice-call native lifecycle is enabled but VOICE_CALL_RESPONSE_URL is missing"
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Voice-call native lifecycle configuration is invalid")
                }
            }
        }
    }

    if config.channels.apns_enabled {
        match apns_native_config_from_env() {
            Ok(Some(apns_config)) => {
                match native_endpoint_registry_from_env("apns", "APNS_ENDPOINT_REGISTRY_PATH").await
                {
                    Ok(registry) => {
                        webhook_config.apns_registry = Some(registry.clone());
                        webhook_config.apns_registration_secret =
                            env_value("APNS_REGISTRATION_SECRET");
                        let client = Arc::new(ApnsNativeClient::with_registry(
                            apns_config,
                            Arc::clone(&http),
                            registry,
                        ));
                        channels
                            .add(Box::new(NativeLifecycleChannel::apns(client)))
                            .await;
                        channel_names.push("apns".to_string());
                        tracing::info!("APNs native lifecycle channel enabled");
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "APNs native lifecycle endpoint registry is invalid")
                    }
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "APNs native lifecycle is enabled but APNS_TEAM_ID, APNS_KEY_ID, APNS_BUNDLE_ID, or APNS_PRIVATE_KEY is missing"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "APNs native lifecycle configuration is invalid")
            }
        }
    }

    if config.channels.browser_push_enabled {
        if !config.channels.browser_push_available {
            tracing::warn!(
                "Browser-push native lifecycle is enabled but the binary was built without the browser feature"
            );
        } else {
            match browser_push_native_config_from_env() {
                Ok(Some(push_config)) => {
                    match native_endpoint_registry_from_env(
                        "browser-push",
                        "BROWSER_PUSH_ENDPOINT_REGISTRY_PATH",
                    )
                    .await
                    {
                        Ok(registry) => {
                            let client = Arc::new(BrowserPushNativeClient::with_registry(
                                push_config,
                                Arc::clone(&http),
                                registry.clone(),
                            ));
                            let channel = NativeLifecycleChannel::browser_push(client);
                            webhook_config.browser_push = Some(channel.ingress());
                            webhook_config.browser_push_registry = Some(registry);
                            webhook_config.browser_push_secret =
                                env_value("BROWSER_PUSH_WEBHOOK_SECRET");
                            channels.add(Box::new(channel)).await;
                            channel_names.push("browser-push".to_string());
                            tracing::info!("Browser-push native lifecycle channel enabled");
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "Browser-push native lifecycle endpoint registry is invalid")
                        }
                    };
                }
                Ok(None) => {
                    tracing::warn!(
                        "Browser-push native lifecycle is enabled but BROWSER_PUSH_VAPID_PUBLIC_KEY, BROWSER_PUSH_VAPID_PRIVATE_KEY, or BROWSER_PUSH_VAPID_SUBJECT is missing"
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Browser-push native lifecycle configuration is invalid")
                }
            }
        }
    }

    if webhook_config.matrix.is_some()
        || webhook_config.voice_call.is_some()
        || webhook_config.browser_push.is_some()
        || webhook_config.apns_registry.is_some()
        || webhook_config.browser_push_registry.is_some()
    {
        vec![native_lifecycle_webhook_routes(webhook_config)]
    } else {
        Vec::new()
    }
}

fn matrix_native_config_from_env() -> Result<Option<MatrixNativeConfig>, String> {
    let Some(homeserver) = env_value("MATRIX_HOMESERVER") else {
        return Ok(None);
    };
    let Some(access_token) = env_value("MATRIX_ACCESS_TOKEN") else {
        return Ok(None);
    };
    Ok(Some(MatrixNativeConfig {
        homeserver,
        access_token,
    }))
}

fn voice_call_native_config_from_env() -> Result<Option<VoiceCallNativeConfig>, String> {
    let Some(response_url) = env_value("VOICE_CALL_RESPONSE_URL") else {
        return Ok(None);
    };
    Ok(Some(VoiceCallNativeConfig {
        response_url,
        webhook_secret: env_value("VOICE_CALL_WEBHOOK_SECRET"),
    }))
}

fn apns_native_config_from_env() -> Result<Option<ApnsNativeConfig>, String> {
    let Some(team_id) = env_value("APNS_TEAM_ID") else {
        return Ok(None);
    };
    let Some(key_id) = env_value("APNS_KEY_ID") else {
        return Ok(None);
    };
    let Some(bundle_id) = env_value("APNS_BUNDLE_ID") else {
        return Ok(None);
    };
    let Some(private_key_pem) = env_value_or_file("APNS_PRIVATE_KEY", "APNS_PRIVATE_KEY_PATH")?
    else {
        return Ok(None);
    };
    Ok(Some(ApnsNativeConfig {
        team_id,
        key_id,
        bundle_id,
        private_key_pem,
        sandbox: env_bool("APNS_SANDBOX")?.unwrap_or(false),
    }))
}

fn browser_push_native_config_from_env() -> Result<Option<BrowserPushNativeConfig>, String> {
    let Some(vapid_public_key) = env_value("BROWSER_PUSH_VAPID_PUBLIC_KEY") else {
        return Ok(None);
    };
    let Some(vapid_private_key_pem) = env_value_or_file(
        "BROWSER_PUSH_VAPID_PRIVATE_KEY",
        "BROWSER_PUSH_VAPID_PRIVATE_KEY_PATH",
    )?
    else {
        return Ok(None);
    };
    let Some(subject) = env_value("BROWSER_PUSH_VAPID_SUBJECT") else {
        return Ok(None);
    };
    let ttl_seconds = match env_value("BROWSER_PUSH_TTL_SECONDS") {
        Some(value) => value.parse::<u32>().map_err(|error| {
            format!("BROWSER_PUSH_TTL_SECONDS must be a positive integer: {error}")
        })?,
        None => 60,
    };
    Ok(Some(BrowserPushNativeConfig {
        vapid_public_key,
        vapid_private_key_pem,
        subject,
        ttl_seconds,
    }))
}

async fn native_endpoint_registry_from_env(
    provider: &str,
    path_env: &str,
) -> Result<NativeEndpointRegistry, String> {
    let path = env_value(path_env).map(PathBuf::from).unwrap_or_else(|| {
        thinclaw_platform::resolve_thinclaw_home()
            .join("native-endpoints")
            .join(format!("{provider}.json"))
    });
    NativeEndpointRegistry::persistent(path)
        .await
        .map_err(|error| error.to_string())
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_value_or_file(value_key: &str, path_key: &str) -> Result<Option<String>, String> {
    if let Some(value) = env_value(value_key) {
        return Ok(Some(value.replace("\\n", "\n")));
    }
    let Some(path) = env_value(path_key) else {
        return Ok(None);
    };
    std::fs::read_to_string(&path)
        .map(|value| Some(value.replace("\\n", "\n")))
        .map_err(|error| format!("failed to read {path_key}={path}: {error}"))
}

fn env_bool(key: &str) -> Result<Option<bool>, String> {
    let Some(value) = env_value(key) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(format!("{key} must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "postgres", feature = "libsql"))]
    #[test]
    fn test_cli_guide_onboarding_keeps_runtime_handoff_enabled() {
        let config = setup_config_for_onboard_command(
            false,
            false,
            Some(thinclaw::setup::GuideTopic::Menu),
            UiMode::Cli,
            None,
        );

        assert_eq!(config.guide_topic, Some(thinclaw::setup::GuideTopic::Menu));
        assert!(!config.pause_after_completion);
    }

    #[cfg(any(feature = "postgres", feature = "libsql"))]
    #[test]
    fn test_startup_onboarding_preserves_explicit_tui_intent() {
        let config = setup_config_for_startup_onboarding(RuntimeEntryMode::Tui);

        assert_eq!(config.ui_mode, UiMode::Tui);
    }

    #[cfg(any(feature = "postgres", feature = "libsql"))]
    #[test]
    fn test_runtime_entry_mode_follows_resolved_wizard_ui() {
        assert_eq!(
            runtime_entry_mode_from_ui_mode(UiMode::Tui),
            RuntimeEntryMode::Tui
        );
        assert_eq!(
            runtime_entry_mode_from_ui_mode(UiMode::Cli),
            RuntimeEntryMode::Cli
        );
        assert_eq!(
            runtime_entry_mode_from_ui_mode(UiMode::Auto),
            RuntimeEntryMode::Cli
        );
    }
}

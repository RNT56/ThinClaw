//! Gateway lifecycle commands: start, stop, status, diagnostics, sync
//!
//! **Phase 3 migration**: `openclaw_get_status` / `openclaw_get_diagnostics` now
//! report IronClaw engine status (always running, in-process). `start_gateway`
//! and `stop_gateway` are now no-ops (IronClaw is in-process, always running).
//! Config reads still come from `OpenClawConfig` (Scrappy's identity.json).

use tauri::State;
use tracing::info;

use super::types::*;
use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;

/// Get OpenClaw status.
///
/// Config fields (API keys, grants, cloud settings) come from `OpenClawConfig`.
/// Engine status fields (`gateway_running`, `ws_connected`) reflect IronClaw's
/// in-process state.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_status(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<OpenClawStatus, String> {
    let config = state.get_config().await;

    // IronClaw is in-process — "running" means the agent was initialized
    let engine_running = ironclaw.is_initialized();

    Ok(OpenClawStatus {
        gateway_mode: config
            .as_ref()
            .map(|c| c.gateway_mode.clone())
            .unwrap_or_else(|| "local".to_string()),
        remote_url: config.as_ref().and_then(|c| c.remote_url.clone()),
        remote_token: config.as_ref().and_then(|c| c.remote_token.clone()),
        port: config.as_ref().map(|c| c.port).unwrap_or(18789),
        device_id: config
            .as_ref()
            .map(|c| c.device_id.clone())
            .unwrap_or_default(),
        auth_token: config
            .as_ref()
            .map(|c| c.auth_token.clone())
            .unwrap_or_default(),
        state_dir: config
            .as_ref()
            .map(|c| c.base_dir.to_string_lossy().to_string())
            .unwrap_or_default(),
        has_huggingface_token: config
            .as_ref()
            .and_then(|c| c.huggingface_token.clone())
            .is_some(),
        huggingface_granted: config
            .as_ref()
            .map(|c| c.huggingface_granted)
            .unwrap_or(false),
        has_anthropic_key: config
            .as_ref()
            .and_then(|c| c.anthropic_api_key.clone())
            .is_some(),
        anthropic_granted: config
            .as_ref()
            .map(|c| c.anthropic_granted)
            .unwrap_or(false),
        has_brave_key: config
            .as_ref()
            .and_then(|c| c.brave_search_api_key.clone())
            .is_some(),
        brave_granted: config.as_ref().map(|c| c.brave_granted).unwrap_or(false),
        has_openai_key: config
            .as_ref()
            .and_then(|c| c.openai_api_key.clone())
            .is_some(),
        openai_granted: config.as_ref().map(|c| c.openai_granted).unwrap_or(false),
        has_openrouter_key: config
            .as_ref()
            .and_then(|c| c.openrouter_api_key.clone())
            .is_some(),
        openrouter_granted: config
            .as_ref()
            .map(|c| c.openrouter_granted)
            .unwrap_or(false),
        has_gemini_key: config
            .as_ref()
            .and_then(|c| c.gemini_api_key.clone())
            .is_some(),
        gemini_granted: config.as_ref().map(|c| c.gemini_granted).unwrap_or(false),
        has_groq_key: config
            .as_ref()
            .and_then(|c| c.groq_api_key.clone())
            .is_some(),
        groq_granted: config.as_ref().map(|c| c.groq_granted).unwrap_or(false),
        // IronClaw engine status
        gateway_running: engine_running,
        ws_connected: engine_running, // In-process = always connected
        slack_enabled: config
            .as_ref()
            .map(|c| {
                c.custom_secrets
                    .iter()
                    .any(|s| s.id == "slack" && s.granted)
            })
            .unwrap_or(false),
        telegram_enabled: config
            .as_ref()
            .map(|c| {
                c.custom_secrets
                    .iter()
                    .any(|s| s.id == "telegram" && s.granted)
            })
            .unwrap_or(false),
        custom_secrets: config
            .as_ref()
            .map(|cfg| cfg.custom_secrets.clone())
            .unwrap_or_default(),
        node_host_enabled: config
            .as_ref()
            .map(|c| c.node_host_enabled)
            .unwrap_or(false),
        local_inference_enabled: config
            .as_ref()
            .map(|c| c.local_inference_enabled)
            .unwrap_or(false),
        selected_cloud_brain: config
            .as_ref()
            .and_then(|cfg| cfg.selected_cloud_brain.clone()),
        selected_cloud_model: config
            .as_ref()
            .and_then(|cfg| cfg.selected_cloud_model.clone()),
        setup_completed: config
            .as_ref()
            .map(|cfg| cfg.setup_completed)
            .unwrap_or(false),
        auto_start_gateway: config
            .as_ref()
            .map(|cfg| cfg.auto_start_gateway)
            .unwrap_or(false),
        dev_mode_wizard: config
            .as_ref()
            .map(|cfg| cfg.dev_mode_wizard)
            .unwrap_or(false),
        custom_llm_url: config.as_ref().and_then(|cfg| cfg.custom_llm_url.clone()),
        custom_llm_key: config.as_ref().and_then(|cfg| cfg.custom_llm_key.clone()),
        custom_llm_model: config.as_ref().and_then(|cfg| cfg.custom_llm_model.clone()),
        custom_llm_enabled: config
            .as_ref()
            .map(|cfg| cfg.custom_llm_enabled)
            .unwrap_or(false),
        enabled_cloud_providers: config
            .as_ref()
            .map(|cfg| cfg.enabled_cloud_providers.clone())
            .unwrap_or_default(),
        enabled_cloud_models: config
            .as_ref()
            .map(|cfg| cfg.enabled_cloud_models.clone())
            .unwrap_or_default(),
        profiles: config
            .as_ref()
            .map(|cfg| cfg.profiles.clone())
            .unwrap_or_default(),
        // Implicit cloud provider status
        has_xai_key: config
            .as_ref()
            .and_then(|c| c.xai_api_key.clone())
            .is_some(),
        xai_granted: config.as_ref().map(|c| c.xai_granted).unwrap_or(false),
        has_venice_key: config
            .as_ref()
            .and_then(|c| c.venice_api_key.clone())
            .is_some(),
        venice_granted: config.as_ref().map(|c| c.venice_granted).unwrap_or(false),
        has_together_key: config
            .as_ref()
            .and_then(|c| c.together_api_key.clone())
            .is_some(),
        together_granted: config.as_ref().map(|c| c.together_granted).unwrap_or(false),
        has_moonshot_key: config
            .as_ref()
            .and_then(|c| c.moonshot_api_key.clone())
            .is_some(),
        moonshot_granted: config.as_ref().map(|c| c.moonshot_granted).unwrap_or(false),
        has_minimax_key: config
            .as_ref()
            .and_then(|c| c.minimax_api_key.clone())
            .is_some(),
        minimax_granted: config.as_ref().map(|c| c.minimax_granted).unwrap_or(false),
        has_nvidia_key: config
            .as_ref()
            .and_then(|c| c.nvidia_api_key.clone())
            .is_some(),
        nvidia_granted: config.as_ref().map(|c| c.nvidia_granted).unwrap_or(false),
        has_qianfan_key: config
            .as_ref()
            .and_then(|c| c.qianfan_api_key.clone())
            .is_some(),
        qianfan_granted: config.as_ref().map(|c| c.qianfan_granted).unwrap_or(false),
        has_mistral_key: config
            .as_ref()
            .and_then(|c| c.mistral_api_key.clone())
            .is_some(),
        mistral_granted: config.as_ref().map(|c| c.mistral_granted).unwrap_or(false),
        has_xiaomi_key: config
            .as_ref()
            .and_then(|c| c.xiaomi_api_key.clone())
            .is_some(),
        xiaomi_granted: config.as_ref().map(|c| c.xiaomi_granted).unwrap_or(false),
        has_cohere_key: config
            .as_ref()
            .and_then(|c| c.cohere_api_key.clone())
            .is_some(),
        cohere_granted: config.as_ref().map(|c| c.cohere_granted).unwrap_or(false),
        has_voyage_key: config
            .as_ref()
            .and_then(|c| c.voyage_api_key.clone())
            .is_some(),
        voyage_granted: config.as_ref().map(|c| c.voyage_granted).unwrap_or(false),
        has_deepgram_key: config
            .as_ref()
            .and_then(|c| c.deepgram_api_key.clone())
            .is_some(),
        deepgram_granted: config.as_ref().map(|c| c.deepgram_granted).unwrap_or(false),
        has_elevenlabs_key: config
            .as_ref()
            .and_then(|c| c.elevenlabs_api_key.clone())
            .is_some(),
        elevenlabs_granted: config
            .as_ref()
            .map(|c| c.elevenlabs_granted)
            .unwrap_or(false),
        has_stability_key: config
            .as_ref()
            .and_then(|c| c.stability_api_key.clone())
            .is_some(),
        stability_granted: config
            .as_ref()
            .map(|c| c.stability_granted)
            .unwrap_or(false),
        has_fal_key: config
            .as_ref()
            .and_then(|c| c.fal_api_key.clone())
            .is_some(),
        fal_granted: config.as_ref().map(|c| c.fal_granted).unwrap_or(false),
        has_bedrock_key: config
            .as_ref()
            .map(|c| c.bedrock_access_key_id.is_some() && c.bedrock_secret_access_key.is_some())
            .unwrap_or(false),
        bedrock_granted: config.as_ref().map(|c| c.bedrock_granted).unwrap_or(false),
    })
}

/// Sync Local LLM config (llama-server) to OpenClaw config.
///
/// Still needed: Scrappy manages the local llama-server sidecar and needs to
/// sync its port/model info to the config that IronClaw reads on restart.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_sync_local_llm(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let local_llm = sidecar.get_chat_config();
    if local_llm.is_none() {
        return Err("Local LLM (llama-server) is not running".into());
    }

    info!(
        "[openclaw] Syncing Local LLM config: {:?}",
        local_llm.as_ref().map(|(p, _, _, _)| *p)
    );

    let existing_openclaw_engine = cfg.load_config().ok();
    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Start the IronClaw engine.
///
/// Initializes the agent, starts background tasks, emits Connected event.
/// If already running, this is a no-op (returns Ok).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_start_gateway(
    _state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    _sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    info!("[ironclaw] Start gateway requested");

    // Create secrets adapter (bridges macOS Keychain to IronClaw)
    let secrets_store: Option<std::sync::Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>> =
        Some(std::sync::Arc::new(
            crate::openclaw::ironclaw_secrets::KeychainSecretsAdapter::new(),
        ));

    match ironclaw.start(secrets_store).await {
        Ok(true) => {
            info!("[ironclaw] Engine started successfully");
            Ok(())
        }
        Ok(false) => {
            info!("[ironclaw] Engine was already running");
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to start IronClaw engine: {}", e);
            tracing::error!("{}", msg);
            Err(msg)
        }
    }
}

/// Stop the IronClaw engine gracefully.
///
/// Shuts down background tasks, channels, and emits Disconnected event.
/// If already stopped, this is a no-op (returns Ok).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_stop_gateway(
    _state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<(), String> {
    info!("[ironclaw] Stop gateway requested");

    let was_running = ironclaw.stop().await;
    if was_running {
        info!("[ironclaw] Engine stopped successfully");
    } else {
        info!("[ironclaw] Engine was already stopped");
    }

    Ok(())
}

/// Reload secrets (API keys) into the running IronClaw agent.
///
/// Performs a graceful stop→start cycle to re-inject keys from macOS Keychain.
/// Called by the frontend after API key save/toggle operations so the IronClaw
/// agent picks up changes without requiring manual restart by the user.
///
/// **Flow:** stop engine → create fresh KeychainSecretsAdapter → start engine
///
/// This is a no-op if the engine isn't running.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reload_secrets(ironclaw: State<'_, IronClawState>) -> Result<(), String> {
    info!("[ironclaw] Reload secrets requested");

    // Create a fresh secrets adapter (reads live from Keychain)
    let secrets_store: Option<std::sync::Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>> =
        Some(std::sync::Arc::new(
            crate::openclaw::ironclaw_secrets::KeychainSecretsAdapter::new(),
        ));

    ironclaw.reload_secrets(secrets_store).await?;

    info!("[ironclaw] Secrets reloaded successfully");
    Ok(())
}

/// Get gateway diagnostic info.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_diagnostics(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<OpenClawDiagnostics, String> {
    let cfg = state.get_config().await;
    let engine_running = ironclaw.is_initialized();

    let (port, state_dir, slack_enabled, telegram_enabled) = if let Some(ref cfg) = cfg {
        let (slack, telegram) = if let Ok(openclaw_engine) = cfg.load_config() {
            (
                Some(openclaw_engine.channels.slack.enabled),
                Some(openclaw_engine.channels.telegram.enabled),
            )
        } else {
            (None, None)
        };
        (
            Some(cfg.port),
            Some(cfg.state_dir().to_string_lossy().to_string()),
            slack,
            telegram,
        )
    } else {
        (None, None, None, None)
    };

    Ok(OpenClawDiagnostics {
        timestamp: chrono::Utc::now().to_rfc3339(),
        gateway_running: engine_running,
        ws_connected: engine_running,
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        port,
        state_dir,
        slack_enabled,
        telegram_enabled,
    })
}

//! Engine lifecycle commands: start, stop, status, diagnostics, sync.
//!
//! Dual-mode operation:
//!   Local mode:  ThinClaw runs in-process via TauriChannel (default)
//!   Remote mode: ThinClaw Desktop connects to an external ThinClaw HTTP gateway
//!                via RemoteGatewayProxy — no local engine is started
//!
//! The mode is selected by `identity.json:gateway_mode`:
//!   "local"  (or empty) → start embedded ThinClaw runtime
//!   "remote"            → connect to remote_url with remote_token

use tauri::State;
use tracing::info;

use super::types::*;
use super::ThinClawManager;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// Get ThinClaw status.
///
/// Config fields (API keys, grants, cloud settings) come from `ThinClawConfig`.
/// Engine status fields (`engine_running`, `engine_connected`) reflect the active
/// gateway mode: embedded runtime in local mode or a connected proxy in remote mode.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_status(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ThinClawStatus, String> {
    let config = state.get_config().await;

    let engine_running = ironclaw.is_initialized() || ironclaw.is_remote_mode().await;
    let remote_mode = ironclaw.is_remote_mode().await
        || config
            .as_ref()
            .map(|c| c.gateway_mode == "remote")
            .unwrap_or(false);

    Ok(ThinClawStatus {
        gateway_mode: config
            .as_ref()
            .map(|c| c.gateway_mode.clone())
            .unwrap_or_else(|| "local".to_string()),
        remote_url: config.as_ref().and_then(|c| c.remote_url.clone()),
        remote_token: if remote_mode {
            None
        } else {
            config.as_ref().and_then(|c| c.remote_token.clone())
        },
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
        // ThinClaw runtime status (in-process = always connected when running)
        engine_running,
        engine_connected: engine_running,
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
        allow_local_tools: config.as_ref().map(|c| c.allow_local_tools).unwrap_or(true),
        workspace_mode: config
            .as_ref()
            .map(|c| c.workspace_mode.clone())
            .unwrap_or_else(|| "sandboxed".to_string()),
        workspace_root: config.as_ref().and_then(|c| c.workspace_root.clone()),
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
        auto_approve_tools: config
            .as_ref()
            .map(|cfg| cfg.auto_approve_tools)
            .unwrap_or(false),
        bootstrap_completed: config
            .as_ref()
            .map(|cfg| cfg.bootstrap_completed)
            .unwrap_or(false),
        custom_llm_url: config.as_ref().and_then(|cfg| cfg.custom_llm_url.clone()),
        custom_llm_key: if remote_mode {
            None
        } else {
            config.as_ref().and_then(|cfg| cfg.custom_llm_key.clone())
        },
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

/// Sync Local LLM config (llama-server) to ThinClaw config.
///
/// Still needed: ThinClaw Desktop manages the local llama-server sidecar and needs to
/// sync its port/model info to the config that ThinClaw reads on restart.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_sync_local_llm(
    state: State<'_, ThinClawManager>,
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
        "[thinclaw] Syncing Local LLM config: {:?}",
        local_llm.as_ref().map(|(p, _, _, _)| *p)
    );

    let existing_thinclaw_engine = cfg.load_config().ok();
    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Start the ThinClaw gateway.
///
/// Behavior depends on `identity.json:gateway_mode`:
///   "local" (default):
///     - Waits for local inference engine if configured
///     - Starts the ThinClaw in-process engine via ThinClawRuntimeState::start()
///   "remote":
///     - Reads remote_url + remote_token from config
///     - Creates a RemoteGatewayProxy, verifies health, opens SSE subscription
///     - No local engine is started
///
/// In both modes, the frontend receives the same events via `thinclaw-event`
/// and invokes the same Tauri commands — all routing is transparent.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_start_gateway(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let oc_config = state.get_config().await;

    // ── Determine mode ──────────────────────────────────────────────────────
    let mode = oc_config
        .as_ref()
        .map(|c| c.gateway_mode.clone())
        .unwrap_or_default();

    info!("[thinclaw-runtime] Engine start requested (mode={})", mode);

    if mode == "remote" {
        // ── Remote mode: connect to external ThinClaw gateway ───────────
        let remote_url = oc_config
            .as_ref()
            .and_then(|c| c.remote_url.clone())
            .ok_or_else(|| {
                "Remote mode selected but no remote_url configured. Set it in Gateway Settings."
                    .to_string()
            })?;

        let remote_token = oc_config
            .as_ref()
            .and_then(|c| c.remote_token.clone())
            .unwrap_or_default();

        // Already in remote mode and connected? No-op.
        if ironclaw.is_remote_mode().await {
            // Check if it's the same URL
            if let Some(existing) = ironclaw.remote_proxy().await {
                if existing.base_url() == remote_url {
                    info!(
                        "[thinclaw-runtime] Already connected to remote {} — no-op",
                        remote_url
                    );
                    return Ok(());
                }
            }
            // Different URL — disconnect first, then reconnect below
            ironclaw.disconnect_remote().await;
        }

        let proxy =
            crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(&remote_url, &remote_token);

        // Verify connectivity before activating
        proxy
            .health_check()
            .await
            .map_err(|e| format!("Cannot connect to remote gateway: {}", e))?;

        // Start SSE subscription (forwards remote events as Tauri events)
        proxy
            .start_sse_subscription(app_handle.clone())
            .await
            .map_err(|e| format!("Failed to start SSE subscription: {}", e))?;

        // Activate in ThinClawRuntimeState
        ironclaw.connect_remote(proxy).await;

        // Emit Connected event so frontend updates status
        use tauri::Emitter;
        let _ = app_handle.emit(
            "thinclaw-event",
            &crate::thinclaw::ui_types::UiEvent::Connected { protocol: 2 },
        );

        info!(
            "[thinclaw-runtime] Remote gateway connected: {}",
            remote_url
        );
        return Ok(());
    }

    // ── Local mode (default): start in-process ThinClaw runtime ─────────────
    if ironclaw.is_remote_mode().await {
        // Switching from remote → local: disconnect proxy first
        ironclaw.disconnect_remote().await;
    }

    // Wait for local inference engine if needed
    let local_inference = oc_config
        .as_ref()
        .map(|c| c.local_inference_enabled)
        .unwrap_or(false);

    if local_inference {
        let has_sidecar = sidecar.get_chat_config().is_some();
        let has_engine = {
            let guard = engine_manager.engine.lock().await;
            guard
                .as_ref()
                .map(|e| e.base_url().is_some())
                .unwrap_or(false)
        };

        if !has_sidecar && !has_engine {
            info!(
                "[thinclaw-runtime] Local inference selected but server not ready — \
                 waiting for engine to come online (up to 30s)..."
            );

            let mut ready = false;
            for attempt in 1..=60 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // Check sidecar first (used by llamacpp builds)
                if sidecar.get_chat_config().is_some() {
                    info!(
                        "[thinclaw-runtime] Sidecar detected after {}ms",
                        attempt * 500
                    );
                    ready = true;
                    break;
                }

                // Check engine manager (MLX/vLLM/Ollama)
                let guard = engine_manager.engine.lock().await;
                if let Some(engine) = guard.as_ref() {
                    if engine.is_ready().await {
                        info!("[thinclaw-runtime] Engine ready after {}ms", attempt * 500);
                        ready = true;
                        break;
                    }
                }
            }

            if !ready {
                return Err("Local inference engine did not start within 30 seconds. \
                     Please ensure a model is loaded and try again."
                    .to_string());
            }
        }
    }

    // ── Start ThinClaw runtime ────────────────────────────────────────
    // Create secrets adapter (bridges macOS Keychain to ThinClaw and enforces grants)
    let cfg_for_secrets = if let Some(cfg) = state.get_config().await {
        cfg
    } else {
        state.init_config().await?
    };
    let secrets_store: Option<
        std::sync::Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>,
    > = Some(std::sync::Arc::new(
        crate::thinclaw::secrets_adapter::KeychainSecretsAdapter::with_config(&cfg_for_secrets),
    ));

    match ironclaw.start(secrets_store).await {
        Ok(true) => {
            info!("[thinclaw-runtime] Engine started successfully");
            Ok(())
        }
        Ok(false) => {
            info!("[thinclaw-runtime] Engine was already running");
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to start ThinClaw runtime: {}", e);
            tracing::error!("{}", msg);
            Err(msg)
        }
    }
}

/// Stop the ThinClaw gateway.
///
/// - Local mode: shuts down in-process engine gracefully.
/// - Remote mode: closes the SSE subscription and clears the proxy.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_stop_gateway(
    _state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<(), String> {
    info!(
        "[thinclaw-runtime] Gateway stop requested (mode={})",
        ironclaw.mode_label().await
    );

    if ironclaw.is_remote_mode().await {
        ironclaw.disconnect_remote().await;
        info!("[thinclaw-runtime] Remote proxy disconnected");
    } else {
        let was_running = ironclaw.stop().await;
        if was_running {
            info!("[thinclaw-runtime] Engine stopped successfully");
        } else {
            info!("[thinclaw-runtime] Engine was already stopped");
        }
    }

    Ok(())
}

/// Reload secrets (API keys) into the running ThinClaw runtime.
///
/// Performs a graceful stop→start cycle to re-inject keys from macOS Keychain.
/// Called by the frontend after API key save/toggle operations so the ThinClaw
/// agent picks up changes without requiring manual restart by the user.
///
/// **Flow:** stop engine → create fresh KeychainSecretsAdapter → start engine
///
/// This is a no-op if the engine isn't running.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_reload_secrets(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<(), String> {
    info!("[thinclaw-runtime] Reload secrets requested");

    // Create a fresh secrets adapter (reads live from Keychain, grants from config)
    let cfg = state
        .get_config()
        .await
        .ok_or_else(|| "ThinClaw config is not initialized".to_string())?;
    let secrets_store: Option<
        std::sync::Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>,
    > = Some(std::sync::Arc::new(
        crate::thinclaw::secrets_adapter::KeychainSecretsAdapter::with_config(&cfg),
    ));

    ironclaw.reload_secrets(secrets_store).await?;

    info!("[thinclaw-runtime] Secrets reloaded successfully");
    Ok(())
}

/// Get engine diagnostic info.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_diagnostics(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ThinClawDiagnostics, String> {
    let cfg = state.get_config().await;
    let engine_running = ironclaw.is_initialized() || ironclaw.is_remote_mode().await;

    let (port, state_dir, slack_enabled, telegram_enabled) = if let Some(ref cfg) = cfg {
        let (slack, telegram) = if let Ok(thinclaw_engine) = cfg.load_config() {
            (
                Some(thinclaw_engine.channels.slack.enabled),
                Some(thinclaw_engine.channels.telegram.enabled),
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

    Ok(ThinClawDiagnostics {
        timestamp: chrono::Utc::now().to_rfc3339(),
        engine_running,
        engine_connected: engine_running,
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        port,
        state_dir,
        slack_enabled,
        telegram_enabled,
    })
}

/// Test connectivity to a remote ThinClaw gateway.
///
/// Called by the frontend's "Test Connection" button in Gateway Settings.
/// Returns Ok(true) if reachable and healthy, Err if not reachable.
///
/// This was previously a stub (command registered but returning error).
/// Now fully implemented using RemoteGatewayProxy.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_test_connection(url: String, token: Option<String>) -> Result<bool, String> {
    let clean_url = url.trim_end_matches('/').to_string();
    let token_str = token.as_deref().unwrap_or("");

    let proxy = crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(&clean_url, token_str);
    proxy.health_check().await
}

/// Switch the active agent to a different profile.
///
/// Stops the current connection (local engine or remote proxy),
/// updates gateway settings from the selected profile, and
/// restarts the connection with the new configuration.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_switch_to_profile(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    app_handle: tauri::AppHandle,
    profile_id: String,
) -> Result<(), String> {
    info!("[thinclaw-runtime] Switching to profile: {}", profile_id);

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Find the requested profile
    let profile = cfg
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or_else(|| format!("Profile '{}' not found", profile_id))?;

    // Update gateway settings from profile
    cfg.gateway_mode = profile.mode.clone();
    cfg.remote_url = if profile.mode == "remote" && !profile.url.is_empty() {
        Some(profile.url.clone())
    } else {
        None
    };
    // Token: update in config (stored separately from Keychain for profiles)
    if let Some(token) = &profile.token {
        cfg.remote_token = Some(token.clone());
    }

    // Persist updated config
    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    info!(
        "[thinclaw-runtime] Profile '{}' (mode={}) activated - restarting gateway...",
        profile.name, profile.mode
    );

    // Restart with new settings
    thinclaw_start_gateway(state, ironclaw, sidecar, engine_manager, app_handle).await
}

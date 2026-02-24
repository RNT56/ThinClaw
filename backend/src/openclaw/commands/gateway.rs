//! Gateway lifecycle commands: start, stop, status, diagnostics, sync

use tauri::{Emitter, State};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::super::ws_client::OpenClawWsClient;
use super::types::*;
use super::OpenClawManager;
use std::sync::atomic::Ordering;

/// Get OpenClaw status
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_status(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawStatus, String> {
    let config = state.get_config().await;

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
        gateway_running: state.is_gateway_running().await,
        ws_connected: state.ws_handle.read().await.is_some(),
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
        has_bedrock_key: config
            .as_ref()
            .map(|c| c.bedrock_access_key_id.is_some() && c.bedrock_secret_access_key.is_some())
            .unwrap_or(false),
        bedrock_granted: config.as_ref().map(|c| c.bedrock_granted).unwrap_or(false),
    })
}

/// Sync Local LLM config (llama-server) to OpenClaw config
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

    // Regenerate config with new local_llm details
    // We preserve existing channels from disk/config
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

/// Start OpenClaw gateway (spawns openclaw_engine binary and connects WS client)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_start_gateway(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    start_gateway_core(&state, &sidecar).await
}

/// Core logic for starting the gateway, reusable for auto-start
pub async fn start_gateway_core(
    state: &OpenClawManager,
    sidecar: &crate::sidecar::SidecarManager,
) -> Result<(), String> {
    // Get or initialize config
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Attempt to get local_llm config, retrying briefly if not yet available
    let mut local_llm = sidecar.get_chat_config();
    if local_llm.is_none() {
        // Check if we suspect it should be running
        info!("[openclaw] Local LLM config not found immediately, waiting for sidecar...");
        for _ in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            local_llm = sidecar.get_chat_config();
            if local_llm.is_some() {
                info!(
                    "[openclaw] Local LLM config detected: {:?}",
                    local_llm.as_ref().map(|(p, _, _, _)| *p)
                );
                break;
            }
        }
    }

    // Pass local_llm to generate_config so it builds the correct models config
    // Inject detected model family for Layer 2 stop token hardening
    let mut cfg = cfg;
    cfg.local_model_family = sidecar
        .detected_model_family
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    // Perform deep migration of sessions/data paths
    if let Err(e) = cfg.deep_migrate() {
        warn!("[openclaw] Deep migration encountered issues: {}", e);
    }

    let is_local = cfg.gateway_mode == "local";
    let gateway_url = cfg.gateway_url();
    let gateway_token = cfg.gateway_token();

    info!("[openclaw] Using Base Dir: {:?}", cfg.base_dir);
    info!("[openclaw] Starting gateway with URL: {}", gateway_url);
    info!("[openclaw] Gateway token length: {}", gateway_token.len());

    // Step 1: Start openclaw_engine processes based on mode
    if is_local {
        // Stop any currently running gateway process first
        if let Some(proc) = state.gateway_process.lock().await.take() {
            info!("[openclaw] Stopping existing gateway process...");
            let _ = proc.kill();
            // forceful wait for port release
            tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        }

        // Double check if port is actually free
        let port = cfg.port;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            warn!(
                "[openclaw] Port {} seems to be in use, waiting longer...",
                port
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
        }

        state.start_openclaw_engine_process(&cfg, "gateway").await?;

        // Step 2: Wait for gateway to be ready
        // We poll the /health HTTP endpoint instead of using a fixed sleep,
        // so we react quickly when the engine is ready AND abort immediately
        // if the engine exits with code 1 before becoming ready.
        let port = cfg.port;
        let is_alive_check = {
            state
                .gateway_process
                .lock()
                .await
                .as_ref()
                .map(|p| p.is_alive.clone())
        };

        let health_url = format!("http://127.0.0.1:{}/health", port);
        let poll_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap_or_default();

        let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(20);
        let mut engine_ready = false;

        loop {
            if tokio::time::Instant::now() >= poll_deadline {
                warn!("[openclaw] Gateway health poll timed out after 20s, proceeding anyway");
                break;
            }

            // If the engine process died, abort immediately
            let alive = is_alive_check
                .as_ref()
                .map(|f| f.load(Ordering::Relaxed))
                .unwrap_or(true);
            if !alive {
                return Err("[openclaw] Gateway engine process exited before becoming ready. Check logs for details.".to_string());
            }

            // Poll /health
            match poll_client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!("[openclaw] Gateway is ready (health OK on port {})", port);
                    engine_ready = true;
                    break;
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                }
            }
        }

        if engine_ready {
            info!("[openclaw] Gateway health confirmed, connecting WS client");
        }
    } else {
        // Stop any local gateway that might be running from a previous switch
        if let Some(proc) = state.gateway_process.lock().await.take() {
            let _ = proc.kill();
        }

        // In Remote mode, if Node Host is enabled, start it as a standalone process
        if cfg.node_host_enabled {
            state.start_openclaw_engine_process(&cfg, "node").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        }
    }

    // Step 3: Connect WS client to the gateway (local or remote)
    let (event_tx, mut event_rx) = mpsc::channel(256);

    let mcp_handler =
        std::sync::Arc::new(super::super::ipc::McpRequestHandler::new(state.app.clone()));

    // Pass is_alive flag so WS client can stop retrying when engine is dead
    let gateway_alive_flag = if is_local {
        state
            .gateway_process
            .lock()
            .await
            .as_ref()
            .map(|p| p.is_alive.clone())
    } else {
        None
    };

    let (client, handle) = OpenClawWsClient::new(
        gateway_url.clone(),
        gateway_token,
        cfg.device_id.clone(),
        cfg.private_key.clone(),
        cfg.public_key.clone(),
        event_tx,
        mcp_handler,
        gateway_alive_flag,
    );

    *state.ws_handle.write().await = Some(handle);
    *state.running.write().await = true;

    // Run the client in the background
    tauri::async_runtime::spawn(async move {
        client.run_forever().await;
    });

    // Step 4: Start event listener task to emit to UI
    let app_handle = state.app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            info!("[openclaw] Emitting UI event: {:?}", event);
            let _ = app_handle.emit("openclaw-event", event);
        }
    });

    info!(
        "Started OpenClaw gateway context. Mode: {}, URL: {}",
        cfg.gateway_mode, gateway_url
    );

    Ok(())
}

/// Stop OpenClaw gateway (stops WS client and openclaw_engine process)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_stop_gateway(state: State<'_, OpenClawManager>) -> Result<(), String> {
    // Stop WS client first
    if let Some(handle) = state.ws_handle.write().await.take() {
        handle.shutdown().await.map_err(|e| e.to_string())?;
    }

    // Stop openclaw_engine process
    state.stop_openclaw_engine_process().await?;

    // Clean up auth-profiles.json (contains plaintext API keys)
    // It is fully regenerated on every gateway start from SecretStore.
    if let Some(cfg) = state.get_config().await {
        let auth_path = cfg
            .state_dir()
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");
        if auth_path.exists() {
            if let Err(e) = std::fs::remove_file(&auth_path) {
                warn!("[openclaw] Failed to clean up auth-profiles.json: {}", e);
            } else {
                info!("[openclaw] Cleaned up auth-profiles.json");
            }
        }
    }

    *state.running.write().await = false;
    info!("Stopped OpenClaw gateway and openclaw_engine process");

    Ok(())
}

/// Get gateway diagnostic info
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_diagnostics(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawDiagnostics, String> {
    let cfg = state.get_config().await;
    let running = state.is_running().await;
    let ws_connected = state.ws_handle.read().await.is_some();

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
        gateway_running: running,
        ws_connected,
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        port,
        state_dir,
        slack_enabled,
        telegram_enabled,
    })
}

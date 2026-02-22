//! Thin WebSocket RPC wrappers for OpenClaw gateway operations
//!
//! Contains commands for: cron management, skills management,
//! config schema/get/set/patch, online status, agent toggles,
//! setup completion, auto-start, dev mode, web login,
//! cloud model selection, cloud config, orchestration, and canvas.

use tauri::{Emitter, State};
use tracing::info;

use super::super::config::*;
use super::types::*;
use super::ws_rpc;
use super::OpenClawManager;

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_list(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_run(
    state: State<'_, OpenClawManager>,
    key: String,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_run(&key).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_history(
    state: State<'_, OpenClawManager>,
    key: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_history(&key, limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_list(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_toggle(
    state: State<'_, OpenClawManager>,
    key: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    ws_rpc(
        state,
        |h| async move { h.skills_update(&key, enabled).await },
    )
    .await
}
#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_status(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_status().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_deps(
    state: State<'_, OpenClawManager>,
    name: String,
    install_id: Option<String>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move {
        h.skills_install(&name, install_id.as_deref()).await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_repo(
    state: State<'_, OpenClawManager>,
    repo_url: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;

    // We'll install skills into the workspace/skills directory
    let skills_dir = cfg.workspace_dir().join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| e.to_string())?;

    // Derive name from URL
    let repo_name = repo_url
        .split('/')
        .last()
        .unwrap_or("unknown-repo")
        .trim_end_matches(".git");

    let target_dir = skills_dir.join(repo_name);

    if target_dir.exists() {
        return Err(format!(
            "Skill repository already installed at {:?}",
            target_dir
        ));
    }

    info!("Cloning skill repo {} into {:?}", repo_url, target_dir);

    let output = std::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(&repo_url)
        .arg(&target_dir)
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git clone failed: {}", stderr));
    }

    Ok(format!("Successfully installed skills from {}", repo_name))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_schema(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_schema().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_get(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_get().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_set(
    state: State<'_, OpenClawManager>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_set(&key, value).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_patch(
    state: State<'_, OpenClawManager>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_patch(patch).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_expose_inference(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        return ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "localInferenceEnabled": enabled }))
                .await
        })
        .await;
    }

    cfg.toggle_expose_inference(enabled)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());

    // We also need to emit an update or re-generate config if running
    // (This works similar to other toggles)
    Ok(serde_json::json!({ "enabled": enabled }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_setup_completed(
    state: State<'_, OpenClawManager>,
    completed: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "setupCompleted": completed }))
                .await
        })
        .await?;
        return Ok(());
    }

    cfg.set_setup_completed(completed)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_auto_start(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "autoStartGateway": enabled }))
                .await
        })
        .await?;
        // Also update local preference so UI state is consistent for next app launch logic
        // though strictly this prefers remote config usually. But auto-start applies to remote?
        // Actually auto-start usually implies starting LOCAL gateway.
        // If remote, "auto-start" might mean "auto-connect"?
        // For now, let's keep it strictly remote config update if remote.
        return Ok(());
    }

    cfg.auto_start_gateway = enabled;
    cfg.save_identity().map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_dev_mode_wizard(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Dev mode wizard is typically a local UI preference, but we sync it just in case
    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "devModeWizard": enabled }))
                .await
        })
        .await?;
        return Ok(());
    }

    cfg.set_dev_mode_wizard(enabled)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_system_presence(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.system_presence().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_logs_tail(
    state: State<'_, OpenClawManager>,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.logs_tail(limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_run(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.update_run().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_whatsapp(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_whatsapp().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_telegram(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_telegram().await }).await
}

/// Save selected cloud model
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_selected_cloud_model(
    state: State<'_, OpenClawManager>,
    model: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = model.clone().unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state.clone(), |h| async move {
            h.config_patch(serde_json::json!({ "selectedCloudModel": val }))
                .await
        })
        .await?;
        // Continue to update local config for UI consistency
    }

    let result = cfg.update_selected_cloud_model(model);
    result.map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Custom LLM config input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct CustomLlmConfigInput {
    pub url: Option<String>,
    pub key: Option<String>,
    pub model: Option<String>,
    pub enabled: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_cloud_config(
    state: State<'_, OpenClawManager>,
    enabled_providers: Vec<String>,
    enabled_models: std::collections::HashMap<String, Vec<String>>,
    custom_llm: Option<CustomLlmConfigInput>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.enabled_cloud_providers = enabled_providers.clone();
    cfg.enabled_cloud_models = enabled_models.clone();

    if let Some(c) = &custom_llm {
        cfg.custom_llm_enabled = c.enabled;
        cfg.custom_llm_url = c.url.clone();
        cfg.custom_llm_key = c.key.clone();
        cfg.custom_llm_model = c.model.clone();
    }

    // Persist to disk local
    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg.clone());

    // Sync to remote if needed
    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            let mut patch = serde_json::Map::new();
            patch.insert(
                "enabledCloudProviders".into(),
                serde_json::json!(enabled_providers),
            );
            patch.insert(
                "enabledCloudModels".into(),
                serde_json::json!(enabled_models),
            );
            if let Some(c) = custom_llm {
                patch.insert("customLlmEnabled".into(), serde_json::json!(c.enabled));
                patch.insert("customLlmUrl".into(), serde_json::json!(c.url));
                patch.insert("customLlmKey".into(), serde_json::json!(c.key));
                patch.insert("customLlmModel".into(), serde_json::json!(c.model));
            }
            h.config_patch(serde_json::Value::Object(patch)).await
        })
        .await;
    }

    Ok(())
}
// ============================================================================
// Orchestration & Canvas Commands
// ============================================================================

/// Spawn a new OpenClaw session for a specific agent
#[tauri::command]
#[specta::specta]
pub async fn openclaw_spawn_session(
    state: State<'_, OpenClawManager>,
    agent_id: String,
    task: String,
) -> Result<String, String> {
    // In a full implementation, this would RPC to the gateway to "spawn" a task on a remote agent.
    // For now, we'll implement it by creating a new session via `chat_start` or similar RPC if available,
    // or just creating a local session entry and sending the first message.

    // Using `chat_send` with a new random session ID is the defacto "spawn".
    let new_session_id = format!("agent:{}:task-{}", agent_id, uuid::Uuid::new_v4());

    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Gateway not connected")?;

    let idempotency_key = format!(
        "spawn:{}:{}",
        new_session_id,
        chrono::Utc::now().timestamp_millis()
    );

    // We send the task as the first message
    handle
        .chat_send(&new_session_id, &idempotency_key, &task, true)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        "[openclaw] Spawned session {} for agent {}",
        new_session_id, agent_id
    );

    Ok(new_session_id)
}

/// List available agents (Discovery)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_list(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<AgentProfile>, String> {
    let cfg = state.get_config().await.ok_or("Config not loaded")?;

    // In the future, this should also query the Gateway for dynamic attributes or mDNS discovered peers
    // For now, return the static config profiles + Local Core if running
    let mut profiles = cfg.profiles.clone();

    if state.is_gateway_running().await && cfg.gateway_mode == "local" {
        // Add implicit local core if not present
        if !profiles.iter().any(|p| p.id == "local-core") {
            profiles.insert(
                0,
                AgentProfile {
                    id: "local-core".to_string(),
                    name: "Local Core".to_string(),
                    url: format!("http://127.0.0.1:{}", cfg.port), // Internal URL
                    token: Some(cfg.auth_token.clone()),
                    mode: "local".to_string(),
                    auto_connect: true,
                },
            );
        }
    }

    Ok(profiles)
}

/// Push content to the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_push(
    state: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    // Emit event to frontend to update CanvasWindow
    state
        .app
        .emit("openclaw-canvas-push", content)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Navigate the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_navigate(
    state: State<'_, OpenClawManager>,
    url: String,
) -> Result<(), String> {
    // Emit event to frontend to update CanvasWindow navigation
    state
        .app
        .emit("openclaw-canvas-navigate", url)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Dispatch an event from the Canvas UI back to the agent session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_dispatch_event(
    state: State<'_, OpenClawManager>,
    session_key: String,
    run_id: Option<String>,
    event_type: String,
    payload: serde_json::Value,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    // Send generic session event via RPC
    let mut params = serde_json::json!({
        "sessionKey": session_key,
        "type": event_type,
        "payload": payload
    });
    if let Some(rid) = run_id {
        params["runId"] = serde_json::json!(rid);
    }
    handle
        .rpc("session.event", params)
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Event dispatched".into()),
    })
}

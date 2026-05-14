//! RPC commands — Configuration, settings toggles, autonomy, bootstrap, cloud model.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;
use tracing::info;

use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Config commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_schema(
    _ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // Config schema is static — return a minimal schema for the UI
    Ok(serde_json::json!({
        "type": "object",
        "properties": {
            "setupCompleted": { "type": "boolean" },
            "autoStartGateway": { "type": "boolean" },
            "devModeWizard": { "type": "boolean" },
        }
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_get(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    if let Some(store) = agent.store() {
        let resp = ironclaw::api::config::list_settings(store, "local_user")
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "settings": [] }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_set(
    ironclaw: State<'_, IronClawState>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    ironclaw::api::config::set_setting(store, "local_user", &key, &value)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_patch(
    ironclaw: State<'_, IronClawState>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    if let Some(obj) = patch.as_object() {
        for (key, value) in obj {
            ironclaw::api::config::set_setting(store, "local_user", key, value)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(serde_json::json!({ "ok": true }))
}

// ============================================================================
// Settings toggles — these write to ThinClaw Desktop identity.json via OpenClawConfig
// ============================================================================

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

    cfg.toggle_expose_inference(enabled)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

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

    cfg.set_setup_completed(completed)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
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

    cfg.set_dev_mode_wizard(enabled)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

// ============================================================================
// Autonomy mode — controls whether the agent needs per-tool approval
// ============================================================================

/// Enable or disable autonomous tool execution.
///
/// When `enabled = true` the agent runs tools without asking for user approval
/// on each call (fully autonomous mode). When `false`, the user approves each
/// tool call interactively (human-in-the-loop mode).
///
/// Persisted to identity.json and applied via env var for next engine start.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_autonomy_mode(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.set_auto_approve_tools(enabled)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    // Also propagate to the running process env so the next engine init picks it up
    std::env::set_var(
        "AGENT_AUTO_APPROVE_TOOLS",
        if enabled { "true" } else { "false" },
    );

    info!(
        "[ironclaw] Autonomy mode set to: {}",
        if enabled {
            "autonomous"
        } else {
            "human-in-the-loop"
        }
    );

    Ok(())
}

/// Get the current autonomy mode setting.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_autonomy_mode(state: State<'_, OpenClawManager>) -> Result<bool, String> {
    let cfg = state.get_config().await;
    Ok(cfg.as_ref().map(|c| c.auto_approve_tools).unwrap_or(false))
}

// ============================================================================
// Bootstrap ritual management
// ============================================================================

/// Mark the first-run identity bootstrap as completed.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_bootstrap_completed(
    state: State<'_, OpenClawManager>,
    completed: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.set_bootstrap_completed(completed)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    info!(
        "[ironclaw] Bootstrap ritual marked as: {}",
        if completed { "completed" } else { "pending" }
    );
    Ok(())
}

/// Check whether the bootstrap ritual needs to run.
///
/// Returns `true` if the agent has NOT completed the first-run identity ritual.
/// Frontend uses this on startup to conditionally show the BootstrapModal.
///
/// Self-healing: if `identity.json` says bootstrap is still needed but
/// `BOOTSTRAP.md` no longer exists in the workspace DB, the agent clearly
/// completed the ritual already (the save just failed silently).  We
/// auto-mark it done here so the button never shows the wrong label again.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_check_bootstrap_needed(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<bool, String> {
    let cfg = state.get_config().await;
    let already_done = cfg.as_ref().map(|c| c.bootstrap_completed).unwrap_or(false);

    if already_done {
        return Ok(false);
    }

    // identity.json says bootstrap needed — but verify by checking whether
    // BOOTSTRAP.md still exists in the agent's workspace DB.
    // If the DB has no BOOTSTRAP.md the ritual was already completed but the
    // save-to-disk step failed (e.g. race condition on first install).
    // Auto-heal: mark it complete now.
    match ironclaw.agent().await {
        Ok(agent) => {
            if let Some(workspace) = agent.workspace() {
                let bootstrap_exists = ironclaw::api::memory::get_file(workspace, "BOOTSTRAP.md")
                    .await
                    .map(|r| !r.content.trim().is_empty())
                    .unwrap_or(false);

                if !bootstrap_exists {
                    tracing::info!(
                        "[ironclaw] BOOTSTRAP.md not found in workspace — \
                         auto-healing bootstrap_completed flag to true"
                    );
                    // Persist the healed state
                    let mut healed_cfg = if let Some(c) = state.get_config().await {
                        c
                    } else {
                        return Ok(false); // Can't heal, return not-needed to avoid boot loop
                    };
                    let _ = healed_cfg.set_bootstrap_completed(true);
                    *state.config.write().await = Some(healed_cfg);
                    return Ok(false); // Bootstrap not needed
                }
            }
        }
        Err(_) => {
            // Engine not ready yet — we can't verify BOOTSTRAP.md existence.
            // Trust identity.json: if bootstrap_completed is false there,
            // bootstrap IS needed. This ensures the frontend correctly shows
            // "Trigger Boot Sequence" after a factory reset even before the
            // engine has fully initialized.
            tracing::debug!(
                "[ironclaw] Agent not available yet for bootstrap check, \
                 trusting identity.json (bootstrap_completed=false → needed)"
            );
            return Ok(true);
        }
    }

    Ok(true) // Bootstrap still needed (BOOTSTRAP.md exists)
}

/// Re-trigger the bootstrap ritual (Reinitiate Identity Ritual).
///
/// Resets bootstrap_completed to false so the BootstrapModal shows again
/// on next startup. The agent will re-run its identity awakening.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_trigger_bootstrap(state: State<'_, OpenClawManager>) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.set_bootstrap_completed(false)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    info!("[ironclaw] Bootstrap ritual re-triggered");
    Ok(())
}

// ============================================================================
// System commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_system_presence(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // Base: engine is always present if we reach this command
    let engine_up = ironclaw.is_initialized();

    // Default fallback when engine isn't running yet
    if !engine_up {
        return Ok(serde_json::json!({
            "online": false,
            "engine": "ironclaw",
            "mode": "embedded",
            "session_count": 0,
            "sub_agent_count": 0,
            "tool_count": 0,
            "hook_count": 0,
            "channel_count": 0,
            "routine_engine_running": false,
            "uptime_secs": null,
        }));
    }

    let agent = ironclaw.agent().await?;

    // --- Session count ---
    let session_mgr = agent.session_manager();
    let session_count: usize = {
        let sessions = session_mgr.list_sessions().await;
        sessions.len()
    };

    // --- Sub-agent count (all children across all parent sessions) ---
    let sub_agent_count: usize = {
        let all_children = super::rpc_orchestration::sub_agent_registry::all_children().await;
        all_children
    };

    // --- Tool count ---
    let tool_count = agent.tools().count();

    // --- Hook count ---
    let hook_count = agent.hooks().list_with_details().await.len();

    // --- Channel count ---
    let channel_count = {
        let mgr = agent.channels();
        mgr.channel_names().await.len()
    };

    // --- Routine engine state ---
    let routine_engine_running = {
        if let Ok(inner_guard) = ironclaw.bg_handle_ref().await {
            if let Some(inner) = inner_guard.as_ref() {
                let bg = inner.bg_handle.lock().await;
                bg.as_ref().and_then(|h| h.routine_engine()).is_some()
            } else {
                false
            }
        } else {
            false
        }
    };

    // --- Uptime (seconds since engine start — tracked via a static timestamp set on first presence call) ---
    static ENGINE_START_SECS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let uptime_secs: Option<u64> = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let start = ENGINE_START_SECS.get_or_init(|| now);
        Some(now.saturating_sub(*start))
    };

    Ok(serde_json::json!({
        "online": true,
        "engine": "ironclaw",
        "mode": "embedded",
        "session_count": session_count,
        "sub_agent_count": sub_agent_count,
        "tool_count": tool_count,
        "hook_count": hook_count,
        "channel_count": channel_count,
        "routine_engine_running": routine_engine_running,
        "uptime_secs": uptime_secs,
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_logs_tail(
    ironclaw: State<'_, IronClawState>,
    limit: u32,
) -> Result<serde_json::Value, String> {
    let broadcaster = ironclaw.log_broadcaster().await?;
    let entries = broadcaster.recent_entries();
    let cap = (limit as usize).max(1).min(2000);
    let entries: Vec<_> = entries.into_iter().rev().take(cap).rev().collect();
    // Return BOTH structured `logs` (for rich UI) and flat `lines` (for existing consumers)
    let lines: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{} [{:>5}] {}  {}",
                e.timestamp, e.level, e.target, e.message
            )
        })
        .collect();
    let logs: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "timestamp": e.timestamp,
                "level": e.level,
                "target": e.target,
                "message": e.message,
            })
        })
        .collect();
    Ok(serde_json::json!({ "lines": lines, "logs": logs }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_run(
    _ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // Alpha compatibility IPC: the public command name remains for existing
    // frontend callers, but embedded IronClaw has no separate updater process.
    Ok(serde_json::json!({ "status": "embedded", "update_available": false }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_whatsapp(
    _state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    // WhatsApp web login not supported in IronClaw desktop mode
    Err("WhatsApp web login is not available in desktop mode".into())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_telegram(
    _state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    // Telegram web login not supported in IronClaw desktop mode
    Err("Telegram web login is not available in desktop mode".into())
}

// ============================================================================
// Cloud model / cloud config — write to ThinClaw Desktop identity.json
// ============================================================================

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

    cfg.update_selected_cloud_model(model)
        .map_err(|e| e.to_string())?;

    // Regenerate engine config so IronClaw picks up the new model selection
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

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

    cfg.enabled_cloud_providers = enabled_providers;
    cfg.enabled_cloud_models = enabled_models;

    if let Some(c) = &custom_llm {
        cfg.custom_llm_enabled = c.enabled;
        cfg.custom_llm_url = c.url.clone();
        // Store custom LLM key in Keychain, not identity.json
        if let Some(ref key) = c.key {
            let _ = crate::openclaw::config::keychain::set_key("custom_llm_key", Some(key));
        }
        cfg.custom_llm_key = c.key.clone();
        cfg.custom_llm_model = c.model.clone();
    }

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate engine config so IronClaw picks up the new model allowlist
    // and provider selections. Without this, changes were lost on engine restart.
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

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

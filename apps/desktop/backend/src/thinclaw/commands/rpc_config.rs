//! RPC commands — Configuration, settings toggles, autonomy, bootstrap, cloud model.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;
use tracing::info;

use super::remote_provider_config::{apply_remote_cloud_config, apply_remote_selected_cloud_model};
use super::ThinClawManager;
use crate::thinclaw::bridge::{gated, BridgeError, RouteMode};
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const MAX_CONFIG_PATCH_ENTRIES: usize = 256;
const MAX_CONFIG_PATCH_BYTES: usize = 1024 * 1024;
const MAX_CONFIG_KEY_BYTES: usize = 256;
const MAX_CONFIG_VALUE_BYTES: usize = 256 * 1024;

fn validated_config_patch(
    patch: serde_json::Value,
) -> Result<std::collections::HashMap<String, serde_json::Value>, String> {
    let object = patch
        .as_object()
        .ok_or_else(|| "configuration patch must be a JSON object".to_string())?;
    if object.len() > MAX_CONFIG_PATCH_ENTRIES {
        return Err(format!(
            "configuration patch exceeds the {MAX_CONFIG_PATCH_ENTRIES}-entry limit"
        ));
    }
    if serde_json::to_vec(&patch)
        .map_err(|error| format!("failed to encode configuration patch: {error}"))?
        .len()
        > MAX_CONFIG_PATCH_BYTES
    {
        return Err("configuration patch exceeds the 1 MiB limit".to_string());
    }

    let mut settings = std::collections::HashMap::with_capacity(object.len());
    for (key, value) in object {
        if key.is_empty()
            || key.len() > MAX_CONFIG_KEY_BYTES
            || key.chars().any(char::is_control)
            || serde_json::to_vec(value)
                .map_err(|error| format!("failed to encode setting '{key}': {error}"))?
                .len()
                > MAX_CONFIG_VALUE_BYTES
        {
            return Err(format!(
                "configuration setting '{key}' is malformed or oversized"
            ));
        }
        settings.insert(key.clone(), value.clone());
    }
    Ok(settings)
}

// ============================================================================
// Config commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_config_schema(
    _ironclaw: State<'_, ThinClawRuntimeState>,
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
pub async fn thinclaw_config_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_settings().await;
    }

    let agent = ironclaw.agent().await?;
    if let Some(store) = agent.store() {
        let resp = thinclaw_core::api::config::list_settings(store, "local_user")
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "settings": [] }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_config_set(
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.set_setting(&key, &value).await?;
        return Ok(serde_json::json!({ "ok": true }));
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;
    let restart_gmail = key.starts_with("channels.gmail_");
    let secrets = restart_gmail
        .then(|| agent.secrets_store().cloned())
        .flatten();

    thinclaw_core::api::config::set_setting(store, "local_user", &key, &value)
        .await
        .map_err(|e| e.to_string())?;
    drop(agent);
    if restart_gmail {
        ironclaw.restart_local(secrets).await?;
    }

    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_config_patch(
    ironclaw: State<'_, ThinClawRuntimeState>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let settings = validated_config_patch(patch)?;
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.patch_settings(&settings).await?;
        return Ok(serde_json::json!({ "ok": true }));
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;
    let restart_gmail = settings
        .keys()
        .any(|key| key.starts_with("channels.gmail_"));
    let secrets = restart_gmail
        .then(|| agent.secrets_store().cloned())
        .flatten();

    thinclaw_core::api::config::import_settings(store, "local_user", &settings)
        .await
        .map_err(|error| error.to_string())?;
    drop(agent);
    if restart_gmail {
        ironclaw.restart_local(secrets).await?;
    }

    Ok(serde_json::json!({ "ok": true }))
}

// ============================================================================
// Settings toggles — these write to ThinClaw Desktop identity.json via ThinClawConfig
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_toggle_expose_inference(
    state: State<'_, ThinClawManager>,
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
pub async fn thinclaw_set_setup_completed(
    state: State<'_, ThinClawManager>,
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
pub async fn thinclaw_toggle_auto_start(
    state: State<'_, ThinClawManager>,
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
pub async fn thinclaw_set_dev_mode_wizard(
    state: State<'_, ThinClawManager>,
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
pub async fn thinclaw_set_autonomy_mode(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    enabled: bool,
) -> Result<(), BridgeError> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(gated(
            "autonomy mode mutation",
            "remote autonomy execution is controlled by the gateway host policy; desktop may only read remote autonomy status",
            "change autonomy mode on the gateway host, or run the desktop in local mode",
            RouteMode::LocalOnly,
        ));
    }

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
        "[thinclaw-runtime] Autonomy mode set to: {}",
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
pub async fn thinclaw_get_autonomy_mode(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<bool, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_autonomy_status().await?;
        return Ok(status
            .get("enabled")
            .or_else(|| status.get("running"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    let cfg = state.get_config().await;
    Ok(cfg.as_ref().map(|c| c.auto_approve_tools).unwrap_or(false))
}

// ============================================================================
// Bootstrap ritual management
// ============================================================================

/// Mark the first-run identity bootstrap as completed.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_set_bootstrap_completed(
    state: State<'_, ThinClawManager>,
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
        "[thinclaw-runtime] Bootstrap ritual marked as: {}",
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
pub async fn thinclaw_check_bootstrap_needed(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
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
                let bootstrap_exists = thinclaw_core::api::memory::get_file_for_identity(
                    workspace,
                    &super::sessions::desktop_memory_identity(),
                    "BOOTSTRAP.md",
                )
                .await
                .map(|r| !r.content.trim().is_empty())
                .unwrap_or(false);

                if !bootstrap_exists {
                    tracing::info!(
                        "[thinclaw-runtime] BOOTSTRAP.md not found in workspace — \
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
                "[thinclaw-runtime] Agent not available yet for bootstrap check, \
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
pub async fn thinclaw_trigger_bootstrap(state: State<'_, ThinClawManager>) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.set_bootstrap_completed(false)
        .map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    info!("[thinclaw-runtime] Bootstrap ritual re-triggered");
    Ok(())
}

// ============================================================================
// System commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_system_presence(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_status().await?;
        return Ok(serde_json::json!({
            "online": true,
            "engine": "thinclaw",
            "mode": "remote",
            "session_count": status.get("thread_count").or_else(|| status.get("session_count")).cloned().unwrap_or(serde_json::json!(0)),
            "sub_agent_count": status.get("sub_agent_count").cloned().unwrap_or(serde_json::json!(0)),
            "tool_count": status.get("tool_count").cloned().unwrap_or(serde_json::json!(0)),
            "hook_count": status.get("hook_count").cloned().unwrap_or(serde_json::json!(0)),
            "channel_count": status.get("channel_count").cloned().unwrap_or(serde_json::json!(0)),
            "routine_engine_running": status.get("routine_engine_running").cloned().unwrap_or(serde_json::json!(null)),
            "uptime_secs": status.get("uptime_secs").cloned().unwrap_or(serde_json::json!(null)),
            "gateway": status,
        }));
    }

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
pub async fn thinclaw_logs_tail(
    ironclaw: State<'_, ThinClawRuntimeState>,
    limit: u32,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.logs_recent().await?;
        let cap = (limit as usize).clamp(1, 2000);
        let logs = raw
            .get("logs")
            .and_then(|value| value.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .rev()
                    .take(cap)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let lines = raw
            .get("lines")
            .and_then(|value| value.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .rev()
                    .take(cap)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return Ok(serde_json::json!({ "lines": lines, "logs": logs }));
    }

    let broadcaster = ironclaw.log_broadcaster().await?;
    let entries = broadcaster.recent_entries();
    let cap = (limit as usize).clamp(1, 2000);
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
pub async fn thinclaw_update_run(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Ok(serde_json::json!({
            "status": "remote",
            "update_available": false,
            "message": "Desktop cannot update a remote ThinClaw gateway"
        }));
    }

    // Alpha compatibility IPC: the public command name remains for existing
    // frontend callers, but embedded ThinClaw has no separate updater process.
    Ok(serde_json::json!({ "status": "embedded", "update_available": false }))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_web_login_whatsapp(
    _state: State<'_, ThinClawManager>,
) -> Result<serde_json::Value, String> {
    // WhatsApp web login not supported in ThinClaw desktop mode
    Err("WhatsApp web login is not available in desktop mode".into())
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_web_login_telegram(
    _state: State<'_, ThinClawManager>,
) -> Result<serde_json::Value, String> {
    // Telegram web login not supported in ThinClaw desktop mode
    Err("Telegram web login is not available in desktop mode".into())
}

// ============================================================================
// Cloud model / cloud config — write to ThinClaw Desktop identity.json
// ============================================================================

/// Save selected cloud model
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_selected_cloud_model(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    model: Option<String>,
) -> Result<(), String> {
    let remote_mode = ironclaw.remote_proxy().await;
    if let Some(proxy) = remote_mode.as_ref() {
        let mut remote_config = proxy
            .get_providers_config()
            .await
            .map_err(|err| format!("unavailable: remote provider config: {}", err))?;
        apply_remote_selected_cloud_model(&mut remote_config, model.as_deref());
        proxy
            .set_providers_config(&remote_config)
            .await
            .map_err(|err| format!("remote provider config update failed: {}", err))?;
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.update_selected_cloud_model(model)
        .map_err(|e| e.to_string())?;

    // Regenerate engine config so ThinClaw picks up the new model selection
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

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
pub async fn thinclaw_save_cloud_config(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    enabled_providers: Vec<String>,
    enabled_models: std::collections::HashMap<String, Vec<String>>,
    custom_llm: Option<CustomLlmConfigInput>,
) -> Result<(), String> {
    let custom_llm = custom_llm.map(|mut custom| {
        custom.url = custom
            .url
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        custom.model = custom
            .model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        custom
    });
    if enabled_providers.len() > 128 || enabled_models.len() > 128 {
        return Err("cloud configuration exceeds the 128-provider limit".to_string());
    }
    let valid_provider = |provider: &str| {
        !provider.is_empty()
            && provider.len() <= 128
            && provider
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    let mut provider_ids = std::collections::HashSet::new();
    if enabled_providers
        .iter()
        .any(|provider| !valid_provider(provider) || !provider_ids.insert(provider.as_str()))
    {
        return Err("cloud providers contain an invalid or duplicated identifier".to_string());
    }
    let mut total_models = 0usize;
    for (provider, models) in &enabled_models {
        total_models = total_models.saturating_add(models.len());
        let mut model_ids = std::collections::HashSet::new();
        if !valid_provider(provider)
            || models.len() > 256
            || total_models > 4_096
            || models.iter().any(|model| {
                model.is_empty()
                    || model.len() > 512
                    || model.chars().any(char::is_control)
                    || !model_ids.insert(model.as_str())
            })
        {
            return Err("cloud model selections are malformed or excessive".to_string());
        }
    }
    if custom_llm.as_ref().is_some_and(|custom| {
        custom.url.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 2_048 || value.chars().any(char::is_control)
        }) || custom.model.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 512 || value.chars().any(char::is_control)
        }) || custom
            .key
            .as_deref()
            .is_some_and(|value| value.len() > 64 * 1024 || value.contains('\0'))
    }) {
        return Err("custom LLM configuration is malformed or oversized".to_string());
    }

    let remote_mode = ironclaw.remote_proxy().await;
    if let Some(proxy) = remote_mode.as_ref() {
        let mut remote_config = proxy
            .get_providers_config()
            .await
            .map_err(|err| format!("unavailable: remote provider config: {}", err))?;
        apply_remote_cloud_config(
            &mut remote_config,
            &enabled_providers,
            &enabled_models,
            custom_llm.as_ref().map(|cfg| cfg.enabled).unwrap_or(false),
            custom_llm.as_ref().and_then(|cfg| cfg.url.as_deref()),
            custom_llm.as_ref().and_then(|cfg| cfg.model.as_deref()),
        );
        if let Some(custom) = custom_llm.as_ref() {
            if let Some(key) = custom.key.as_deref() {
                let key = key.trim();
                if key.is_empty() {
                    proxy
                        .delete_provider_key("openai_compatible")
                        .await
                        .map_err(|err| format!("remote provider key delete failed: {err}"))?;
                } else {
                    proxy
                        .save_provider_key("openai_compatible", key)
                        .await
                        .map_err(|err| format!("remote provider key save failed: {}", err))?;
                }
            }
        }
        proxy
            .set_providers_config(&remote_config)
            .await
            .map_err(|err| format!("remote provider config update failed: {}", err))?;
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.enabled_cloud_providers = enabled_providers;
    cfg.enabled_cloud_models = enabled_models;

    let old_custom_llm_key = crate::thinclaw::config::keychain::get_key("custom_llm_key");
    let mut custom_llm_key_changed = false;
    if let Some(c) = &custom_llm {
        cfg.custom_llm_enabled = c.enabled;
        cfg.custom_llm_url = c.url.clone();
        // Store custom LLM key in Keychain, not identity.json
        if remote_mode.is_none() {
            if let Some(key) = c.key.as_deref() {
                let key = key.trim();
                crate::thinclaw::config::keychain::set_key(
                    "custom_llm_key",
                    (!key.is_empty()).then_some(key),
                )
                .map_err(|error| format!("failed to store custom LLM credential: {error}"))?;
                custom_llm_key_changed = true;
                cfg.custom_llm_key = (!key.is_empty()).then(|| key.to_string());
            }
        }
        if remote_mode.is_some() {
            cfg.custom_llm_key = None;
        }
        cfg.custom_llm_model = c.model.clone();
    }

    if let Err(error) = cfg.save_identity() {
        if custom_llm_key_changed {
            if let Err(rollback_error) = crate::thinclaw::config::keychain::set_key(
                "custom_llm_key",
                old_custom_llm_key.as_deref(),
            ) {
                return Err(format!(
                    "failed to persist cloud configuration ({error}); credential rollback also failed: {rollback_error}"
                ));
            }
        }
        return Err(error.to_string());
    }

    // Regenerate engine config so ThinClaw picks up the new model allowlist
    // and provider selections. Without this, changes were lost on engine restart.
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

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

#[cfg(test)]
mod tests {
    use super::validated_config_patch;

    #[test]
    fn config_patch_requires_a_bounded_object() {
        assert!(validated_config_patch(serde_json::json!([])).is_err());
        let oversized = serde_json::Value::Object(
            (0..257)
                .map(|index| (format!("key_{index}"), serde_json::json!(index)))
                .collect(),
        );
        assert!(validated_config_patch(oversized).is_err());
    }

    #[test]
    fn config_patch_rejects_bad_keys_and_large_values() {
        assert!(validated_config_patch(serde_json::json!({ "bad\nkey": true })).is_err());
        assert!(validated_config_patch(serde_json::json!({
            "valid": "x".repeat(256 * 1024 + 1)
        }))
        .is_err());
    }

    #[test]
    fn config_patch_preserves_valid_entries() {
        let settings = validated_config_patch(serde_json::json!({
            "channels.gmail_enabled": true,
            "channels.gmail_project_id": "project"
        }))
        .expect("valid patch");
        assert_eq!(settings.len(), 2);
        assert_eq!(settings["channels.gmail_enabled"], serde_json::json!(true));
    }
}

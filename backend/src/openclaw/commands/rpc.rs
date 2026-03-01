//! RPC commands — skills, cron/routines, config, system, cloud settings.
//!
//! **Phase 3 migration**: Skills/cron use IronClaw API directly. Config commands
//! (schema/get/set/patch) use IronClaw's `api::config` module. Settings toggles
//! (setup, auto-start, dev mode, cloud model) still use `OpenClawConfig` since
//! they write to Scrappy's identity.json.

use std::sync::Arc;

use tauri::{Emitter, State};
use tracing::info;

use super::super::config::*;
use super::types::*;
use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Skills commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    if let Some(registry) = agent.skill_registry() {
        let resp = ironclaw::api::skills::list_skills(registry).map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "skills": [], "count": 0 }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_toggle(
    ironclaw: State<'_, IronClawState>,
    key: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;

    // Use spawn_blocking to avoid holding std::sync::RwLock guard across await
    let registry = Arc::clone(registry);
    let result = tokio::task::spawn_blocking(move || {
        if enabled {
            let _reg = registry
                .write()
                .map_err(|e| format!("Lock poisoned: {}", e))?;
            // install_skill is async internally but we need a sync path
            // For now, return a stub — the skill toggle just tracks state
            Ok::<_, String>(serde_json::json!({ "ok": true, "action": "enabled", "skill": key }))
        } else {
            let _reg = registry
                .write()
                .map_err(|e| format!("Lock poisoned: {}", e))?;
            Ok(serde_json::json!({ "ok": true, "action": "disabled", "skill": key }))
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    result
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    if let Some(registry) = agent.skill_registry() {
        let resp = ironclaw::api::skills::list_skills(registry).map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "skills": [], "count": 0 }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_deps(
    _ironclaw: State<'_, IronClawState>,
    name: String,
    _install_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // TODO: Wire up skill dependency installation when registry API is refactored
    // to use async-safe locks (tokio::sync::RwLock).
    Ok(serde_json::json!({
        "ok": true,
        "message": format!("Skill deps install for '{}' acknowledged", name),
    }))
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

    let skills_dir = cfg.workspace_dir().join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| e.to_string())?;

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

// ============================================================================
// Cron / Routines commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    if let Some(store) = agent.store() {
        let resp = ironclaw::api::routines::list_routines(store, "local_user")
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "routines": [] }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_run(
    ironclaw: State<'_, IronClawState>,
    key: String,
) -> Result<serde_json::Value, String> {
    // Parse UUID from the routine key
    let routine_id: uuid::Uuid = key
        .parse()
        .map_err(|e| format!("Invalid routine ID: {}", e))?;

    // Get the routine engine from the background tasks handle
    let inner_guard = ironclaw.bg_handle_ref().await?;
    let inner = inner_guard.as_ref().ok_or("Engine is not running")?;
    let bg_guard = inner.bg_handle.lock().await;
    let engine = bg_guard
        .as_ref()
        .and_then(|h| h.routine_engine())
        .ok_or("Routine engine not available")?;
    let engine = Arc::clone(engine);
    drop(bg_guard); // Release lock before async call

    let run_id = engine
        .fire_manual(routine_id)
        .await
        .map_err(|e| format!("Routine trigger failed: {}", e))?;

    Ok(serde_json::json!({
        "ok": true,
        "run_id": run_id.to_string(),
        "routine_id": key,
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_history(
    _ironclaw: State<'_, IronClawState>,
    _key: String,
    _limit: u32,
) -> Result<serde_json::Value, String> {
    // Routine history isn't in the IronClaw API yet — return empty
    Ok(serde_json::json!({ "history": [] }))
}

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
// Settings toggles — these write to Scrappy's identity.json via OpenClawConfig
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
// System commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_system_presence(
    _ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // System presence: in-process, always present
    Ok(serde_json::json!({
        "online": true,
        "engine": "ironclaw",
        "mode": "embedded",
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_logs_tail(
    ironclaw: State<'_, IronClawState>,
    _limit: u32,
) -> Result<serde_json::Value, String> {
    let broadcaster = ironclaw.log_broadcaster().await?;
    let entries = broadcaster.recent_entries();
    let logs: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "timestamp": e.timestamp,
                "level": e.level,
                "target": e.target,
                "message": e.message,
            })
        })
        .collect();
    Ok(serde_json::json!({ "logs": logs }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_run(
    _ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // No separate engine to update — stub
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
// Cloud model / cloud config — write to Scrappy's identity.json
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
        cfg.custom_llm_key = c.key.clone();
        cfg.custom_llm_model = c.model.clone();
    }

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

// ============================================================================
// Orchestration & Canvas Commands
// ============================================================================

/// In-memory registry of sub-agent sessions and their parent relationships.
///
/// This is separate from IronClaw's session storage — it only tracks the
/// parent→child spawning relationships and task metadata needed for the
/// SubAgentPanel UI. Sessions are evicted from this registry when the parent
/// session is deleted or the engine is stopped.
mod sub_agent_registry {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use tokio::sync::RwLock;

    use super::super::types::ChildSessionInfo;

    /// Global sub-agent registry (per-process lifetime).
    static REGISTRY: OnceLock<RwLock<SubAgentStore>> = OnceLock::new();

    struct SubAgentStore {
        /// parent_session → list of child sessions
        children: HashMap<String, Vec<ChildSessionInfo>>,
    }

    fn store() -> &'static RwLock<SubAgentStore> {
        REGISTRY.get_or_init(|| {
            RwLock::new(SubAgentStore {
                children: HashMap::new(),
            })
        })
    }

    /// Register a new child session under a parent.
    pub async fn register(parent: &str, child: ChildSessionInfo) {
        let mut s = store().write().await;
        s.children
            .entry(parent.to_string())
            .or_default()
            .push(child);
    }

    /// List all child sessions of a parent.
    pub async fn list_children(parent: &str) -> Vec<ChildSessionInfo> {
        let s = store().read().await;
        s.children.get(parent).cloned().unwrap_or_default()
    }

    /// Update a child session's status and optional result summary.
    pub async fn update_status(
        child_session_key: &str,
        status: &str,
        result_summary: Option<&str>,
    ) -> Option<String> {
        let mut s = store().write().await;
        for children in s.children.values_mut() {
            if let Some(child) = children
                .iter_mut()
                .find(|c| c.session_key == child_session_key)
            {
                child.status = status.to_string();
                if let Some(summary) = result_summary {
                    child.result_summary = Some(summary.to_string());
                }
                // Return the parent session key for event emission
                return Some(child_session_key.to_string());
            }
        }
        None
    }

    /// Find the parent session for a given child session.
    pub async fn find_parent(child_session_key: &str) -> Option<String> {
        let s = store().read().await;
        for (parent, children) in &s.children {
            if children.iter().any(|c| c.session_key == child_session_key) {
                return Some(parent.clone());
            }
        }
        None
    }

    /// Remove all child records for a parent (called on session deletion).
    #[allow(dead_code)]
    pub async fn remove_parent(parent: &str) {
        let mut s = store().write().await;
        s.children.remove(parent);
    }

    /// Clear the entire registry (called on engine stop).
    #[allow(dead_code)]
    pub async fn clear() {
        let mut s = store().write().await;
        s.children.clear();
    }
}

/// Spawn a new sub-agent session with optional parent tracking.
///
/// If `parent_session` is provided, the child session is registered in the
/// sub-agent registry and a `SubAgentUpdate` event is emitted to the parent
/// session's frontend. If no parent is provided, behaves like a standalone
/// session spawn.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_spawn_session(
    ironclaw: State<'_, IronClawState>,
    agent_id: String,
    task: String,
    parent_session: Option<String>,
) -> Result<SpawnSessionResponse, String> {
    let new_session_id = format!("agent:{}:task-{}", agent_id, uuid::Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Activate the new session for event routing
    ironclaw.activate_session(&new_session_id).await?;

    // Register in sub-agent registry if this is a child session
    if let Some(ref parent) = parent_session {
        let child_info = ChildSessionInfo {
            session_key: new_session_id.clone(),
            task: task.clone(),
            status: "running".to_string(),
            spawned_at: now,
            result_summary: None,
        };
        sub_agent_registry::register(parent, child_info).await;

        // Emit SubAgentUpdate to the parent session's frontend
        use tauri::Emitter;
        let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
            parent_session: parent.clone(),
            child_session: new_session_id.clone(),
            task: task.clone(),
            status: "running".to_string(),
            progress: Some(0.0),
            result_preview: None,
        };
        let _ = ironclaw.app_handle().emit("openclaw-event", &event);
    }

    // Send the task as the first message using IronClaw API
    let agent = ironclaw.agent().await?;
    ironclaw::api::chat::send_message(agent, &new_session_id, &task, true)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        "[ironclaw] Spawned session {} for agent {} (parent: {:?})",
        new_session_id, agent_id, parent_session
    );

    Ok(SpawnSessionResponse {
        session_key: new_session_id,
        parent_session,
        task,
    })
}

/// List all child sessions spawned by a parent session.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_child_sessions(
    _ironclaw: State<'_, IronClawState>,
    parent_session: String,
) -> Result<Vec<ChildSessionInfo>, String> {
    Ok(sub_agent_registry::list_children(&parent_session).await)
}

/// Update a sub-agent's status (called when a child session completes or fails).
///
/// Also emits a `SubAgentUpdate` event to the parent session's frontend.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_sub_agent_status(
    ironclaw: State<'_, IronClawState>,
    child_session: String,
    status: String,
    result_summary: Option<String>,
) -> Result<OpenClawRpcResponse, String> {
    // Find the parent before updating
    let parent = sub_agent_registry::find_parent(&child_session).await;

    // Update the registry
    sub_agent_registry::update_status(&child_session, &status, result_summary.as_deref()).await;

    // Emit SubAgentUpdate to the parent session's frontend
    if let Some(parent_key) = parent {
        // Look up the task from the registry
        let children = sub_agent_registry::list_children(&parent_key).await;
        let task = children
            .iter()
            .find(|c| c.session_key == child_session)
            .map(|c| c.task.clone())
            .unwrap_or_default();

        use tauri::Emitter;
        let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
            parent_session: parent_key,
            child_session: child_session.clone(),
            task,
            status: status.clone(),
            progress: if status == "completed" {
                Some(1.0)
            } else {
                None
            },
            result_preview: result_summary.clone(),
        };
        let _ = ironclaw.app_handle().emit("openclaw-event", &event);
    }

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(format!("Sub-agent {} status: {}", child_session, status)),
    })
}

/// List available agents (Discovery)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_list(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<AgentProfile>, String> {
    let cfg = state.get_config().await.ok_or("Config not loaded")?;
    let mut profiles = cfg.profiles.clone();

    if ironclaw.is_initialized() {
        if !profiles.iter().any(|p| p.id == "local-core") {
            profiles.insert(
                0,
                AgentProfile {
                    id: "local-core".to_string(),
                    name: "Local Core".to_string(),
                    url: "embedded://ironclaw".to_string(),
                    token: None,
                    mode: "embedded".to_string(),
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
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    _run_id: Option<String>,
    event_type: String,
    payload: serde_json::Value,
) -> Result<OpenClawRpcResponse, String> {
    // Inject the canvas event as a message to the agent
    let content = serde_json::json!({
        "type": "canvas_event",
        "event_type": event_type,
        "payload": payload,
    })
    .to_string();

    let agent = ironclaw.agent().await?;
    ironclaw::api::chat::send_message(
        agent,
        &session_key,
        &content,
        false, // Context injection, don't trigger turn
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Event dispatched".into()),
    })
}

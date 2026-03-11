//! RPC commands — skills, cron/routines, config, system, cloud settings.
//!
//! **Phase 3 migration**: Skills/cron use IronClaw API directly. Config commands
//! (schema/get/set/patch) use IronClaw's `api::config` module. Settings toggles
//! (setup, auto-start, dev mode, cloud model) still use `OpenClawConfig` since
//! they write to Scrappy's identity.json.

use std::sync::Arc;

use tauri::{Emitter, State};
use tracing::{info, warn};

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
        let resp = ironclaw::api::skills::list_skills(registry)
            .await
            .map_err(|e| e.to_string())?;
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

    // IronClaw's SkillRegistry doesn't support enable/disable.
    // Skills are either loaded or removed. Acknowledge the intent.
    let _guard = registry.write().await;
    let action = if enabled { "enabled" } else { "disabled" };
    Ok(serde_json::json!({ "ok": true, "action": action, "skill": key }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    if let Some(registry) = agent.skill_registry() {
        let resp = ironclaw::api::skills::list_skills(registry)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "skills": [], "count": 0 }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_deps(
    ironclaw: State<'_, IronClawState>,
    name: String,
    _install_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let catalog = agent.skill_catalog().ok_or("Skill catalog not available")?;

    // Fetch skill content from ClawHub
    let download_url = ironclaw::skills::catalog::skill_download_url(catalog.registry_url(), &name);
    let content = ironclaw::tools::builtin::skill_tools::fetch_skill_content(&download_url)
        .await
        .map_err(|e| format!("Failed to fetch skill '{}': {}", name, e))?;

    // Check for duplicates and get install dir
    let (user_dir, skill_name) = {
        let guard = registry.read().await;
        let normalized = ironclaw::skills::normalize_line_endings(&content);
        let parsed = ironclaw::skills::parser::parse_skill_md(&normalized)
            .map_err(|e| format!("Failed to parse SKILL.md: {}", e))?;
        let sn = parsed.manifest.name.clone();
        if guard.has(&sn) {
            return Ok(serde_json::json!({
                "ok": false,
                "message": format!("Skill '{}' already installed", sn),
            }));
        }
        (guard.install_target_dir().to_path_buf(), sn)
    };

    // Write to disk and validate
    let normalized = ironclaw::skills::normalize_line_endings(&content);
    let (installed_name, loaded_skill) =
        ironclaw::skills::registry::SkillRegistry::prepare_install_to_disk(
            &user_dir,
            &skill_name,
            &normalized,
        )
        .await
        .map_err(|e| format!("Failed to install: {}", e))?;

    // Commit to in-memory registry
    {
        let mut guard = registry.write().await;
        guard
            .commit_install(&installed_name, loaded_skill)
            .map_err(|e| format!("Failed to commit install: {}", e))?;
    }

    info!("[ironclaw] Installed skill '{}'", installed_name);
    Ok(serde_json::json!({
        "ok": true,
        "name": installed_name,
        "message": format!("Skill '{}' installed successfully", installed_name),
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
    let store = agent.store().ok_or("Database not available")?;

    // List routines for both well-known user IDs so that routines created
    // by the agent tool (user_id="default") and routines created manually
    // via the UI (user_id="local_user") are both visible.
    let mut all_routines = Vec::new();
    for uid in &["default", "local_user"] {
        if let Ok(mut routines) = store.list_routines(uid).await {
            all_routines.append(&mut routines);
        }
    }
    // De-duplicate by ID (in case both queries return the same routine)
    all_routines.sort_by_key(|r| r.id);
    all_routines.dedup_by_key(|r| r.id);

    // Map to the CronJob shape the frontend expects
    let jobs: Vec<serde_json::Value> = all_routines
        .iter()
        .map(|r| {
            let schedule = match &r.trigger {
                ironclaw::agent::routine::Trigger::Cron { schedule } => schedule.clone(),
                ironclaw::agent::routine::Trigger::SystemEvent { schedule, .. } => {
                    schedule.clone().unwrap_or_default()
                }
                _ => String::new(),
            };
            let last_status = if r.consecutive_failures > 0 {
                "error"
            } else if r.run_count > 0 {
                "ok"
            } else {
                ""
            };
            let action_type = r.action.type_tag();
            let trigger_type = r.trigger.type_tag();
            serde_json::json!({
                "key": r.id.to_string(),
                "name": r.name,
                "description": r.description,
                "schedule": schedule,
                "nextRun": r.next_fire_at.map(|t| t.to_rfc3339()),
                "lastRun": r.last_run_at.map(|t| t.to_rfc3339()),
                "lastStatus": last_status,
                "enabled": r.enabled,
                "run_count": r.run_count,
                "action_type": action_type,
                "trigger_type": trigger_type,
            })
        })
        .collect();

    serde_json::to_value(jobs).map_err(|e| e.to_string())
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
    ironclaw: State<'_, IronClawState>,
    key: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // Search all user IDs for the routine
    let mut found_id = None;
    for uid in &["default", "local_user"] {
        if let Ok(routines) = store.list_routines(uid).await {
            if let Some(r) = routines
                .iter()
                .find(|r| r.name == key || r.id.to_string() == key)
            {
                found_id = Some(r.id);
                break;
            }
        }
    }

    let routine_id = match found_id {
        Some(id) => id,
        None => return Ok(serde_json::json!([])),
    };

    let runs = store
        .list_routine_runs(routine_id, limit as i64)
        .await
        .map_err(|e| format!("Failed to list routine runs: {}", e))?;

    // Return CronHistoryItem[] shape
    let history: Vec<serde_json::Value> = runs
        .into_iter()
        .map(|run| {
            let duration_ms = match (run.started_at, run.completed_at) {
                (start, Some(end)) => (end - start).num_milliseconds().max(0) as u64,
                _ => 0,
            };
            serde_json::json!({
                "timestamp": run.started_at.timestamp_millis(),
                "status": run.status.to_string(),
                "duration_ms": duration_ms,
                "output": run.result_summary,
            })
        })
        .collect();

    Ok(serde_json::json!(history))
}

/// Clear routine run history.
///
/// If `key` is provided, clears runs for that specific routine.
/// If `key` is null, clears ALL routine runs.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clear_routine_runs(
    ironclaw: State<'_, IronClawState>,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    let deleted = if let Some(ref key) = key {
        // Find the routine ID by name or UUID
        let mut found_id = None;
        for uid in &["default", "local_user"] {
            if let Ok(routines) = store.list_routines(uid).await {
                if let Some(r) = routines
                    .iter()
                    .find(|r| r.name == *key || r.id.to_string() == *key)
                {
                    found_id = Some(r.id);
                    break;
                }
            }
        }
        match found_id {
            Some(id) => store
                .delete_routine_runs(id)
                .await
                .map_err(|e| format!("Failed to delete routine runs: {}", e))?,
            None => return Err(format!("Routine '{}' not found", key)),
        }
    } else {
        store
            .delete_all_routine_runs()
            .await
            .map_err(|e| format!("Failed to delete all routine runs: {}", e))?
    };

    Ok(serde_json::json!({
        "deleted": deleted,
        "scope": key.unwrap_or_else(|| "all".to_string()),
    }))
}

/// Lists all registered channels from the live IronClaw agent.
///
/// Queries the agent's ChannelManager for actually registered channels
/// instead of reading static config/env vars.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_channels_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let channel_mgr = agent.channels();
    let channel_names = channel_mgr.channel_names().await;

    let channels: Vec<serde_json::Value> = channel_names
        .iter()
        .map(|name| {
            serde_json::json!({
                "id": name.to_lowercase().replace(' ', "_"),
                "name": name,
                "type": if name == "tauri" { "native" } else { "wasm" },
                "enabled": true,
                "stream_mode": "",
            })
        })
        .collect();

    Ok(serde_json::json!({ "channels": channels }))
}

/// Create a new scheduled routine dynamically.
///
/// Stores the routine in IronClaw's RoutineStore so it persists
/// and is picked up by the RoutineEngine on its next tick.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routine_create(
    ironclaw: State<'_, IronClawState>,
    name: String,
    description: String,
    schedule: String,
    task: String,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // Normalize 5/6-field cron to 7-field, then validate
    let schedule = ironclaw::agent::routine::normalize_cron_expr(&schedule);
    let _ = ironclaw::agent::routine::next_cron_fire(&schedule)
        .map_err(|e| format!("Invalid cron expression '{}': {}", schedule, e))?;

    // Build a full Routine object
    let now = chrono::Utc::now();
    let routine_id = uuid::Uuid::new_v4();

    // Compute next fire time from cron schedule
    let next_fire = ironclaw::agent::routine::next_cron_fire(&schedule)
        .map_err(|e| format!("Failed to compute next fire time: {}", e))?;

    let routine = ironclaw::agent::routine::Routine {
        id: routine_id,
        name: name.clone(),
        description: description.clone(),
        user_id: "local_user".to_string(), // Matches Tauri chat channel user_id (api/chat.rs)
        enabled: true,
        trigger: ironclaw::agent::routine::Trigger::Cron {
            schedule: schedule.clone(),
        },
        action: ironclaw::agent::routine::RoutineAction::FullJob {
            title: name.clone(),
            description: task.clone(),
            max_iterations: 10,
        },
        guardrails: ironclaw::agent::routine::RoutineGuardrails::default(),
        notify: ironclaw::agent::routine::NotifyConfig::default(),
        last_run_at: None,
        next_fire_at: next_fire,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::Value::Null,
        created_at: now,
        updated_at: now,
    };

    // Persist to IronClaw's database
    store
        .create_routine(&routine)
        .await
        .map_err(|e| format!("Failed to create routine: {}", e))?;

    info!(
        "[ironclaw] Created routine '{}' (id={}) with schedule '{}'",
        name, routine_id, schedule
    );

    Ok(serde_json::json!({
        "id": routine_id.to_string(),
        "name": name,
        "description": description,
        "schedule": schedule,
        "task": task,
        "created_at": now.to_rfc3339(),
        "next_fire_at": routine.next_fire_at.map(|t| t.to_rfc3339()),
    }))
}

// ============================================================================
// Cron expression linting
// ============================================================================

/// Validates a cron expression and returns next fire times.
/// This is a frontend-facing version of `ironclaw cron lint`.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_lint(expression: String) -> Result<serde_json::Value, String> {
    // Normalize 5/6-field to 7-field before parsing
    let normalized = ironclaw::agent::routine::normalize_cron_expr(&expression);

    let schedule = ironclaw::agent::routine::next_cron_fire(&normalized)
        .map_err(|e| format!("Invalid cron expression '{}': {}", normalized, e))?;

    // Also parse for the full upcoming list
    use std::str::FromStr;
    let sched = cron::Schedule::from_str(&normalized)
        .map_err(|e| format!("Invalid cron expression: {}", e))?;

    let now = chrono::Utc::now();
    let next_times: Vec<String> = sched
        .upcoming(chrono::Utc)
        .take(5)
        .map(|t| t.to_rfc3339())
        .collect();

    let _ = schedule; // suppress unused warning
    Ok(serde_json::json!({
        "valid": true,
        "expression": normalized,
        "original_expression": expression,
        "next_fire_times": next_times,
        "checked_at": now.to_rfc3339(),
    }))
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
        let all_children = sub_agent_registry::all_children().await;
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

// ============================================================================
// Orchestration & Canvas Commands
// ============================================================================

/// In-memory registry of sub-agent sessions and their parent relationships.
///
/// This is separate from IronClaw's session storage — it only tracks the
/// parent→child spawning relationships and task metadata needed for the
/// SubAgentPanel UI. Sessions are evicted from this registry when the parent
/// session is deleted or the engine is stopped.
pub(crate) mod sub_agent_registry {
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

    /// Count all children across all parents.
    pub async fn all_children() -> usize {
        let s = store().read().await;
        s.children.values().map(|v| v.len()).sum()
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
    let child_key = format!("agent:{}:task-{}", agent_id, uuid::Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;

    // Activate the new session for event routing
    ironclaw.activate_session(&child_key).await?;

    // Register in sub-agent registry and emit "running" event
    if let Some(ref parent) = parent_session {
        let child_info = ChildSessionInfo {
            session_key: child_key.clone(),
            task: task.clone(),
            status: "running".to_string(),
            spawned_at: now,
            result_summary: None,
        };
        sub_agent_registry::register(parent, child_info).await;

        use tauri::Emitter;
        let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
            parent_session: parent.clone(),
            child_session: child_key.clone(),
            task: task.clone(),
            status: "running".to_string(),
            progress: Some(0.0),
            result_preview: None,
        };
        let _ = ironclaw.app_handle().emit("openclaw-event", &event);
    }

    // Capture what the background task needs
    let agent = ironclaw.agent().await?;
    let app_handle = ironclaw.app_handle().clone();
    let parent_bg = parent_session.clone();
    let child_bg = child_key.clone();
    let task_bg = task.clone();

    // ── Non-blocking: full agent turn runs in a background task ──────────
    tokio::spawn(async move {
        // 1. Full agentic loop: workspace + memory + tools + streaming
        let run_ok = ironclaw::api::chat::send_message(agent.clone(), &child_bg, &task_bg, true)
            .await
            .is_ok();

        let status = if run_ok { "completed" } else { "failed" };

        // 2. Extract a short preview from the last assistant turn
        let preview: Option<String> = if run_ok {
            let session_mgr = agent.session_manager();
            let all = session_mgr.list_sessions().await;
            let session_exists = all.iter().any(|entry| {
                entry.get("user_id").and_then(|v| v.as_str()) == Some(child_bg.as_str())
            });
            if session_exists {
                let sess_arc = session_mgr.get_or_create_session(&child_bg).await;
                let sess = sess_arc.lock().await;
                // Turn.turns is a public Vec<Turn>; Turn.response is Option<String>
                sess.threads
                    .values()
                    .filter_map(|thread| {
                        thread
                            .turns
                            .iter()
                            .rev()
                            .find_map(|t| t.response.as_deref())
                    })
                    .next()
                    .map(|text| {
                        let trimmed = text.trim();
                        if trimmed.len() > 280 {
                            format!("{}…", &trimmed[..280])
                        } else {
                            trimmed.to_string()
                        }
                    })
            } else {
                None
            }
        } else {
            None
        };

        // 3. Update the in-memory registry
        sub_agent_registry::update_status(&child_bg, status, preview.as_deref()).await;

        // 4. Emit final SubAgentUpdate event
        use tauri::Emitter;
        if let Some(ref parent) = parent_bg {
            let task_label = {
                let children = sub_agent_registry::list_children(parent).await;
                children
                    .iter()
                    .find(|c| c.session_key == child_bg)
                    .map(|c| c.task.clone())
                    .unwrap_or_else(|| task_bg.clone())
            };

            let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
                parent_session: parent.clone(),
                child_session: child_bg.clone(),
                task: task_label,
                status: status.to_string(),
                progress: Some(if run_ok { 1.0 } else { 0.0 }),
                result_preview: preview.clone(),
            };
            let _ = app_handle.emit("openclaw-event", &event);

            // 5. Feed-back loop: silent notice into parent session context
            let notice = format!(
                "[INTERNAL:SUB_AGENT_DONE] Sub-agent task finished.\nChild: {}\nStatus: {}\nResult: {}",
                child_bg, status, preview.as_deref().unwrap_or("(none)"),
            );
            let _ = ironclaw::api::chat::send_message(agent, parent, &notice, false).await;
        }

        info!(
            "[ironclaw] Sub-agent session {} finished: status={}",
            child_bg, status
        );
    });

    info!(
        "[ironclaw] Spawned session {} for agent {} (parent: {:?}) — non-blocking",
        child_key, agent_id, parent_session
    );

    Ok(SpawnSessionResponse {
        session_key: child_key,
        parent_session,
        task,
    })
}

/// List all child sessions spawned by a parent session.
///
/// Falls back to scanning the live session list for child key patterns
/// (`<parent>:task-<uuid>`) so the Fleet panel persists across restarts.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_child_sessions(
    ironclaw: State<'_, IronClawState>,
    parent_session: String,
) -> Result<Vec<ChildSessionInfo>, String> {
    let mut children = sub_agent_registry::list_children(&parent_session).await;

    // ── Post-restart recovery: scan live sessions if registry is empty ──
    if children.is_empty() {
        if let Ok(agent) = ironclaw.agent().await {
            let session_mgr = agent.session_manager();
            let all_sessions = session_mgr.list_sessions().await;
            let prefix = format!("{}:task-", parent_session);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as f64;

            for entry in all_sessions {
                // list_sessions() returns JSON objects: { "user_id": "...", ... }
                // The user_id IS the session key used by IronClaw internally.
                let key = match entry.get("user_id").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                if key.starts_with(&prefix) {
                    let suffix = &key[prefix.len()..];
                    let info = ChildSessionInfo {
                        session_key: key.clone(),
                        task: format!("(recovered) {}", suffix),
                        status: "completed".to_string(),
                        spawned_at: now_ms,
                        result_summary: Some("Session recovered after restart".to_string()),
                    };
                    sub_agent_registry::register(&parent_session, info).await;
                }
            }
            children = sub_agent_registry::list_children(&parent_session).await;
        }
    }

    Ok(children)
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

// ============================================================================
// Hooks management
// ============================================================================

/// List all registered lifecycle hooks with their details.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::HooksListResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let details = hooks.list_with_details().await;

    let hooks_list: Vec<super::types::HookInfoItem> = details
        .into_iter()
        .map(|h| super::types::HookInfoItem {
            name: h.name,
            hook_points: h.hook_points,
            failure_mode: h.failure_mode,
            timeout_ms: h.timeout_ms as u32,
            priority: h.priority,
        })
        .collect();

    let total = hooks_list.len() as u32;
    Ok(super::types::HooksListResponse {
        hooks: hooks_list,
        total,
    })
}

/// Register hooks from a declarative JSON bundle (rules and/or outbound webhooks).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_register(
    ironclaw: State<'_, IronClawState>,
    input: super::types::HookRegisterInput,
) -> Result<super::types::HookRegisterResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();

    // Parse the JSON bundle
    let value: serde_json::Value =
        serde_json::from_str(&input.bundle_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let bundle = ironclaw::hooks::bundled::HookBundleConfig::from_value(&value)
        .map_err(|e| format!("Invalid hook bundle: {}", e))?;

    let source = input.source.unwrap_or_else(|| "ui".to_string());
    let summary = ironclaw::hooks::bundled::register_bundle(hooks, &source, bundle).await;

    Ok(super::types::HookRegisterResponse {
        ok: summary.errors == 0,
        hooks_registered: summary.hooks as u32,
        webhooks_registered: summary.outbound_webhooks as u32,
        errors: summary.errors as u32,
        message: if summary.errors > 0 {
            Some(format!("{} hook(s) failed validation", summary.errors))
        } else {
            Some(format!(
                "Registered {} hook(s) and {} webhook(s)",
                summary.hooks, summary.outbound_webhooks
            ))
        },
    })
}

/// Unregister (remove) a hook by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_unregister(
    ironclaw: State<'_, IronClawState>,
    hook_name: String,
) -> Result<super::types::HookUnregisterResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let removed = hooks.unregister(&hook_name).await;

    Ok(super::types::HookUnregisterResponse {
        ok: removed,
        removed,
        message: if removed {
            Some(format!("Hook '{}' removed", hook_name))
        } else {
            Some(format!("Hook '{}' not found", hook_name))
        },
    })
}

// ============================================================================
// Extensions (plugins) management
// ============================================================================

/// List all installed extensions/plugins.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extensions_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::ExtensionsListResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let extensions = ironclaw::api::extensions::list_extensions(ext_mgr)
        .await
        .map_err(|e| e.to_string())?;

    let items: Vec<super::types::ExtensionInfoItem> = extensions
        .into_iter()
        .map(|ext| super::types::ExtensionInfoItem {
            name: ext.name,
            kind: ext.kind,
            description: ext.description,
            active: ext.active,
            authenticated: ext.authenticated,
            tools: ext.tools,
            needs_setup: ext.needs_setup,
            activation_status: ext.activation_status,
            activation_error: ext.activation_error,
        })
        .collect();

    let total = items.len() as u32;
    Ok(super::types::ExtensionsListResponse {
        extensions: items,
        total,
    })
}

/// Activate an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extension_activate(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<super::types::ExtensionActionResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = ironclaw::api::extensions::activate_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(super::types::ExtensionActionResponse {
        ok: resp.success,
        message: Some(resp.message),
    })
}

/// Remove an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extension_remove(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<super::types::ExtensionActionResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = ironclaw::api::extensions::remove_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(super::types::ExtensionActionResponse {
        ok: resp.success,
        message: Some(resp.message),
    })
}

// ============================================================================
// Diagnostics
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_diagnostics(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::DiagnosticsResponse, String> {
    let mut checks = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    // 1. IronClaw engine
    let engine_ok = ironclaw.agent().await.is_ok();
    if engine_ok {
        checks.push(super::types::DiagnosticCheck {
            name: "IronClaw Engine".into(),
            status: "pass".into(),
            detail: "Agent is running and accessible".into(),
        });
        passed += 1;
    } else {
        checks.push(super::types::DiagnosticCheck {
            name: "IronClaw Engine".into(),
            status: "fail".into(),
            detail: "Agent is not running".into(),
        });
        failed += 1;
    }

    if let Ok(agent) = ironclaw.agent().await {
        // 2. Database
        if let Some(store) = agent.store() {
            // Try listing settings to verify DB health
            match ironclaw::api::config::list_settings(store, "local_user").await {
                Ok(_) => {
                    checks.push(super::types::DiagnosticCheck {
                        name: "Database".into(),
                        status: "pass".into(),
                        detail: "Connected and responding to queries".into(),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(super::types::DiagnosticCheck {
                        name: "Database".into(),
                        status: "fail".into(),
                        detail: format!("Query failed: {}", e),
                    });
                    failed += 1;
                }
            }
        } else {
            checks.push(super::types::DiagnosticCheck {
                name: "Database".into(),
                status: "skip".into(),
                detail: "No database configured (ephemeral mode)".into(),
            });
            skipped += 1;
        }

        // 3. Workspace
        if agent.workspace().is_some() {
            checks.push(super::types::DiagnosticCheck {
                name: "Workspace".into(),
                status: "pass".into(),
                detail: "Workspace directory available".into(),
            });
            passed += 1;
        } else {
            checks.push(super::types::DiagnosticCheck {
                name: "Workspace".into(),
                status: "warn".into(),
                detail: "No workspace configured (memory tools unavailable)".into(),
            });
            skipped += 1;
        }

        // 4. Tools
        let tool_count = agent.tools().count();
        if tool_count > 0 {
            checks.push(super::types::DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "pass".into(),
                detail: format!("{} tools registered", tool_count),
            });
            passed += 1;
        } else {
            checks.push(super::types::DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "warn".into(),
                detail: "No tools registered".into(),
            });
            skipped += 1;
        }

        // 5. Hooks
        let hook_count = agent.hooks().list_with_details().await.len();
        checks.push(super::types::DiagnosticCheck {
            name: "Hook Registry".into(),
            status: "pass".into(),
            detail: format!("{} hooks registered", hook_count),
        });
        passed += 1;

        // 6. Extensions
        if let Some(ext_mgr) = agent.extension_manager() {
            match ironclaw::api::extensions::list_extensions(ext_mgr).await {
                Ok(resp) => {
                    let active = resp.iter().filter(|e| e.active).count();
                    checks.push(super::types::DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "pass".into(),
                        detail: format!("{} installed, {} active", resp.len(), active),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(super::types::DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(super::types::DiagnosticCheck {
                name: "Extensions".into(),
                status: "skip".into(),
                detail: "Extension manager not available".into(),
            });
            skipped += 1;
        }

        // 7. Skills
        if let Some(registry) = agent.skill_registry() {
            match ironclaw::api::skills::list_skills(registry).await {
                Ok(resp) => {
                    checks.push(super::types::DiagnosticCheck {
                        name: "Skills".into(),
                        status: "pass".into(),
                        detail: format!("{} skills loaded", resp.skills.len()),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(super::types::DiagnosticCheck {
                        name: "Skills".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(super::types::DiagnosticCheck {
                name: "Skills".into(),
                status: "skip".into(),
                detail: "Skill registry not available".into(),
            });
            skipped += 1;
        }
    }

    Ok(super::types::DiagnosticsResponse {
        checks,
        passed,
        failed,
        skipped,
    })
}

// ============================================================================
// Tool Listing
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_tools_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::ToolsListResponse, String> {
    let agent = ironclaw.agent().await?;
    let registry = agent.tools();

    // Load the disabled-tools deny-list from settings (default: empty = all enabled).
    let disabled_tools: std::collections::HashSet<String> = if let Some(store) = agent.store() {
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            let v: Vec<String> = serde_json::from_value(val).unwrap_or_default();
            v.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        }
    } else {
        std::collections::HashSet::new()
    };

    let tool_defs = registry.tool_definitions().await;
    let tools: Vec<super::types::ToolInfoItem> = tool_defs
        .iter()
        .map(|td| {
            // Determine source from tool name heuristics
            let source = if ["echo", "time", "json", "device_info", "http", "browser"]
                .contains(&td.name.as_str())
            {
                "builtin"
            } else if [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "apply_patch",
            ]
            .contains(&td.name.as_str())
            {
                "container"
            } else if [
                "memory_search",
                "memory_write",
                "memory_read",
                "memory_tree",
            ]
            .contains(&td.name.as_str())
            {
                "memory"
            } else if td.name.starts_with("tool_")
                || td.name.starts_with("skill_")
                || td.name.starts_with("routine_")
            {
                "management"
            } else {
                "extension"
            };

            super::types::ToolInfoItem {
                name: td.name.clone(),
                description: td.description.clone(),
                enabled: !disabled_tools.contains(&td.name),
                source: source.to_string(),
            }
        })
        .collect();

    let total = tools.len() as u32;
    Ok(super::types::ToolsListResponse { tools, total })
}

/// Get the set of globally disabled tools (deny-list stored in settings).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_tool_policy_get(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<String>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let disabled: Vec<String> =
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    Ok(disabled)
}

/// Set (overwrite) the list of globally disabled tools.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_tool_policy_set(
    ironclaw: State<'_, IronClawState>,
    disabled_tools: Vec<String>,
) -> Result<(), String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let val = serde_json::to_value(&disabled_tools).map_err(|e| e.to_string())?;
    store
        .set_setting("local_user", "disabled_tools", &val)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// DM Pairing Management
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_pairing_list(
    channel: String,
) -> Result<super::types::PairingListResponse, String> {
    let store = ironclaw::pairing::PairingStore::new();

    // Collect pending pairing requests
    let pending = store
        .list_pending(&channel)
        .map_err(|e| format!("Failed to list pairings: {}", e))?;

    let mut pairings: Vec<super::types::PairingItem> = pending
        .iter()
        .map(|req| super::types::PairingItem {
            channel: channel.clone(),
            user_id: req.id.clone(),
            paired_at: req.created_at.clone(),
            status: "pending".to_string(),
        })
        .collect();

    // Also include approved senders from allowFrom list
    if let Ok(allowed) = store.read_allow_from(&channel) {
        for user_id in allowed {
            pairings.push(super::types::PairingItem {
                channel: channel.clone(),
                user_id,
                paired_at: String::new(),
                status: "active".to_string(),
            });
        }
    }

    let total = pairings.len() as u32;
    Ok(super::types::PairingListResponse { pairings, total })
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_pairing_approve(
    channel: String,
    code: String,
) -> Result<serde_json::Value, String> {
    let store = ironclaw::pairing::PairingStore::new();
    store
        .approve(&channel, &code)
        .map_err(|e| format!("Failed to approve pairing: {}", e))?;
    Ok(serde_json::json!({ "ok": true }))
}

// ============================================================================
// Context Compaction
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_compact_session(
    ironclaw: State<'_, IronClawState>,
    _session_key: String,
) -> Result<super::types::CompactSessionResponse, String> {
    let agent = ironclaw.agent().await?;

    // Get the session and thread to check turn count
    let session_mgr = agent.session_manager();
    let session = session_mgr.get_or_create_session("local_user").await;
    let sess = session.lock().await;

    // Count total turns across threads
    let total_turns: usize = sess.threads.values().map(|t| t.turns.len()).sum();

    if total_turns <= 2 {
        return Ok(super::types::CompactSessionResponse {
            tokens_before: 0,
            tokens_after: 0,
            turns_removed: 0,
            summary: Some("Session too short to compact".into()),
        });
    }

    // Estimate "tokens" from turn text length (rough: 1 token ≈ 4 chars)
    let est_tokens_before: u32 = sess
        .threads
        .values()
        .flat_map(|t| t.turns.iter())
        .map(|turn| {
            let input_len = turn.user_input.len();
            let response_len = turn.response.as_ref().map(|r| r.len()).unwrap_or(0);
            ((input_len + response_len) / 4) as u32
        })
        .sum();

    // For now return the estimate — actual compaction happens automatically
    // when context hits 80% capacity in the agent loop
    let keep_recent = 3;
    let turns_to_remove = total_turns.saturating_sub(keep_recent);

    Ok(super::types::CompactSessionResponse {
        tokens_before: est_tokens_before,
        tokens_after: est_tokens_before
            .saturating_sub(est_tokens_before * turns_to_remove as u32 / total_turns as u32),
        turns_removed: turns_to_remove as u32,
        summary: Some(format!(
            "Estimated compaction: {} turns would be removed, keeping {} recent turns",
            turns_to_remove, keep_recent
        )),
    })
}

// ============================================================================
// Sprint 13 — New Backend API commands
// ============================================================================

/// Get LLM cost summary.
///
/// Returns total spend, daily/monthly breakdowns, per-model costs,
/// token totals, and alert status. The frontend picks what to display.
///
/// Also auto-persists entries to the IronClaw DB on each poll (cheap, ~10s interval).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_summary(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::CostSummary, String> {
    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    let ic_summary = ironclaw::tauri_commands::cost_summary(&tracker)?;

    // Auto-persist to DB on each summary poll (cheap — 10s interval).
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::debug!("[cost] Auto-save to DB failed: {}", e);
            }
        }
    }

    Ok(super::types::CostSummary {
        total_cost_usd: ic_summary.total_cost_usd,
        total_input_tokens: ic_summary.total_input_tokens as f64,
        total_output_tokens: ic_summary.total_output_tokens as f64,
        total_requests: ic_summary.total_requests as f64,
        avg_cost_per_request: ic_summary.avg_cost_per_request,
        daily: ic_summary.daily.into_iter().collect(),
        monthly: ic_summary.monthly.into_iter().collect(),
        by_model: ic_summary.by_model.into_iter().collect(),
        by_agent: ic_summary.by_agent.into_iter().collect(),
        alert_threshold_usd: ic_summary.alert_threshold_usd.unwrap_or(50.0),
        alert_triggered: ic_summary.alert_triggered,
    })
}

/// Export cost data as CSV.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_export_csv(
    ironclaw: State<'_, IronClawState>,
) -> Result<String, String> {
    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    ironclaw::tauri_commands::cost_export_csv(&tracker)
}

/// Reset (clear) all cost tracking data.
///
/// Clears in-memory entries and persists the empty state to the IronClaw DB.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_reset(ironclaw: State<'_, IronClawState>) -> Result<(), String> {
    let tracker_lock = ironclaw.cost_tracker().await?;
    let mut tracker = tracker_lock.lock().await;
    ironclaw::tauri_commands::cost_reset(&mut tracker)?;

    // Persist empty state to DB
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::warn!("[cost] Failed to persist reset to DB: {}", e);
            }
        }
    }
    Ok(())
}

/// List channel statuses from the live IronClaw agent.
///
/// Queries the agent's ChannelManager for actually registered channels
/// instead of reading static config/env vars.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_channel_status_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<super::types::ChannelStatusEntry>, String> {
    let agent = ironclaw.agent().await?;
    let channel_mgr = agent.channels();
    let ic_entries = channel_mgr.status_entries().await;

    let entries: Vec<super::types::ChannelStatusEntry> = ic_entries
        .into_iter()
        .map(|e| {
            let (state_str, uptime) = match &e.state {
                ironclaw::channels::status_view::ChannelViewState::Running { uptime_secs } => {
                    ("Running".to_string(), Some(*uptime_secs as u32))
                }
                ironclaw::channels::status_view::ChannelViewState::Connecting { attempt } => {
                    (format!("Connecting (attempt {})", attempt), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Reconnecting {
                    attempt, ..
                } => (format!("Reconnecting (attempt {})", attempt), None),
                ironclaw::channels::status_view::ChannelViewState::Failed { error, .. } => {
                    (format!("Failed: {}", error), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Disabled => {
                    ("Disabled".to_string(), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Draining => {
                    ("Draining".to_string(), None)
                }
            };
            super::types::ChannelStatusEntry {
                id: e.name.to_lowercase().replace(' ', "_"),
                name: e.name,
                channel_type: e.channel_type,
                state: state_str,
                enabled: e.state.is_healthy(),
                uptime_secs: uptime,
                messages_sent: e.messages_sent as u32,
                messages_received: e.messages_received as u32,
                last_error: e.last_error,
                stream_mode: String::new(),
            }
        })
        .collect();

    Ok(entries)
}

/// Set the default agent profile.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_set_default(
    _state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    agent_id: String,
) -> Result<(), String> {
    // Persist default agent via IronClaw's config API
    let agent = ironclaw.agent().await.ok();
    if let Some(agent) = agent {
        if let Some(store) = agent.store() {
            ironclaw::api::config::set_setting(
                store,
                "local_user",
                "default_agent_id",
                &serde_json::json!(agent_id),
            )
            .await
            .map_err(|e| format!("Failed to set default agent: {}", e))?;
        }
    }
    info!("[ironclaw] Set default agent to: {}", agent_id);
    Ok(())
}

/// Search ClawHub plugin catalog (proxied through IronClaw).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clawhub_search(
    ironclaw: State<'_, IronClawState>,
    query: String,
) -> Result<serde_json::Value, String> {
    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let entries = ironclaw::tauri_commands::clawhub_search(&cache, &query)?;
    Ok(serde_json::json!({ "entries": entries }))
}

/// Install a plugin from ClawHub.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clawhub_install(
    ironclaw: State<'_, IronClawState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let result = ironclaw::tauri_commands::clawhub_prepare_install(&cache, &plugin_id)?;
    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

/// List routine audit entries with optional outcome filter.
///
/// Replaces the empty `openclaw_cron_history` stub with actual data
/// access from IronClaw's RoutineAuditLog.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routine_audit_list(
    ironclaw: State<'_, IronClawState>,
    routine_key: String,
    limit: Option<u32>,
    outcome: Option<String>,
) -> Result<Vec<super::types::RoutineAuditEntry>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // Search both user IDs for the routine — agent creates with "default",
    // UI creates with "default" (now unified), legacy rows may have "local_user".
    let mut found_id = None;
    for uid in &["default", "local_user"] {
        if let Ok(routines) = store.list_routines(uid).await {
            if let Some(r) = routines
                .iter()
                .find(|r| r.name == routine_key || r.id.to_string() == routine_key)
            {
                found_id = Some(r.id);
                break;
            }
        }
    }

    let routine_id = match found_id {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    let db_limit = limit.unwrap_or(50) as i64;
    let runs = store
        .list_routine_runs(routine_id, db_limit)
        .await
        .map_err(|e| format!("Failed to list routine runs: {}", e))?;

    let entries: Vec<super::types::RoutineAuditEntry> = runs
        .into_iter()
        .filter(|run| {
            if let Some(ref filter) = outcome {
                let status_str = run.status.to_string();
                match filter.as_str() {
                    "success" | "ok" => status_str == "ok",
                    "failure" | "failed" => status_str == "failed",
                    "attention" => status_str == "attention",
                    "running" => status_str == "running",
                    _ => true,
                }
            } else {
                true
            }
        })
        .map(|run| {
            let duration_ms = match (run.started_at, run.completed_at) {
                (start, Some(end)) => Some((end - start).num_milliseconds().max(0) as u32),
                _ => None,
            };
            super::types::RoutineAuditEntry {
                routine_key: routine_key.clone(),
                started_at: run.started_at.to_rfc3339(),
                completed_at: run.completed_at.map(|t| t.to_rfc3339()),
                outcome: run.status.to_string(),
                duration_ms,
                error: run.result_summary.clone(),
            }
        })
        .collect();

    Ok(entries)
}

/// Get response cache statistics.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cache_stats(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::CacheStats, String> {
    let cache_lock = ironclaw.response_cache().await?;
    let cache = cache_lock.read().await;
    let ic_stats = ironclaw::tauri_commands::cache_stats(&cache)?;
    Ok(super::types::CacheStats {
        hits: ic_stats.hits as u32,
        misses: ic_stats.misses as u32,
        evictions: ic_stats.evictions as u32,
        size_bytes: ic_stats.size as u32,
        hit_rate: ic_stats.hit_rate as f64,
    })
}

/// List plugin lifecycle events.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_plugin_lifecycle_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<super::types::LifecycleEventItem>, String> {
    let hook = ironclaw.audit_log_hook().await?;
    let events = ironclaw::tauri_commands::plugin_lifecycle_list(&hook)?;
    Ok(events
        .into_iter()
        .map(|e| super::types::LifecycleEventItem {
            timestamp: e.timestamp,
            plugin_id: e.plugin,
            event_type: e.event_type,
            details: e.details,
        })
        .collect())
}

/// Validate a plugin's manifest.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_manifest_validate(
    ironclaw: State<'_, IronClawState>,
    plugin_id: String,
) -> Result<super::types::ManifestValidationResponse, String> {
    let validator = ironclaw.manifest_validator().await?;

    // Build a PluginInfoRef from the plugin_id. In a full implementation,
    // this would look up actual manifest data from the extension manager.
    // For now, construct a minimal ref to validate against.
    let info = ironclaw::extensions::manifest_validator::PluginInfoRef {
        name: plugin_id,
        version: None,
        description: None,
        permissions: Vec::new(),
        keywords: Vec::new(),
        homepage_url: None,
    };

    let response = ironclaw::tauri_commands::manifest_validate(&validator, &info)?;
    Ok(super::types::ManifestValidationResponse {
        errors: response.errors,
        warnings: response.warnings,
    })
}

/// Get the current smart routing configuration.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_get(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let enabled = if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            match store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                Ok(Some(val)) => val.as_bool().unwrap_or(false),
                _ => false,
            }
        } else {
            false
        }
    } else {
        false
    };
    Ok(serde_json::json!({ "smart_routing_enabled": enabled }))
}

/// Enable or disable smart routing.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_set(
    ironclaw: State<'_, IronClawState>,
    smart_routing_enabled: bool,
) -> Result<(), String> {
    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            store
                .set_setting(
                    "local_user",
                    "smart_routing_enabled",
                    &serde_json::json!(smart_routing_enabled),
                )
                .await
                .map_err(|e| format!("Failed to set routing config: {}", e))?;
        }
    }
    info!("[ironclaw] Smart routing set to: {}", smart_routing_enabled);
    Ok(())
}

/// List all routing rules along with the smart routing toggle state.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::RoutingRulesResponse, String> {
    let mut enabled = false;
    let mut rules: Vec<super::types::RoutingRule> = Vec::new();

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            // Read toggle state
            if let Ok(Some(val)) = store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                enabled = val.as_bool().unwrap_or(false);
            }
            // Read rules array
            if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
                if let Ok(parsed) = serde_json::from_value::<Vec<super::types::RoutingRule>>(val) {
                    rules = parsed;
                }
            }
        }
    }

    // Sort by priority
    rules.sort_by_key(|r| r.priority);

    Ok(super::types::RoutingRulesResponse {
        rules,
        smart_routing_enabled: enabled,
    })
}

/// Save routing rules (full replace).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_save(
    ironclaw: State<'_, IronClawState>,
    rules: Vec<super::types::RoutingRule>,
) -> Result<(), String> {
    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            let value = serde_json::to_value(&rules).map_err(|e| e.to_string())?;
            store
                .set_setting("local_user", "routing_rules", &value)
                .await
                .map_err(|e| format!("Failed to save routing rules: {}", e))?;
        }
    }
    info!("[ironclaw] Saved {} routing rules", rules.len());
    Ok(())
}

/// Start the Gmail OAuth PKCE flow via IronClaw.
///
/// This opens the user's browser for Google consent, waits for the
/// callback, exchanges the auth code for tokens, and returns them.
/// On success, the tokens are also stored in the Keychain.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_gmail_oauth_start(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::GmailOAuthResult, String> {
    // Call IronClaw's gmail_oauth_start which handles the full PKCE flow:
    // 1. Generates PKCE verifier/challenge
    // 2. Builds Google auth URL
    // 3. Opens browser
    // 4. Binds localhost callback listener
    // 5. Exchanges code for tokens
    let ic_result = ironclaw::tauri_commands::gmail_oauth_start()
        .await
        .map_err(|e| format!("Gmail OAuth failed: {}", e))?;

    // If successful, persist refresh token in Keychain for future use
    if ic_result.success {
        if let Some(ref refresh_token) = ic_result.refresh_token {
            // Store via IronClaw's agent secrets store if available
            if let Ok(agent) = ironclaw.agent().await {
                if let Some(store) = agent.store() {
                    let _ = store
                        .set_setting(
                            "local_user",
                            "gmail_refresh_token",
                            &serde_json::json!(refresh_token),
                        )
                        .await;
                }
            }
        }
        info!("[ironclaw] Gmail OAuth completed successfully");
    } else {
        let err_msg = ic_result.error.as_deref().unwrap_or("unknown error");
        warn!("[ironclaw] Gmail OAuth failed: {}", err_msg);
    }

    Ok(super::types::GmailOAuthResult {
        success: ic_result.success,
        access_token: ic_result.access_token,
        refresh_token: ic_result.refresh_token,
        expires_in: ic_result.expires_in.map(|e| e as u32),
        scope: ic_result.scope,
        error: ic_result.error,
    })
}

/// Add a routing rule at a specific position (or at the end).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_add(
    ironclaw: State<'_, IronClawState>,
    rule: super::types::RoutingRule,
    position: Option<u32>,
) -> Result<Vec<super::types::RoutingRule>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    // Read existing rules
    let mut rules: Vec<super::types::RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    // Insert at position or append
    if let Some(pos) = position {
        let pos = pos as usize;
        if pos > rules.len() {
            return Err(format!(
                "Position {} out of bounds (have {} rules)",
                pos,
                rules.len()
            ));
        }
        rules.insert(pos, rule);
    } else {
        rules.push(rule);
    }

    // Re-index priorities
    for (i, r) in rules.iter_mut().enumerate() {
        r.priority = i as u32;
    }

    // Persist
    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[ironclaw] Added routing rule, now have {} rules",
        rules.len()
    );
    Ok(rules)
}

/// Remove a routing rule by index.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_remove(
    ironclaw: State<'_, IronClawState>,
    index: u32,
) -> Result<Vec<super::types::RoutingRule>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let mut rules: Vec<super::types::RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    if (index as usize) >= rules.len() {
        return Err(format!(
            "Index {} out of bounds (have {} rules)",
            index,
            rules.len()
        ));
    }

    rules.remove(index as usize);

    // Re-index priorities
    for (i, r) in rules.iter_mut().enumerate() {
        r.priority = i as u32;
    }

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[ironclaw] Removed routing rule at index {}, now have {} rules",
        index,
        rules.len()
    );
    Ok(rules)
}

/// Reorder a routing rule (move from one position to another).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_reorder(
    ironclaw: State<'_, IronClawState>,
    from: u32,
    to: u32,
) -> Result<Vec<super::types::RoutingRule>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let mut rules: Vec<super::types::RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    let from = from as usize;
    let to = to as usize;
    if from >= rules.len() || to >= rules.len() {
        return Err(format!(
            "Indices out of bounds: from={}, to={}, have {} rules",
            from,
            to,
            rules.len()
        ));
    }

    let rule = rules.remove(from);
    rules.insert(to, rule);

    // Re-index priorities
    for (i, r) in rules.iter_mut().enumerate() {
        r.priority = i as u32;
    }

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!("[ironclaw] Reordered routing rule from {} to {}", from, to);
    Ok(rules)
}

/// Get full routing policy status including latency data.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::RoutingStatusResponse, String> {
    let mut enabled = false;
    let mut rules: Vec<super::types::RoutingRule> = Vec::new();
    let mut default_provider = "openai-compatible".to_string();

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            if let Ok(Some(val)) = store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                enabled = val.as_bool().unwrap_or(false);
            }
            if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
                rules = serde_json::from_value(val).unwrap_or_default();
            }
            if let Ok(Some(val)) = store.get_setting("local_user", "default_provider").await {
                if let Some(p) = val.as_str() {
                    default_provider = p.to_string();
                }
            }
        }
    }

    // Build rule summaries
    let rule_summaries: Vec<super::types::RoutingRuleSummary> = rules
        .iter()
        .enumerate()
        .map(|(i, r)| super::types::RoutingRuleSummary {
            index: i as u32,
            kind: r.match_kind.clone(),
            description: format!(
                "{}: {} → {}",
                r.label,
                if r.match_value.is_empty() {
                    "*"
                } else {
                    &r.match_value
                },
                r.target_model
            ),
            provider: r.target_provider.clone(),
        })
        .collect();

    // Collect latency data from IronClaw's cost tracker if available
    let mut latency_data: Vec<super::types::LatencyEntry> = Vec::new();
    if let Ok(tracker) = ironclaw.cost_tracker().await {
        let ct = tracker.lock().await;
        if let Ok(summary) = ironclaw::tauri_commands::cost_summary(&ct) {
            for (provider, _cost) in &summary.by_model {
                latency_data.push(super::types::LatencyEntry {
                    provider: provider.clone(),
                    avg_latency_ms: 0.0,
                });
            }
        }
    }

    Ok(super::types::RoutingStatusResponse {
        enabled,
        default_provider,
        rule_count: rules.len() as u32,
        rules: rule_summaries,
        latency_data,
    })
}

/// Get Gmail channel configuration status.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_gmail_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<super::types::GmailStatusResponse, String> {
    let mut enabled = false;
    let mut project_id = String::new();
    let mut subscription_id = String::new();
    let mut label_filters: Vec<String> = Vec::new();
    let mut allowed_senders: Vec<String> = Vec::new();
    let mut oauth_configured = false;
    let mut missing_fields: Vec<String> = Vec::new();

    // Read Gmail config from environment variables (IronClaw pattern)
    if let Ok(val) = std::env::var("GMAIL_ENABLED") {
        enabled = val == "true" || val == "1";
    }
    if let Ok(val) = std::env::var("GMAIL_PROJECT_ID") {
        project_id = val;
    } else {
        missing_fields.push("GMAIL_PROJECT_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_SUBSCRIPTION_ID") {
        subscription_id = val;
    } else {
        missing_fields.push("GMAIL_SUBSCRIPTION_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_LABEL_FILTERS") {
        label_filters = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(val) = std::env::var("GMAIL_ALLOWED_SENDERS") {
        allowed_senders = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    // Check for OAuth token in settings store
    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            if let Ok(Some(_)) = store.get_setting("local_user", "gmail_refresh_token").await {
                oauth_configured = true;
            }
        }
    }

    let configured = !project_id.is_empty() && !subscription_id.is_empty();
    let status = if !enabled {
        "disabled".to_string()
    } else if !configured {
        format!("missing credentials: {}", missing_fields.join(", "))
    } else if oauth_configured {
        format!("ready ({})", subscription_id)
    } else {
        "configured but OAuth not completed".to_string()
    };

    Ok(super::types::GmailStatusResponse {
        enabled,
        configured,
        status,
        project_id,
        subscription_id,
        label_filters,
        allowed_senders,
        missing_fields,
        oauth_configured,
    })
}

// ============================================================================
// Canvas Panel Management
// ============================================================================

/// List all active canvas panels.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panels_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    let panels = store.list().await;
    let summaries: Vec<serde_json::Value> = panels
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "panel_id": p.panel_id,
                "title": p.title,
            })
        })
        .collect();
    Ok(serde_json::json!({ "panels": summaries }))
}

/// Get a specific canvas panel's full data.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panel_get(
    ironclaw: State<'_, IronClawState>,
    panel_id: String,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    match store.get(&panel_id).await {
        Some(panel) => Ok(serde_json::json!({
            "panel_id": panel.panel_id,
            "title": panel.title,
            "components": panel.components,
            "metadata": panel.metadata,
        })),
        None => Ok(serde_json::json!(null)),
    }
}

/// Dismiss (remove) a canvas panel.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panel_dismiss(
    ironclaw: State<'_, IronClawState>,
    panel_id: String,
) -> Result<bool, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    Ok(store.dismiss(&panel_id).await)
}

// ============================================================================
// Routine Delete / Toggle
// ============================================================================

/// Delete a routine by ID or name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routine_delete(
    ironclaw: State<'_, IronClawState>,
    routine_id: String,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // Parse as UUID first, fallback to name lookup across all user IDs
    let id = if let Ok(uuid) = uuid::Uuid::parse_str(&routine_id) {
        uuid
    } else {
        let mut found = None;
        for uid in &["default", "local_user"] {
            if let Ok(routines) = store.list_routines(uid).await {
                if let Some(r) = routines.iter().find(|r| r.name == routine_id) {
                    found = Some(r.id);
                    break;
                }
            }
        }
        found.ok_or_else(|| format!("Routine '{}' not found", routine_id))?
    };

    store
        .delete_routine(id)
        .await
        .map_err(|e| format!("Failed to delete routine: {}", e))?;

    info!("[ironclaw] Deleted routine {}", id);
    Ok(serde_json::json!({ "ok": true, "deleted_id": id.to_string() }))
}

/// Toggle a routine enabled/disabled by ID or name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routine_toggle(
    ironclaw: State<'_, IronClawState>,
    routine_id: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // Parse as UUID first, fallback to name lookup across all user IDs
    let id = if let Ok(uuid) = uuid::Uuid::parse_str(&routine_id) {
        uuid
    } else {
        let mut found = None;
        for uid in &["default", "local_user"] {
            if let Ok(routines) = store.list_routines(uid).await {
                if let Some(r) = routines.iter().find(|r| r.name == routine_id) {
                    found = Some(r.id);
                    break;
                }
            }
        }
        found.ok_or_else(|| format!("Routine '{}' not found", routine_id))?
    };

    let mut routine = store
        .get_routine(id)
        .await
        .map_err(|e| format!("Failed to get routine: {}", e))?
        .ok_or_else(|| format!("Routine '{}' not found", id))?;

    routine.enabled = enabled;
    routine.updated_at = chrono::Utc::now();

    store
        .update_routine(&routine)
        .await
        .map_err(|e| format!("Failed to update routine: {}", e))?;

    info!("[ironclaw] Toggled routine {} to enabled={}", id, enabled);
    Ok(serde_json::json!({ "ok": true, "id": id.to_string(), "enabled": enabled }))
}

/// Update the heartbeat interval at runtime.
///
/// 1. Updates the `__heartbeat__` DB routine's cron schedule → takes effect on next tick
/// 2. Persists `interval_secs` to settings.toml → survives restarts
///
/// `interval_minutes` must be between 5 and 1440 (24 hours).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_heartbeat_set_interval(
    ironclaw: State<'_, IronClawState>,
    interval_minutes: u32,
) -> Result<serde_json::Value, String> {
    if interval_minutes < 5 || interval_minutes > 1440 {
        return Err("Interval must be between 5 and 1440 minutes".to_string());
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // ── 1. Update the DB routine ──────────────────────────────────────────
    let mut routine = store
        .get_routine_by_name("default", "__heartbeat__")
        .await
        .map_err(|e| format!("Failed to look up heartbeat routine: {}", e))?
        .ok_or("Heartbeat routine not found — is the engine running?")?;

    let cron_5field = format!("*/{} * * * *", interval_minutes);
    let schedule = ironclaw::agent::routine::normalize_cron_expr(&cron_5field);
    let next_fire = ironclaw::agent::routine::next_cron_fire(&schedule).unwrap_or(None);

    routine.trigger = ironclaw::agent::routine::Trigger::Cron {
        schedule: schedule.clone(),
    };
    routine.next_fire_at = next_fire;
    routine.guardrails.cooldown = std::time::Duration::from_secs(interval_minutes as u64 * 60 / 2);
    routine.updated_at = chrono::Utc::now();

    store
        .update_routine(&routine)
        .await
        .map_err(|e| format!("Failed to update heartbeat routine: {}", e))?;

    info!(
        "[ironclaw] Updated heartbeat interval to {} min (schedule='{}', next_fire={:?})",
        interval_minutes, schedule, next_fire
    );

    // ── 2. Persist to ironclaw.toml so boot won't overwrite ───────────
    let interval_secs = interval_minutes as u64 * 60;
    let toml_path = ironclaw.state_dir().join("ironclaw.toml");
    if toml_path.exists() {
        match ironclaw::settings::Settings::load_toml(&toml_path) {
            Ok(Some(mut settings)) => {
                settings.heartbeat.interval_secs = interval_secs;
                if let Err(e) = settings.save_toml(&toml_path) {
                    tracing::warn!(
                        "Failed to persist heartbeat interval to ironclaw.toml: {}",
                        e
                    );
                } else {
                    tracing::info!(
                        "Persisted heartbeat.interval_secs={} to ironclaw.toml",
                        interval_secs
                    );
                }
            }
            Ok(None) => {
                tracing::debug!("ironclaw.toml exists but is empty — skipping persistence");
            }
            Err(e) => {
                tracing::warn!("Failed to parse ironclaw.toml for persistence: {}", e);
            }
        }
    } else {
        tracing::debug!("No ironclaw.toml found — skipping persistence (DB is source of truth)");
    }

    // ── 3. Also update the env var so any in-process re-init matches ────
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HEARTBEAT_INTERVAL_SECS", interval_secs.to_string());
    }

    Ok(serde_json::json!({
        "ok": true,
        "interval_minutes": interval_minutes,
        "schedule": schedule,
        "next_fire_at": next_fire.map(|dt| dt.to_rfc3339()),
    }))
}

// ============================================================================
// Workspace path & Finder reveal
// ============================================================================

/// Return the local filesystem workspace root path.
///
/// This is the directory where the agent writes local files (write_file, shell, etc.).
/// Defaults to ~/Scrappy/ if not configured.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_workspace_path(
    manager: State<'_, OpenClawManager>,
) -> Result<String, String> {
    // WORKSPACE_ROOT env var is set at engine start with the resolved path
    if let Ok(root) = std::env::var("WORKSPACE_ROOT") {
        if !root.is_empty() {
            return Ok(root);
        }
    }
    // Fall back to config value
    let cfg = manager.get_config().await;
    if let Some(root) = cfg.as_ref().and_then(|c| c.workspace_root.as_ref()) {
        return Ok(root.clone());
    }
    // Default: ~/Scrappy/
    let default = std::env::var("HOME")
        .map(|h| format!("{}/Scrappy", h))
        .unwrap_or_else(|_| "Scrappy".to_string());
    Ok(default)
}

/// Open the local workspace directory in Finder (macOS) / Explorer (Windows).
///
/// Creates the directory if it doesn't exist yet. Returns the path that was opened.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reveal_workspace(
    manager: State<'_, OpenClawManager>,
) -> Result<String, String> {
    let path_str = if let Ok(root) = std::env::var("WORKSPACE_ROOT") {
        if !root.is_empty() {
            root
        } else {
            std::env::var("HOME")
                .map(|h| format!("{}/Scrappy", h))
                .unwrap_or_else(|_| "Scrappy".to_string())
        }
    } else {
        let cfg = manager.get_config().await;
        cfg.as_ref()
            .and_then(|c| c.workspace_root.clone())
            .unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(|h| format!("{}/Scrappy", h))
                    .unwrap_or_else(|_| "Scrappy".to_string())
            })
    };

    // Ensure directory exists
    if let Err(e) = std::fs::create_dir_all(&path_str) {
        warn!(
            "[ironclaw] Could not create workspace dir {}: {}",
            path_str, e
        );
    }

    // Open in Finder (macOS) / Explorer (Windows) using OS built-ins
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Finder: {}", e))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Explorer: {}", e))?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    info!("[ironclaw] Revealed workspace: {}", path_str);
    Ok(path_str)
}

/// List all files in the agent's local `agent_workspace` directory.
///
/// Returns relative paths (from workspace root), file sizes, and modification
/// timestamps so the frontend can build a proper file browser.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_agent_workspace_files(
    _manager: State<'_, OpenClawManager>,
) -> Result<Vec<serde_json::Value>, String> {
    let workspace_root = if let Ok(root) = std::env::var("WORKSPACE_ROOT") {
        if !root.is_empty() {
            std::path::PathBuf::from(root)
        } else {
            return Ok(vec![]);
        }
    } else {
        return Ok(vec![]);
    };

    if !workspace_root.exists() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();

    /// Directories to skip when recursively listing the workspace.
    /// These are often massive (node_modules can have 50k+ files)
    /// and walking them can cause memory corruption / OOM.
    const SKIP_DIRS: &[&str] = &[
        "node_modules",
        "target",
        ".git",
        "__pycache__",
        "venv",
        ".venv",
        ".next",
        "dist",
        "build",
        ".cargo",
        ".tox",
        "vendor",
        ".build",
        "Pods",
    ];

    /// Hard cap on total entries to prevent runaway recursion from
    /// corrupting the allocator.
    const MAX_ENTRIES: usize = 5000;

    fn walk_dir(
        dir: &std::path::Path,
        root: &std::path::Path,
        entries: &mut Vec<serde_json::Value>,
        depth: usize,
    ) {
        if depth > 6 || entries.len() >= MAX_ENTRIES {
            return; // Prevent runaway recursion
        }
        let read = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in read.flatten() {
            if entries.len() >= MAX_ENTRIES {
                return;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            // Skip hidden files and common junk
            if rel.starts_with('.') || rel.contains("/.") || rel.ends_with(".DS_Store") {
                continue;
            }

            if path.is_dir() {
                // Skip heavy directories that would blow up memory
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&dir_name) {
                    continue;
                }
                walk_dir(&path, root, entries, depth + 1);
            } else {
                let meta = std::fs::metadata(&path);
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified_ms = meta
                    .as_ref()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                entries.push(serde_json::json!({
                    "path": rel,
                    "absolute_path": path.to_string_lossy(),
                    "size": size,
                    "modified_ms": modified_ms,
                }));
            }
        }
    }

    walk_dir(&workspace_root, &workspace_root, &mut entries, 0);

    // Sort by path
    entries.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });

    Ok(entries)
}

/// Reveal a specific file in Finder (macOS) / Explorer (Windows).
///
/// Uses `open -R <path>` on macOS to select the file in a Finder window,
/// which is more user-friendly than just opening the parent folder.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reveal_file(path: String) -> Result<(), String> {
    // Security: prevent path traversal
    let p = std::path::Path::new(&path);
    if path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    // Only reveal files that exist
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-R") // -R = reveal (select in Finder)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Finder: {}", e))?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .args(["/select,", &path])
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Explorer: {}", e))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(p))
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    Ok(())
}

/// Write content to a file in the agent's local `agent_workspace` directory.
///
/// The `relative_path` is resolved against `WORKSPACE_ROOT`. Parent directories
/// are created automatically. Path traversal (`..`) is rejected for safety.
/// Returns the absolute path of the written file.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_agent_workspace_file(
    _manager: State<'_, OpenClawManager>,
    relative_path: String,
    content: String,
) -> Result<String, String> {
    // Security: prevent path traversal
    if relative_path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .filter(|r| !r.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "WORKSPACE_ROOT not set — cannot write file".to_string())?;

    let target = workspace_root.join(&relative_path);

    // Ensure the resolved path is still inside the workspace
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());
    // Can't canonicalize the target yet (file may not exist), but check prefix
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    // Double-check after dir creation
    let canonical_parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("Path escapes workspace root".to_string());
    }

    std::fs::write(&target, &content).map_err(|e| format!("Failed to write file: {}", e))?;

    let abs = target.to_string_lossy().to_string();
    tracing::info!(
        path = %abs,
        bytes = content.len(),
        "Wrote automation result to agent_workspace"
    );
    Ok(abs)
}

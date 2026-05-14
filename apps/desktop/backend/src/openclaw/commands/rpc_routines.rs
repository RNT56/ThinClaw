//! RPC commands — Cron / Routines management.
//!
//! Extracted from `rpc.rs` for better modularity.

use std::sync::Arc;

use tauri::State;
use tracing::info;

use crate::openclaw::ironclaw_bridge::IronClawState;

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
        actor_id: "local_user".to_string(),
        enabled: true,
        trigger: ironclaw::agent::routine::Trigger::Cron {
            schedule: schedule.clone(),
        },
        action: ironclaw::agent::routine::RoutineAction::FullJob {
            title: name.clone(),
            description: task.clone(),
            max_iterations: 10,
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
        },
        guardrails: ironclaw::agent::routine::RoutineGuardrails::default(),
        notify: ironclaw::agent::routine::NotifyConfig::default(),
        policy: ironclaw::agent::routine::RoutinePolicy::default(),
        last_run_at: None,
        next_fire_at: next_fire,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::Value::Null,
        config_version: 1,
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

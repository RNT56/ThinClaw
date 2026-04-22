//! Routines API — list, trigger, toggle, delete routines.
//!
//! Extracted from `channels/web/handlers/routines.rs`.

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::{outcomes, routine_engine::RoutineEngine};
use crate::channels::web::types::*;
use crate::db::Database;

use super::error::{ApiError, ApiResult};

/// List all routines for a user.
pub async fn list_routines(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<RoutineListResponse> {
    let routines = store
        .list_routines(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(routine_to_info).collect();

    Ok(RoutineListResponse { routines: items })
}

/// Trigger a routine manually.
pub async fn trigger_routine(
    engine: &Arc<RoutineEngine>,
    store: &Arc<dyn Database>,
    routine_id: &str,
) -> ApiResult<serde_json::Value> {
    let id = Uuid::parse_str(routine_id)?;

    let routine = store
        .get_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Routine {} not found", routine_id)))?;

    engine
        .fire_manual(routine.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(serde_json::json!({
        "status": "triggered",
        "routine_id": routine_id
    }))
}

/// Toggle routine enabled/disabled.
pub async fn toggle_routine(
    store: &Arc<dyn Database>,
    routine_id: &str,
    enabled: Option<bool>,
) -> ApiResult<serde_json::Value> {
    let id = Uuid::parse_str(routine_id)?;

    let mut routine = store
        .get_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Routine {} not found", routine_id)))?;

    routine.enabled = enabled.unwrap_or(!routine.enabled);
    store
        .update_routine(&routine)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !routine.enabled {
        let _ = outcomes::observe_routine_state_change(store, &routine, "routine_disabled").await;
    }

    Ok(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    }))
}

/// Delete a routine.
pub async fn delete_routine(
    store: &Arc<dyn Database>,
    routine_id: &str,
) -> ApiResult<serde_json::Value> {
    let id = Uuid::parse_str(routine_id)?;
    let routine = store
        .get_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let deleted = store
        .delete_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if deleted {
        if let Some(routine) = routine.as_ref() {
            let _ = outcomes::observe_routine_state_change(store, routine, "routine_deleted").await;
        }
        Ok(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        }))
    } else {
        Err(ApiError::SessionNotFound(format!(
            "Routine {} not found",
            routine_id
        )))
    }
}

/// Convert a `Routine` to the trimmed `RoutineInfo` for list display.
fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
    let (trigger_type, trigger_summary) = match &r.trigger {
        crate::agent::routine::Trigger::Cron { schedule } => (
            "cron".to_string(),
            if schedule.starts_with("every ") {
                format!("schedule: {}", schedule)
            } else {
                format!("cron: {}", schedule)
            },
        ),
        crate::agent::routine::Trigger::Event {
            pattern,
            channel,
            event_type,
            actor,
            priority,
            ..
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            let event_label = event_type.as_deref().unwrap_or("message");
            let actor_label = actor
                .as_deref()
                .map(|value| format!(" actor {}", value))
                .unwrap_or_default();
            let summary = if *priority == 0 {
                format!("on {} {}{} /{}/", ch, event_label, actor_label, pattern)
            } else {
                format!(
                    "on {} {}{} /{}/ (prio {})",
                    ch, event_label, actor_label, pattern, priority
                )
            };
            ("event".to_string(), summary)
        }
        crate::agent::routine::Trigger::Webhook { path, .. } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        crate::agent::routine::Trigger::Manual => ("manual".to_string(), "manual only".to_string()),
        crate::agent::routine::Trigger::SystemEvent { message, schedule } => {
            let sched = schedule.as_deref().unwrap_or("on-demand");
            ("system_event".to_string(), {
                let truncated: String = message.chars().take(40).collect();
                format!(
                    "event: {} ({}, {})",
                    truncated,
                    sched,
                    match r.policy.catch_up_mode {
                        crate::agent::routine::RoutineCatchUpMode::Skip => "skip",
                        crate::agent::routine::RoutineCatchUpMode::RunOnceNow => "run once",
                        crate::agent::routine::RoutineCatchUpMode::Replay => "replay",
                    }
                )
            })
        }
    };

    let action_type = match &r.action {
        crate::agent::routine::RoutineAction::Lightweight { .. } => "lightweight",
        crate::agent::routine::RoutineAction::FullJob { .. } => "full_job",
        crate::agent::routine::RoutineAction::Heartbeat { .. } => "heartbeat",
        crate::agent::routine::RoutineAction::ExperimentCampaign { .. } => "experiment_campaign",
    };

    let status = if !r.enabled {
        "disabled"
    } else if r.consecutive_failures > 0 {
        "failing"
    } else {
        "active"
    };

    RoutineInfo {
        id: r.id,
        name: r.name.clone(),
        description: r.description.clone(),
        enabled: r.enabled,
        trigger_type,
        trigger_summary,
        action_type: action_type.to_string(),
        last_run_at: r.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: r.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: r.run_count,
        consecutive_failures: r.consecutive_failures,
        status: status.to_string(),
    }
}

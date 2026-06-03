//! Routines API — list, trigger, toggle, delete routines.
//!
//! Extracted from `channels/web/handlers/routines.rs`.

use std::sync::Arc;

use crate::agent::{outcomes, routine_engine::RoutineEngine};
use crate::channels::web::types::*;
use crate::db::Database;
use thinclaw_gateway::web::routines::{
    RoutineInfoAction, RoutineInfoCatchUpMode, RoutineInfoInput, RoutineInfoTrigger,
    parse_routine_uuid, project_routine_info, routine_deleted_action_response,
    routine_list_response, routine_not_found_message, routine_toggle_action_response,
    routine_triggered_action_response,
};

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

    Ok(routine_list_response(items))
}

/// Trigger a routine manually.
pub async fn trigger_routine(
    engine: &Arc<RoutineEngine>,
    store: &Arc<dyn Database>,
    routine_id: &str,
) -> ApiResult<serde_json::Value> {
    let id = parse_routine_uuid(routine_id)?;

    let routine = store
        .get_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(routine_not_found_message(routine_id)))?;

    engine
        .fire_manual(routine.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    serde_json::to_value(routine_triggered_action_response(id))
        .map_err(|error| ApiError::Internal(error.to_string()))
}

/// Toggle routine enabled/disabled.
pub async fn toggle_routine(
    store: &Arc<dyn Database>,
    routine_id: &str,
    enabled: Option<bool>,
) -> ApiResult<serde_json::Value> {
    let id = parse_routine_uuid(routine_id)?;

    let mut routine = store
        .get_routine(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(routine_not_found_message(routine_id)))?;

    routine.enabled = enabled.unwrap_or(!routine.enabled);
    store
        .update_routine(&routine)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !routine.enabled {
        let _ = outcomes::observe_routine_state_change(store, &routine, "routine_disabled").await;
    }

    serde_json::to_value(routine_toggle_action_response(routine.enabled, id))
        .map_err(|error| ApiError::Internal(error.to_string()))
}

/// Delete a routine.
pub async fn delete_routine(
    store: &Arc<dyn Database>,
    routine_id: &str,
) -> ApiResult<serde_json::Value> {
    let id = parse_routine_uuid(routine_id)?;
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
        serde_json::to_value(routine_deleted_action_response(id))
            .map_err(|error| ApiError::Internal(error.to_string()))
    } else {
        Err(ApiError::SessionNotFound(routine_not_found_message(
            routine_id,
        )))
    }
}

/// Convert a `Routine` to the trimmed `RoutineInfo` for list display.
pub(crate) fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
    project_routine_info(RoutineInfoInput {
        id: r.id,
        name: r.name.clone(),
        description: r.description.clone(),
        enabled: r.enabled,
        trigger: match &r.trigger {
            crate::agent::routine::Trigger::Cron { schedule } => RoutineInfoTrigger::Cron {
                schedule: schedule.clone(),
            },
            crate::agent::routine::Trigger::Event {
                pattern,
                channel,
                event_type,
                actor,
                priority,
                ..
            } => RoutineInfoTrigger::Event {
                pattern: pattern.clone(),
                channel: channel.clone(),
                event_type: event_type.clone(),
                actor: actor.clone(),
                priority: *priority,
            },
            crate::agent::routine::Trigger::Webhook { path, .. } => {
                RoutineInfoTrigger::Webhook { path: path.clone() }
            }
            crate::agent::routine::Trigger::Manual => RoutineInfoTrigger::Manual,
            crate::agent::routine::Trigger::SystemEvent { message, schedule } => {
                RoutineInfoTrigger::SystemEvent {
                    message: message.clone(),
                    schedule: schedule.clone(),
                    catch_up_mode: match r.policy.catch_up_mode {
                        crate::agent::routine::RoutineCatchUpMode::Skip => {
                            RoutineInfoCatchUpMode::Skip
                        }
                        crate::agent::routine::RoutineCatchUpMode::RunOnceNow => {
                            RoutineInfoCatchUpMode::RunOnceNow
                        }
                        crate::agent::routine::RoutineCatchUpMode::Replay => {
                            RoutineInfoCatchUpMode::Replay
                        }
                    },
                }
            }
        },
        action: match &r.action {
            crate::agent::routine::RoutineAction::Lightweight { .. } => {
                RoutineInfoAction::Lightweight
            }
            crate::agent::routine::RoutineAction::FullJob { .. } => RoutineInfoAction::FullJob,
            crate::agent::routine::RoutineAction::Heartbeat { .. } => RoutineInfoAction::Heartbeat,
            crate::agent::routine::RoutineAction::ExperimentCampaign { .. } => {
                RoutineInfoAction::ExperimentCampaign
            }
        },
        last_run_at: r.last_run_at,
        next_fire_at: r.next_fire_at,
        run_count: r.run_count,
        consecutive_failures: r.consecutive_failures,
    })
}

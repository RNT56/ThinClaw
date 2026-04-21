use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::agent::outcomes;
use crate::channels::IncomingMessage;
use crate::channels::web::identity_helpers::{GatewayRequestIdentity, gateway_identity};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub(crate) async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines_for_actor(&request_identity.principal_id, &request_identity.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(routine_to_info).collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

pub(crate) async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines_for_actor(&request_identity.principal_id, &request_identity.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let enabled = routines.iter().filter(|r| r.enabled).count() as u64;
    let disabled = total - enabled;
    let failing = routines
        .iter()
        .filter(|r| r.consecutive_failures > 0)
        .count() as u64;

    let today_start = crate::timezone::local_day_start_utc(
        Some(&request_identity.principal_id),
        None,
        crate::timezone::today_for_user(Some(&request_identity.principal_id), None),
    );
    let runs_today = routines
        .iter()
        .filter(|r| r.last_run_at.is_some_and(|ts| ts >= today_start))
        .count() as u64;

    Ok(Json(RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }))
}

pub(crate) async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<RoutineDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.user_id != request_identity.principal_id
        || routine.owner_actor_id() != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_runs: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(RoutineDetailResponse {
        id: routine.id,
        name: routine.name.clone(),
        description: routine.description.clone(),
        enabled: routine.enabled,
        trigger: serde_json::to_value(&routine.trigger).unwrap_or_default(),
        action: serde_json::to_value(&routine.action).unwrap_or_default(),
        guardrails: serde_json::to_value(&routine.guardrails).unwrap_or_default(),
        notify: serde_json::to_value(&routine.notify).unwrap_or_default(),
        last_run_at: routine.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: routine.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: routine.run_count,
        consecutive_failures: routine.consecutive_failures,
        created_at: routine.created_at.to_rfc3339(),
        recent_runs,
    }))
}

pub(crate) async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.user_id != request_identity.principal_id
        || routine.owner_actor_id() != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
        crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => prompt
            .clone()
            .unwrap_or_else(|| "Heartbeat check".to_string()),
        crate::agent::routine::RoutineAction::ExperimentCampaign { project_id, .. } => {
            format!("Run experiment campaign for project {project_id}")
        }
    };

    let content = format!("[routine:{}] {}", routine.name, prompt);
    let mut msg = IncomingMessage::new("gateway", &request_identity.principal_id, content);
    msg = msg.with_identity(gateway_identity(
        &request_identity.principal_id,
        &request_identity.actor_id,
        None,
    ));

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine_id,
    })))
}

#[derive(Deserialize)]
pub(crate) struct ToggleRequest {
    enabled: Option<bool>,
}

pub(crate) async fn routines_toggle_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    body: Option<Json<ToggleRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let mut routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.user_id != request_identity.principal_id
        || routine.owner_actor_id() != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    routine.enabled = match body {
        Some(Json(req)) => req.enabled.unwrap_or(!routine.enabled),
        None => !routine.enabled,
    };

    store
        .update_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !routine.enabled {
        let _ = outcomes::observe_routine_state_change(store, &routine, "routine_disabled").await;
    }

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

pub(crate) async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.user_id != request_identity.principal_id
        || routine.owner_actor_id() != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        let _ = outcomes::observe_routine_state_change(store, &routine, "routine_deleted").await;
        Ok(Json(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Routine not found".to_string()))
    }
}

pub(crate) async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.user_id != request_identity.principal_id
        || routine.owner_actor_id() != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

pub(crate) async fn webhook_routine_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.len() > 65_536 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Request body exceeds 64KB limit".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    let (secret, allow_unsigned_webhook) = match &routine.trigger {
        crate::agent::routine::Trigger::Webhook {
            secret,
            allow_unsigned_webhook,
            ..
        } => (secret.clone(), *allow_unsigned_webhook),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Routine is not a webhook trigger".to_string(),
            ));
        }
    };

    if !routine.enabled {
        return Err((StatusCode::CONFLICT, "Routine is disabled".to_string()));
    }

    if !allow_unsigned_webhook && secret.is_none() {
        return Err((
            StatusCode::FORBIDDEN,
            "Unsigned webhooks are disabled for this routine; configure a secret or opt in explicitly".to_string(),
        ));
    }

    if let Some(ref expected_secret) = secret {
        let sig_header = headers
            .get("x-webhook-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or((
                StatusCode::UNAUTHORIZED,
                "Missing X-Webhook-Signature header".to_string(),
            ))?;

        let hex_digest = sig_header.strip_prefix("sha256=").ok_or((
            StatusCode::BAD_REQUEST,
            "Signature must use sha256= prefix".to_string(),
        ))?;

        let expected_digest = hmac_sha256(expected_secret.as_bytes(), &body);
        if !constant_time_eq(hex_digest.as_bytes(), expected_digest.as_bytes()) {
            return Err((
                StatusCode::FORBIDDEN,
                "Invalid webhook signature".to_string(),
            ));
        }
    } else if !allow_unsigned_webhook {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Missing webhook secret configuration".to_string(),
        ));
    }

    if let Some(ref engine) = state.routine_engine {
        let run_id = engine
            .fire_manual(routine_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        tracing::info!(
            routine_id = %routine_id,
            run_id = %run_id,
            "Webhook triggered routine",
        );

        Ok(Json(serde_json::json!({
            "status": "triggered",
            "routine_id": routine_id,
            "run_id": run_id,
        })))
    } else {
        let prompt = match &routine.action {
            crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
            crate::agent::routine::RoutineAction::FullJob {
                title, description, ..
            } => format!("{}: {}", title, description),
            crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => prompt
                .clone()
                .unwrap_or_else(|| "Heartbeat check".to_string()),
            crate::agent::routine::RoutineAction::ExperimentCampaign { project_id, .. } => {
                format!("Run experiment campaign for project {project_id}")
            }
        };

        let content = format!("[webhook:{}] {}", routine.name, prompt);
        let mut msg = IncomingMessage::new("webhook", &routine.user_id, content);
        msg = msg.with_identity(gateway_identity(
            &routine.user_id,
            routine.owner_actor_id(),
            None,
        ));

        let tx_guard = state.msg_tx.read().await;
        let tx = tx_guard.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Channel not started".to_string(),
        ))?;

        tx.send(msg).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Channel closed".to_string(),
            )
        })?;

        Ok(Json(serde_json::json!({
            "status": "triggered",
            "routine_id": routine_id,
        })))
    }
}

pub(crate) fn hmac_sha256(key: &[u8], data: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let block_size = 64;
    let mut key_padded = vec![0u8; block_size];

    if key.len() > block_size {
        let hash = Sha256::digest(key);
        key_padded[..hash.len()].copy_from_slice(&hash);
    } else {
        key_padded[..key.len()].copy_from_slice(key);
    }

    let mut ipad = vec![0x36u8; block_size];
    let mut opad = vec![0x5cu8; block_size];
    for i in 0..block_size {
        ipad[i] ^= key_padded[i];
        opad[i] ^= key_padded[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    let digest = outer.finalize();

    digest
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

pub(crate) fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
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
            pattern, channel, ..
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            ("event".to_string(), format!("on {} /{}/", ch, pattern))
        }
        crate::agent::routine::Trigger::Webhook { path, .. } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        crate::agent::routine::Trigger::Manual => ("manual".to_string(), "manual only".to_string()),
        crate::agent::routine::Trigger::SystemEvent { message, schedule } => {
            let sched = schedule.as_deref().unwrap_or("on-demand");
            (
                "system_event".to_string(),
                format!("event: {} ({})", &message[..message.len().min(40)], sched),
            )
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

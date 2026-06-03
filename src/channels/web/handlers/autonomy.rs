use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{
    AutonomyBootstrapResponse, AutonomyChecksResponse, AutonomyEvidenceResponse,
    AutonomyPauseRequest, AutonomyPauseResponse, AutonomyRolloutsResponse, AutonomyStatusResponse,
};
use thinclaw_gateway::web::autonomy::{
    autonomy_bad_request_error, autonomy_internal_error, autonomy_pause_response,
    autonomy_rollback_error, desktop_autonomy_manager_inactive_error,
};

fn active_manager()
-> Result<Arc<crate::desktop_autonomy::DesktopAutonomyManager>, (StatusCode, String)> {
    crate::desktop_autonomy::desktop_autonomy_manager()
        .ok_or_else(desktop_autonomy_manager_inactive_error)
}

pub(crate) async fn autonomy_status_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyStatusResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    Ok(Json(manager.status().await))
}

pub(crate) async fn autonomy_bootstrap_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyBootstrapResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .bootstrap()
        .await
        .map(Json)
        .map_err(autonomy_internal_error)
}

pub(crate) async fn autonomy_pause_handler(
    State(_state): State<Arc<GatewayState>>,
    body: Option<Json<AutonomyPauseRequest>>,
) -> Result<Json<AutonomyPauseResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    let reason = body.and_then(|value| value.reason.clone());
    manager.pause(reason).await;
    Ok(Json(autonomy_pause_response(true)))
}

pub(crate) async fn autonomy_resume_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyPauseResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager.resume().await.map_err(autonomy_bad_request_error)?;
    Ok(Json(autonomy_pause_response(false)))
}

pub(crate) async fn autonomy_permissions_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .desktop_permission_status()
        .await
        .map(Json)
        .map_err(autonomy_internal_error)
}

pub(crate) async fn autonomy_rollback_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .rollback()
        .await
        .map(Json)
        .map_err(autonomy_rollback_error)
}

pub(crate) async fn autonomy_rollouts_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyRolloutsResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .rollout_summary()
        .await
        .map(Json)
        .map_err(autonomy_internal_error)
}

pub(crate) async fn autonomy_checks_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyChecksResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .checks_summary()
        .await
        .map(Json)
        .map_err(autonomy_internal_error)
}

pub(crate) async fn autonomy_evidence_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<AutonomyEvidenceResponse>, (StatusCode, String)> {
    let manager = active_manager()?;
    manager
        .evidence_summary()
        .await
        .map(Json)
        .map_err(autonomy_internal_error)
}

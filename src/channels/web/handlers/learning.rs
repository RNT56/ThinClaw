use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::api::learning as learning_api;
use crate::channels::web::identity_helpers::{
    request_actor_id, request_user_id, requested_identity_override,
};
use crate::channels::web::server::GatewayState;

fn learning_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(50).clamp(1, 200)
}

fn learning_orchestrator(
    state: &GatewayState,
) -> Result<crate::agent::learning::LearningOrchestrator, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    Ok(crate::agent::learning::LearningOrchestrator::new(
        store.clone(),
        state.workspace.clone(),
        state.skill_registry.clone(),
    ))
}

pub(crate) async fn learning_status_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningStatusResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let orchestrator = learning_orchestrator(&state)?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::status(store, &orchestrator, &user_id, limit)
        .await
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_history_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningHistoryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, query.actor_id.as_deref(), &user_id);
    let limit = learning_limit(query.limit);
    let actor_filter = requested_identity_override(query.actor_id.as_deref()).unwrap_or(actor_id);

    learning_api::history(
        store,
        &user_id,
        Some(actor_filter.as_str()),
        query.channel.as_deref(),
        query.thread_id.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_candidates_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningCandidateResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::candidates(
        store,
        &user_id,
        query.candidate_type.as_deref(),
        query.risk_tier.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_artifact_versions_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningArtifactVersionResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::artifact_versions(
        store,
        &user_id,
        query.artifact_type.as_deref(),
        query.artifact_name.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_feedback_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningFeedbackResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::feedback(
        store,
        &user_id,
        query.target_type.as_deref(),
        query.target_id.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_feedback_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<learning_api::LearningFeedbackRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_api::LearningFeedbackActionResponse>,
    ),
    (StatusCode, String),
> {
    let orchestrator = learning_orchestrator(&state)?;
    let user_id = request_user_id(&state, None).await;

    let response = learning_api::submit_feedback(
        &orchestrator,
        &user_id,
        &req.target_type,
        &req.target_id,
        &req.verdict,
        req.note.as_deref(),
        req.metadata.as_ref(),
    )
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn learning_provider_health_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningProviderHealthResponse>, (StatusCode, String)> {
    let orchestrator = learning_orchestrator(&state)?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;

    learning_api::provider_health(&orchestrator, &user_id)
        .await
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_code_proposals_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningCodeProposalResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::code_proposals(store, &user_id, query.status.as_deref(), limit)
        .await
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_code_proposal_review_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<learning_api::LearningCodeProposalReviewRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_api::LearningCodeProposalReviewResponse>,
    ),
    (StatusCode, String),
> {
    let orchestrator = learning_orchestrator(&state)?;
    let user_id = request_user_id(&state, None).await;
    let proposal_id = Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid proposal ID (expected UUID)".to_string(),
        )
    })?;

    let response = learning_api::review_code_proposal(
        &orchestrator,
        &user_id,
        proposal_id,
        &req.decision,
        req.note.as_deref(),
    )
    .await
    .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;

    Ok((StatusCode::OK, Json(response)))
}

pub(crate) async fn learning_rollbacks_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningRollbackResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let limit = learning_limit(query.limit);

    learning_api::rollbacks(
        store,
        &user_id,
        query.artifact_type.as_deref(),
        query.artifact_name.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn learning_rollback_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<learning_api::LearningRollbackRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_api::LearningRollbackActionResponse>,
    ),
    (StatusCode, String),
> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let user_id = request_user_id(&state, None).await;

    let response = learning_api::record_rollback(
        store,
        &user_id,
        &req.artifact_type,
        &req.artifact_name,
        req.artifact_version_id,
        &req.reason,
        req.metadata.as_ref(),
    )
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok((StatusCode::CREATED, Json(response)))
}

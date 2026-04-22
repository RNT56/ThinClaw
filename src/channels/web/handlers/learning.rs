use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::agent::outcomes::OutcomeService;
use crate::api::learning as learning_api;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, request_identity_with_overrides, requested_identity_override,
};
use crate::channels::web::server::GatewayState;

fn learning_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(50).clamp(1, 200)
}

fn learning_api_error(error: crate::api::ApiError) -> (StatusCode, String) {
    match error {
        crate::api::ApiError::InvalidInput(message) => (StatusCode::BAD_REQUEST, message),
        crate::api::ApiError::SessionNotFound(message) => (StatusCode::NOT_FOUND, message),
        crate::api::ApiError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
        crate::api::ApiError::FeatureDisabled(message) => (StatusCode::FORBIDDEN, message),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
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
    )
    .with_routine_engine(state.routine_engine.clone()))
}

pub(crate) async fn learning_status_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningStatusResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let orchestrator = learning_orchestrator(&state)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::status(store, &orchestrator, &request_identity.principal_id, limit)
        .await
        .map(Json)
        .map_err(learning_api_error)
}

pub(crate) async fn learning_history_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningHistoryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);
    let actor_filter = requested_identity_override(query.actor_id.as_deref())
        .unwrap_or_else(|| request_identity.actor_id.clone());

    learning_api::history(
        store,
        &request_identity.principal_id,
        Some(actor_filter.as_str()),
        query.channel.as_deref(),
        query.thread_id.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_candidates_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningCandidateResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::candidates(
        store,
        &request_identity.principal_id,
        query.candidate_type.as_deref(),
        query.risk_tier.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_artifact_versions_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningArtifactVersionResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::artifact_versions(
        store,
        &request_identity.principal_id,
        query.artifact_type.as_deref(),
        query.artifact_name.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_feedback_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningFeedbackResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::feedback(
        store,
        &request_identity.principal_id,
        query.target_type.as_deref(),
        query.target_id.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_feedback_submit_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<learning_api::LearningFeedbackRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_api::LearningFeedbackActionResponse>,
    ),
    (StatusCode, String),
> {
    let orchestrator = learning_orchestrator(&state)?;

    let response = learning_api::submit_feedback(
        &orchestrator,
        &request_identity.principal_id,
        &req.target_type,
        &req.target_id,
        &req.verdict,
        req.note.as_deref(),
        req.metadata.as_ref(),
    )
    .await
    .map_err(learning_api_error)?;

    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn learning_provider_health_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningProviderHealthResponse>, (StatusCode, String)> {
    let orchestrator = learning_orchestrator(&state)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;

    learning_api::provider_health(&orchestrator, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(learning_api_error)
}

pub(crate) async fn learning_code_proposals_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningCodeProposalResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::code_proposals(
        store,
        &request_identity.principal_id,
        query.status.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_code_proposal_review_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
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
    let proposal_id = Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid proposal ID (expected UUID)".to_string(),
        )
    })?;

    let response = learning_api::review_code_proposal(
        &orchestrator,
        &request_identity.principal_id,
        proposal_id,
        &req.decision,
        req.note.as_deref(),
    )
    .await
    .map_err(learning_api_error)?;

    Ok((StatusCode::OK, Json(response)))
}

pub(crate) async fn learning_rollbacks_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningRollbackResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);

    learning_api::rollbacks(
        store,
        &request_identity.principal_id,
        query.artifact_type.as_deref(),
        query.artifact_name.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_outcomes_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<learning_api::LearningListQuery>,
) -> Result<Json<learning_api::LearningOutcomeResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let actor_filter = requested_identity_override(query.actor_id.as_deref())
        .unwrap_or_else(|| request_identity.actor_id.clone());
    let limit = learning_limit(query.limit);

    learning_api::outcomes(
        store,
        &request_identity.principal_id,
        Some(actor_filter.as_str()),
        query.status.as_deref(),
        query.contract_type.as_deref(),
        query.source_kind.as_deref(),
        query.thread_id.as_deref(),
        limit,
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_outcome_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<learning_api::LearningOutcomeDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let contract_id = Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid outcome contract ID".to_string(),
        )
    })?;

    learning_api::outcome_detail(store, &request_identity.principal_id, contract_id)
        .await
        .map(Json)
        .map_err(learning_api_error)
}

pub(crate) async fn learning_outcome_review_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<learning_api::LearningOutcomeReviewRequest>,
) -> Result<Json<learning_api::LearningOutcomeReviewResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let contract_id = Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid outcome contract ID".to_string(),
        )
    })?;

    learning_api::review_outcome(
        store,
        &request_identity.principal_id,
        contract_id,
        &req.decision,
        req.verdict.as_deref(),
    )
    .await
    .map(Json)
    .map_err(learning_api_error)
}

pub(crate) async fn learning_outcomes_evaluate_now_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<learning_api::LearningOutcomeEvaluateNowResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let safety = Arc::new(crate::safety::SafetyLayer::new(
        &crate::config::SafetyConfig::default(),
    ));
    let service = OutcomeService::new(store.clone(), state.llm_provider.clone(), safety)
        .with_learning_context(
            state.workspace.clone(),
            state.skill_registry.clone(),
            state.routine_engine.clone(),
        );
    let processed = service
        .run_once_for_user(&request_identity.principal_id)
        .await
        .map_err(|error| learning_api_error(crate::api::ApiError::Internal(error)))?;

    Ok(Json(learning_api::LearningOutcomeEvaluateNowResponse {
        status: "processed",
        processed,
    }))
}

pub(crate) async fn learning_rollback_submit_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
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

    let response = learning_api::record_rollback(
        store,
        &request_identity.principal_id,
        &req.artifact_type,
        &req.artifact_name,
        req.artifact_version_id,
        &req.reason,
        req.metadata.as_ref(),
    )
    .await
    .map_err(learning_api_error)?;

    Ok((StatusCode::CREATED, Json(response)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{OutcomeContract, OutcomeObservation};
    use chrono::{Duration, Utc};

    fn test_request_identity(user_id: &str) -> GatewayRequestIdentity {
        GatewayRequestIdentity::new(
            user_id,
            user_id,
            crate::channels::web::identity_helpers::GatewayAuthSource::BearerHeader,
            false,
        )
    }

    fn test_gateway_state(
        user_id: &str,
        store: Option<Arc<dyn crate::db::Database>>,
    ) -> GatewayState {
        GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: crate::channels::web::sse::SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store,
            job_manager: None,
            prompt_queue: None,
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: user_id.to_string(),
            actor_id: user_id.to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: crate::channels::web::rate_limiter::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        }
    }

    fn outcome_contract(user_id: &str) -> OutcomeContract {
        let now = Utc::now();
        OutcomeContract {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            actor_id: Some(user_id.to_string()),
            channel: Some("gateway".to_string()),
            thread_id: Some("thread-1".to_string()),
            source_kind: "artifact_version".to_string(),
            source_id: Uuid::new_v4().to_string(),
            contract_type: "tool_durability".to_string(),
            status: "open".to_string(),
            summary: Some("Test outcome contract".to_string()),
            due_at: now - Duration::minutes(5),
            expires_at: now + Duration::hours(72),
            final_verdict: None,
            final_score: None,
            evaluation_details: serde_json::json!({}),
            metadata: serde_json::json!({
                "pattern_key": "artifact:test",
                "artifact_type": "memory",
                "artifact_name": "MEMORY.md"
            }),
            dedupe_key: format!("contract:{user_id}:{}", Uuid::new_v4()),
            claimed_at: None,
            evaluated_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn outcome_observation(contract_id: Uuid) -> OutcomeObservation {
        let now = Utc::now();
        OutcomeObservation {
            id: Uuid::new_v4(),
            contract_id,
            observation_kind: "explicit_approval".to_string(),
            polarity: "positive".to_string(),
            weight: 1.0,
            summary: Some("Looks good".to_string()),
            evidence: serde_json::json!({"source": "test"}),
            fingerprint: format!("obs:{contract_id}:approval"),
            observed_at: now,
            created_at: now,
        }
    }

    #[cfg(feature = "libsql")]
    async fn enable_outcomes(store: &Arc<dyn crate::db::Database>, user_id: &str) {
        store
            .set_setting(user_id, "learning.enabled", &serde_json::json!(true))
            .await
            .expect("set learning.enabled");
        store
            .set_setting(
                user_id,
                "learning.outcomes.enabled",
                &serde_json::json!(true),
            )
            .await
            .expect("set learning.outcomes.enabled");
        store
            .set_setting(
                user_id,
                "learning.outcomes.evaluation_interval_secs",
                &serde_json::json!(1),
            )
            .await
            .expect("set learning.outcomes.evaluation_interval_secs");
        store
            .set_setting(
                user_id,
                "learning.outcomes.max_due_per_tick",
                &serde_json::json!(10),
            )
            .await
            .expect("set learning.outcomes.max_due_per_tick");
    }

    #[test]
    fn learning_api_error_maps_known_statuses() {
        assert_eq!(
            learning_api_error(crate::api::ApiError::InvalidInput("bad".to_string())).0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            learning_api_error(crate::api::ApiError::SessionNotFound("missing".to_string())).0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            learning_api_error(crate::api::ApiError::Unavailable("down".to_string())).0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            learning_api_error(crate::api::ApiError::FeatureDisabled("off".to_string())).0,
            StatusCode::FORBIDDEN
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn outcome_detail_handler_rejects_invalid_uuid() {
        let (db, _guard) = crate::testing::test_db().await;
        let state = Arc::new(test_gateway_state("gateway-user", Some(db)));

        let response = learning_outcome_detail_handler(
            State(state),
            test_request_identity("gateway-user"),
            Path("not-a-uuid".to_string()),
        )
        .await;

        let error = response.expect_err("invalid UUID should fail");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn outcome_detail_handler_returns_not_found_for_missing_contract() {
        let (db, _guard) = crate::testing::test_db().await;
        let state = Arc::new(test_gateway_state("gateway-user", Some(db)));

        let response = learning_outcome_detail_handler(
            State(state),
            test_request_identity("gateway-user"),
            Path(Uuid::new_v4().to_string()),
        )
        .await;

        let error = response.expect_err("missing contract should fail");
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn outcome_review_handler_returns_bad_request_for_missing_confirm_verdict() {
        let (db, _guard) = crate::testing::test_db().await;
        let contract = outcome_contract("gateway-user");
        db.insert_outcome_contract(&contract)
            .await
            .expect("insert outcome contract");
        let state = Arc::new(test_gateway_state("gateway-user", Some(db)));

        let response = learning_outcome_review_handler(
            State(state),
            test_request_identity("gateway-user"),
            Path(contract.id.to_string()),
            Json(learning_api::LearningOutcomeReviewRequest {
                decision: "confirm".to_string(),
                verdict: None,
            }),
        )
        .await;

        let error = response.expect_err("confirm without verdict should fail");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn evaluate_now_handler_only_processes_requested_user() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_a = "gateway-user-a";
        let user_b = "gateway-user-b";
        enable_outcomes(&db, user_a).await;
        enable_outcomes(&db, user_b).await;

        let contract_a = outcome_contract(user_a);
        let contract_b = outcome_contract(user_b);
        db.insert_outcome_contract(&contract_a)
            .await
            .expect("insert outcome contract a");
        db.insert_outcome_contract(&contract_b)
            .await
            .expect("insert outcome contract b");
        db.insert_outcome_observation(&outcome_observation(contract_a.id))
            .await
            .expect("insert observation a");
        db.insert_outcome_observation(&outcome_observation(contract_b.id))
            .await
            .expect("insert observation b");

        let state = Arc::new(test_gateway_state(user_a, Some(db.clone())));
        let Json(response) =
            learning_outcomes_evaluate_now_handler(State(state), test_request_identity(user_a))
                .await
                .expect("evaluate now should succeed");

        assert_eq!(response.status, "processed");
        assert_eq!(response.processed, 1);

        let updated_a = db
            .get_outcome_contract(user_a, contract_a.id)
            .await
            .expect("get contract a")
            .expect("contract a exists");
        let updated_b = db
            .get_outcome_contract(user_b, contract_b.id)
            .await
            .expect("get contract b")
            .expect("contract b exists");
        assert_eq!(updated_a.status, "evaluated");
        assert_eq!(updated_b.status, "open");
    }
}

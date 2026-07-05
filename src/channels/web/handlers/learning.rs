use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::agent::outcomes::OutcomeService;
use crate::api::learning as learning_api;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, request_identity_with_overrides,
};
use crate::channels::web::server::GatewayState;
use thinclaw_gateway::web::{
    api::{FeatureDisabledStatus, gateway_api_error_response, learning_limit},
    learning as learning_web,
};

fn learning_api_error(error: crate::api::ApiError) -> (StatusCode, String) {
    gateway_api_error_response(error, FeatureDisabledStatus::Forbidden)
}

fn learning_orchestrator(
    state: &GatewayState,
) -> Result<crate::agent::learning::LearningOrchestrator, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;

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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningStatusResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningHistoryResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let limit = learning_limit(query.limit);
    let actor_filter =
        learning_web::learning_actor_filter(query.actor_id.as_deref(), &request_identity.actor_id);

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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningCandidateResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningArtifactVersionResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningFeedbackResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Json(req): Json<learning_web::LearningFeedbackRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_web::LearningFeedbackActionResponse>,
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningProviderHealthResponse>, (StatusCode, String)> {
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningCodeProposalResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Json(req): Json<learning_web::LearningCodeProposalReviewRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_web::LearningCodeProposalReviewResponse>,
    ),
    (StatusCode, String),
> {
    let orchestrator = learning_orchestrator(&state)?;
    let proposal_id = learning_web::parse_learning_proposal_id(&id)?;

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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningRollbackResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
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
    Query(query): Query<learning_web::LearningListQuery>,
) -> Result<Json<learning_web::LearningOutcomeResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let actor_filter =
        learning_web::learning_actor_filter(query.actor_id.as_deref(), &request_identity.actor_id);
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
) -> Result<Json<learning_web::LearningOutcomeDetailResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
    let contract_id = learning_web::parse_learning_outcome_contract_id(&id)?;

    learning_api::outcome_detail(store, &request_identity.principal_id, contract_id)
        .await
        .map(Json)
        .map_err(learning_api_error)
}

pub(crate) async fn learning_outcome_review_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<learning_web::LearningOutcomeReviewRequest>,
) -> Result<Json<learning_web::LearningOutcomeReviewResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
    let contract_id = learning_web::parse_learning_outcome_contract_id(&id)?;

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
) -> Result<Json<learning_web::LearningOutcomeEvaluateNowResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;
    let service = OutcomeService::new(store.clone(), state.llm_provider.clone())
        .with_learning_context(
            state.workspace.clone(),
            state.skill_registry.clone(),
            state.routine_engine.clone(),
        );
    let processed = service
        .run_once_for_user(&request_identity.principal_id)
        .await
        .map_err(|error| learning_api_error(crate::api::ApiError::Internal(error)))?;

    Ok(Json(learning_web::learning_outcome_evaluate_now_response(
        processed,
    )))
}

pub(crate) async fn learning_rollback_submit_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<learning_web::LearningRollbackRequest>,
) -> Result<
    (
        StatusCode,
        Json<learning_web::LearningRollbackActionResponse>,
    ),
    (StatusCode, String),
> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(learning_web::learning_database_unavailable)?;

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
    #[cfg(feature = "libsql")]
    use crate::history::{OutcomeContract, OutcomeObservation};
    #[cfg(feature = "libsql")]
    use chrono::{Duration, Utc};
    #[cfg(feature = "libsql")]
    use uuid::Uuid;

    #[cfg(feature = "libsql")]
    fn test_request_identity(user_id: &str) -> GatewayRequestIdentity {
        GatewayRequestIdentity::new(
            user_id,
            user_id,
            crate::channels::web::identity_helpers::GatewayAuthSource::BearerHeader,
            false,
        )
    }

    #[cfg(feature = "libsql")]
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
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: crate::channels::web::rate_limiter::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            metrics_registry: None,
            response_cache: None,
            routine_engine: None,
            repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            hooks: None,
        }
    }

    #[cfg(feature = "libsql")]
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

    #[cfg(feature = "libsql")]
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
            Json(learning_web::LearningOutcomeReviewRequest {
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

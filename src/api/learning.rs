//! Learning API — status, history, candidates, provider health, and review actions.

use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::agent::{learning::LearningOrchestrator, outcomes};
use crate::db::Database;
use crate::history::LearningRollbackRecord as DbLearningRollbackRecord;

use super::error::{ApiError, ApiResult};

pub use thinclaw_gateway::web::learning::*;

const MAX_LEARNING_LIST_RESULTS: usize = 500;

fn bounded_learning_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_LEARNING_LIST_RESULTS)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub async fn status(
    store: &Arc<dyn Database>,
    orchestrator: &LearningOrchestrator,
    user_id: &str,
    limit: usize,
) -> ApiResult<LearningStatusResponse> {
    let limit = bounded_learning_limit(limit);
    let settings = orchestrator.load_settings_for_user(user_id).await;
    let recent_limit = (limit.saturating_add(1)) as i64;

    let events = store
        .list_learning_events(user_id, None, None, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let evaluations = store
        .list_learning_evaluations(user_id, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let candidates = store
        .list_learning_candidates(user_id, None, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let artifact_versions = store
        .list_learning_artifact_versions(user_id, None, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let feedback = store
        .list_learning_feedback(user_id, None, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let rollbacks = store
        .list_learning_rollbacks(user_id, None, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let proposals = store
        .list_learning_code_proposals(user_id, None, recent_limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let outcome_stats = store
        .outcome_summary_stats(user_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let outcomes_evaluator_healthy = outcomes::evaluator_is_healthy(store, user_id)
        .await
        .map_err(ApiError::Internal)?;
    let provider_health = orchestrator.provider_health(user_id).await;
    let provider_items: Vec<LearningProviderHealthItem> = provider_health
        .into_iter()
        .map(|health| {
            learning_provider_health_item(LearningProviderHealthItemInput {
                provider: health.provider,
                active: health.active,
                enabled: health.enabled,
                healthy: health.healthy,
                readiness: health.readiness.as_str().to_string(),
                latency_ms: health.latency_ms,
                error: health.error,
                capabilities: health.capabilities,
                metadata: health.metadata,
            })
        })
        .collect();

    let recent = learning_recent_counts(LearningRecentCountsInput {
        events: events.len(),
        evaluations: evaluations.len(),
        candidates: candidates.len(),
        artifact_versions: artifact_versions.len(),
        feedback: feedback.len(),
        rollbacks: rollbacks.len(),
        code_proposals: proposals.len(),
        limit,
    });

    Ok(LearningStatusResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        enabled: settings.enabled,
        auto_apply_classes: settings.auto_apply_classes.clone(),
        safe_mode_enabled: settings.safe_mode.enabled,
        safe_mode_rollback_ratio: settings.safe_mode.thresholds.rollback_ratio,
        safe_mode_negative_feedback_ratio: settings.safe_mode.thresholds.negative_feedback_ratio,
        safe_mode_min_samples: settings.safe_mode.thresholds.min_samples,
        reflection_min_tool_calls: settings.reflection.min_tool_calls,
        reflection_user_correction_threshold: settings.reflection.user_correction_threshold,
        prompt_mutation_enabled: settings.prompt_mutation.enabled,
        code_proposals_enabled: settings.code_proposals.enabled,
        code_proposal_publish_mode: settings.code_proposals.publish_mode.clone(),
        exports_enabled: settings.exports.enabled,
        outcomes_enabled: settings.enabled && settings.outcomes.enabled,
        outcomes_open: outcome_stats.open,
        outcomes_due: outcome_stats.due,
        outcomes_evaluated_last_7d: outcome_stats.evaluated_last_7d,
        outcomes_negative_ratio_last_7d: outcome_stats.negative_ratio_last_7d,
        outcomes_evaluator_healthy,
        recent,
        provider_health: summarize_learning_provider_health(&provider_items),
    })
}

pub async fn history(
    store: &Arc<dyn Database>,
    user_id: &str,
    actor_id: Option<&str>,
    channel: Option<&str>,
    thread_id: Option<&str>,
    limit: usize,
) -> ApiResult<LearningHistoryResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let events = store
        .list_learning_events(user_id, actor_id, channel, thread_id, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let evaluations = store
        .list_learning_evaluations(user_id, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = events.len() > limit;
    Ok(LearningHistoryResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        events: events
            .iter()
            .take(limit)
            .map(learning_event_item_from_record)
            .collect(),
        evaluations: evaluations
            .iter()
            .take(limit)
            .map(learning_evaluation_item_from_record)
            .collect(),
    })
}

pub async fn candidates(
    store: &Arc<dyn Database>,
    user_id: &str,
    candidate_type: Option<&str>,
    risk_tier: Option<&str>,
    limit: usize,
) -> ApiResult<LearningCandidateResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let candidates = store
        .list_learning_candidates(user_id, candidate_type, risk_tier, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = candidates.len() > limit;
    Ok(LearningCandidateResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        candidates: candidates
            .iter()
            .take(limit)
            .map(learning_candidate_item_from_record)
            .collect(),
    })
}

pub async fn artifact_versions(
    store: &Arc<dyn Database>,
    user_id: &str,
    artifact_type: Option<&str>,
    artifact_name: Option<&str>,
    limit: usize,
) -> ApiResult<LearningArtifactVersionResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let versions = store
        .list_learning_artifact_versions(user_id, artifact_type, artifact_name, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = versions.len() > limit;
    Ok(LearningArtifactVersionResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        versions: versions
            .iter()
            .take(limit)
            .map(learning_artifact_version_item_from_record)
            .collect(),
    })
}

pub async fn feedback(
    store: &Arc<dyn Database>,
    user_id: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    limit: usize,
) -> ApiResult<LearningFeedbackResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let entries = store
        .list_learning_feedback(user_id, target_type, target_id, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = entries.len() > limit;
    Ok(LearningFeedbackResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        feedback: entries
            .iter()
            .take(limit)
            .map(learning_feedback_item_from_record)
            .collect(),
    })
}

pub async fn provider_health(
    orchestrator: &LearningOrchestrator,
    user_id: &str,
) -> ApiResult<LearningProviderHealthResponse> {
    let providers = orchestrator.provider_health(user_id).await;
    let providers: Vec<LearningProviderHealthItem> = providers
        .into_iter()
        .map(|health| {
            learning_provider_health_item(LearningProviderHealthItemInput {
                provider: health.provider,
                active: health.active,
                enabled: health.enabled,
                healthy: health.healthy,
                readiness: health.readiness.as_str().to_string(),
                latency_ms: health.latency_ms,
                error: health.error,
                capabilities: health.capabilities,
                metadata: health.metadata,
            })
        })
        .collect();
    Ok(LearningProviderHealthResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        summary: summarize_learning_provider_health(&providers),
        providers,
    })
}

pub async fn code_proposals(
    store: &Arc<dyn Database>,
    user_id: &str,
    status: Option<&str>,
    limit: usize,
) -> ApiResult<LearningCodeProposalResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let proposals = store
        .list_learning_code_proposals(user_id, status, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = proposals.len() > limit;
    Ok(LearningCodeProposalResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        proposals: proposals
            .iter()
            .take(limit)
            .map(learning_code_proposal_item_from_record)
            .collect(),
    })
}

pub async fn rollbacks(
    store: &Arc<dyn Database>,
    user_id: &str,
    artifact_type: Option<&str>,
    artifact_name: Option<&str>,
    limit: usize,
) -> ApiResult<LearningRollbackResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let entries = store
        .list_learning_rollbacks(user_id, artifact_type, artifact_name, limit_plus_one)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = entries.len() > limit;
    Ok(LearningRollbackResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        rollbacks: entries
            .iter()
            .take(limit)
            .map(learning_rollback_item_from_record)
            .collect(),
    })
}

pub async fn outcomes(
    store: &Arc<dyn Database>,
    user_id: &str,
    actor_id: Option<&str>,
    status: Option<&str>,
    contract_type: Option<&str>,
    source_kind: Option<&str>,
    thread_id: Option<&str>,
    limit: usize,
) -> ApiResult<LearningOutcomeResponse> {
    let limit = bounded_learning_limit(limit);
    let limit_plus_one = (limit.saturating_add(1)) as i64;
    let contracts = store
        .list_outcome_contracts(&crate::history::OutcomeContractQuery {
            user_id: user_id.to_string(),
            actor_id: actor_id.map(str::to_string),
            status: status.map(str::to_string),
            contract_type: contract_type.map(str::to_string),
            source_kind: source_kind.map(str::to_string),
            source_id: None,
            thread_id: thread_id.map(str::to_string),
            limit: limit_plus_one,
        })
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let has_more = contracts.len() > limit;
    Ok(LearningOutcomeResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        has_more,
        outcomes: contracts
            .iter()
            .take(limit)
            .map(learning_outcome_contract_item_from_record)
            .collect(),
    })
}

pub async fn outcome_detail(
    store: &Arc<dyn Database>,
    user_id: &str,
    contract_id: Uuid,
) -> ApiResult<LearningOutcomeDetailResponse> {
    let contract = store
        .get_outcome_contract(user_id, contract_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(outcome_contract_not_found_message(contract_id))
        })?;
    let observations = store
        .list_outcome_observations(contract_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(LearningOutcomeDetailResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        contract: learning_outcome_contract_item_from_record(&contract),
        observations: observations
            .iter()
            .map(learning_outcome_observation_item_from_record)
            .collect(),
    })
}

pub async fn review_outcome(
    store: &Arc<dyn Database>,
    user_id: &str,
    contract_id: Uuid,
    decision: &str,
    verdict: Option<&str>,
) -> ApiResult<LearningOutcomeReviewResponse> {
    let decision = decision.trim().to_ascii_lowercase();
    let mut contract = store
        .get_outcome_contract(user_id, contract_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(outcome_contract_not_found_message(contract_id))
        })?;

    match decision.as_str() {
        "confirm" => {
            let verdict = verdict
                .map(|value| value.trim().to_ascii_lowercase())
                .ok_or_else(|| {
                    ApiError::InvalidInput(outcome_review_verdict_required_message().to_string())
                })?;
            if !matches!(verdict.as_str(), "positive" | "neutral" | "negative") {
                return Err(ApiError::InvalidInput(
                    unsupported_outcome_review_verdict_message(&verdict),
                ));
            }
            contract.status = "evaluated".to_string();
            contract.final_verdict = Some(verdict.clone());
            contract.final_score = Some(match verdict.as_str() {
                "positive" => 1.0,
                "negative" => -1.0,
                _ => 0.0,
            });
            contract.evaluated_at = Some(Utc::now());
            contract.evaluation_details = json!({
                "strategy": "manual_review",
                "review_decision": "confirm",
                "manual_verdict": verdict,
            });
        }
        "dismiss" => {
            contract.status = "dismissed".to_string();
            contract.evaluation_details = json!({
                "strategy": "manual_review",
                "review_decision": "dismiss",
            });
        }
        "requeue" => {
            contract.status = "open".to_string();
            contract.claimed_at = None;
            contract.final_verdict = None;
            contract.final_score = None;
            contract.evaluated_at = None;
            contract.due_at = Utc::now();
            contract.evaluation_details = json!({
                "strategy": "manual_review",
                "review_decision": "requeue",
            });
        }
        other => {
            return Err(ApiError::InvalidInput(
                unsupported_outcome_review_decision_message(other),
            ));
        }
    }

    outcomes::persist_manual_review_to_learning_ledger(store, &mut contract, &decision)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    contract.updated_at = Utc::now();
    store
        .update_outcome_contract(&contract)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;

    Ok(LearningOutcomeReviewResponse {
        status: "updated",
        contract: Some(learning_outcome_contract_item_from_record(&contract)),
    })
}

pub async fn submit_feedback(
    orchestrator: &LearningOrchestrator,
    user_id: &str,
    target_type: &str,
    target_id: &str,
    verdict: &str,
    note: Option<&str>,
    metadata: Option<&serde_json::Value>,
) -> ApiResult<LearningFeedbackActionResponse> {
    let id = orchestrator
        .submit_feedback(user_id, target_type, target_id, verdict, note, metadata)
        .await
        .map_err(ApiError::Internal)?;
    Ok(learning_feedback_action_response(
        LearningFeedbackActionResponseInput {
            id,
            user_id: user_id.to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
            verdict: verdict.to_string(),
            note: note.map(str::to_string),
            metadata: metadata.cloned(),
            created_at: Utc::now(),
        },
    ))
}

pub async fn review_code_proposal(
    orchestrator: &LearningOrchestrator,
    user_id: &str,
    proposal_id: Uuid,
    decision: &str,
    note: Option<&str>,
) -> ApiResult<LearningCodeProposalReviewResponse> {
    let decision_normalized = decision.trim().to_ascii_lowercase();
    if !matches!(decision_normalized.as_str(), "approve" | "reject") {
        return Err(ApiError::InvalidInput(format!(
            "Unsupported code proposal decision: {}",
            decision
        )));
    }

    let proposal = orchestrator
        .review_code_proposal(user_id, proposal_id, &decision_normalized, note)
        .await
        .map_err(ApiError::Internal)?;

    Ok(LearningCodeProposalReviewResponse {
        status: match decision_normalized.as_str() {
            "approve" => "approved",
            "reject" => "rejected",
            _ => "updated",
        },
        proposal: proposal
            .as_ref()
            .map(learning_code_proposal_item_from_record),
    })
}

pub async fn record_rollback(
    store: &Arc<dyn Database>,
    user_id: &str,
    artifact_type: &str,
    artifact_name: &str,
    artifact_version_id: Option<Uuid>,
    reason: &str,
    metadata: Option<&serde_json::Value>,
) -> ApiResult<LearningRollbackActionResponse> {
    let mut record = DbLearningRollbackRecord {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        artifact_type: artifact_type.to_string(),
        artifact_name: artifact_name.to_string(),
        artifact_version_id,
        reason: reason.to_string(),
        metadata: metadata.cloned().unwrap_or_else(|| serde_json::json!({})),
        created_at: Utc::now(),
    };
    let id = store
        .insert_learning_rollback(&record)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    record.id = id;
    if let Err(error) = outcomes::observe_rollback(store, &record).await {
        tracing::debug!(user_id = %user_id, error = %error, "Outcome rollback hook skipped");
    }
    Ok(learning_rollback_action_response(&record))
}

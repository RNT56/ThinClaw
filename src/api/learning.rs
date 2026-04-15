//! Learning API — status, history, candidates, provider health, and review actions.

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::agent::{learning::LearningOrchestrator, outcomes};
use crate::db::Database;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion, LearningCandidate as DbLearningCandidate,
    LearningCodeProposal as DbLearningCodeProposal, LearningEvaluation as DbLearningEvaluation,
    LearningEvent as DbLearningEvent, LearningFeedbackRecord as DbLearningFeedbackRecord,
    LearningRollbackRecord as DbLearningRollbackRecord, OutcomeContract as DbOutcomeContract,
    OutcomeObservation as DbOutcomeObservation,
};

use super::error::{ApiError, ApiResult};

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn event_class(event: &DbLearningEvent) -> String {
    event
        .payload
        .get("class")
        .and_then(|value| value.as_str())
        .unwrap_or(event.event_type.as_str())
        .to_string()
}

fn event_risk_tier(event: &DbLearningEvent) -> String {
    event
        .payload
        .get("risk_tier")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn event_summary(event: &DbLearningEvent) -> String {
    event
        .payload
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(&event.source)
        .to_string()
}

fn event_target(event: &DbLearningEvent) -> Option<String> {
    event
        .payload
        .get("target")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn event_confidence(event: &DbLearningEvent) -> Option<f64> {
    event
        .payload
        .get("confidence")
        .and_then(|value| value.as_f64())
}

fn to_event_item(event: &DbLearningEvent) -> LearningEventItem {
    LearningEventItem {
        id: event.id,
        user_id: event.user_id.clone(),
        actor_id: event.actor_id.clone(),
        channel: event.channel.clone(),
        thread_id: event.thread_id.clone(),
        conversation_id: event.conversation_id,
        message_id: event.message_id,
        job_id: event.job_id,
        event_type: event.event_type.clone(),
        source: event.source.clone(),
        class: event_class(event),
        risk_tier: event_risk_tier(event),
        summary: event_summary(event),
        target: event_target(event),
        confidence: event_confidence(event),
        payload: event.payload.clone(),
        metadata: event.metadata.clone(),
        created_at: event.created_at.to_rfc3339(),
    }
}

fn to_evaluation_item(evaluation: &DbLearningEvaluation) -> LearningEvaluationItem {
    LearningEvaluationItem {
        id: evaluation.id,
        learning_event_id: evaluation.learning_event_id,
        user_id: evaluation.user_id.clone(),
        evaluator: evaluation.evaluator.clone(),
        status: evaluation.status.clone(),
        score: evaluation.score,
        details: evaluation.details.clone(),
        created_at: evaluation.created_at.to_rfc3339(),
    }
}

fn to_candidate_item(candidate: &DbLearningCandidate) -> LearningCandidateItem {
    LearningCandidateItem {
        id: candidate.id,
        learning_event_id: candidate.learning_event_id,
        user_id: candidate.user_id.clone(),
        candidate_type: candidate.candidate_type.clone(),
        risk_tier: candidate.risk_tier.clone(),
        confidence: candidate.confidence,
        target_type: candidate.target_type.clone(),
        target_name: candidate.target_name.clone(),
        summary: candidate.summary.clone(),
        proposal: candidate.proposal.clone(),
        created_at: candidate.created_at.to_rfc3339(),
    }
}

fn to_artifact_item(version: &DbLearningArtifactVersion) -> LearningArtifactVersionItem {
    LearningArtifactVersionItem {
        id: version.id,
        candidate_id: version.candidate_id,
        user_id: version.user_id.clone(),
        artifact_type: version.artifact_type.clone(),
        artifact_name: version.artifact_name.clone(),
        version_label: version.version_label.clone(),
        status: version.status.clone(),
        diff_summary: version.diff_summary.clone(),
        before_content: version.before_content.clone(),
        after_content: version.after_content.clone(),
        provenance: version.provenance.clone(),
        created_at: version.created_at.to_rfc3339(),
    }
}

fn to_feedback_item(record: &DbLearningFeedbackRecord) -> LearningFeedbackItem {
    LearningFeedbackItem {
        id: record.id,
        user_id: record.user_id.clone(),
        target_type: record.target_type.clone(),
        target_id: record.target_id.clone(),
        verdict: record.verdict.clone(),
        note: record.note.clone(),
        metadata: record.metadata.clone(),
        created_at: record.created_at.to_rfc3339(),
    }
}

fn to_rollback_item(record: &DbLearningRollbackRecord) -> LearningRollbackItem {
    LearningRollbackItem {
        id: record.id,
        user_id: record.user_id.clone(),
        artifact_type: record.artifact_type.clone(),
        artifact_name: record.artifact_name.clone(),
        artifact_version_id: record.artifact_version_id,
        reason: record.reason.clone(),
        metadata: record.metadata.clone(),
        created_at: record.created_at.to_rfc3339(),
    }
}

fn to_proposal_item(proposal: &DbLearningCodeProposal) -> LearningCodeProposalItem {
    LearningCodeProposalItem {
        id: proposal.id,
        learning_event_id: proposal.learning_event_id,
        user_id: proposal.user_id.clone(),
        status: proposal.status.clone(),
        title: proposal.title.clone(),
        rationale: proposal.rationale.clone(),
        target_files: proposal.target_files.clone(),
        diff: proposal.diff.clone(),
        validation_results: proposal.validation_results.clone(),
        rollback_note: proposal.rollback_note.clone(),
        confidence: proposal.confidence,
        branch_name: proposal.branch_name.clone(),
        pr_url: proposal.pr_url.clone(),
        metadata: proposal.metadata.clone(),
        created_at: proposal.created_at.to_rfc3339(),
        updated_at: proposal.updated_at.to_rfc3339(),
    }
}

fn to_outcome_item(contract: &DbOutcomeContract) -> LearningOutcomeContractItem {
    let source_ref = outcome_source_ref(contract);
    LearningOutcomeContractItem {
        id: contract.id,
        user_id: contract.user_id.clone(),
        actor_id: contract.actor_id.clone(),
        channel: contract.channel.clone(),
        thread_id: contract.thread_id.clone(),
        source_kind: contract.source_kind.clone(),
        source_id: contract.source_id.clone(),
        source_ref,
        contract_type: contract.contract_type.clone(),
        status: contract.status.clone(),
        summary: contract.summary.clone(),
        due_at: contract.due_at.to_rfc3339(),
        expires_at: contract.expires_at.to_rfc3339(),
        final_verdict: contract.final_verdict.clone(),
        final_score: contract.final_score,
        ledger_learning_event_id: outcomes::ledger_learning_event_id(contract),
        last_evaluator: outcomes::contract_last_evaluator(contract),
        evaluation_details: contract.evaluation_details.clone(),
        metadata: contract.metadata.clone(),
        dedupe_key: contract.dedupe_key.clone(),
        claimed_at: contract.claimed_at.map(|dt| dt.to_rfc3339()),
        evaluated_at: contract.evaluated_at.map(|dt| dt.to_rfc3339()),
        created_at: contract.created_at.to_rfc3339(),
        updated_at: contract.updated_at.to_rfc3339(),
    }
}

fn outcome_source_ref(contract: &DbOutcomeContract) -> LearningOutcomeSourceRef {
    LearningOutcomeSourceRef {
        kind: contract.source_kind.clone(),
        id: contract.source_id.clone(),
        routine_id: contract
            .metadata
            .get("routine_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        artifact_name: contract
            .metadata
            .get("artifact_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        artifact_type: contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }
}

fn to_outcome_observation_item(
    observation: &DbOutcomeObservation,
) -> LearningOutcomeObservationItem {
    LearningOutcomeObservationItem {
        id: observation.id,
        contract_id: observation.contract_id,
        observation_kind: observation.observation_kind.clone(),
        polarity: observation.polarity.clone(),
        weight: observation.weight,
        summary: observation.summary.clone(),
        evidence: observation.evidence.clone(),
        fingerprint: observation.fingerprint.clone(),
        observed_at: observation.observed_at.to_rfc3339(),
        created_at: observation.created_at.to_rfc3339(),
    }
}

fn to_provider_item(
    health: crate::agent::learning::ProviderHealthStatus,
) -> LearningProviderHealthItem {
    LearningProviderHealthItem {
        provider: health.provider,
        enabled: health.enabled,
        healthy: health.healthy,
        latency_ms: health.latency_ms,
        error: health.error,
        metadata: health.metadata,
    }
}

fn summarize_provider_health(
    items: &[LearningProviderHealthItem],
) -> LearningProviderHealthSummary {
    let total = items.len();
    let healthy = items.iter().filter(|item| item.healthy).count();
    let enabled = items.iter().filter(|item| item.enabled).count();
    let unhealthy = total.saturating_sub(healthy);
    LearningProviderHealthSummary {
        total,
        enabled,
        healthy,
        unhealthy,
    }
}

fn recent_counts(
    events: &[DbLearningEvent],
    evaluations: &[DbLearningEvaluation],
    candidates: &[DbLearningCandidate],
    artifacts: &[DbLearningArtifactVersion],
    feedback: &[DbLearningFeedbackRecord],
    rollbacks: &[DbLearningRollbackRecord],
    proposals: &[DbLearningCodeProposal],
    limit: usize,
) -> LearningRecentCounts {
    LearningRecentCounts {
        events: events.len().min(limit),
        evaluations: evaluations.len().min(limit),
        candidates: candidates.len().min(limit),
        artifact_versions: artifacts.len().min(limit),
        feedback: feedback.len().min(limit),
        rollbacks: rollbacks.len().min(limit),
        code_proposals: proposals.len().min(limit),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearningListQuery {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub candidate_type: Option<String>,
    #[serde(default)]
    pub risk_tier: Option<String>,
    #[serde(default)]
    pub artifact_type: Option<String>,
    #[serde(default)]
    pub artifact_name: Option<String>,
    #[serde(default)]
    pub contract_type: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub target_type: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearningFeedbackRequest {
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearningCodeProposalReviewRequest {
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearningRollbackRequest {
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default)]
    pub artifact_version_id: Option<Uuid>,
    pub reason: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearningOutcomeReviewRequest {
    pub decision: String,
    #[serde(default)]
    pub verdict: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningRecentCounts {
    pub events: usize,
    pub evaluations: usize,
    pub candidates: usize,
    pub artifact_versions: usize,
    pub feedback: usize,
    pub rollbacks: usize,
    pub code_proposals: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningProviderHealthSummary {
    pub total: usize,
    pub enabled: usize,
    pub healthy: usize,
    pub unhealthy: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningStatusResponse {
    pub generated_at: String,
    pub user_id: String,
    pub enabled: bool,
    pub auto_apply_classes: Vec<String>,
    pub safe_mode_enabled: bool,
    pub safe_mode_rollback_ratio: f64,
    pub safe_mode_negative_feedback_ratio: f64,
    pub safe_mode_min_samples: u32,
    pub reflection_min_tool_calls: u32,
    pub reflection_user_correction_threshold: u32,
    pub prompt_mutation_enabled: bool,
    pub code_proposals_enabled: bool,
    pub code_proposal_publish_mode: String,
    pub exports_enabled: bool,
    pub outcomes_enabled: bool,
    pub outcomes_open: u64,
    pub outcomes_due: u64,
    pub outcomes_evaluated_last_7d: u64,
    pub outcomes_negative_ratio_last_7d: f64,
    pub outcomes_evaluator_healthy: bool,
    pub recent: LearningRecentCounts,
    pub provider_health: LearningProviderHealthSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningHistoryResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub events: Vec<LearningEventItem>,
    pub evaluations: Vec<LearningEvaluationItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningCandidateResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub candidates: Vec<LearningCandidateItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningArtifactVersionResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub versions: Vec<LearningArtifactVersionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningFeedbackResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub feedback: Vec<LearningFeedbackItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningProviderHealthResponse {
    pub generated_at: String,
    pub user_id: String,
    pub summary: LearningProviderHealthSummary,
    pub providers: Vec<LearningProviderHealthItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningCodeProposalResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub proposals: Vec<LearningCodeProposalItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningRollbackResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub rollbacks: Vec<LearningRollbackItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeResponse {
    pub generated_at: String,
    pub user_id: String,
    pub has_more: bool,
    pub outcomes: Vec<LearningOutcomeContractItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeDetailResponse {
    pub generated_at: String,
    pub user_id: String,
    pub contract: LearningOutcomeContractItem,
    pub observations: Vec<LearningOutcomeObservationItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningFeedbackActionResponse {
    pub feedback: LearningFeedbackItem,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningCodeProposalReviewResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<LearningCodeProposalItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningRollbackActionResponse {
    pub rollback: LearningRollbackItem,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeReviewResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<LearningOutcomeContractItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeEvaluateNowResponse {
    pub status: &'static str,
    pub processed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningProviderHealthItem {
    pub provider: String,
    pub enabled: bool,
    pub healthy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningEventItem {
    pub id: Uuid,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
    pub event_type: String,
    pub source: String,
    pub class: String,
    pub risk_tier: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningEvaluationItem {
    pub id: Uuid,
    pub learning_event_id: Uuid,
    pub user_id: String,
    pub evaluator: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub details: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningCandidateItem {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub candidate_type: String,
    pub risk_tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub proposal: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningArtifactVersionItem {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_label: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_content: Option<String>,
    pub provenance: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningFeedbackItem {
    pub id: Uuid,
    pub user_id: String,
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningRollbackItem {
    pub id: Uuid,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_version_id: Option<Uuid>,
    pub reason: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningCodeProposalItem {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub status: String,
    pub title: String,
    pub rationale: String,
    pub target_files: Vec<String>,
    pub diff: String,
    pub validation_results: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeSourceRef {
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeContractItem {
    pub id: Uuid,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub source_kind: String,
    pub source_id: String,
    pub source_ref: LearningOutcomeSourceRef,
    pub contract_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub due_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_learning_event_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluator: Option<String>,
    pub evaluation_details: serde_json::Value,
    pub metadata: serde_json::Value,
    pub dedupe_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearningOutcomeObservationItem {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub observation_kind: String,
    pub polarity: String,
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub evidence: serde_json::Value,
    pub fingerprint: String,
    pub observed_at: String,
    pub created_at: String,
}

pub async fn status(
    store: &Arc<dyn Database>,
    orchestrator: &LearningOrchestrator,
    user_id: &str,
    limit: usize,
) -> ApiResult<LearningStatusResponse> {
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
    let provider_items: Vec<LearningProviderHealthItem> =
        provider_health.into_iter().map(to_provider_item).collect();

    let recent = recent_counts(
        &events,
        &evaluations,
        &candidates,
        &artifact_versions,
        &feedback,
        &rollbacks,
        &proposals,
        limit,
    );

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
        provider_health: summarize_provider_health(&provider_items),
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
        events: events.iter().take(limit).map(to_event_item).collect(),
        evaluations: evaluations
            .iter()
            .take(limit)
            .map(to_evaluation_item)
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
            .map(to_candidate_item)
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
        versions: versions.iter().take(limit).map(to_artifact_item).collect(),
    })
}

pub async fn feedback(
    store: &Arc<dyn Database>,
    user_id: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    limit: usize,
) -> ApiResult<LearningFeedbackResponse> {
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
        feedback: entries.iter().take(limit).map(to_feedback_item).collect(),
    })
}

pub async fn provider_health(
    orchestrator: &LearningOrchestrator,
    user_id: &str,
) -> ApiResult<LearningProviderHealthResponse> {
    let providers = orchestrator.provider_health(user_id).await;
    let providers: Vec<LearningProviderHealthItem> =
        providers.into_iter().map(to_provider_item).collect();
    Ok(LearningProviderHealthResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        summary: summarize_provider_health(&providers),
        providers,
    })
}

pub async fn code_proposals(
    store: &Arc<dyn Database>,
    user_id: &str,
    status: Option<&str>,
    limit: usize,
) -> ApiResult<LearningCodeProposalResponse> {
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
        proposals: proposals.iter().take(limit).map(to_proposal_item).collect(),
    })
}

pub async fn rollbacks(
    store: &Arc<dyn Database>,
    user_id: &str,
    artifact_type: Option<&str>,
    artifact_name: Option<&str>,
    limit: usize,
) -> ApiResult<LearningRollbackResponse> {
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
        rollbacks: entries.iter().take(limit).map(to_rollback_item).collect(),
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
        outcomes: contracts.iter().take(limit).map(to_outcome_item).collect(),
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
            ApiError::SessionNotFound(format!("Outcome contract {} not found", contract_id))
        })?;
    let observations = store
        .list_outcome_observations(contract_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(LearningOutcomeDetailResponse {
        generated_at: now_rfc3339(),
        user_id: user_id.to_string(),
        contract: to_outcome_item(&contract),
        observations: observations
            .iter()
            .map(to_outcome_observation_item)
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
            ApiError::SessionNotFound(format!("Outcome contract {} not found", contract_id))
        })?;

    match decision.as_str() {
        "confirm" => {
            let verdict = verdict
                .map(|value| value.trim().to_ascii_lowercase())
                .ok_or_else(|| {
                    ApiError::InvalidInput("verdict is required for confirm".to_string())
                })?;
            if !matches!(verdict.as_str(), "positive" | "neutral" | "negative") {
                return Err(ApiError::InvalidInput(format!(
                    "unsupported verdict: {}",
                    verdict
                )));
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
            return Err(ApiError::InvalidInput(format!(
                "unsupported outcome review decision: {}",
                other
            )));
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
        contract: Some(to_outcome_item(&contract)),
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
    let feedback = LearningFeedbackItem {
        id,
        user_id: user_id.to_string(),
        target_type: target_type.to_string(),
        target_id: target_id.to_string(),
        verdict: verdict.to_string(),
        note: note.map(str::to_string),
        metadata: metadata.cloned().unwrap_or_else(|| serde_json::json!({})),
        created_at: now_rfc3339(),
    };
    Ok(LearningFeedbackActionResponse {
        feedback,
        status: "recorded",
    })
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
        proposal: proposal.as_ref().map(to_proposal_item),
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
    let record = DbLearningRollbackRecord {
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
    if let Err(error) = outcomes::observe_rollback(store, &record).await {
        tracing::debug!(user_id = %user_id, error = %error, "Outcome rollback hook skipped");
    }
    let rollback = LearningRollbackItem {
        id,
        user_id: user_id.to_string(),
        artifact_type: artifact_type.to_string(),
        artifact_name: artifact_name.to_string(),
        artifact_version_id,
        reason: reason.to_string(),
        metadata: metadata.cloned().unwrap_or_else(|| serde_json::json!({})),
        created_at: now_rfc3339(),
    };
    Ok(LearningRollbackActionResponse {
        rollback,
        status: "recorded",
    })
}

//! Root-independent learning gateway DTOs.

use std::fmt::Display;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_history::{
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, LearningFeedbackRecord, LearningRollbackRecord, OutcomeContract,
    OutcomeObservation,
};
use uuid::Uuid;

use super::identity::requested_identity_override;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LearningRecentCountsInput {
    pub events: usize,
    pub evaluations: usize,
    pub candidates: usize,
    pub artifact_versions: usize,
    pub feedback: usize,
    pub rollbacks: usize,
    pub code_proposals: usize,
    pub limit: usize,
}

pub fn learning_recent_counts(input: LearningRecentCountsInput) -> LearningRecentCounts {
    LearningRecentCounts {
        events: input.events.min(input.limit),
        evaluations: input.evaluations.min(input.limit),
        candidates: input.candidates.min(input.limit),
        artifact_versions: input.artifact_versions.min(input.limit),
        feedback: input.feedback.min(input.limit),
        rollbacks: input.rollbacks.min(input.limit),
        code_proposals: input.code_proposals.min(input.limit),
    }
}

pub fn outcome_contract_not_found_message(contract_id: impl Display) -> String {
    format!("Outcome contract {contract_id} not found")
}

pub fn outcome_review_verdict_required_message() -> &'static str {
    "verdict is required for confirm"
}

pub fn unsupported_outcome_review_verdict_message(verdict: impl Display) -> String {
    format!("unsupported verdict: {verdict}")
}

pub fn unsupported_outcome_review_decision_message(decision: impl Display) -> String {
    format!("unsupported outcome review decision: {decision}")
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    pub active: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub readiness: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LearningProviderHealthItemInput {
    pub provider: String,
    pub active: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub readiness: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub capabilities: Vec<String>,
    pub metadata: serde_json::Value,
}

pub fn learning_provider_health_item(
    input: LearningProviderHealthItemInput,
) -> LearningProviderHealthItem {
    LearningProviderHealthItem {
        provider: input.provider,
        active: input.active,
        enabled: input.enabled,
        healthy: input.healthy,
        readiness: input.readiness,
        latency_ms: input.latency_ms,
        error: input.error,
        capabilities: input.capabilities,
        metadata: input.metadata,
    }
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

pub fn learning_event_item_from_record(event: &LearningEvent) -> LearningEventItem {
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
        class: event
            .payload
            .get("class")
            .and_then(|value| value.as_str())
            .unwrap_or(event.event_type.as_str())
            .to_string(),
        risk_tier: event
            .payload
            .get("risk_tier")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        summary: event
            .payload
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or(&event.source)
            .to_string(),
        target: event
            .payload
            .get("target")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        confidence: event
            .payload
            .get("confidence")
            .and_then(|value| value.as_f64()),
        payload: event.payload.clone(),
        metadata: event.metadata.clone(),
        created_at: event.created_at.to_rfc3339(),
    }
}

pub fn learning_evaluation_item_from_record(
    evaluation: &LearningEvaluation,
) -> LearningEvaluationItem {
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

pub fn learning_candidate_item_from_record(candidate: &LearningCandidate) -> LearningCandidateItem {
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

pub fn learning_artifact_version_item_from_record(
    version: &LearningArtifactVersion,
) -> LearningArtifactVersionItem {
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

pub fn learning_feedback_item_from_record(record: &LearningFeedbackRecord) -> LearningFeedbackItem {
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

pub fn learning_rollback_item_from_record(record: &LearningRollbackRecord) -> LearningRollbackItem {
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

pub fn learning_feedback_action_response(
    id: Uuid,
    user_id: impl Into<String>,
    target_type: impl Into<String>,
    target_id: impl Into<String>,
    verdict: impl Into<String>,
    note: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
) -> LearningFeedbackActionResponse {
    LearningFeedbackActionResponse {
        feedback: LearningFeedbackItem {
            id,
            user_id: user_id.into(),
            target_type: target_type.into(),
            target_id: target_id.into(),
            verdict: verdict.into(),
            note,
            metadata: metadata.unwrap_or_else(|| serde_json::json!({})),
            created_at: created_at.to_rfc3339(),
        },
        status: "recorded",
    }
}

pub fn learning_rollback_action_response(
    record: &LearningRollbackRecord,
) -> LearningRollbackActionResponse {
    LearningRollbackActionResponse {
        rollback: learning_rollback_item_from_record(record),
        status: "recorded",
    }
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

pub fn learning_code_proposal_item_from_record(
    proposal: &LearningCodeProposal,
) -> LearningCodeProposalItem {
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

const LEDGER_EVENT_ID_KEY: &str = "ledger_learning_event_id";

pub fn learning_outcome_source_ref(contract: &OutcomeContract) -> LearningOutcomeSourceRef {
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

pub fn learning_outcome_ledger_event_id(contract: &OutcomeContract) -> Option<Uuid> {
    contract
        .metadata
        .get(LEDGER_EVENT_ID_KEY)
        .or_else(|| contract.evaluation_details.get(LEDGER_EVENT_ID_KEY))
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok())
}

pub fn learning_outcome_last_evaluator(contract: &OutcomeContract) -> Option<String> {
    contract
        .evaluation_details
        .get("last_evaluator")
        .or_else(|| contract.metadata.get("last_evaluator"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

pub fn learning_outcome_contract_item_from_record(
    contract: &OutcomeContract,
) -> LearningOutcomeContractItem {
    LearningOutcomeContractItem {
        id: contract.id,
        user_id: contract.user_id.clone(),
        actor_id: contract.actor_id.clone(),
        channel: contract.channel.clone(),
        thread_id: contract.thread_id.clone(),
        source_kind: contract.source_kind.clone(),
        source_id: contract.source_id.clone(),
        source_ref: learning_outcome_source_ref(contract),
        contract_type: contract.contract_type.clone(),
        status: contract.status.clone(),
        summary: contract.summary.clone(),
        due_at: contract.due_at.to_rfc3339(),
        expires_at: contract.expires_at.to_rfc3339(),
        final_verdict: contract.final_verdict.clone(),
        final_score: contract.final_score,
        ledger_learning_event_id: learning_outcome_ledger_event_id(contract),
        last_evaluator: learning_outcome_last_evaluator(contract),
        evaluation_details: contract.evaluation_details.clone(),
        metadata: contract.metadata.clone(),
        dedupe_key: contract.dedupe_key.clone(),
        claimed_at: contract.claimed_at.map(|dt| dt.to_rfc3339()),
        evaluated_at: contract.evaluated_at.map(|dt| dt.to_rfc3339()),
        created_at: contract.created_at.to_rfc3339(),
        updated_at: contract.updated_at.to_rfc3339(),
    }
}

pub fn learning_outcome_observation_item_from_record(
    observation: &OutcomeObservation,
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

pub fn learning_database_unavailable() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    )
}

pub fn learning_actor_filter(query_actor_id: Option<&str>, request_actor_id: &str) -> String {
    requested_identity_override(query_actor_id).unwrap_or_else(|| request_actor_id.to_string())
}

pub fn parse_learning_proposal_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid proposal ID (expected UUID)".to_string(),
        )
    })
}

pub fn parse_learning_outcome_contract_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid outcome contract ID".to_string(),
        )
    })
}

pub fn learning_outcome_evaluate_now_response(
    processed: usize,
) -> LearningOutcomeEvaluateNowResponse {
    LearningOutcomeEvaluateNowResponse {
        status: "processed",
        processed,
    }
}

pub fn summarize_learning_provider_health(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_counts_apply_limit_per_bucket() {
        assert_eq!(
            serde_json::to_value(learning_recent_counts(LearningRecentCountsInput {
                events: 5,
                evaluations: 4,
                candidates: 3,
                artifact_versions: 2,
                feedback: 1,
                rollbacks: 6,
                code_proposals: 7,
                limit: 3,
            }))
            .unwrap(),
            serde_json::json!({
                "events": 3,
                "evaluations": 3,
                "candidates": 3,
                "artifact_versions": 2,
                "feedback": 1,
                "rollbacks": 3,
                "code_proposals": 3,
            })
        );
    }

    #[test]
    fn outcome_contract_not_found_message_preserves_api_text() {
        assert_eq!(
            outcome_contract_not_found_message("contract-1"),
            "Outcome contract contract-1 not found"
        );
        assert_eq!(
            outcome_review_verdict_required_message(),
            "verdict is required for confirm"
        );
        assert_eq!(
            unsupported_outcome_review_verdict_message("mixed"),
            "unsupported verdict: mixed"
        );
        assert_eq!(
            unsupported_outcome_review_decision_message("skip"),
            "unsupported outcome review decision: skip"
        );
    }

    #[test]
    fn provider_health_item_preserves_shape() {
        let item = learning_provider_health_item(LearningProviderHealthItemInput {
            provider: "openai".to_string(),
            active: true,
            enabled: true,
            healthy: true,
            readiness: "ready".to_string(),
            latency_ms: Some(25),
            error: None,
            capabilities: vec!["chat".to_string()],
            metadata: serde_json::json!({"region": "us"}),
        });

        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::json!({
                "provider": "openai",
                "active": true,
                "enabled": true,
                "healthy": true,
                "readiness": "ready",
                "latency_ms": 25,
                "capabilities": ["chat"],
                "metadata": {"region": "us"},
            })
        );
    }

    #[test]
    fn provider_health_summary_counts_enabled_and_healthy() {
        let items = vec![
            LearningProviderHealthItem {
                provider: "openai".to_string(),
                active: true,
                enabled: true,
                healthy: true,
                readiness: "ready".to_string(),
                capabilities: vec!["chat".to_string()],
                latency_ms: Some(25),
                error: None,
                metadata: serde_json::json!({}),
            },
            LearningProviderHealthItem {
                provider: "local".to_string(),
                active: false,
                enabled: true,
                healthy: false,
                readiness: "error".to_string(),
                capabilities: Vec::new(),
                latency_ms: None,
                error: Some("offline".to_string()),
                metadata: serde_json::json!({}),
            },
        ];

        assert_eq!(
            summarize_learning_provider_health(&items),
            LearningProviderHealthSummary {
                total: 2,
                enabled: 2,
                healthy: 1,
                unhealthy: 1,
            }
        );
    }

    #[test]
    fn actor_filter_prefers_trimmed_query_actor() {
        assert_eq!(
            learning_actor_filter(Some(" actor-from-query "), "request-actor"),
            "actor-from-query"
        );
    }

    #[test]
    fn actor_filter_falls_back_to_request_actor() {
        assert_eq!(
            learning_actor_filter(Some(" "), "request-actor"),
            "request-actor"
        );
        assert_eq!(
            learning_actor_filter(None, "request-actor"),
            "request-actor"
        );
    }

    #[test]
    fn path_id_parsers_return_uuid_or_endpoint_message() {
        let id = Uuid::new_v4();
        assert_eq!(parse_learning_proposal_id(&id.to_string()), Ok(id));
        assert_eq!(
            parse_learning_proposal_id("not-a-uuid").expect_err("invalid proposal ID"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid proposal ID (expected UUID)".to_string()
            )
        );
        assert_eq!(
            parse_learning_outcome_contract_id("not-a-uuid")
                .expect_err("invalid outcome contract ID"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid outcome contract ID".to_string()
            )
        );
    }

    #[test]
    fn evaluate_now_response_uses_processed_status() {
        let response = learning_outcome_evaluate_now_response(3);

        assert_eq!(response.status, "processed");
        assert_eq!(response.processed, 3);
    }

    #[test]
    fn feedback_and_rollback_records_project_to_web_items() {
        let now = chrono::Utc::now();
        let feedback = learning_feedback_item_from_record(&LearningFeedbackRecord {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            target_type: "candidate".to_string(),
            target_id: "candidate-1".to_string(),
            verdict: "positive".to_string(),
            note: Some("useful".to_string()),
            metadata: serde_json::json!({"source": "test"}),
            created_at: now,
        });
        assert_eq!(feedback.target_type, "candidate");
        assert_eq!(feedback.created_at, now.to_rfc3339());
        assert_eq!(feedback.metadata["source"], "test");

        let version_id = Uuid::new_v4();
        let rollback = learning_rollback_item_from_record(&LearningRollbackRecord {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: "demo".to_string(),
            artifact_version_id: Some(version_id),
            reason: "bad output".to_string(),
            metadata: serde_json::json!({"source": "test"}),
            created_at: now,
        });
        assert_eq!(rollback.artifact_version_id, Some(version_id));
        assert_eq!(rollback.reason, "bad output");
    }

    #[test]
    fn learning_action_response_builders_preserve_recorded_status() {
        let now = chrono::Utc::now();
        let feedback = learning_feedback_action_response(
            Uuid::new_v4(),
            "user-1",
            "candidate",
            "candidate-1",
            "positive",
            Some("useful".to_string()),
            None,
            now,
        );
        assert_eq!(feedback.status, "recorded");
        assert_eq!(feedback.feedback.created_at, now.to_rfc3339());
        assert_eq!(feedback.feedback.metadata, serde_json::json!({}));

        let rollback = learning_rollback_action_response(&LearningRollbackRecord {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: "demo".to_string(),
            artifact_version_id: None,
            reason: "bad output".to_string(),
            metadata: serde_json::json!({}),
            created_at: now,
        });
        assert_eq!(rollback.status, "recorded");
        assert_eq!(rollback.rollback.created_at, now.to_rfc3339());
    }

    #[test]
    fn learning_event_record_projection_uses_payload_defaults() {
        let now = chrono::Utc::now();
        let event = learning_event_item_from_record(&LearningEvent {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            channel: Some("web".to_string()),
            thread_id: Some("thread-1".to_string()),
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: "reflection".to_string(),
            source: "agent".to_string(),
            payload: serde_json::json!({
                "class": "prompt",
                "risk_tier": "low",
                "summary": "learned something",
                "target": "SOUL.md",
                "confidence": 0.9,
            }),
            metadata: Some(serde_json::json!({"trace": "x"})),
            created_at: now,
        });

        assert_eq!(event.class, "prompt");
        assert_eq!(event.risk_tier, "low");
        assert_eq!(event.summary, "learned something");
        assert_eq!(event.target.as_deref(), Some("SOUL.md"));
        assert_eq!(event.confidence, Some(0.9));
        assert_eq!(event.created_at, now.to_rfc3339());
    }

    #[test]
    fn outcome_contract_projection_extracts_source_and_evaluator_metadata() {
        let now = chrono::Utc::now();
        let ledger_id = Uuid::new_v4();
        let contract = OutcomeContract {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            channel: Some("web".to_string()),
            thread_id: Some("thread-1".to_string()),
            source_kind: "routine_run".to_string(),
            source_id: "run-1".to_string(),
            contract_type: "routine_usefulness".to_string(),
            status: "evaluated".to_string(),
            summary: Some("routine summary".to_string()),
            due_at: now,
            expires_at: now,
            final_verdict: Some("positive".to_string()),
            final_score: Some(1.0),
            evaluation_details: serde_json::json!({
                "ledger_learning_event_id": ledger_id.to_string(),
                "last_evaluator": "deterministic",
            }),
            metadata: serde_json::json!({
                "routine_id": "routine-1",
                "artifact_name": "artifact",
                "artifact_type": "skill",
            }),
            dedupe_key: "routine:run-1".to_string(),
            claimed_at: None,
            evaluated_at: Some(now),
            created_at: now,
            updated_at: now,
        };

        let item = learning_outcome_contract_item_from_record(&contract);
        assert_eq!(item.source_ref.routine_id.as_deref(), Some("routine-1"));
        assert_eq!(item.source_ref.artifact_name.as_deref(), Some("artifact"));
        assert_eq!(item.ledger_learning_event_id, Some(ledger_id));
        assert_eq!(item.last_evaluator.as_deref(), Some("deterministic"));
    }
}

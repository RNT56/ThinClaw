//! Root-independent outcome evaluation policy helpers.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use serde_json::json;
use thinclaw_history::{
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, OutcomeContract, OutcomeEvaluatorHealth, OutcomeObservation,
};
use thinclaw_llm_core::{ChatMessage, CompletionRequest};
use thinclaw_workspace::paths;
use uuid::Uuid;

use crate::learning_policy::{
    append_markdown_section, ensure_prompt_document_root, ensure_prompt_trailing_newline,
    remove_markdown_section, upsert_markdown_section,
};
use crate::routine::{Routine, RoutineAction, RoutineRun, RunStatus};

pub const CONTRACT_TURN: &str = "turn_usefulness";
pub const CONTRACT_TOOL: &str = "tool_durability";
pub const CONTRACT_ROUTINE: &str = "routine_usefulness";

pub const STATUS_OPEN: &str = "open";
pub const STATUS_EVALUATING: &str = "evaluating";
pub const STATUS_EVALUATED: &str = "evaluated";

pub const EVALUATOR_OUTCOME: &str = "outcome_evaluator_v1";
pub const EVALUATOR_MANUAL_REVIEW: &str = "outcome_manual_review_v1";

pub const DEFAULT_EVALUATION_INTERVAL_SECS: u64 = 600;

pub const VERDICT_POSITIVE: &str = "positive";
pub const VERDICT_NEUTRAL: &str = "neutral";
pub const VERDICT_NEGATIVE: &str = "negative";

pub const SOURCE_LEARNING_EVENT: &str = "learning_event";
pub const SOURCE_ARTIFACT_VERSION: &str = "artifact_version";
pub const SOURCE_CODE_PROPOSAL: &str = "learning_code_proposal";
pub const SOURCE_ROUTINE_RUN: &str = "routine_run";

pub const LEDGER_EVENT_ID_KEY: &str = "ledger_learning_event_id";
pub const OUTCOME_CANDIDATE_ROUTE_KEY: &str = "outcome_candidate_route";

const MIN_EVALUATION_INTERVAL_SECS: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeRuntimeSettings {
    pub learning_enabled: bool,
    pub outcomes_enabled: bool,
    pub evaluation_interval_secs: u64,
    pub max_due_per_tick: u32,
    pub default_ttl_hours: u32,
    pub llm_assist_enabled: bool,
    pub heartbeat_summary_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomePendingUserSettings {
    pub user_id: String,
    pub settings: OutcomeRuntimeSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeSchedulerPlan {
    pub user_ids: Vec<String>,
    pub sleep_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeContractEvaluationPlan {
    pub status: String,
    pub final_verdict: String,
    pub final_score: f64,
    pub evaluation_details: serde_json::Value,
    pub evaluator: String,
    pub evaluated_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeContractRequeuePlan {
    pub status: String,
    pub claimed_at: Option<DateTime<Utc>>,
    pub evaluation_details: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeCandidateSupplementKind {
    None,
    Prompt,
    Code,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeCandidateSeed {
    pub candidate_type: String,
    pub risk_tier: String,
    pub confidence: f64,
    pub target_type: String,
    pub target_name: Option<String>,
    pub summary: String,
    pub pattern_key: String,
    pub pattern_count: usize,
    pub dedupe_key: String,
    pub supplement_kind: OutcomeCandidateSupplementKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeCandidateClass {
    Memory,
    Skill,
    Prompt,
    Routine,
    Code,
    Unknown,
}

impl OutcomeCandidateClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Skill => "skill",
            Self::Prompt => "prompt",
            Self::Routine => "routine",
            Self::Code => "code",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeRiskTier {
    Low,
    Medium,
    Critical,
}

impl OutcomeRiskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutcomeScore {
    pub verdict: String,
    pub score: f64,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeCandidateRouteRecord {
    pub status: String,
    pub evaluator: String,
    pub routed_at: DateTime<Utc>,
    pub terminal: bool,
    pub requires_operator_review: bool,
    pub auto_applied: bool,
    pub code_proposal_id: Option<Uuid>,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

pub fn outcomes_enabled(settings: &OutcomeRuntimeSettings) -> bool {
    settings.learning_enabled && settings.outcomes_enabled
}

pub fn scheduler_plan_for_pending_users(
    pending_users: impl IntoIterator<Item = OutcomePendingUserSettings>,
) -> OutcomeSchedulerPlan {
    let mut user_ids = Vec::new();
    let mut min_interval = DEFAULT_EVALUATION_INTERVAL_SECS;
    for pending in pending_users {
        if !outcomes_enabled(&pending.settings) {
            continue;
        }
        min_interval = min_interval.min(
            pending
                .settings
                .evaluation_interval_secs
                .max(MIN_EVALUATION_INTERVAL_SECS),
        );
        user_ids.push(pending.user_id);
    }
    OutcomeSchedulerPlan {
        user_ids,
        sleep_interval_secs: min_interval.max(MIN_EVALUATION_INTERVAL_SECS),
    }
}

pub fn max_due_per_tick(settings: &OutcomeRuntimeSettings) -> i64 {
    i64::from(settings.max_due_per_tick.max(1))
}

pub fn default_ttl_hours(settings: &OutcomeRuntimeSettings) -> u64 {
    u64::from(settings.default_ttl_hours)
}

pub fn should_use_llm_assisted_score(
    settings: &OutcomeRuntimeSettings,
    observations: &[OutcomeObservation],
) -> bool {
    settings.llm_assist_enabled && has_mixed_observations(observations)
}

pub fn heartbeat_summary_enabled(settings: &OutcomeRuntimeSettings) -> bool {
    outcomes_enabled(settings) && settings.heartbeat_summary_enabled
}

pub fn evaluator_health_status(
    health: &OutcomeEvaluatorHealth,
    evaluation_interval_secs: u64,
    now: DateTime<Utc>,
) -> bool {
    let stale_after_secs = (evaluation_interval_secs.max(MIN_EVALUATION_INTERVAL_SECS) * 2) as i64;
    let stale_before = now - chrono::Duration::seconds(stale_after_secs);
    let due_is_stale = health
        .oldest_due_at
        .is_some_and(|due_at| due_at <= stale_before);
    let evaluating_is_stale = health
        .oldest_evaluating_claimed_at
        .is_some_and(|claimed_at| claimed_at <= stale_before);
    !(due_is_stale || evaluating_is_stale)
}

pub fn evaluated_contract_plan(
    score: &OutcomeScore,
    evaluator: &str,
    now: DateTime<Utc>,
) -> OutcomeContractEvaluationPlan {
    OutcomeContractEvaluationPlan {
        status: STATUS_EVALUATED.to_string(),
        final_verdict: score.verdict.clone(),
        final_score: score.score,
        evaluation_details: score.details.clone(),
        evaluator: evaluator.to_string(),
        evaluated_at: now,
        updated_at: now,
    }
}

pub fn apply_evaluated_contract_plan(
    contract: &mut OutcomeContract,
    plan: &OutcomeContractEvaluationPlan,
) {
    contract.status = plan.status.clone();
    contract.final_verdict = Some(plan.final_verdict.clone());
    contract.final_score = Some(plan.final_score);
    contract.evaluation_details = plan.evaluation_details.clone();
    annotate_contract_with_last_evaluator(contract, &plan.evaluator);
    contract.evaluated_at = Some(plan.evaluated_at);
    contract.updated_at = plan.updated_at;
}

pub fn failed_contract_requeue_plan(
    contract: &OutcomeContract,
    reason: &str,
    now: DateTime<Utc>,
) -> Option<OutcomeContractRequeuePlan> {
    if contract.status != STATUS_EVALUATING || contract.evaluated_at.is_some() {
        return None;
    }
    let mut evaluation_details = contract.evaluation_details.clone();
    upsert_json_string(&mut evaluation_details, "last_error", reason.to_string());
    Some(OutcomeContractRequeuePlan {
        status: STATUS_OPEN.to_string(),
        claimed_at: None,
        evaluation_details,
        updated_at: now,
    })
}

pub fn apply_failed_contract_requeue_plan(
    contract: &mut OutcomeContract,
    plan: &OutcomeContractRequeuePlan,
) {
    contract.status = plan.status.clone();
    contract.claimed_at = plan.claimed_at;
    contract.evaluation_details = plan.evaluation_details.clone();
    contract.updated_at = plan.updated_at;
}

pub fn deterministic_score(
    contract: &OutcomeContract,
    observations: &[OutcomeObservation],
) -> OutcomeScore {
    let durability_survived = contract.contract_type == CONTRACT_TOOL
        && observations
            .iter()
            .all(|obs| obs.polarity.as_str() != VERDICT_NEGATIVE);
    if observations.is_empty() {
        if durability_survived {
            return OutcomeScore {
                verdict: VERDICT_POSITIVE.to_string(),
                score: 0.5,
                details: json!({
                    "strategy": "deterministic",
                    "reason": "durability_survived_until_due",
                }),
            };
        }
        return OutcomeScore {
            verdict: VERDICT_NEUTRAL.to_string(),
            score: 0.0,
            details: json!({
                "strategy": "deterministic",
                "reason": "no_observations",
            }),
        };
    }

    let total = observations
        .iter()
        .fold(0.0, |acc, obs| match obs.polarity.as_str() {
            VERDICT_NEGATIVE => acc - obs.weight,
            VERDICT_POSITIVE => acc + obs.weight,
            _ => acc,
        })
        + if durability_survived { 0.5 } else { 0.0 };
    let has_strong_negative = observations.iter().any(|obs| {
        matches!(
            obs.observation_kind.as_str(),
            "rollback"
                | "proposal_rejection"
                | "routine_disabled"
                | "routine_deleted"
                | "repeated_request"
                | "explicit_correction"
        )
    });

    let (verdict, score) = if has_strong_negative || total <= -0.75 {
        (VERDICT_NEGATIVE, total.max(-1.0))
    } else if total >= 0.5 {
        (VERDICT_POSITIVE, total.min(1.0))
    } else {
        (VERDICT_NEUTRAL, total.clamp(-0.49, 0.49))
    };

    OutcomeScore {
        verdict: verdict.to_string(),
        score,
        details: json!({
            "strategy": "deterministic",
            "contract_type": contract.contract_type,
            "total_weight": total,
            "strong_negative": has_strong_negative,
            "durability_survived_until_due": durability_survived,
            "observations": observations,
        }),
    }
}

pub fn has_mixed_observations(observations: &[OutcomeObservation]) -> bool {
    let has_positive = observations
        .iter()
        .any(|obs| obs.polarity == VERDICT_POSITIVE);
    let has_negative = observations
        .iter()
        .any(|obs| obs.polarity == VERDICT_NEGATIVE);
    has_positive && has_negative
}

pub fn llm_assisted_score_request(
    contract: &OutcomeContract,
    observations: &[OutcomeObservation],
) -> CompletionRequest {
    let observation_text = observations
        .iter()
        .map(|obs| {
            format!(
                "- kind={} polarity={} weight={} summary={}",
                obs.observation_kind,
                obs.polarity,
                obs.weight,
                obs.summary.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    CompletionRequest::new(vec![
        ChatMessage::system(
            "You score outcome contracts for an autonomous agent. Return JSON only with keys verdict, score, rationale.",
        ),
        ChatMessage::user(format!(
            "Contract type: {}\nSummary: {}\nSource kind: {}\nObservations:\n{}\n\nReturn verdict as positive, neutral, or negative. Return score in [-1.0, 1.0].",
            contract.contract_type,
            contract.summary.as_deref().unwrap_or(""),
            contract.source_kind,
            observation_text
        )),
    ])
    .with_temperature(0.1)
    .with_max_tokens(250)
}

pub fn parse_llm_assisted_score(
    content: &str,
    observations: &[OutcomeObservation],
) -> Result<OutcomeScore, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(content.trim()).map_err(|err| err.to_string())?;
    let verdict = parsed
        .get("verdict")
        .and_then(|value| value.as_str())
        .unwrap_or(VERDICT_NEUTRAL)
        .to_ascii_lowercase();
    let score = parsed
        .get("score")
        .and_then(|value| value.as_f64())
        .unwrap_or_default()
        .clamp(-1.0, 1.0);
    Ok(OutcomeScore {
        verdict,
        score,
        details: json!({
            "strategy": "llm_assisted",
            "llm_result": parsed,
            "observations": observations,
        }),
    })
}

pub fn latest_turn_observation_target_id(
    contracts: &[OutcomeContract],
    observed_at: DateTime<Utc>,
) -> Option<Uuid> {
    contracts
        .iter()
        .filter(|entry| entry.created_at <= observed_at)
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|entry| entry.id)
}

pub fn apply_user_turn_follow_up(
    contract: &mut OutcomeContract,
    event_id: Uuid,
    now: DateTime<Utc>,
) -> u64 {
    let follow_up_turns = contract
        .metadata
        .get("follow_up_turns")
        .and_then(|value| value.as_u64())
        .unwrap_or_default()
        + 1;
    if let Some(meta) = contract.metadata.as_object_mut() {
        meta.insert("follow_up_turns".to_string(), json!(follow_up_turns));
        meta.insert(
            "last_follow_up_event_id".to_string(),
            json!(event_id.to_string()),
        );
    }
    if follow_up_turns >= 2 {
        contract.due_at = now;
    }
    contract.updated_at = now;
    follow_up_turns
}

pub fn user_turn_observation(
    correction_count: u64,
    content: &str,
) -> Option<(String, String, f64, Option<String>)> {
    if correction_count > 0 {
        return Some((
            "explicit_correction".to_string(),
            VERDICT_NEGATIVE.to_string(),
            1.0,
            Some("User explicitly corrected the assistant".to_string()),
        ));
    }
    if detect_repeated_request_signal(content) {
        return Some((
            "repeated_request".to_string(),
            VERDICT_NEGATIVE.to_string(),
            0.9,
            Some("User repeated or re-opened the request".to_string()),
        ));
    }
    if detect_thanks_signal(content) {
        return Some((
            "explicit_approval".to_string(),
            VERDICT_POSITIVE.to_string(),
            0.6,
            Some("User explicitly approved the result".to_string()),
        ));
    }
    if !content.trim().is_empty() {
        return Some((
            "next_step_continuation".to_string(),
            VERDICT_POSITIVE.to_string(),
            0.2,
            Some("User moved to the next step without correcting the assistant".to_string()),
        ));
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn build_observation(
    contract_id: Uuid,
    observation_kind: &str,
    polarity: &str,
    weight: f64,
    summary: Option<&str>,
    evidence: serde_json::Value,
    fingerprint: &str,
    observed_at: DateTime<Utc>,
) -> OutcomeObservation {
    OutcomeObservation {
        id: Uuid::new_v4(),
        contract_id,
        observation_kind: observation_kind.to_string(),
        polarity: polarity.to_string(),
        weight,
        summary: summary.map(str::to_string),
        evidence,
        fingerprint: fingerprint.to_string(),
        observed_at,
        created_at: Utc::now(),
    }
}

pub fn synthetic_learning_event(
    contract: &OutcomeContract,
    score: &OutcomeScore,
    observations: &[OutcomeObservation],
) -> LearningEvent {
    let class = candidate_class_for_contract(contract);
    let risk = candidate_risk_for_class(class);
    let summary = format!(
        "Outcome evaluation for {} -> {}",
        contract.contract_type, score.verdict
    );
    let mut payload = json!({
        "contract_id": contract.id,
        "contract_type": contract.contract_type,
        "source_kind": contract.source_kind,
        "source_id": contract.source_id,
        "final_verdict": score.verdict,
        "final_score": score.score,
        "observations": observations,
        "summary": contract.summary,
    });
    if contract.contract_type == CONTRACT_TURN {
        copy_turn_trajectory_metadata(contract, &mut payload);
    }
    build_learning_event_record(
        contract,
        format!("outcome::{}", contract.source_kind),
        class,
        risk,
        summary,
        payload,
    )
}

pub fn build_learning_evaluation_record(
    id: Uuid,
    learning_event_id: Uuid,
    contract: &OutcomeContract,
    score: &OutcomeScore,
    observations: &[OutcomeObservation],
    evaluator: &str,
    created_at: DateTime<Utc>,
) -> LearningEvaluation {
    LearningEvaluation {
        id,
        learning_event_id,
        user_id: contract.user_id.clone(),
        evaluator: evaluator.to_string(),
        status: contract
            .final_verdict
            .clone()
            .unwrap_or_else(|| VERDICT_NEUTRAL.to_string()),
        score: Some(score.score),
        details: json!({
            "contract_id": contract.id,
            "contract_type": contract.contract_type,
            "source_kind": contract.source_kind,
            "source_id": contract.source_id,
            "final_verdict": score.verdict,
            "observations": observations,
            "strategy": score.details.get("strategy").cloned().unwrap_or_else(|| json!("deterministic")),
        }),
        created_at,
    }
}

pub fn build_manual_review_evaluation_record(
    id: Uuid,
    learning_event_id: Uuid,
    contract: &OutcomeContract,
    decision: &str,
    observations: &[OutcomeObservation],
    created_at: DateTime<Utc>,
) -> LearningEvaluation {
    LearningEvaluation {
        id,
        learning_event_id,
        user_id: contract.user_id.clone(),
        evaluator: EVALUATOR_MANUAL_REVIEW.to_string(),
        status: manual_review_status(contract, decision),
        score: Some(manual_review_score(contract, decision)),
        details: json!({
            "contract_id": contract.id,
            "contract_type": contract.contract_type,
            "source_kind": contract.source_kind,
            "source_id": contract.source_id,
            "review_decision": decision,
            "manual_verdict": contract.final_verdict,
            "contract_status": contract.status,
            "final_score": contract.final_score,
            "ledger_learning_event_id": learning_event_id,
            "observations": observations,
            "strategy": "manual_review",
        }),
        created_at,
    }
}

pub fn manual_review_learning_event(
    contract: &OutcomeContract,
    decision: &str,
    observations: &[OutcomeObservation],
) -> LearningEvent {
    let class = candidate_class_for_contract(contract);
    let risk = candidate_risk_for_class(class);
    let summary = match decision {
        "confirm" => format!(
            "Manual outcome review for {} -> {}",
            contract.contract_type,
            contract.final_verdict.as_deref().unwrap_or(VERDICT_NEUTRAL)
        ),
        "dismiss" => format!(
            "Manual outcome review for {} dismissed",
            contract.contract_type
        ),
        "requeue" => format!(
            "Manual outcome review for {} requeued",
            contract.contract_type
        ),
        _ => format!("Manual outcome review for {}", contract.contract_type),
    };
    let mut payload = json!({
        "contract_id": contract.id,
        "contract_type": contract.contract_type,
        "source_kind": contract.source_kind,
        "source_id": contract.source_id,
        "review_decision": decision,
        "manual_verdict": contract.final_verdict,
        "final_score": contract.final_score,
        "observations": observations,
        "summary": contract.summary,
    });
    if contract.contract_type == CONTRACT_TURN {
        copy_turn_trajectory_metadata(contract, &mut payload);
    }
    build_learning_event_record(
        contract,
        format!("outcome_review::{}", contract.source_kind),
        class,
        risk,
        summary,
        payload,
    )
}

fn build_learning_event_record(
    contract: &OutcomeContract,
    source: String,
    class: OutcomeCandidateClass,
    risk: OutcomeRiskTier,
    summary: String,
    mut payload: serde_json::Value,
) -> LearningEvent {
    if !payload.is_object() {
        payload = json!({});
    }
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("class".to_string(), json!(class.as_str()));
        obj.insert("risk_tier".to_string(), json!(risk.as_str()));
        obj.insert("summary".to_string(), json!(summary.clone()));
    }

    LearningEvent {
        id: Uuid::new_v4(),
        user_id: contract.user_id.clone(),
        actor_id: contract.actor_id.clone(),
        channel: contract.channel.clone(),
        thread_id: contract.thread_id.clone(),
        conversation_id: None,
        message_id: None,
        job_id: None,
        event_type: class.as_str().to_string(),
        source,
        payload,
        metadata: Some(json!({
            "risk_tier": risk.as_str(),
            "summary": summary,
            "target": null,
            "confidence": null,
        })),
        created_at: Utc::now(),
    }
}

fn copy_turn_trajectory_metadata(contract: &OutcomeContract, metadata: &mut serde_json::Value) {
    let Some(target) = metadata.as_object_mut() else {
        return;
    };
    for key in [
        "trajectory_target_id",
        "turn_number",
        "session_id",
        "thread_id",
    ] {
        if let Some(value) = contract.metadata.get(key) {
            target.insert(key.to_string(), value.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn user_turn_observation_record(
    contract_id: Uuid,
    event_id: Uuid,
    content_preview: &str,
    kind: &str,
    polarity: &str,
    weight: f64,
    summary: Option<&str>,
    observed_at: DateTime<Utc>,
) -> OutcomeObservation {
    build_observation(
        contract_id,
        kind,
        polarity,
        weight,
        summary,
        json!({
            "event_id": event_id,
            "content_preview": content_preview,
        }),
        &stable_key(&[&contract_id.to_string(), &event_id.to_string(), kind]),
        observed_at,
    )
}

pub fn detect_repeated_request_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    [
        "still not",
        "not right",
        "again",
        "you missed",
        "can you try again",
        "redo",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub fn detect_thanks_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    ["thanks", "thank you", "looks good", "perfect", "great"]
        .iter()
        .any(|needle| normalized.contains(needle))
}

pub fn candidate_class_for_contract(contract: &OutcomeContract) -> OutcomeCandidateClass {
    match contract.contract_type.as_str() {
        CONTRACT_ROUTINE => OutcomeCandidateClass::Routine,
        CONTRACT_TURN => OutcomeCandidateClass::Prompt,
        CONTRACT_TOOL => match contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
        {
            Some("skill") => OutcomeCandidateClass::Skill,
            Some("prompt") if is_outcome_prompt_target_allowed(contract) => {
                OutcomeCandidateClass::Prompt
            }
            Some("prompt") => OutcomeCandidateClass::Unknown,
            Some("memory") => OutcomeCandidateClass::Memory,
            _ if contract.source_kind == SOURCE_CODE_PROPOSAL => OutcomeCandidateClass::Code,
            _ => OutcomeCandidateClass::Unknown,
        },
        _ => OutcomeCandidateClass::Unknown,
    }
}

pub fn candidate_risk_for_class(class: OutcomeCandidateClass) -> OutcomeRiskTier {
    match class {
        OutcomeCandidateClass::Memory | OutcomeCandidateClass::Skill => OutcomeRiskTier::Low,
        OutcomeCandidateClass::Prompt
        | OutcomeCandidateClass::Routine
        | OutcomeCandidateClass::Unknown => OutcomeRiskTier::Medium,
        OutcomeCandidateClass::Code => OutcomeRiskTier::Critical,
    }
}

pub fn outcome_candidate_seed(
    contract: &OutcomeContract,
    score: &OutcomeScore,
    repeated_negative_pattern_count: usize,
) -> Option<OutcomeCandidateSeed> {
    if score.verdict != VERDICT_NEGATIVE || score.score.abs() < 0.6 {
        return None;
    }
    let pattern_key = contract
        .metadata
        .get("pattern_key")
        .and_then(|value| value.as_str())?;
    if repeated_negative_pattern_count < 2 {
        return None;
    }

    let class = candidate_class_for_contract(contract);
    if class == OutcomeCandidateClass::Unknown {
        return None;
    }
    let supplement_kind = match class {
        OutcomeCandidateClass::Prompt => OutcomeCandidateSupplementKind::Prompt,
        OutcomeCandidateClass::Code => {
            code_candidate_payload(contract)?;
            OutcomeCandidateSupplementKind::Code
        }
        _ => OutcomeCandidateSupplementKind::None,
    };
    let target_name = candidate_target_name(contract);
    let dedupe_key = stable_key(&[
        class.as_str(),
        &target_name.clone().unwrap_or_default(),
        pattern_key,
    ]);

    Some(OutcomeCandidateSeed {
        candidate_type: class.as_str().to_string(),
        risk_tier: candidate_risk_for_class(class).as_str().to_string(),
        confidence: score.score.abs(),
        target_type: candidate_target_type(contract),
        target_name,
        summary: format!(
            "Repeated negative outcome pattern detected for {} ({})",
            contract.contract_type, pattern_key
        ),
        pattern_key: pattern_key.to_string(),
        pattern_count: repeated_negative_pattern_count,
        dedupe_key,
        supplement_kind,
    })
}

pub fn candidate_dedupe_exists(candidates: &[LearningCandidate], dedupe_key: &str) -> bool {
    candidates.iter().any(|candidate| {
        candidate
            .proposal
            .get("dedupe_key")
            .and_then(|value| value.as_str())
            == Some(dedupe_key)
    })
}

pub fn outcome_candidate_route_success_record(
    evaluator: &str,
    auto_applied: bool,
    code_proposal_id: Option<Uuid>,
    notes: &[String],
    routed_at: DateTime<Utc>,
) -> OutcomeCandidateRouteRecord {
    let status = outcome_candidate_success_route_status(auto_applied, code_proposal_id, notes);
    OutcomeCandidateRouteRecord {
        status: status.to_string(),
        evaluator: evaluator.to_string(),
        routed_at,
        terminal: true,
        requires_operator_review: matches!(
            status,
            "code_proposal" | "held_for_review" | "manual_review"
        ),
        auto_applied,
        code_proposal_id,
        notes: notes.iter().map(|note| bounded_route_text(note)).collect(),
        error: None,
    }
}

pub fn outcome_candidate_route_failure_record(
    evaluator: &str,
    error: &str,
    routed_at: DateTime<Utc>,
) -> OutcomeCandidateRouteRecord {
    OutcomeCandidateRouteRecord {
        status: "quarantined".to_string(),
        evaluator: evaluator.to_string(),
        routed_at,
        terminal: true,
        requires_operator_review: true,
        auto_applied: false,
        code_proposal_id: None,
        notes: vec![
            "outcome candidate routing failed; candidate quarantined for manual review".to_string(),
        ],
        error: Some(bounded_route_text(error)),
    }
}

pub fn annotate_candidate_proposal_with_route(
    proposal: &serde_json::Value,
    record: &OutcomeCandidateRouteRecord,
) -> serde_json::Value {
    let mut next = if proposal.is_object() {
        proposal.clone()
    } else {
        json!({ "original_proposal": proposal })
    };
    upsert_json_value(
        &mut next,
        OUTCOME_CANDIDATE_ROUTE_KEY,
        outcome_candidate_route_record_value(record),
    );
    next
}

pub fn annotate_contract_with_outcome_candidate_route(
    contract: &mut OutcomeContract,
    candidate_id: Uuid,
    record: &OutcomeCandidateRouteRecord,
) {
    let mut route = outcome_candidate_route_record_value(record);
    upsert_json_string(&mut route, "candidate_id", candidate_id.to_string());
    upsert_json_value(
        &mut contract.metadata,
        OUTCOME_CANDIDATE_ROUTE_KEY,
        route.clone(),
    );
    upsert_json_value(
        &mut contract.evaluation_details,
        OUTCOME_CANDIDATE_ROUTE_KEY,
        route,
    );
    contract.updated_at = record.routed_at;
}

fn outcome_candidate_success_route_status(
    auto_applied: bool,
    code_proposal_id: Option<Uuid>,
    notes: &[String],
) -> &'static str {
    if auto_applied {
        return "auto_applied";
    }
    if code_proposal_id.is_some() {
        return "code_proposal";
    }
    let note_contains = |needle: &str| {
        notes
            .iter()
            .any(|note| note.to_ascii_lowercase().contains(needle))
    };
    if note_contains("held for review") || note_contains("safe mode") {
        "held_for_review"
    } else if note_contains("manual review") {
        "manual_review"
    } else if note_contains("persisted only") {
        "persisted_only"
    } else {
        "routed"
    }
}

fn outcome_candidate_route_record_value(record: &OutcomeCandidateRouteRecord) -> serde_json::Value {
    json!({
        "status": record.status.clone(),
        "evaluator": record.evaluator.clone(),
        "routed_at": record.routed_at.to_rfc3339(),
        "terminal": record.terminal,
        "requires_operator_review": record.requires_operator_review,
        "auto_applied": record.auto_applied,
        "code_proposal_id": record.code_proposal_id.map(|id| id.to_string()),
        "notes": record.notes.clone(),
        "error": record.error.clone(),
    })
}

fn bounded_route_text(value: &str) -> String {
    const MAX_ROUTE_TEXT_CHARS: usize = 2_048;
    if value.chars().count() <= MAX_ROUTE_TEXT_CHARS {
        return value.to_string();
    }
    let mut text = value.chars().take(MAX_ROUTE_TEXT_CHARS).collect::<String>();
    text.push_str("...");
    text
}

#[derive(Debug, Clone)]
pub struct BuildOutcomeCandidateInput<'a> {
    pub id: Uuid,
    pub learning_event_id: Uuid,
    pub contract: &'a OutcomeContract,
    pub score: &'a OutcomeScore,
    pub observations: &'a [OutcomeObservation],
    pub seed: &'a OutcomeCandidateSeed,
    pub prompt_payload: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

pub fn build_outcome_candidate(input: BuildOutcomeCandidateInput<'_>) -> Option<LearningCandidate> {
    let target_name = input.seed.target_name.clone();
    let evidence = json!({
        "source": "outcome_backed_learning",
        "contract_id": input.contract.id,
        "contract_type": input.contract.contract_type.clone(),
        "pattern_key": input.seed.pattern_key.clone(),
        "pattern_count": input.seed.pattern_count,
        "final_verdict": input.score.verdict.clone(),
        "observations": input.observations,
        "target": target_name.clone(),
        "target_type": input.seed.target_type.clone(),
        "routine_patch": routine_candidate_patch(input.contract, input.observations),
    });

    let mut proposal = serde_json::Map::new();
    proposal.insert(
        "dedupe_key".to_string(),
        json!(input.seed.dedupe_key.clone()),
    );
    proposal.insert("source".to_string(), json!("outcome_backed_learning"));
    proposal.insert(
        "pattern_key".to_string(),
        json!(input.seed.pattern_key.clone()),
    );
    proposal.insert("pattern_count".to_string(), json!(input.seed.pattern_count));
    proposal.insert(
        "contract_type".to_string(),
        json!(input.contract.contract_type.clone()),
    );
    proposal.insert("verdict".to_string(), json!(input.score.verdict.clone()));
    proposal.insert("evidence".to_string(), evidence);
    proposal.insert(
        "routine_patch".to_string(),
        routine_candidate_patch(input.contract, input.observations),
    );

    match input.seed.supplement_kind {
        OutcomeCandidateSupplementKind::Prompt => {
            merge_json_object(&mut proposal, input.prompt_payload?);
        }
        OutcomeCandidateSupplementKind::Code => {
            merge_json_object(&mut proposal, code_candidate_payload(input.contract)?);
        }
        OutcomeCandidateSupplementKind::None => {}
    }

    Some(LearningCandidate {
        id: input.id,
        learning_event_id: Some(input.learning_event_id),
        user_id: input.contract.user_id.clone(),
        candidate_type: input.seed.candidate_type.clone(),
        risk_tier: input.seed.risk_tier.clone(),
        confidence: Some(input.seed.confidence),
        target_type: Some(input.seed.target_type.clone()),
        target_name,
        summary: Some(input.seed.summary.clone()),
        proposal: serde_json::Value::Object(proposal),
        created_at: input.created_at,
    })
}

pub fn candidate_target_type(contract: &OutcomeContract) -> String {
    match contract.contract_type.as_str() {
        CONTRACT_ROUTINE => "routine".to_string(),
        CONTRACT_TURN => "prompt".to_string(),
        CONTRACT_TOOL => contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .unwrap_or("artifact")
            .to_string(),
        _ => "unknown".to_string(),
    }
}

pub fn candidate_target_name(contract: &OutcomeContract) -> Option<String> {
    match contract.contract_type.as_str() {
        CONTRACT_ROUTINE => contract
            .metadata
            .get("routine_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        CONTRACT_TURN => Some(paths::USER.to_string()),
        CONTRACT_TOOL => contract
            .metadata
            .get("artifact_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        _ => None,
    }
}

pub fn code_candidate_payload(contract: &OutcomeContract) -> Option<serde_json::Value> {
    if contract.source_kind != SOURCE_CODE_PROPOSAL {
        return None;
    }
    let diff = contract
        .metadata
        .get("diff")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(json!({
        "title": contract
            .metadata
            .get("title")
            .and_then(|value| value.as_str())
            .or(contract.summary.as_deref())
            .unwrap_or("Outcome-backed learning code proposal"),
        "rationale": contract
            .metadata
            .get("rationale")
            .and_then(|value| value.as_str())
            .or(contract.summary.as_deref())
            .unwrap_or("Repeated negative durability outcomes indicate this change needs revision."),
        "target_files": contract
            .metadata
            .get("target_files")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "diff": diff,
        "validation_results": contract
            .metadata
            .get("validation_results")
            .cloned()
            .unwrap_or_else(|| json!({"status":"not_run"})),
        "rollback_note": contract.metadata.get("rollback_note").cloned().unwrap_or(serde_json::Value::Null),
        "confidence": contract.metadata.get("confidence").cloned().unwrap_or(serde_json::Value::Null),
    }))
}

pub fn routine_candidate_patch(
    contract: &OutcomeContract,
    observations: &[OutcomeObservation],
) -> serde_json::Value {
    if contract.contract_type != CONTRACT_ROUTINE {
        return serde_json::Value::Null;
    }
    let Some(routine_id) = contract
        .metadata
        .get("routine_id")
        .and_then(|value| value.as_str())
    else {
        return json!({
            "suppressed_reason": "routine_id_missing",
        });
    };
    let on_success_enabled = contract
        .metadata
        .get("notify")
        .and_then(|value| value.get("on_success"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !on_success_enabled {
        return json!({
            "suppressed_reason": "on_success_notifications_already_disabled",
        });
    }
    let run_status = contract
        .metadata
        .get("run_status")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if !run_status.eq_ignore_ascii_case("ok") {
        return json!({
            "suppressed_reason": "routine_notification_noise_only_applies_to_ok_runs",
        });
    }
    let noise_signal = observations.iter().any(|obs| {
        matches!(
            obs.observation_kind.as_str(),
            "routine_disabled" | "routine_paused" | "routine_muted" | "routine_deleted"
        ) && obs.polarity == VERDICT_NEGATIVE
    });
    if !noise_signal {
        return json!({
            "suppressed_reason": "negative_pattern_is_not_notification_noise",
        });
    }
    json!({
        "type": "notification_noise_reduction",
        "routine_id": routine_id,
        "changes": {
            "notify": {
                "on_success": false
            }
        }
    })
}

pub fn is_outcome_prompt_target_allowed(contract: &OutcomeContract) -> bool {
    contract
        .metadata
        .get("artifact_name")
        .and_then(|value| value.as_str())
        .is_some_and(is_prompt_candidate_target_name_allowed)
}

pub fn is_prompt_candidate_target_name_allowed(name: &str) -> bool {
    name.eq_ignore_ascii_case(paths::USER)
        || name
            .to_ascii_lowercase()
            .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()))
}

pub fn feedback_polarity(verdict: &str) -> (&'static str, f64) {
    match verdict.to_ascii_lowercase().as_str() {
        "helpful" | "approve" => (VERDICT_POSITIVE, 0.8),
        "harmful" | "revert" | "dont_learn" | "reject" => (VERDICT_NEGATIVE, 1.0),
        _ => (VERDICT_NEUTRAL, 0.0),
    }
}

pub fn feedback_target_source_kind(target_type: &str) -> Option<&'static str> {
    match target_type {
        "learning_event" => Some(SOURCE_LEARNING_EVENT),
        "artifact_version" => Some(SOURCE_ARTIFACT_VERSION),
        "code_proposal" => Some(SOURCE_CODE_PROPOSAL),
        _ => None,
    }
}

pub fn contract_accepts_observation(contract: &OutcomeContract) -> bool {
    contract.status == STATUS_OPEN || contract.status == STATUS_EVALUATING
}

pub fn should_due_contract_for_observation_polarity(polarity: &str) -> bool {
    polarity == VERDICT_NEGATIVE
}

pub fn artifact_version_outcome_eligible(status: &str) -> bool {
    status.eq_ignore_ascii_case("applied") || status.eq_ignore_ascii_case("promoted")
}

pub fn learning_event_turn_outcome_eligible(event: &LearningEvent) -> bool {
    event
        .payload
        .get("role")
        .and_then(|value| value.as_str())
        .is_some_and(|role| role.eq_ignore_ascii_case("assistant"))
}

pub fn is_user_visible_routine_run(routine: &Routine, run: &RoutineRun) -> bool {
    if matches!(run.status, RunStatus::Attention | RunStatus::Failed) {
        return true;
    }
    match &routine.action {
        RoutineAction::Heartbeat { .. } => run
            .result_summary
            .as_deref()
            .is_some_and(|summary| !summary.contains("HEARTBEAT_OK")),
        _ => run.result_summary.is_some() && routine.notify.on_success,
    }
}

pub fn build_turn_contract(event: &LearningEvent, default_ttl_hours: u64) -> OutcomeContract {
    let now = Utc::now();
    OutcomeContract {
        id: Uuid::new_v4(),
        user_id: event.user_id.clone(),
        actor_id: event.actor_id.clone(),
        channel: event.channel.clone(),
        thread_id: event.thread_id.clone(),
        source_kind: SOURCE_LEARNING_EVENT.to_string(),
        source_id: event.id.to_string(),
        contract_type: CONTRACT_TURN.to_string(),
        status: STATUS_OPEN.to_string(),
        summary: event
            .payload
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        due_at: event.created_at + chrono::Duration::hours(24),
        expires_at: event.created_at + chrono::Duration::hours(default_ttl_hours as i64),
        final_verdict: None,
        final_score: None,
        evaluation_details: json!({}),
        metadata: json!({
            "pattern_key": turn_pattern_key(event),
            "message_id": event.message_id.map(|value| value.to_string()),
            "conversation_id": event.conversation_id.map(|value| value.to_string()),
            "follow_up_turns": 0,
            "trajectory_target_id": event.payload.get("trajectory_target_id").cloned(),
            "turn_number": event.payload.get("turn_number").cloned(),
            "session_id": event.payload.get("session_id").cloned(),
        }),
        dedupe_key: stable_key(&[CONTRACT_TURN, SOURCE_LEARNING_EVENT, &event.id.to_string()]),
        claimed_at: None,
        evaluated_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn build_artifact_contract(
    version: &LearningArtifactVersion,
    default_ttl_hours: u64,
) -> OutcomeContract {
    let now = Utc::now();
    let actor_id = json_string(&version.provenance, "actor_id");
    let channel = json_string(&version.provenance, "channel");
    let thread_id = json_string(&version.provenance, "thread_id");
    let pattern_key = format!(
        "artifact:{}:{}",
        version.artifact_type, version.artifact_name
    );
    OutcomeContract {
        id: Uuid::new_v4(),
        user_id: version.user_id.clone(),
        actor_id,
        channel,
        thread_id,
        source_kind: SOURCE_ARTIFACT_VERSION.to_string(),
        source_id: version.id.to_string(),
        contract_type: CONTRACT_TOOL.to_string(),
        status: STATUS_OPEN.to_string(),
        summary: version.diff_summary.clone(),
        due_at: version.created_at + chrono::Duration::hours(24),
        expires_at: version.created_at + chrono::Duration::hours(default_ttl_hours as i64),
        final_verdict: None,
        final_score: None,
        evaluation_details: json!({}),
        metadata: json!({
            "pattern_key": pattern_key,
            "artifact_type": version.artifact_type,
            "artifact_name": version.artifact_name,
            "candidate_id": version.candidate_id.map(|value| value.to_string()),
            "provenance": version.provenance,
        }),
        dedupe_key: stable_key(&[
            CONTRACT_TOOL,
            SOURCE_ARTIFACT_VERSION,
            &version.id.to_string(),
        ]),
        claimed_at: None,
        evaluated_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn build_proposal_contract(
    proposal: &LearningCodeProposal,
    default_ttl_hours: u64,
) -> OutcomeContract {
    let now = Utc::now();
    let actor_id = proposal
        .metadata
        .get("actor_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let thread_id = proposal
        .metadata
        .get("thread_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    OutcomeContract {
        id: Uuid::new_v4(),
        user_id: proposal.user_id.clone(),
        actor_id,
        channel: None,
        thread_id,
        source_kind: SOURCE_CODE_PROPOSAL.to_string(),
        source_id: proposal.id.to_string(),
        contract_type: CONTRACT_TOOL.to_string(),
        status: STATUS_OPEN.to_string(),
        summary: Some(proposal.title.clone()),
        due_at: now + chrono::Duration::hours(24),
        expires_at: now + chrono::Duration::hours(default_ttl_hours as i64),
        final_verdict: None,
        final_score: None,
        evaluation_details: json!({}),
        metadata: json!({
            "artifact_type": "code",
            "artifact_name": proposal.title,
            "pattern_key": format!("code_proposal:{}", stable_key(&[&proposal.title, &proposal.target_files.join(",")])),
            "title": proposal.title,
            "rationale": proposal.rationale,
            "target_files": proposal.target_files,
            "diff": proposal.diff,
            "validation_results": proposal.validation_results,
            "rollback_note": proposal.rollback_note,
            "confidence": proposal.confidence,
        }),
        dedupe_key: stable_key(&[
            CONTRACT_TOOL,
            SOURCE_CODE_PROPOSAL,
            &proposal.id.to_string(),
        ]),
        claimed_at: None,
        evaluated_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn build_routine_contract(routine: &Routine, run: &RoutineRun) -> OutcomeContract {
    let now = Utc::now();
    OutcomeContract {
        id: Uuid::new_v4(),
        user_id: routine.user_id.clone(),
        actor_id: Some(routine.owner_actor_id().to_string()),
        channel: routine.notify.channel.clone(),
        thread_id: None,
        source_kind: SOURCE_ROUTINE_RUN.to_string(),
        source_id: run.id.to_string(),
        contract_type: CONTRACT_ROUTINE.to_string(),
        status: STATUS_OPEN.to_string(),
        summary: run.result_summary.clone(),
        due_at: now + chrono::Duration::days(7),
        expires_at: now + chrono::Duration::days(7),
        final_verdict: None,
        final_score: None,
        evaluation_details: json!({}),
        metadata: json!({
            "pattern_key": format!("routine:{}", routine.id),
            "routine_id": routine.id,
            "routine_name": routine.name,
            "run_status": run.status.to_string(),
            "notify": {
                "channel": routine.notify.channel,
                "user": routine.notify.user,
                "on_attention": routine.notify.on_attention,
                "on_failure": routine.notify.on_failure,
                "on_success": routine.notify.on_success,
            }
        }),
        dedupe_key: stable_key(&[CONTRACT_ROUTINE, SOURCE_ROUTINE_RUN, &run.id.to_string()]),
        claimed_at: None,
        evaluated_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn stable_key(parts: &[&str]) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub fn turn_pattern_key(event: &LearningEvent) -> String {
    format!(
        "turn:{}:{}",
        event.actor_id.as_deref().unwrap_or(event.user_id.as_str()),
        event.thread_id.as_deref().unwrap_or("no-thread")
    )
}

pub fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

pub fn annotate_contract_with_ledger_event_id(
    contract: &mut OutcomeContract,
    learning_event_id: Uuid,
) {
    upsert_json_string(
        &mut contract.metadata,
        LEDGER_EVENT_ID_KEY,
        learning_event_id.to_string(),
    );
    upsert_json_string(
        &mut contract.evaluation_details,
        LEDGER_EVENT_ID_KEY,
        learning_event_id.to_string(),
    );
}

pub fn annotate_contract_with_last_evaluator(contract: &mut OutcomeContract, evaluator: &str) {
    upsert_json_string(
        &mut contract.evaluation_details,
        "last_evaluator",
        evaluator.to_string(),
    );
    upsert_json_string(
        &mut contract.metadata,
        "last_evaluator",
        evaluator.to_string(),
    );
}

pub fn manual_review_status(contract: &OutcomeContract, decision: &str) -> String {
    match decision {
        "confirm" => contract
            .final_verdict
            .clone()
            .unwrap_or_else(|| VERDICT_NEUTRAL.to_string()),
        "dismiss" | "requeue" => "review".to_string(),
        _ => VERDICT_NEUTRAL.to_string(),
    }
}

pub fn manual_review_score(contract: &OutcomeContract, decision: &str) -> f64 {
    match decision {
        "confirm" => contract.final_score.unwrap_or_else(|| {
            verdict_score(contract.final_verdict.as_deref().unwrap_or(VERDICT_NEUTRAL))
        }),
        "dismiss" | "requeue" => 0.0,
        _ => 0.0,
    }
}

pub fn verdict_score(verdict: &str) -> f64 {
    match verdict {
        VERDICT_POSITIVE => 1.0,
        VERDICT_NEGATIVE => -1.0,
        _ => 0.0,
    }
}

pub fn upsert_json_string(target: &mut serde_json::Value, key: &str, value: String) {
    upsert_json_value(target, key, json!(value));
}

pub fn upsert_json_value(target: &mut serde_json::Value, key: &str, value: serde_json::Value) {
    if !target.is_object() {
        *target = json!({});
    }
    if let Some(map) = target.as_object_mut() {
        map.insert(key.to_string(), value);
    }
}

fn merge_json_object(
    target: &mut serde_json::Map<String, serde_json::Value>,
    patch: serde_json::Value,
) {
    let Some(patch_obj) = patch.as_object() else {
        return;
    };
    for (key, value) in patch_obj {
        target.insert(key.clone(), value.clone());
    }
}

pub fn prompt_guidance_section(
    contract: &OutcomeContract,
    observations: &[OutcomeObservation],
) -> String {
    let mut bullets = Vec::new();
    let has_kind = |kind: &str| observations.iter().any(|obs| obs.observation_kind == kind);

    if has_kind("explicit_correction") || has_kind("repeated_request") {
        bullets.push(
            "When the user asks for a concrete fix or implementation, finish the requested work before concluding."
                .to_string(),
        );
    }
    if has_kind("explicit_correction") {
        bullets.push(
            "Treat direct corrections as a signal to revise the answer immediately around the exact requested deliverable."
                .to_string(),
        );
    }
    if has_kind("repeated_request") {
        bullets.push(
            "If the user repeats a request or says it is still not right, treat the earlier response as incomplete and close the remaining gap explicitly."
                .to_string(),
        );
    }
    if has_kind("rollback") || has_kind("proposal_rejected") {
        bullets.push(
            "Only propose durable, reviewable changes that include the concrete content or diff needed to apply them."
                .to_string(),
        );
    }
    if has_kind("routine_disabled") || has_kind("routine_muted") || has_kind("routine_paused") {
        bullets.push(
            "Avoid proactive follow-ups unless there is clear user-visible value; low-signal notifications create noise."
                .to_string(),
        );
    }
    if bullets.is_empty() {
        bullets.push(
            "This user benefits from direct execution, clear verification, and concise close-out notes instead of partial analysis."
                .to_string(),
        );
    }
    if contract.contract_type == CONTRACT_TURN {
        bullets.push(
            "Before replying, verify that the response fully satisfies the latest user request and includes any promised verification."
                .to_string(),
        );
    }

    bullets.sort();
    bullets.dedup();
    bullets
        .into_iter()
        .take(4)
        .map(|bullet| format!("- {bullet}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn apply_prompt_patch_content(
    current: &str,
    patch: &serde_json::Value,
    target: &str,
) -> Result<String, String> {
    let operation = patch
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("replace");
    let base = ensure_prompt_document_root(current, target);
    let next = match operation {
        "replace" => patch
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "prompt patch missing content".to_string())?
            .to_string(),
        "upsert_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            upsert_markdown_section(&base, heading, section_content)
        }
        "append_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            append_markdown_section(&base, heading, section_content)
        }
        "remove_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            remove_markdown_section(&base, heading)?
        }
        other => return Err(format!("unsupported prompt patch operation '{}'", other)),
    };
    Ok(ensure_prompt_trailing_newline(&next))
}

pub fn ledger_learning_event_id(contract: &OutcomeContract) -> Option<Uuid> {
    contract
        .metadata
        .get(LEDGER_EVENT_ID_KEY)
        .or_else(|| contract.evaluation_details.get(LEDGER_EVENT_ID_KEY))
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok())
}

pub fn contract_last_evaluator(contract: &OutcomeContract) -> Option<String> {
    contract
        .evaluation_details
        .get("last_evaluator")
        .or_else(|| contract.metadata.get("last_evaluator"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests;

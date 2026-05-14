//! Root-independent outcome evaluation policy helpers.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use serde_json::json;
use thinclaw_history::{
    LearningArtifactVersion, LearningCodeProposal, LearningEvent, OutcomeContract,
    OutcomeEvaluatorHealth, OutcomeObservation,
};
use thinclaw_llm_core::{ChatMessage, CompletionRequest};
use thinclaw_workspace::paths;
use uuid::Uuid;

use crate::routine::{Routine, RoutineAction, RoutineRun, RunStatus};

pub const CONTRACT_TURN: &str = "turn_usefulness";
pub const CONTRACT_TOOL: &str = "tool_durability";
pub const CONTRACT_ROUTINE: &str = "routine_usefulness";

pub const STATUS_OPEN: &str = "open";

pub const VERDICT_POSITIVE: &str = "positive";
pub const VERDICT_NEUTRAL: &str = "neutral";
pub const VERDICT_NEGATIVE: &str = "negative";

pub const SOURCE_LEARNING_EVENT: &str = "learning_event";
pub const SOURCE_ARTIFACT_VERSION: &str = "artifact_version";
pub const SOURCE_CODE_PROPOSAL: &str = "learning_code_proposal";
pub const SOURCE_ROUTINE_RUN: &str = "routine_run";

pub const LEDGER_EVENT_ID_KEY: &str = "ledger_learning_event_id";

const MIN_EVALUATION_INTERVAL_SECS: u64 = 1;

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
    if !target.is_object() {
        *target = json!({});
    }
    if let Some(map) = target.as_object_mut() {
        map.insert(key.to_string(), json!(value));
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

fn ensure_prompt_document_root(current: &str, target: &str) -> String {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        return ensure_prompt_trailing_newline(trimmed);
    }
    if target.ends_with(paths::SOUL_LOCAL) {
        let mut sections = std::collections::BTreeMap::new();
        for section in thinclaw_soul::LOCAL_SECTIONS {
            sections.insert((*section).to_string(), String::new());
        }
        return thinclaw_soul::render_local_soul_overlay(&thinclaw_soul::LocalSoulOverlay {
            sections,
        });
    }
    if target.ends_with(paths::SOUL) {
        return thinclaw_soul::compose_seeded_soul("balanced").unwrap_or_else(|_| {
            "# SOUL.md - Who You Are\n\n- **Schema:** v2\n- **Seed Pack:** balanced\n\n## Core Truths\n\n## Boundaries\n\n## Vibe\n\n## Default Behaviors\n\n## Continuity\n\n## Change Contract\n"
                .to_string()
        });
    }
    let title = if target.ends_with(paths::USER) {
        "USER.md"
    } else if target.ends_with(paths::AGENTS) {
        "AGENTS.md"
    } else {
        target.rsplit('/').next().unwrap_or("PROMPT.md")
    };
    format!("# {title}\n")
}

fn ensure_prompt_trailing_newline(content: &str) -> String {
    let trimmed = content.trim_end();
    format!("{trimmed}\n")
}

fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title.to_string()))
}

fn find_section_byte_range(doc: &str, heading_name: &str) -> Option<(usize, usize, usize, String)> {
    let target = normalize_heading_name(heading_name);
    let mut offset = 0usize;
    let mut start: Option<(usize, usize, usize, String)> = None;

    for line in doc.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end;

        if let Some((level, title)) = parse_markdown_heading(line) {
            if let Some((start_offset, current_level, _, current_title)) = &start
                && level <= *current_level
            {
                return Some((
                    *start_offset,
                    line_start,
                    *current_level,
                    current_title.clone(),
                ));
            }

            if normalize_heading_name(&title) == target {
                start = Some((line_start, level, line_end, title));
            }
        }
    }

    start.map(|(start_offset, level, _, title)| (start_offset, doc.len(), level, title))
}

fn upsert_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let normalized_content = section_content.trim();
    let body = if normalized_content.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", normalized_content)
    };

    if let Some((start, end, level, title)) = find_section_byte_range(doc, heading) {
        let heading_line = format!("{} {}", "#".repeat(level.max(1)), title.trim());
        let replacement = format!("{heading_line}{body}");
        let mut merged = String::with_capacity(doc.len() + replacement.len());
        merged.push_str(&doc[..start]);
        merged.push_str(replacement.trim_end_matches('\n'));
        merged.push('\n');
        merged.push_str(doc[end..].trim_start_matches('\n'));
        return ensure_prompt_trailing_newline(merged.trim());
    }

    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    if !normalized_content.is_empty() {
        merged.push_str(normalized_content);
        merged.push('\n');
    }
    ensure_prompt_trailing_newline(&merged)
}

fn append_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    let content = section_content.trim();
    if !content.is_empty() {
        merged.push_str(content);
        merged.push('\n');
    }
    ensure_prompt_trailing_newline(&merged)
}

fn remove_markdown_section(doc: &str, heading: &str) -> Result<String, String> {
    let Some((start, end, _, _)) = find_section_byte_range(doc, heading) else {
        return Err(format!("section '{}' not found", heading));
    };

    let mut merged = String::with_capacity(doc.len());
    merged.push_str(&doc[..start]);
    merged.push_str(doc[end..].trim_start_matches('\n'));
    Ok(ensure_prompt_trailing_newline(merged.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(kind: &str, polarity: &str, weight: f64) -> OutcomeObservation {
        OutcomeObservation {
            id: Uuid::new_v4(),
            contract_id: Uuid::new_v4(),
            observation_kind: kind.to_string(),
            polarity: polarity.to_string(),
            weight,
            summary: None,
            evidence: json!({}),
            fingerprint: kind.to_string(),
            observed_at: Utc::now(),
            created_at: Utc::now(),
        }
    }

    fn contract() -> OutcomeContract {
        OutcomeContract {
            id: Uuid::new_v4(),
            user_id: "user".to_string(),
            actor_id: Some("actor".to_string()),
            channel: Some("web".to_string()),
            thread_id: Some("thread".to_string()),
            source_kind: "learning_event".to_string(),
            source_id: Uuid::new_v4().to_string(),
            contract_type: CONTRACT_TURN.to_string(),
            status: "open".to_string(),
            summary: Some("test".to_string()),
            due_at: Utc::now(),
            expires_at: Utc::now(),
            final_verdict: None,
            final_score: None,
            evaluation_details: json!({}),
            metadata: json!({}),
            dedupe_key: "dedupe".to_string(),
            claimed_at: None,
            evaluated_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn deterministic_scoring_stays_neutral_on_silence() {
        let score = deterministic_score(&contract(), &[]);
        assert_eq!(score.verdict, VERDICT_NEUTRAL);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn deterministic_scoring_flags_strong_negative() {
        let score = deterministic_score(
            &contract(),
            &[observation("rollback", VERDICT_NEGATIVE, 1.0)],
        );
        assert_eq!(score.verdict, VERDICT_NEGATIVE);
        assert!(score.score <= -1.0 || score.score < 0.0);
    }

    #[test]
    fn deterministic_scoring_detects_positive_follow_up() {
        let score = deterministic_score(
            &contract(),
            &[observation("next_step_continuation", VERDICT_POSITIVE, 0.6)],
        );
        assert_eq!(score.verdict, VERDICT_POSITIVE);
    }

    #[test]
    fn llm_score_parser_normalizes_and_clamps() {
        let observations = [observation("explicit_correction", VERDICT_NEGATIVE, 1.0)];
        let score = parse_llm_assisted_score(
            r#"{"verdict":"NEGATIVE","score":-2.0,"rationale":"bad"}"#,
            &observations,
        )
        .expect("score");
        assert_eq!(score.verdict, VERDICT_NEGATIVE);
        assert_eq!(score.score, -1.0);
        assert_eq!(
            score
                .details
                .get("strategy")
                .and_then(|value| value.as_str()),
            Some("llm_assisted")
        );
    }

    #[test]
    fn llm_score_request_includes_contract_context() {
        let contract = contract();
        let request = llm_assisted_score_request(&contract, &[]);
        assert_eq!(request.max_tokens, Some(250));
        assert_eq!(request.temperature, Some(0.1));
        assert_eq!(request.messages.len(), 2);
        assert!(
            request.messages[1]
                .content
                .contains(&contract.contract_type)
        );
    }

    #[test]
    fn deterministic_scoring_rewards_tool_durability_survival() {
        let mut contract = contract();
        contract.contract_type = CONTRACT_TOOL.to_string();
        let score = deterministic_score(&contract, &[]);
        assert_eq!(score.verdict, VERDICT_POSITIVE);
        assert_eq!(score.score, 0.5);
    }

    #[test]
    fn outcome_prompt_candidates_ignore_non_user_targets() {
        let mut contract = contract();
        contract.contract_type = CONTRACT_TOOL.to_string();
        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::SOUL,
        });
        assert_eq!(
            candidate_class_for_contract(&contract),
            OutcomeCandidateClass::Unknown
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::USER,
        });
        assert_eq!(
            candidate_class_for_contract(&contract),
            OutcomeCandidateClass::Prompt
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::actor_user("alice"),
        });
        assert_eq!(
            candidate_class_for_contract(&contract),
            OutcomeCandidateClass::Prompt
        );
    }

    #[test]
    fn turn_observation_targets_only_latest_eligible_contract() {
        let base_time = Utc::now();
        let older_id = Uuid::new_v4();
        let newer_id = Uuid::new_v4();
        let older = OutcomeContract {
            id: older_id,
            created_at: base_time,
            ..contract()
        };
        let newer = OutcomeContract {
            id: newer_id,
            created_at: base_time + chrono::Duration::seconds(5),
            ..contract()
        };

        let target = latest_turn_observation_target_id(
            &[older, newer],
            base_time + chrono::Duration::seconds(10),
        );
        assert_eq!(target, Some(newer_id));
    }

    #[test]
    fn follow_up_mutation_updates_metadata_and_due_at() {
        let mut contract = contract();
        let original_due = contract.due_at;
        let event_id = Uuid::new_v4();
        let now = Utc::now();

        let first = apply_user_turn_follow_up(&mut contract, event_id, now);
        assert_eq!(first, 1);
        assert_eq!(contract.due_at, original_due);
        assert_eq!(
            contract
                .metadata
                .get("last_follow_up_event_id")
                .and_then(|value| value.as_str()),
            Some(event_id.to_string().as_str())
        );

        let second = apply_user_turn_follow_up(&mut contract, event_id, now);
        assert_eq!(second, 2);
        assert_eq!(contract.due_at, now);
    }

    #[test]
    fn observation_builder_sets_fingerprint_and_evidence() {
        let contract_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let observed_at = Utc::now();
        let observation = user_turn_observation_record(
            contract_id,
            event_id,
            "preview",
            "explicit_correction",
            VERDICT_NEGATIVE,
            1.0,
            Some("summary"),
            observed_at,
        );
        assert_eq!(observation.contract_id, contract_id);
        assert_eq!(observation.observation_kind, "explicit_correction");
        assert_eq!(observation.summary.as_deref(), Some("summary"));
        assert_eq!(observation.observed_at, observed_at);
        assert_eq!(
            observation
                .evidence
                .get("content_preview")
                .and_then(|value| value.as_str()),
            Some("preview")
        );
        assert!(observation.fingerprint.contains(char::is_alphanumeric));
    }

    #[test]
    fn user_turn_observation_detects_corrections_and_acknowledgements() {
        let correction = user_turn_observation(1, "content").expect("correction");
        assert_eq!(correction.0, "explicit_correction");
        assert_eq!(correction.1, VERDICT_NEGATIVE);

        let repeated = user_turn_observation(0, "you missed this again").expect("repeat");
        assert_eq!(repeated.0, "repeated_request");

        let thanks = user_turn_observation(0, "looks good, thanks").expect("thanks");
        assert_eq!(thanks.0, "explicit_approval");
        assert_eq!(thanks.1, VERDICT_POSITIVE);

        let continuation = user_turn_observation(0, "next do the tests").expect("continuation");
        assert_eq!(continuation.0, "next_step_continuation");

        assert!(user_turn_observation(0, "   ").is_none());
    }

    #[test]
    fn evaluator_health_flags_stale_due_work() {
        let now = Utc::now();
        let healthy = OutcomeEvaluatorHealth {
            oldest_due_at: Some(now - chrono::Duration::seconds(30)),
            oldest_evaluating_claimed_at: None,
        };
        assert!(evaluator_health_status(&healthy, 60, now));

        let stale = OutcomeEvaluatorHealth {
            oldest_due_at: Some(now - chrono::Duration::seconds(121)),
            oldest_evaluating_claimed_at: None,
        };
        assert!(!evaluator_health_status(&stale, 60, now));
    }

    #[test]
    fn routine_patch_only_emits_for_notification_noise() {
        let mut contract = contract();
        contract.contract_type = CONTRACT_ROUTINE.to_string();
        contract.metadata = json!({
            "routine_id": Uuid::new_v4().to_string(),
            "routine_name": "digest",
            "run_status": "Ok",
            "notify": {
                "on_success": true
            }
        });
        let patch = routine_candidate_patch(
            &contract,
            &[observation("routine_muted", VERDICT_NEGATIVE, 1.0)],
        );
        assert_eq!(
            patch.get("type").and_then(|value| value.as_str()),
            Some("notification_noise_reduction")
        );
    }

    #[test]
    fn code_candidate_payload_requires_non_empty_diff() {
        let mut code_contract = contract();
        code_contract.contract_type = CONTRACT_TOOL.to_string();
        code_contract.source_kind = SOURCE_CODE_PROPOSAL.to_string();
        code_contract.metadata = json!({
            "title": "Fix contract drift",
            "rationale": "Repeated negative durability outcomes",
            "target_files": ["src/agent/outcomes.rs"],
            "diff": "",
        });
        assert!(code_candidate_payload(&code_contract).is_none());

        code_contract.metadata["diff"] =
            json!("diff --git a/src/agent/outcomes.rs b/src/agent/outcomes.rs");
        let payload = code_candidate_payload(&code_contract).expect("code payload");
        assert_eq!(
            payload.get("title").and_then(|value| value.as_str()),
            Some("Fix contract drift")
        );
        assert_eq!(
            payload.get("diff").and_then(|value| value.as_str()),
            Some("diff --git a/src/agent/outcomes.rs b/src/agent/outcomes.rs")
        );
    }

    #[test]
    fn manual_review_helpers_annotate_and_score_contracts() {
        let mut contract = contract();
        contract.final_verdict = Some(VERDICT_NEGATIVE.to_string());
        contract.final_score = Some(-0.75);
        let event_id = Uuid::new_v4();

        annotate_contract_with_ledger_event_id(&mut contract, event_id);
        annotate_contract_with_last_evaluator(&mut contract, "manual");

        assert_eq!(ledger_learning_event_id(&contract), Some(event_id));
        assert_eq!(
            contract_last_evaluator(&contract).as_deref(),
            Some("manual")
        );
        assert_eq!(manual_review_status(&contract, "confirm"), VERDICT_NEGATIVE);
        assert_eq!(manual_review_score(&contract, "confirm"), -0.75);
        assert_eq!(manual_review_status(&contract, "dismiss"), "review");
        assert_eq!(manual_review_score(&contract, "dismiss"), 0.0);
    }

    #[test]
    fn synthetic_turn_events_copy_trajectory_metadata_only_for_turn_contracts() {
        let mut turn_contract = contract();
        turn_contract.metadata = json!({
            "trajectory_target_id": "session:thread:7",
            "session_id": Uuid::new_v4().to_string(),
            "turn_number": 7,
            "thread_id": turn_contract.thread_id.clone().unwrap_or_default(),
        });
        let score = OutcomeScore {
            verdict: VERDICT_NEGATIVE.to_string(),
            score: -1.0,
            details: json!({"strategy":"deterministic"}),
        };
        let turn_event = synthetic_learning_event(&turn_contract, &score, &[]);
        assert_eq!(
            turn_event
                .payload
                .get("trajectory_target_id")
                .and_then(|value| value.as_str()),
            Some("session:thread:7")
        );
        assert_eq!(turn_event.event_type, "prompt");

        let mut tool_contract = turn_contract.clone();
        tool_contract.contract_type = CONTRACT_TOOL.to_string();
        tool_contract.metadata = json!({
            "artifact_type": "memory",
            "artifact_name": paths::MEMORY,
            "trajectory_target_id": "session:thread:7",
            "session_id": Uuid::new_v4().to_string(),
            "turn_number": 7,
            "thread_id": tool_contract.thread_id.clone().unwrap_or_default(),
        });
        let tool_event = synthetic_learning_event(&tool_contract, &score, &[]);
        assert!(tool_event.payload.get("trajectory_target_id").is_none());
        assert_eq!(tool_event.event_type, "memory");
    }

    #[test]
    fn manual_review_events_use_review_source_and_status() {
        let mut contract = contract();
        contract.final_verdict = Some(VERDICT_POSITIVE.to_string());
        let event = manual_review_learning_event(&contract, "confirm", &[]);
        assert_eq!(
            event.source,
            format!("outcome_review::{}", contract.source_kind)
        );
        assert_eq!(
            event
                .payload
                .get("manual_verdict")
                .and_then(|value| value.as_str()),
            Some(VERDICT_POSITIVE)
        );
    }
}

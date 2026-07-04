//! Root-independent learning policy helpers.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use thinclaw_workspace::paths;
use uuid::Uuid;

use crate::learning_types::{ImprovementClass, RiskTier};
use crate::session::{Turn, TurnToolCall};

pub fn ensure_auto_apply_class(classes: &mut Vec<String>, value: &str) {
    if !classes
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(value))
    {
        classes.push(value.to_string());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedSkillLifecycle {
    Draft,
    Shadow,
    Proposed,
    Active,
    Frozen,
    RolledBack,
}

impl GeneratedSkillLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Shadow => "shadow",
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::RolledBack => "rolled_back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSynthesisTrigger {
    ComplexSuccess,
    DeadEndRecovery,
    UserCorrection,
    NonTrivialWorkflow,
}

impl SkillSynthesisTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ComplexSuccess => "complex_success",
            Self::DeadEndRecovery => "dead_end_recovery",
            Self::UserCorrection => "user_correction",
            Self::NonTrivialWorkflow => "non_trivial_workflow",
        }
    }
}

pub fn detect_generated_skill_correction_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    [
        "actually",
        "correction:",
        "to clarify",
        "that's incorrect",
        "that is incorrect",
        "not quite",
        "use this instead",
        "please use",
        "instead:",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

pub fn generated_tool_category(tool_name: &str) -> &'static str {
    match tool_name {
        name if name.contains("file") || name.contains("search") => "files",
        name if name.contains("memory") || name.contains("session") => "memory",
        name if name.contains("http") || name.contains("browser") => "web",
        name if name.contains("skill") || name.contains("prompt") => "learning",
        "execute_code" | "shell" | "process" | "create_job" => "execution",
        _ => "other",
    }
}

pub fn generated_skill_lifecycle_for_reuse(
    reuse_count: u32,
) -> (GeneratedSkillLifecycle, Option<String>, bool) {
    if reuse_count >= 4 {
        (
            GeneratedSkillLifecycle::Proposed,
            Some("proposal_reuse_threshold".to_string()),
            false,
        )
    } else if reuse_count >= 2 {
        (
            GeneratedSkillLifecycle::Shadow,
            Some("shadow_candidate".to_string()),
            false,
        )
    } else {
        (GeneratedSkillLifecycle::Draft, None, false)
    }
}

pub fn generated_skill_feedback_polarity(verdict: &str) -> i8 {
    let normalized = verdict.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "helpful" | "approve" | "approved" | "accept" | "accepted" | "good" | "works"
        | "success" | "positive" => 1,
        "harmful" | "reject" | "rejected" | "bad" | "broken" | "regression" | "dont_learn"
        | "negative" | "rollback" | "rolled_back" => -1,
        _ => 0,
    }
}

pub fn generated_skill_transition_entry(
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    serde_json::json!({
        "status": lifecycle.as_str(),
        "at": transition_at,
        "activation_reason": activation_reason,
        "feedback_verdict": feedback_verdict,
        "feedback_note": feedback_note,
        "artifact_version_id": artifact_version_id,
    })
}

pub fn generated_skill_triggers(
    turn: &Turn,
    user_input: &str,
    reuse_count: u32,
    min_tool_calls: u32,
) -> Vec<SkillSynthesisTrigger> {
    let distinct_categories = turn
        .tool_calls
        .iter()
        .map(|call| generated_tool_category(&call.name))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let has_multi_tool_pattern =
        turn.tool_calls.len() as u32 >= min_tool_calls && distinct_categories >= 2;
    let recovered_from_failure = turn
        .tool_calls
        .iter()
        .any(|call| call.error.is_some() && call.result.is_none());
    let corrected_then_succeeded = detect_generated_skill_correction_signal(user_input)
        && !turn.tool_calls.is_empty()
        && turn.tool_calls.iter().all(|call| call.error.is_none());
    let repeated_workflow_match = reuse_count >= 2;
    let mut triggers = Vec::new();
    if has_multi_tool_pattern {
        triggers.push(SkillSynthesisTrigger::ComplexSuccess);
    }
    if recovered_from_failure {
        triggers.push(SkillSynthesisTrigger::DeadEndRecovery);
    }
    if corrected_then_succeeded {
        triggers.push(SkillSynthesisTrigger::UserCorrection);
    }
    if repeated_workflow_match {
        triggers.push(SkillSynthesisTrigger::NonTrivialWorkflow);
    }
    triggers
}

pub fn updated_generated_skill_proposal(
    proposal: &serde_json::Value,
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    let mut proposal = if proposal.is_object() {
        proposal.clone()
    } else {
        serde_json::json!({})
    };
    let entry = generated_skill_transition_entry(
        lifecycle,
        activation_reason,
        feedback_verdict,
        feedback_note,
        artifact_version_id,
        transition_at,
    );
    let obj = proposal
        .as_object_mut()
        .expect("generated skill proposal should be object");
    obj.insert("provenance".to_string(), serde_json::json!("generated"));
    obj.insert(
        "lifecycle_status".to_string(),
        serde_json::json!(lifecycle.as_str()),
    );
    obj.insert(
        "last_transition_at".to_string(),
        serde_json::json!(transition_at),
    );
    if let Some(reason) = activation_reason.filter(|value| !value.trim().is_empty()) {
        obj.insert(
            "activation_reason".to_string(),
            serde_json::json!(reason.to_string()),
        );
    }
    if let Some(version_id) = artifact_version_id {
        obj.insert(
            "last_artifact_version_id".to_string(),
            serde_json::json!(version_id),
        );
    }
    if let Some(verdict) = feedback_verdict {
        obj.insert(
            "last_feedback".to_string(),
            serde_json::json!({
                "verdict": verdict,
                "note": feedback_note,
                "at": transition_at,
            }),
        );
    }
    match lifecycle {
        GeneratedSkillLifecycle::Active => {
            obj.insert("activated_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::Frozen => {
            obj.insert("frozen_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::RolledBack => {
            obj.insert(
                "rolled_back_at".to_string(),
                serde_json::json!(transition_at),
            );
        }
        _ => {}
    }
    let history = obj
        .entry("state_history".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !history.is_array() {
        *history = serde_json::json!([]);
    }
    history
        .as_array_mut()
        .expect("state_history should be array")
        .push(entry);
    proposal
}

pub fn normalize_generated_skill_text(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn canonicalize_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string())
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonicalize_json_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| {
                        let value = map
                            .get(key)
                            .map(canonicalize_json_value)
                            .unwrap_or_else(|| "null".to_string());
                        format!(
                            "{}:{}",
                            serde_json::to_string(key).unwrap_or_else(|_| "\"<key>\"".to_string()),
                            value
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

pub fn stable_json_hash(value: &serde_json::Value) -> u64 {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    hasher.finish()
}

pub fn proposal_fingerprint(
    title: &str,
    rationale: &str,
    target_files: &[String],
    diff: &str,
) -> String {
    let canonical = serde_json::json!({
        "title": title.trim(),
        "rationale": rationale.trim(),
        "target_files": target_files,
        "diff": diff.trim(),
    });
    format!("{:016x}", stable_json_hash(&canonical))
}

#[derive(Debug, Clone, PartialEq)]
pub struct LearningEventEvaluation {
    pub quality_score: u32,
    pub evaluator_status: String,
    pub class: ImprovementClass,
    pub risk_tier: RiskTier,
    pub confidence: f32,
}

pub fn evaluate_learning_event(
    event_type: &str,
    payload: &serde_json::Value,
) -> LearningEventEvaluation {
    let success = payload
        .get("success")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let wasted_tool_calls = payload
        .get("wasted_tool_calls")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let repeated_failures = payload
        .get("repeated_failures")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let correction_count = payload
        .get("correction_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let safety_incident = payload
        .get("safety_incident")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let class = classify_learning_event(event_type, payload);
    let mut risk_tier = default_risk_tier_for_class(class);
    if safety_incident {
        risk_tier = RiskTier::Critical;
    }

    let mut score: i32 = if success { 82 } else { 45 };
    score -= (wasted_tool_calls as i32) * 4;
    score -= (repeated_failures as i32) * 7;
    score -= (correction_count as i32) * 5;
    if safety_incident {
        score -= 35;
    }
    score = score.clamp(0, 100);

    let confidence = ((score as f32 / 100.0)
        + if correction_count > 0 { 0.15 } else { 0.0 }
        + if repeated_failures > 0 { 0.1 } else { 0.0 })
    .clamp(0.0, 1.0);

    let evaluator_status = if score >= 70 {
        "accepted"
    } else if score >= 45 {
        "review"
    } else {
        "poor"
    }
    .to_string();

    LearningEventEvaluation {
        quality_score: score as u32,
        evaluator_status,
        class,
        risk_tier,
        confidence,
    }
}

pub fn default_risk_tier_for_class(class: ImprovementClass) -> RiskTier {
    match class {
        ImprovementClass::Code => RiskTier::Critical,
        ImprovementClass::Prompt | ImprovementClass::Routine | ImprovementClass::Unknown => {
            RiskTier::Medium
        }
        ImprovementClass::Skill | ImprovementClass::Memory => RiskTier::Low,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningRouteAction {
    PersistedOnly,
    HeldForReview,
    CodeProposal { auto_approve: bool },
    AutoApply,
    ManualReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LearningRoutePolicy {
    pub learning_enabled: bool,
    pub safe_mode_active: bool,
    pub auto_apply_allowed: bool,
    pub code_auto_apply_without_review: bool,
}

pub fn route_learning_candidate(
    class: ImprovementClass,
    risk: RiskTier,
    policy: LearningRoutePolicy,
) -> LearningRouteAction {
    if !policy.learning_enabled {
        return LearningRouteAction::PersistedOnly;
    }
    if policy.safe_mode_active {
        return LearningRouteAction::HeldForReview;
    }
    if risk.rank() >= RiskTier::High.rank() || class == ImprovementClass::Code {
        return LearningRouteAction::CodeProposal {
            auto_approve: class == ImprovementClass::Code && policy.code_auto_apply_without_review,
        };
    }
    if policy.auto_apply_allowed {
        LearningRouteAction::AutoApply
    } else {
        LearningRouteAction::ManualReview
    }
}

pub fn auto_apply_allowed_for_class(
    class: ImprovementClass,
    auto_apply_classes: &[String],
    prompt_mutation_enabled: bool,
) -> bool {
    if class == ImprovementClass::Prompt && !prompt_mutation_enabled {
        return false;
    }
    auto_apply_classes
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(class.as_str()))
}

pub fn is_negative_learning_feedback_verdict(verdict: &str) -> bool {
    matches!(
        verdict.to_ascii_lowercase().as_str(),
        "harmful" | "revert" | "dont_learn" | "reject"
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeModeTripInput {
    pub enabled: bool,
    pub min_samples: u32,
    pub negative_feedback_ratio_threshold: f64,
    pub rollback_ratio_threshold: f64,
    pub feedback_count: usize,
    pub negative_feedback_count: usize,
    pub rollback_count: usize,
    pub outcome_evaluated_last_7d: u64,
    pub outcome_negative_ratio_last_7d: f64,
}

pub fn safe_mode_should_trip(input: SafeModeTripInput) -> bool {
    if !input.enabled {
        return false;
    }

    let sample = input.feedback_count.max(input.rollback_count) as u32;
    if sample < input.min_samples {
        return false;
    }

    let feedback_ratio = input.negative_feedback_count as f64 / sample as f64;
    let rollback_ratio = input.rollback_count as f64 / sample as f64;

    feedback_ratio >= input.negative_feedback_ratio_threshold
        || rollback_ratio >= input.rollback_ratio_threshold
        || (input.outcome_evaluated_last_7d >= input.min_samples as u64
            && input.outcome_negative_ratio_last_7d >= input.negative_feedback_ratio_threshold)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeProposalFields {
    pub title: String,
    pub rationale: String,
    pub target_files: Vec<String>,
    pub diff: String,
    pub validation_results: serde_json::Value,
    pub rollback_note: Option<String>,
    pub evidence: serde_json::Value,
    pub fingerprint: String,
    pub metadata: serde_json::Value,
}

pub fn build_code_proposal_fields(
    source: &str,
    payload: &serde_json::Value,
    candidate_id: Uuid,
    candidate_summary: Option<&str>,
    candidate_confidence: Option<f64>,
) -> Result<CodeProposalFields, String> {
    let title = payload
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Learning-driven code proposal")
        .to_string();
    let rationale = payload
        .get("rationale")
        .and_then(|value| value.as_str())
        .or(candidate_summary)
        .unwrap_or("Distilled from repeated failures/corrections")
        .to_string();
    let target_files = payload
        .get("target_files")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let diff = payload
        .get("diff")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if diff.trim().is_empty() {
        return Err("code proposal missing diff".to_string());
    }

    let fingerprint = proposal_fingerprint(&title, &rationale, &target_files, &diff);
    let evidence = payload
        .get("evidence")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "event_payload": payload }));
    let validation_results = payload
        .get("validation_results")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"status": "not_run"}));
    let rollback_note = payload
        .get("rollback_note")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let metadata = serde_json::json!({
        "candidate_id": candidate_id,
        "source": source,
        "fingerprint": fingerprint,
        "package": {
            "problem_statement": title,
            "evidence": evidence,
            "candidate_rationale": rationale,
            "target_files": target_files,
            "unified_diff": diff,
            "validation_results": validation_results,
            "rollback_note": payload.get("rollback_note").cloned().unwrap_or(serde_json::Value::Null),
            "confidence": candidate_confidence,
        },
    });

    Ok(CodeProposalFields {
        title,
        rationale,
        target_files,
        diff,
        validation_results,
        rollback_note,
        evidence,
        fingerprint,
        metadata,
    })
}

pub fn rejected_code_proposal_suppression_message(
    fingerprint: &str,
    prior_fingerprint: &str,
    prior_updated_at: DateTime<Utc>,
    now: DateTime<Utc>,
    suppression_window_hours: i64,
) -> Option<String> {
    if prior_fingerprint != fingerprint {
        return None;
    }
    let age_hours = (now - prior_updated_at).num_hours().abs();
    if age_hours <= suppression_window_hours {
        Some(format!(
            "similar proposal was rejected {}h ago (fingerprint={}); cooldown active",
            age_hours, fingerprint
        ))
    } else {
        None
    }
}

pub fn code_proposal_review_metadata(
    metadata: &serde_json::Value,
    decision: &str,
    note: Option<&str>,
    reviewed_at: DateTime<Utc>,
    suppression_window_hours: i64,
) -> serde_json::Value {
    let mut metadata = if metadata.is_object() {
        metadata.clone()
    } else {
        serde_json::json!({})
    };
    let Some(obj) = metadata.as_object_mut() else {
        return metadata;
    };
    obj.insert(
        "review".to_string(),
        serde_json::json!({
            "decision": decision,
            "at": reviewed_at.to_rfc3339(),
            "note": note,
        }),
    );
    if decision.eq_ignore_ascii_case("reject")
        && let Some(fingerprint) = obj.get("fingerprint").cloned()
    {
        obj.insert(
            "anti_learning".to_string(),
            serde_json::json!({
                "fingerprint": fingerprint,
                "suppressed_until": (reviewed_at + chrono::Duration::hours(suppression_window_hours)).to_rfc3339(),
            }),
        );
    }
    metadata
}

pub fn generated_workflow_digest(user_input: &str, tool_calls: &[TurnToolCall]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_generated_skill_text(user_input).as_bytes());
    for call in tool_calls {
        hasher.update(b"|tool:");
        hasher.update(call.name.as_bytes());
        hasher.update(b"|params:");
        hasher.update(canonicalize_json_value(&call.parameters).as_bytes());
        hasher.update(b"|status:");
        hasher.update(if call.error.is_some() {
            b"error".as_slice()
        } else {
            b"ok".as_slice()
        });
        hasher.update(b"|signature:");
        hasher.update(compact_tool_outcome_signature(call).as_bytes());
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub fn compact_tool_outcome_signature(call: &TurnToolCall) -> String {
    let signature_input = if let Some(error) = call.error.as_deref() {
        format!("error:{}", normalize_generated_skill_text(error))
    } else if let Some(result) = call.result.as_ref() {
        format!("ok:{}", canonicalize_json_value(result))
    } else {
        "ok:null".to_string()
    };

    let mut hasher = Sha256::new();
    hasher.update(signature_input.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("sha256:{}", &digest[..16])
}

pub fn render_generated_skill_markdown(
    skill_name: &str,
    user_input: &str,
    tool_calls: &[TurnToolCall],
    lifecycle: GeneratedSkillLifecycle,
    reuse_count: u32,
    activation_reason: Option<String>,
) -> String {
    let description = user_input
        .split_whitespace()
        .take(18)
        .collect::<Vec<_>>()
        .join(" ");
    let keywords = tool_calls
        .iter()
        .map(|call| call.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let workflow_steps = tool_calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let parameter_keys = call
                .parameters
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            if parameter_keys.is_empty() {
                format!("{}. Use `{}`.", index + 1, call.name)
            } else {
                format!(
                    "{}. Use `{}` with parameters touching: {}.",
                    index + 1,
                    call.name,
                    parameter_keys
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let yaml_keywords = if keywords.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            keywords
                .iter()
                .map(|keyword| format!("\"{keyword}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let activation_reason = activation_reason.unwrap_or_else(|| "draft".to_string());
    format!(
        "---\nname: {skill_name}\nversion: 0.1.0\ndescription: \"Generated workflow skill for {description}\"\nactivation:\n  keywords: {yaml_keywords}\nmetadata:\n  openclaw:\n    provenance: generated\n    lifecycle_status: {}\n    outcome_score: {}\n    reuse_count: {reuse_count}\n    activation_reason: \"{}\"\n---\n\nYou are a reusable workflow skill distilled from a successful ThinClaw turn.\n\nUse this skill when the user is asking for work that resembles:\n- {description}\n\nPreferred workflow:\n{workflow_steps}\n\nSafety notes:\n- Verify tool results before moving to the next step.\n- Prefer deterministic file/memory reads before mutations.\n- Stop and surface blockers instead of guessing when required tools fail.\n",
        lifecycle.as_str(),
        if reuse_count >= 2 { "0.92" } else { "0.78" },
        activation_reason,
    )
}

pub fn validate_prompt_content(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("prompt content cannot be empty".to_string());
    }
    if !trimmed.contains('#') {
        return Err("prompt content must include markdown headings".to_string());
    }
    let lowered = trimmed.to_ascii_lowercase();
    let suspicious_markers = ["role: user", "role: assistant", "tool_result", "<tool_call"];
    if suspicious_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err("prompt content appears to include transcript/tool residue".to_string());
    }
    Ok(())
}

pub fn classify_learning_event(event_type: &str, payload: &serde_json::Value) -> ImprovementClass {
    let et = event_type.to_ascii_lowercase();
    if et.contains("code") || payload.get("diff").is_some() {
        return ImprovementClass::Code;
    }
    if et.contains("prompt") {
        return ImprovementClass::Prompt;
    }
    if et.contains("skill") {
        return ImprovementClass::Skill;
    }
    if et.contains("routine") {
        return ImprovementClass::Routine;
    }
    if let Some(target) = payload.get("target").and_then(|v| v.as_str())
        && matches!(
            target,
            paths::SOUL | paths::SOUL_LOCAL | paths::AGENTS | paths::USER
        )
    {
        return ImprovementClass::Prompt;
    }
    if payload.get("skill_content").is_some() {
        return ImprovementClass::Skill;
    }
    ImprovementClass::Memory
}

pub fn validate_prompt_target_content(target: &str, content: &str) -> Result<(), String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return thinclaw_soul::validate_canonical_soul(content);
    }
    if target.eq_ignore_ascii_case(paths::SOUL_LOCAL) {
        return thinclaw_soul::validate_local_overlay(content);
    }
    if target.eq_ignore_ascii_case(paths::AGENTS) {
        let lowered = content.to_ascii_lowercase();
        let required_markers = ["red lines", "ask first", "don't"];
        if required_markers
            .iter()
            .all(|marker| !lowered.contains(marker))
        {
            return Err(format!(
                "{} update rejected: core safety guidance appears to be missing",
                target
            ));
        }
    }
    Ok(())
}

pub fn is_prompt_target_supported(target: &str) -> bool {
    matches!(
        target,
        paths::SOUL | paths::SOUL_LOCAL | paths::AGENTS | paths::USER
    ) || target
        .to_ascii_lowercase()
        .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()))
}

pub fn ensure_prompt_document_root(current: &str, target: &str) -> String {
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

pub fn materialize_prompt_candidate_content(
    current: &str,
    proposal: &serde_json::Value,
    target: &str,
) -> Result<String, String> {
    let patch = proposal
        .get("prompt_patch")
        .ok_or_else(|| "prompt candidate missing content".to_string())?;
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

pub fn ensure_prompt_trailing_newline(content: &str) -> String {
    let trimmed = content.trim_end();
    format!("{trimmed}\n")
}

pub fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

pub fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
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

pub fn find_section_byte_range(
    doc: &str,
    heading_name: &str,
) -> Option<(usize, usize, usize, String)> {
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

pub fn upsert_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
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

pub fn append_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
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

pub fn remove_markdown_section(doc: &str, heading: &str) -> Result<String, String> {
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

    #[test]
    fn generated_skill_policy_helpers_preserve_defaults() {
        let mut classes = vec!["memory".to_string()];
        ensure_auto_apply_class(&mut classes, "MEMORY");
        ensure_auto_apply_class(&mut classes, "skill");
        assert_eq!(classes, vec!["memory".to_string(), "skill".to_string()]);

        assert!(detect_generated_skill_correction_signal(
            "Actually, use this"
        ));
        assert_eq!(generated_tool_category("memory_search"), "files");
        assert_eq!(
            generated_skill_lifecycle_for_reuse(4).0,
            GeneratedSkillLifecycle::Proposed
        );
        assert_eq!(generated_skill_feedback_polarity("APPROVED"), 1);
        assert_eq!(generated_skill_feedback_polarity("reject"), -1);
    }

    #[test]
    fn generated_skill_triggers_detect_workflow_signals() {
        let mut turn = Turn::new(0, "Actually use the browser then write memory", false);
        turn.tool_calls.push(TurnToolCall {
            name: "browser_open".to_string(),
            parameters: serde_json::json!({}),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        });
        turn.tool_calls.push(TurnToolCall {
            name: "memory_write".to_string(),
            parameters: serde_json::json!({}),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        });

        assert_eq!(
            generated_skill_triggers(&turn, "Actually use this instead", 2, 2),
            vec![
                SkillSynthesisTrigger::ComplexSuccess,
                SkillSynthesisTrigger::UserCorrection,
                SkillSynthesisTrigger::NonTrivialWorkflow,
            ]
        );

        turn.tool_calls[1].result = None;
        turn.tool_calls[1].error = Some("failed".to_string());
        assert!(
            generated_skill_triggers(&turn, "run it", 0, 3)
                .contains(&SkillSynthesisTrigger::DeadEndRecovery)
        );
    }

    #[test]
    fn generated_skill_proposal_update_appends_history_and_lifecycle_fields() {
        let transition_at = Utc::now();
        let artifact_version_id = Uuid::new_v4();
        let proposal = updated_generated_skill_proposal(
            &serde_json::json!({"state_history": "bad"}),
            GeneratedSkillLifecycle::Active,
            Some("approved"),
            Some("helpful"),
            Some("worked well"),
            Some(artifact_version_id),
            transition_at,
        );

        assert_eq!(proposal["provenance"], "generated");
        assert_eq!(proposal["lifecycle_status"], "active");
        assert_eq!(proposal["activation_reason"], "approved");
        assert_eq!(
            proposal["last_artifact_version_id"],
            artifact_version_id.to_string()
        );
        assert_eq!(proposal["last_feedback"]["verdict"], "helpful");
        assert_eq!(proposal["activated_at"], serde_json::json!(transition_at));
        assert_eq!(proposal["state_history"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn canonical_json_and_fingerprint_are_stable() {
        let first = serde_json::json!({"b": 2, "a": [true, "x"]});
        let second = serde_json::json!({"a": [true, "x"], "b": 2});

        assert_eq!(
            canonicalize_json_value(&first),
            canonicalize_json_value(&second)
        );
        assert_eq!(
            proposal_fingerprint("Fix", " because ", &["a.rs".to_string()], "diff"),
            proposal_fingerprint(" Fix ", "because", &["a.rs".to_string()], " diff ")
        );
    }

    #[test]
    fn prompt_content_validation_rejects_transcript_residue() {
        assert!(validate_prompt_content("# Header\nNormal content").is_ok());
        assert!(validate_prompt_content("# Header\nrole: user\nfoo").is_err());
    }

    #[test]
    fn learning_event_classification_uses_type_and_payload_markers() {
        assert_eq!(
            classify_learning_event("memory_observation", &serde_json::json!({"diff": "patch"})),
            ImprovementClass::Code
        );
        assert_eq!(
            classify_learning_event("observation", &serde_json::json!({"target": "SOUL.md"})),
            ImprovementClass::Prompt
        );
        assert_eq!(
            classify_learning_event("observation", &serde_json::json!({"skill_content": "body"})),
            ImprovementClass::Skill
        );
        assert_eq!(
            classify_learning_event("routine_run", &serde_json::json!({})),
            ImprovementClass::Routine
        );
        assert_eq!(
            classify_learning_event("observation", &serde_json::json!({})),
            ImprovementClass::Memory
        );
    }

    #[test]
    fn learning_event_evaluation_scores_and_risks_match_orchestrator_policy() {
        let evaluation = evaluate_learning_event(
            "prompt_correction",
            &serde_json::json!({
                "success": false,
                "wasted_tool_calls": 2,
                "repeated_failures": 1,
                "correction_count": 1
            }),
        );

        assert_eq!(evaluation.quality_score, 25);
        assert_eq!(evaluation.evaluator_status, "poor");
        assert_eq!(evaluation.class, ImprovementClass::Prompt);
        assert_eq!(evaluation.risk_tier, RiskTier::Medium);
        assert!((evaluation.confidence - 0.5).abs() < f32::EPSILON);

        let safety = evaluate_learning_event(
            "skill_observation",
            &serde_json::json!({"safety_incident": true}),
        );
        assert_eq!(safety.risk_tier, RiskTier::Critical);
    }

    #[test]
    fn learning_route_policy_preserves_tier_and_reckless_code_rules() {
        let base = LearningRoutePolicy {
            learning_enabled: true,
            safe_mode_active: false,
            auto_apply_allowed: true,
            code_auto_apply_without_review: false,
        };

        assert_eq!(
            route_learning_candidate(ImprovementClass::Memory, RiskTier::Low, base),
            LearningRouteAction::AutoApply
        );
        assert_eq!(
            route_learning_candidate(
                ImprovementClass::Memory,
                RiskTier::Low,
                LearningRoutePolicy {
                    auto_apply_allowed: false,
                    ..base
                }
            ),
            LearningRouteAction::ManualReview
        );
        assert_eq!(
            route_learning_candidate(
                ImprovementClass::Code,
                RiskTier::Critical,
                LearningRoutePolicy {
                    code_auto_apply_without_review: true,
                    ..base
                }
            ),
            LearningRouteAction::CodeProposal { auto_approve: true }
        );
        assert_eq!(
            route_learning_candidate(
                ImprovementClass::Skill,
                RiskTier::Low,
                LearningRoutePolicy {
                    safe_mode_active: true,
                    ..base
                }
            ),
            LearningRouteAction::HeldForReview
        );
    }

    #[test]
    fn auto_apply_and_safe_mode_policy_preserve_thresholds() {
        assert!(auto_apply_allowed_for_class(
            ImprovementClass::Memory,
            &["memory".to_string()],
            false
        ));
        assert!(!auto_apply_allowed_for_class(
            ImprovementClass::Prompt,
            &["prompt".to_string()],
            false
        ));
        assert!(is_negative_learning_feedback_verdict("dont_learn"));
        assert!(!is_negative_learning_feedback_verdict("helpful"));

        assert!(!safe_mode_should_trip(SafeModeTripInput {
            enabled: true,
            min_samples: 8,
            negative_feedback_ratio_threshold: 0.2,
            rollback_ratio_threshold: 0.25,
            feedback_count: 7,
            negative_feedback_count: 7,
            rollback_count: 0,
            outcome_evaluated_last_7d: 100,
            outcome_negative_ratio_last_7d: 1.0,
        }));
        assert!(safe_mode_should_trip(SafeModeTripInput {
            enabled: true,
            min_samples: 8,
            negative_feedback_ratio_threshold: 0.2,
            rollback_ratio_threshold: 0.25,
            feedback_count: 10,
            negative_feedback_count: 2,
            rollback_count: 0,
            outcome_evaluated_last_7d: 0,
            outcome_negative_ratio_last_7d: 0.0,
        }));
        assert!(safe_mode_should_trip(SafeModeTripInput {
            enabled: true,
            min_samples: 8,
            negative_feedback_ratio_threshold: 0.2,
            rollback_ratio_threshold: 0.25,
            feedback_count: 2,
            negative_feedback_count: 0,
            rollback_count: 8,
            outcome_evaluated_last_7d: 0,
            outcome_negative_ratio_last_7d: 0.0,
        }));
        assert!(safe_mode_should_trip(SafeModeTripInput {
            enabled: true,
            min_samples: 8,
            negative_feedback_ratio_threshold: 0.2,
            rollback_ratio_threshold: 0.25,
            feedback_count: 8,
            negative_feedback_count: 0,
            rollback_count: 0,
            outcome_evaluated_last_7d: 8,
            outcome_negative_ratio_last_7d: 0.2,
        }));
    }

    #[test]
    fn learning_code_proposal_fields_and_review_metadata_are_portable() {
        let candidate_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "title": "Fix retries",
            "rationale": "Repeated failures",
            "target_files": ["src/a.rs", 5, "src/b.rs"],
            "diff": "diff --git a/src/a.rs b/src/a.rs\n",
            "validation_results": {"status": "passed"},
            "rollback_note": "revert patch"
        });
        let fields =
            build_code_proposal_fields("learning_test", &payload, candidate_id, None, Some(0.75))
                .unwrap();

        assert_eq!(fields.title, "Fix retries");
        assert_eq!(
            fields.target_files,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        assert_eq!(fields.metadata["candidate_id"], candidate_id.to_string());
        assert_eq!(fields.metadata["source"], "learning_test");
        assert_eq!(fields.metadata["package"]["confidence"], 0.75);

        let reviewed_at = Utc::now();
        let rejected = code_proposal_review_metadata(
            &fields.metadata,
            "reject",
            Some("bad idea"),
            reviewed_at,
            24,
        );
        assert_eq!(rejected["review"]["decision"], "reject");
        assert_eq!(rejected["anti_learning"]["fingerprint"], fields.fingerprint);
        assert_eq!(
            rejected_code_proposal_suppression_message(
                &fields.fingerprint,
                rejected["anti_learning"]["fingerprint"].as_str().unwrap(),
                reviewed_at,
                reviewed_at + chrono::Duration::hours(1),
                24,
            )
            .unwrap(),
            format!(
                "similar proposal was rejected 1h ago (fingerprint={}); cooldown active",
                fields.fingerprint
            )
        );
        assert!(
            build_code_proposal_fields(
                "learning_test",
                &serde_json::json!({"diff": "  "}),
                candidate_id,
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn generated_workflow_digest_distinguishes_parameters_and_outcomes() {
        let first = vec![TurnToolCall {
            name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "echo one"}),
            result: Some(serde_json::json!({"stdout": "one"})),
            error: None,
        }];
        let second = vec![TurnToolCall {
            name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "echo two"}),
            result: Some(serde_json::json!({"stdout": "two"})),
            error: None,
        }];

        assert_ne!(
            generated_workflow_digest("run the shell command", &first),
            generated_workflow_digest("run the shell command", &second)
        );
    }

    #[test]
    fn generated_workflow_digest_is_stable_for_reordered_object_keys() {
        let first = vec![TurnToolCall {
            name: "http".to_string(),
            parameters: serde_json::json!({"url": "https://example.com", "method": "GET"}),
            result: Some(serde_json::json!({"status": 200, "ok": true})),
            error: None,
        }];
        let second = vec![TurnToolCall {
            name: "http".to_string(),
            parameters: serde_json::json!({"method": "GET", "url": "https://example.com"}),
            result: Some(serde_json::json!({"ok": true, "status": 200})),
            error: None,
        }];

        assert_eq!(
            generated_workflow_digest("fetch the endpoint", &first),
            generated_workflow_digest("fetch the endpoint", &second)
        );
    }

    #[test]
    fn generated_skill_markdown_renders_legacy_metadata() {
        let content = render_generated_skill_markdown(
            "generated-summary",
            "Help the user collect a file summary and write it down.",
            &[TurnToolCall {
                name: "shell".to_string(),
                parameters: serde_json::json!({"cmd": "echo hi"}),
                result: Some(serde_json::json!({"stdout": "hi"})),
                error: None,
            }],
            GeneratedSkillLifecycle::Shadow,
            3,
            Some("shadow_candidate".to_string()),
        );

        assert!(content.contains("name: generated-summary"));
        assert!(content.contains("lifecycle_status: shadow"));
        assert!(content.contains("outcome_score: 0.92"));
        assert!(content.contains("1. Use `shell` with parameters touching: cmd."));
    }

    #[test]
    fn prompt_target_helpers_preserve_supported_targets_and_seed_defaults() {
        assert!(is_prompt_target_supported("SOUL.md"));
        assert!(is_prompt_target_supported("nested/USER.md"));
        assert!(!is_prompt_target_supported("README.md"));

        assert_eq!(
            ensure_prompt_document_root("", "AGENTS.md"),
            "# AGENTS.md\n"
        );
        assert_eq!(
            ensure_prompt_document_root("  # Existing\n\n", "AGENTS.md"),
            "# Existing\n"
        );
        assert!(ensure_prompt_document_root("", "SOUL.local.md").contains("## Workspace Context"));
        assert!(ensure_prompt_document_root("", "SOUL.md").contains("## Core Truths"));
    }

    #[test]
    fn prompt_target_validation_and_materialization_match_legacy_errors() {
        assert!(validate_prompt_target_content("AGENTS.md", "# AGENTS.md\nUse red lines.").is_ok());
        assert_eq!(
            validate_prompt_target_content("AGENTS.md", "# AGENTS.md\nNo marker").unwrap_err(),
            "AGENTS.md update rejected: core safety guidance appears to be missing"
        );

        let proposal = serde_json::json!({
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Guidance",
                "section_content": "Ask first before destructive changes."
            }
        });
        assert_eq!(
            materialize_prompt_candidate_content("# AGENTS.md\n", &proposal, "AGENTS.md").unwrap(),
            "# AGENTS.md\n\n## Guidance\nAsk first before destructive changes.\n"
        );

        let proposal = serde_json::json!({"prompt_patch": {"operation": "unknown"}});
        assert_eq!(
            materialize_prompt_candidate_content("", &proposal, "USER.md").unwrap_err(),
            "unsupported prompt patch operation 'unknown'"
        );
    }

    #[test]
    fn markdown_section_helpers_preserve_heading_levels() {
        let doc = "# Root\n\n## Existing\nold\n\n## Next\nstill here\n";
        let updated = upsert_markdown_section(doc, "existing", "new");

        assert_eq!(updated, "# Root\n\n## Existing\nnew\n## Next\nstill here\n");
    }

    #[test]
    fn markdown_section_helpers_append_and_remove_sections() {
        let doc = "# Root\n";
        let appended = append_markdown_section(doc, "Follow Up", "- item");
        assert_eq!(appended, "# Root\n\n## Follow Up\n- item\n");

        let removed = remove_markdown_section(&appended, "follow up").unwrap();
        assert_eq!(removed, "# Root\n");
        assert!(remove_markdown_section(&removed, "missing").is_err());
    }
}

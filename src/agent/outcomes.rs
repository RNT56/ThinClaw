//! Outcome-backed learning helpers and evaluator.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use thinclaw_agent::outcomes::{self as outcome_policy, OutcomeScore};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub use outcome_policy::{contract_last_evaluator, ledger_learning_event_id};

use crate::agent::learning::{ImprovementClass, LearningOrchestrator, RiskTier};
use crate::agent::routine::{Routine, RoutineRun};
use crate::db::Database;
use crate::history::{
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, LearningFeedbackRecord, LearningRollbackRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeObservation,
};
use crate::llm::{LlmProvider, Reasoning};
use crate::safety::SafetyLayer;
use crate::settings::LearningSettings;
use crate::skills::SkillRegistry;
use crate::workspace::Workspace;
use crate::workspace::paths;

const CONTRACT_TURN: &str = "turn_usefulness";
const CONTRACT_TOOL: &str = "tool_durability";
const CONTRACT_ROUTINE: &str = "routine_usefulness";

const STATUS_OPEN: &str = "open";
const STATUS_EVALUATING: &str = "evaluating";
const STATUS_EVALUATED: &str = "evaluated";

#[cfg(test)]
const VERDICT_POSITIVE: &str = "positive";
const VERDICT_NEUTRAL: &str = "neutral";
const VERDICT_NEGATIVE: &str = "negative";

const SOURCE_LEARNING_EVENT: &str = "learning_event";
const SOURCE_ARTIFACT_VERSION: &str = "artifact_version";
const SOURCE_CODE_PROPOSAL: &str = "learning_code_proposal";
const SOURCE_ROUTINE_RUN: &str = "routine_run";

const EVALUATOR_OUTCOME: &str = "outcome_evaluator_v1";
const EVALUATOR_MANUAL_REVIEW: &str = "outcome_manual_review_v1";

const DEFAULT_EVALUATION_INTERVAL_SECS: u64 = 600;
const MIN_EVALUATION_INTERVAL_SECS: u64 = 1;

#[derive(Debug, Clone)]
struct OutcomeSchedulerPlan {
    user_ids: Vec<String>,
    sleep_interval_secs: u64,
}

pub struct OutcomeService {
    store: Arc<dyn Database>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
    safety: Arc<SafetyLayer>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    routine_engine: Option<Arc<crate::agent::routine_engine::RoutineEngine>>,
}

impl OutcomeService {
    pub fn new(
        store: Arc<dyn Database>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
        safety: Arc<SafetyLayer>,
    ) -> Self {
        Self {
            store,
            cheap_llm,
            safety,
            workspace: None,
            skill_registry: None,
            routine_engine: None,
        }
    }

    pub fn with_learning_context(
        mut self,
        workspace: Option<Arc<Workspace>>,
        skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
        routine_engine: Option<Arc<crate::agent::routine_engine::RoutineEngine>>,
    ) -> Self {
        self.workspace = workspace;
        self.skill_registry = skill_registry;
        self.routine_engine = routine_engine;
        self
    }

    pub async fn run_once(&self) -> Result<usize, String> {
        let plan = self.scheduler_plan(Utc::now()).await?;
        let mut processed = 0usize;
        for user_id in plan.user_ids {
            processed += self.run_once_for_user(&user_id).await?;
        }
        Ok(processed)
    }

    pub async fn run_once_for_user(&self, user_id: &str) -> Result<usize, String> {
        let settings = load_learning_settings(&*self.store, user_id).await;
        if !outcomes_enabled(&settings) {
            return Ok(0);
        }
        let now = Utc::now();
        let limit = i64::from(settings.outcomes.max_due_per_tick.max(1));
        let contracts = self
            .store
            .claim_due_outcome_contracts_for_user(user_id, limit, now)
            .await
            .map_err(|err| err.to_string())?;
        let mut processed = 0usize;
        for contract in contracts {
            match self.evaluate_contract(contract.clone()).await {
                Ok(()) => {
                    processed += 1;
                }
                Err(err) => {
                    tracing::debug!(
                        contract_id = %contract.id,
                        user_id,
                        error = %err,
                        "Outcome evaluation failed; requeueing contract"
                    );
                    if let Err(requeue_err) = self.requeue_failed_contract(&contract, &err).await {
                        tracing::debug!(
                            contract_id = %contract.id,
                            user_id,
                            error = %requeue_err,
                            "Outcome contract requeue failed"
                        );
                    }
                }
            }
        }
        Ok(processed)
    }

    async fn scheduler_plan(&self, now: DateTime<Utc>) -> Result<OutcomeSchedulerPlan, String> {
        let pending_users: Vec<crate::history::OutcomePendingUser> = self
            .store
            .list_users_with_pending_outcome_work(now)
            .await
            .map_err(|err| err.to_string())?;
        let mut user_ids = Vec::new();
        let mut min_interval = DEFAULT_EVALUATION_INTERVAL_SECS;
        for pending in pending_users {
            let settings = load_learning_settings(&*self.store, &pending.user_id).await;
            if !outcomes_enabled(&settings) {
                continue;
            }
            min_interval = min_interval.min(settings.outcomes.evaluation_interval_secs.max(1));
            user_ids.push(pending.user_id);
        }
        Ok(OutcomeSchedulerPlan {
            user_ids,
            sleep_interval_secs: min_interval.max(MIN_EVALUATION_INTERVAL_SECS),
        })
    }

    async fn evaluate_contract(&self, mut contract: OutcomeContract) -> Result<(), String> {
        let observations = self
            .store
            .list_outcome_observations(contract.id)
            .await
            .map_err(|err| err.to_string())?;
        let settings = load_learning_settings(&*self.store, &contract.user_id).await;
        let mut score = outcome_policy::deterministic_score(&contract, &observations);
        if settings.outcomes.llm_assist_enabled
            && outcome_policy::has_mixed_observations(&observations)
            && let Some(llm_score) = self
                .llm_assisted_score(&contract, &observations)
                .await
                .ok()
                .flatten()
        {
            score = llm_score;
        }

        contract.status = STATUS_EVALUATED.to_string();
        contract.final_verdict = Some(score.verdict.clone());
        contract.final_score = Some(score.score);
        contract.evaluation_details = score.details.clone();
        outcome_policy::annotate_contract_with_last_evaluator(&mut contract, EVALUATOR_OUTCOME);
        contract.evaluated_at = Some(Utc::now());
        contract.updated_at = Utc::now();
        self.store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;

        let learning_event_id = self
            .persist_learning_evaluation(&contract, &score, &observations)
            .await?;
        outcome_policy::annotate_contract_with_ledger_event_id(&mut contract, learning_event_id);
        contract.updated_at = Utc::now();
        self.store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;
        if let Some(candidate) = self
            .maybe_generate_candidate(&contract, &score, &observations, learning_event_id)
            .await?
            && let Err(err) = self.route_outcome_candidate(&candidate).await
        {
            tracing::debug!(
                contract_id = %contract.id,
                candidate_id = %candidate.id,
                error = %err,
                "Outcome candidate routing failed"
            );
        }
        Ok(())
    }

    async fn requeue_failed_contract(
        &self,
        contract: &OutcomeContract,
        reason: &str,
    ) -> Result<(), String> {
        let Some(mut current) = self
            .store
            .get_outcome_contract(&contract.user_id, contract.id)
            .await
            .map_err(|err| err.to_string())?
        else {
            return Ok(());
        };
        if current.status != STATUS_EVALUATING || current.evaluated_at.is_some() {
            return Ok(());
        }
        current.status = STATUS_OPEN.to_string();
        current.claimed_at = None;
        current.updated_at = Utc::now();
        outcome_policy::upsert_json_string(
            &mut current.evaluation_details,
            "last_error",
            reason.to_string(),
        );
        self.store
            .update_outcome_contract(&current)
            .await
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    async fn llm_assisted_score(
        &self,
        contract: &OutcomeContract,
        observations: &[OutcomeObservation],
    ) -> Result<Option<OutcomeScore>, String> {
        let Some(llm) = self.cheap_llm.clone() else {
            return Ok(None);
        };
        let request = outcome_policy::llm_assisted_score_request(contract, observations);
        let reasoning = Reasoning::new(llm, self.safety.clone());
        let (content, _) = reasoning
            .complete(request)
            .await
            .map_err(|err| err.to_string())?;
        outcome_policy::parse_llm_assisted_score(&content, observations).map(Some)
    }

    async fn persist_learning_evaluation(
        &self,
        contract: &OutcomeContract,
        score: &OutcomeScore,
        observations: &[OutcomeObservation],
    ) -> Result<Uuid, String> {
        let evaluation_event_id = if contract.source_kind == SOURCE_LEARNING_EVENT {
            Uuid::parse_str(&contract.source_id).map_err(|err| err.to_string())?
        } else {
            let event = outcome_policy::synthetic_learning_event(contract, score, observations);
            let event_id = self
                .store
                .insert_learning_event(&event)
                .await
                .map_err(|err| err.to_string())?;
            if event_id.is_nil() {
                event.id
            } else {
                event_id
            }
        };

        let evaluation = LearningEvaluation {
            id: Uuid::new_v4(),
            learning_event_id: evaluation_event_id,
            user_id: contract.user_id.clone(),
            evaluator: EVALUATOR_OUTCOME.to_string(),
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
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_evaluation(&evaluation)
            .await
            .map_err(|err| err.to_string())?;
        Ok(evaluation_event_id)
    }

    async fn maybe_generate_candidate(
        &self,
        contract: &OutcomeContract,
        score: &OutcomeScore,
        observations: &[OutcomeObservation],
        learning_event_id: Uuid,
    ) -> Result<Option<LearningCandidate>, String> {
        if score.verdict != VERDICT_NEGATIVE {
            return Ok(None);
        }
        if score.score.abs() < 0.6 {
            return Ok(None);
        }
        let Some(pattern_key) = contract
            .metadata
            .get("pattern_key")
            .and_then(|value| value.as_str())
        else {
            return Ok(None);
        };

        let recent = self
            .store
            .list_outcome_contracts(&OutcomeContractQuery {
                user_id: contract.user_id.clone(),
                actor_id: contract.actor_id.clone(),
                status: Some(STATUS_EVALUATED.to_string()),
                contract_type: Some(contract.contract_type.clone()),
                source_kind: None,
                source_id: None,
                thread_id: contract.thread_id.clone(),
                limit: 128,
            })
            .await
            .map_err(|err| err.to_string())?;

        let same_pattern = recent
            .into_iter()
            .filter(|entry| {
                entry.id != contract.id
                    && entry.final_verdict.as_deref() == Some(VERDICT_NEGATIVE)
                    && entry
                        .metadata
                        .get("pattern_key")
                        .and_then(|value| value.as_str())
                        == Some(pattern_key)
            })
            .count()
            + 1;

        if same_pattern < 2 {
            return Ok(None);
        }

        let outcome_class = outcome_policy::candidate_class_for_contract(contract);
        let class = ImprovementClass::from_str(outcome_class.as_str());
        if class == ImprovementClass::Unknown {
            return Ok(None);
        }
        let risk =
            RiskTier::from_str(outcome_policy::candidate_risk_for_class(outcome_class).as_str());
        let target_name = outcome_policy::candidate_target_name(contract);
        let summary = format!(
            "Repeated negative outcome pattern detected for {} ({})",
            contract.contract_type, pattern_key
        );
        let evidence = json!({
            "source": "outcome_backed_learning",
            "contract_id": contract.id,
            "contract_type": contract.contract_type,
            "pattern_key": pattern_key,
            "pattern_count": same_pattern,
            "final_verdict": score.verdict,
            "observations": observations,
            "target": target_name.clone(),
            "target_type": outcome_policy::candidate_target_type(contract),
            "routine_patch": outcome_policy::routine_candidate_patch(contract, observations),
        });

        let recent_candidates = self
            .store
            .list_learning_candidates(&contract.user_id, Some(class.as_str()), None, 50)
            .await
            .map_err(|err| err.to_string())?;
        let dedupe = outcome_policy::stable_key(&[
            class.as_str(),
            &target_name.clone().unwrap_or_default(),
            pattern_key,
        ]);
        if recent_candidates.iter().any(|candidate| {
            candidate
                .proposal
                .get("dedupe_key")
                .and_then(|value| value.as_str())
                == Some(dedupe.as_str())
        }) {
            return Ok(None);
        }

        let mut proposal = serde_json::Map::new();
        proposal.insert("dedupe_key".to_string(), json!(dedupe));
        proposal.insert("source".to_string(), json!("outcome_backed_learning"));
        proposal.insert("pattern_key".to_string(), json!(pattern_key));
        proposal.insert("pattern_count".to_string(), json!(same_pattern));
        proposal.insert("contract_type".to_string(), json!(contract.contract_type));
        proposal.insert("verdict".to_string(), json!(score.verdict));
        proposal.insert("evidence".to_string(), evidence);
        proposal.insert(
            "routine_patch".to_string(),
            outcome_policy::routine_candidate_patch(contract, observations),
        );

        match class {
            ImprovementClass::Prompt => {
                let Some(prompt_payload) = self
                    .prompt_candidate_payload(contract, observations, target_name.as_deref())
                    .await?
                else {
                    return Ok(None);
                };
                merge_json_object(&mut proposal, prompt_payload);
            }
            ImprovementClass::Code => {
                let Some(code_payload) = outcome_policy::code_candidate_payload(contract) else {
                    return Ok(None);
                };
                merge_json_object(&mut proposal, code_payload);
            }
            _ => {}
        }

        let candidate = LearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: Some(learning_event_id),
            user_id: contract.user_id.clone(),
            candidate_type: class.as_str().to_string(),
            risk_tier: risk.as_str().to_string(),
            confidence: Some(score.score.abs()),
            target_type: Some(outcome_policy::candidate_target_type(contract)),
            target_name,
            summary: Some(summary),
            proposal: serde_json::Value::Object(proposal),
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_candidate(&candidate)
            .await
            .map_err(|err| err.to_string())?;
        Ok(Some(candidate))
    }

    async fn route_outcome_candidate(&self, candidate: &LearningCandidate) -> Result<(), String> {
        let orchestrator = LearningOrchestrator::new(
            Arc::clone(&self.store),
            self.workspace.clone(),
            self.skill_registry.clone(),
        )
        .with_routine_engine(self.routine_engine.clone());
        let outcome = orchestrator
            .route_existing_candidate("outcome_evaluator_v1", candidate)
            .await?;
        tracing::debug!(
            candidate_id = %candidate.id,
            auto_applied = outcome.auto_applied,
            code_proposal_id = ?outcome.code_proposal_id,
            notes = ?outcome.notes,
            "Outcome candidate routed through learning orchestrator"
        );
        Ok(())
    }

    async fn prompt_candidate_payload(
        &self,
        contract: &OutcomeContract,
        observations: &[OutcomeObservation],
        target_name: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        let target = target_name.unwrap_or(paths::USER);
        if !outcome_policy::is_prompt_candidate_target_name_allowed(target) {
            return Ok(None);
        }
        let heading = "Outcome-Backed Guidance";
        let section_content = outcome_policy::prompt_guidance_section(contract, observations);
        let existing = if target.eq_ignore_ascii_case(paths::SOUL) {
            crate::identity::soul_store::read_home_soul().unwrap_or_default()
        } else if let Some(workspace) = self.workspace.as_ref() {
            workspace
                .read(target)
                .await
                .ok()
                .map(|doc| doc.content)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let patch = json!({
            "operation": "upsert_section",
            "heading": heading,
            "section_content": section_content,
        });
        let content = outcome_policy::apply_prompt_patch_content(&existing, &patch, target)?;
        Ok(Some(json!({
            "target": target,
            "content": content,
            "prompt_patch": patch,
        })))
    }
}

pub async fn persist_manual_review_to_learning_ledger(
    store: &Arc<dyn Database>,
    contract: &mut OutcomeContract,
    decision: &str,
) -> Result<Uuid, String> {
    let normalized_decision = decision.trim().to_ascii_lowercase();
    let observations = store
        .list_outcome_observations(contract.id)
        .await
        .map_err(|err| err.to_string())?;
    let learning_event_id = resolve_learning_event_for_manual_review(
        store,
        contract,
        &normalized_decision,
        &observations,
    )
    .await?;
    outcome_policy::annotate_contract_with_ledger_event_id(contract, learning_event_id);
    outcome_policy::annotate_contract_with_last_evaluator(contract, EVALUATOR_MANUAL_REVIEW);

    let evaluation = LearningEvaluation {
        id: Uuid::new_v4(),
        learning_event_id,
        user_id: contract.user_id.clone(),
        evaluator: EVALUATOR_MANUAL_REVIEW.to_string(),
        status: outcome_policy::manual_review_status(contract, &normalized_decision),
        score: Some(outcome_policy::manual_review_score(
            contract,
            &normalized_decision,
        )),
        details: json!({
            "contract_id": contract.id,
            "contract_type": contract.contract_type,
            "source_kind": contract.source_kind,
            "source_id": contract.source_id,
            "review_decision": normalized_decision,
            "manual_verdict": contract.final_verdict,
            "contract_status": contract.status,
            "final_score": contract.final_score,
            "ledger_learning_event_id": learning_event_id,
            "observations": observations,
            "strategy": "manual_review",
        }),
        created_at: Utc::now(),
    };
    store
        .insert_learning_evaluation(&evaluation)
        .await
        .map_err(|err| err.to_string())?;
    Ok(learning_event_id)
}

pub fn spawn_outcome_service(service: Arc<OutcomeService>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let plan = match service.scheduler_plan(Utc::now()).await {
                Ok(plan) => plan,
                Err(err) => {
                    tracing::debug!(error = %err, "Outcome service scheduler plan failed");
                    OutcomeSchedulerPlan {
                        user_ids: Vec::new(),
                        sleep_interval_secs: DEFAULT_EVALUATION_INTERVAL_SECS,
                    }
                }
            };
            tokio::time::sleep(Duration::from_secs(plan.sleep_interval_secs)).await;
            for user_id in plan.user_ids {
                if let Err(err) = service.run_once_for_user(&user_id).await {
                    tracing::debug!(
                        user_id = %user_id,
                        error = %err,
                        "Outcome service tick failed"
                    );
                }
            }
        }
    })
}

pub async fn maybe_create_turn_contract(
    store: &Arc<dyn Database>,
    event: &LearningEvent,
) -> Result<Option<Uuid>, String> {
    if !event
        .payload
        .get("role")
        .and_then(|value| value.as_str())
        .is_some_and(|role| role.eq_ignore_ascii_case("assistant"))
    {
        return Ok(None);
    }
    let settings = load_learning_settings(&**store, &event.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(None);
    }
    let contract =
        outcome_policy::build_turn_contract(event, u64::from(settings.outcomes.default_ttl_hours));
    let id = store
        .insert_outcome_contract(&contract)
        .await
        .map_err(|err| err.to_string())?;
    Ok(Some(id))
}

pub async fn observe_user_turn(
    store: &Arc<dyn Database>,
    event: &LearningEvent,
) -> Result<(), String> {
    let settings = load_learning_settings(&**store, &event.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(());
    }
    let content = event
        .payload
        .get("content_preview")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let correction_count = event
        .payload
        .get("correction_count")
        .and_then(|value| value.as_u64())
        .unwrap_or_default();
    let observation = outcome_policy::user_turn_observation(correction_count, content);
    let contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: event.user_id.clone(),
            actor_id: event.actor_id.clone(),
            status: Some(STATUS_OPEN.to_string()),
            contract_type: Some(CONTRACT_TURN.to_string()),
            source_kind: None,
            source_id: None,
            thread_id: event.thread_id.clone(),
            limit: 32,
        })
        .await
        .map_err(|err| err.to_string())?;

    let observation_target_id =
        outcome_policy::latest_turn_observation_target_id(&contracts, event.created_at);

    for mut contract in contracts
        .into_iter()
        .filter(|entry| entry.created_at <= event.created_at)
    {
        outcome_policy::apply_user_turn_follow_up(&mut contract, event.id, Utc::now());
        store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;

        if Some(contract.id) == observation_target_id
            && let Some((kind, polarity, weight, summary)) = observation.clone()
        {
            let observation = outcome_policy::user_turn_observation_record(
                contract.id,
                event.id,
                content,
                &kind,
                &polarity,
                weight,
                summary.as_deref(),
                event.created_at,
            );
            store
                .insert_outcome_observation(&observation)
                .await
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

pub async fn maybe_create_artifact_contract(
    store: &Arc<dyn Database>,
    version: &LearningArtifactVersion,
) -> Result<Option<Uuid>, String> {
    if !version.status.eq_ignore_ascii_case("applied")
        && !version.status.eq_ignore_ascii_case("promoted")
    {
        return Ok(None);
    }
    let settings = load_learning_settings(&**store, &version.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(None);
    }
    let contract = outcome_policy::build_artifact_contract(
        version,
        u64::from(settings.outcomes.default_ttl_hours),
    );
    let id = store
        .insert_outcome_contract(&contract)
        .await
        .map_err(|err| err.to_string())?;
    Ok(Some(id))
}

pub async fn maybe_create_proposal_contract(
    store: &Arc<dyn Database>,
    proposal: &LearningCodeProposal,
) -> Result<Option<Uuid>, String> {
    let settings = load_learning_settings(&**store, &proposal.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(None);
    }
    let contract = outcome_policy::build_proposal_contract(
        proposal,
        u64::from(settings.outcomes.default_ttl_hours),
    );
    let id = store
        .insert_outcome_contract(&contract)
        .await
        .map_err(|err| err.to_string())?;
    Ok(Some(id))
}

pub async fn observe_feedback(
    store: &Arc<dyn Database>,
    feedback: &LearningFeedbackRecord,
) -> Result<(), String> {
    let source_kind = match feedback.target_type.as_str() {
        "learning_event" => SOURCE_LEARNING_EVENT,
        "artifact_version" => SOURCE_ARTIFACT_VERSION,
        "code_proposal" => SOURCE_CODE_PROPOSAL,
        _ => return Ok(()),
    };
    let contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: feedback.user_id.clone(),
            actor_id: None,
            status: None,
            contract_type: None,
            source_kind: Some(source_kind.to_string()),
            source_id: Some(feedback.target_id.clone()),
            thread_id: None,
            limit: 16,
        })
        .await
        .map_err(|err| err.to_string())?;
    for mut contract in contracts
        .into_iter()
        .filter(|entry| entry.status == STATUS_OPEN || entry.status == STATUS_EVALUATING)
    {
        let (polarity, weight) = outcome_policy::feedback_polarity(&feedback.verdict);
        insert_observation(
            store,
            contract.id,
            "feedback",
            polarity,
            weight,
            feedback.note.as_deref(),
            json!({
                "feedback_id": feedback.id,
                "verdict": feedback.verdict,
                "target_type": feedback.target_type,
            }),
            &outcome_policy::stable_key(&[
                &contract.id.to_string(),
                "feedback",
                &feedback.id.to_string(),
            ]),
            feedback.created_at,
        )
        .await?;
        if polarity == VERDICT_NEGATIVE {
            contract.due_at = Utc::now();
            contract.updated_at = Utc::now();
            store
                .update_outcome_contract(&contract)
                .await
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

pub async fn observe_rollback(
    store: &Arc<dyn Database>,
    rollback: &LearningRollbackRecord,
) -> Result<(), String> {
    let contracts = if let Some(version_id) = rollback.artifact_version_id {
        store
            .list_outcome_contracts(&OutcomeContractQuery {
                user_id: rollback.user_id.clone(),
                actor_id: None,
                status: None,
                contract_type: Some(CONTRACT_TOOL.to_string()),
                source_kind: Some(SOURCE_ARTIFACT_VERSION.to_string()),
                source_id: Some(version_id.to_string()),
                thread_id: None,
                limit: 8,
            })
            .await
            .map_err(|err| err.to_string())?
    } else {
        store
            .list_outcome_contracts(&OutcomeContractQuery {
                user_id: rollback.user_id.clone(),
                actor_id: None,
                status: None,
                contract_type: Some(CONTRACT_TOOL.to_string()),
                source_kind: Some(SOURCE_ARTIFACT_VERSION.to_string()),
                source_id: None,
                thread_id: None,
                limit: 32,
            })
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .filter(|contract| {
                contract
                    .metadata
                    .get("artifact_name")
                    .and_then(|value| value.as_str())
                    == Some(rollback.artifact_name.as_str())
            })
            .take(1)
            .collect()
    };

    for mut contract in contracts
        .into_iter()
        .filter(|entry| entry.status == STATUS_OPEN || entry.status == STATUS_EVALUATING)
    {
        insert_observation(
            store,
            contract.id,
            "rollback",
            VERDICT_NEGATIVE,
            1.0,
            Some(&rollback.reason),
            json!({
                "rollback_id": rollback.id,
                "artifact_type": rollback.artifact_type,
                "artifact_name": rollback.artifact_name,
            }),
            &outcome_policy::stable_key(&[
                &contract.id.to_string(),
                "rollback",
                &rollback.id.to_string(),
            ]),
            rollback.created_at,
        )
        .await?;
        contract.due_at = Utc::now();
        contract.updated_at = Utc::now();
        store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub async fn observe_proposal_rejection(
    store: &Arc<dyn Database>,
    proposal: &LearningCodeProposal,
    note: Option<&str>,
) -> Result<(), String> {
    let contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: proposal.user_id.clone(),
            actor_id: None,
            status: None,
            contract_type: Some(CONTRACT_TOOL.to_string()),
            source_kind: Some(SOURCE_CODE_PROPOSAL.to_string()),
            source_id: Some(proposal.id.to_string()),
            thread_id: None,
            limit: 8,
        })
        .await
        .map_err(|err| err.to_string())?;
    for mut contract in contracts
        .into_iter()
        .filter(|entry| entry.status == STATUS_OPEN || entry.status == STATUS_EVALUATING)
    {
        insert_observation(
            store,
            contract.id,
            "proposal_rejection",
            VERDICT_NEGATIVE,
            1.0,
            note,
            json!({
                "proposal_id": proposal.id,
                "title": proposal.title,
            }),
            &outcome_policy::stable_key(&[
                &contract.id.to_string(),
                "proposal_rejection",
                &proposal.id.to_string(),
            ]),
            Utc::now(),
        )
        .await?;
        contract.due_at = Utc::now();
        contract.updated_at = Utc::now();
        store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub async fn maybe_create_routine_contract(
    store: &Arc<dyn Database>,
    routine: &Routine,
    run: &RoutineRun,
) -> Result<Option<Uuid>, String> {
    if !outcome_policy::is_user_visible_routine_run(routine, run) {
        return Ok(None);
    }
    let settings = load_learning_settings(&**store, &routine.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(None);
    }
    let contract = outcome_policy::build_routine_contract(routine, run);
    let id = store
        .insert_outcome_contract(&contract)
        .await
        .map_err(|err| err.to_string())?;
    Ok(Some(id))
}

pub async fn observe_routine_state_change(
    store: &Arc<dyn Database>,
    routine: &Routine,
    observation_kind: &str,
) -> Result<(), String> {
    let contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: routine.user_id.clone(),
            actor_id: Some(routine.owner_actor_id().to_string()),
            status: None,
            contract_type: Some(CONTRACT_ROUTINE.to_string()),
            source_kind: Some(SOURCE_ROUTINE_RUN.to_string()),
            source_id: None,
            thread_id: None,
            limit: 16,
        })
        .await
        .map_err(|err| err.to_string())?;
    let Some(mut contract) = contracts
        .into_iter()
        .filter(|entry| entry.status == STATUS_OPEN || entry.status == STATUS_EVALUATING)
        .find(|entry| {
            entry
                .metadata
                .get("routine_id")
                .and_then(|value| value.as_str())
                == Some(routine.id.to_string().as_str())
        })
    else {
        return Ok(());
    };

    insert_observation(
        store,
        contract.id,
        observation_kind,
        VERDICT_NEGATIVE,
        1.0,
        Some(&format!("Routine {} {}", routine.name, observation_kind)),
        json!({
            "routine_id": routine.id,
            "routine_name": routine.name,
        }),
        &outcome_policy::stable_key(&[
            &contract.id.to_string(),
            observation_kind,
            &routine.id.to_string(),
        ]),
        Utc::now(),
    )
    .await?;
    contract.due_at = Utc::now();
    contract.updated_at = Utc::now();
    store
        .update_outcome_contract(&contract)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

pub async fn heartbeat_review_summary(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> Result<Option<String>, String> {
    let settings = load_learning_settings(&**store, user_id).await;
    if !outcomes_enabled(&settings) || !settings.outcomes.heartbeat_summary_enabled {
        return Ok(None);
    }
    let stats = store
        .outcome_summary_stats(user_id)
        .await
        .map_err(|err| err.to_string())?;
    if stats.due == 0 && stats.open == 0 && stats.negative_ratio_last_7d <= 0.0 {
        return Ok(None);
    }
    Ok(Some(format!(
        "Outcome Review Queue\n- Open contracts: {}\n- Due now: {}\n- Evaluated last 7d: {}\n- Negative ratio last 7d: {:.0}%",
        stats.open,
        stats.due,
        stats.evaluated_last_7d,
        stats.negative_ratio_last_7d * 100.0
    )))
}

pub async fn evaluator_is_healthy(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> Result<bool, String> {
    let settings = load_learning_settings(&**store, user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(true);
    }
    let now = Utc::now();
    let health: crate::history::OutcomeEvaluatorHealth = store
        .outcome_evaluator_health(user_id, now)
        .await
        .map_err(|err| err.to_string())?;
    Ok(outcome_policy::evaluator_health_status(
        &health,
        settings.outcomes.evaluation_interval_secs,
        now,
    ))
}

async fn load_learning_settings(store: &dyn Database, user_id: &str) -> LearningSettings {
    match store.get_all_settings(user_id).await {
        Ok(map) => crate::settings::Settings::from_db_map(&map).learning,
        Err(_) => LearningSettings::default(),
    }
}

fn outcomes_enabled(settings: &LearningSettings) -> bool {
    settings.enabled && settings.outcomes.enabled
}

async fn insert_observation(
    store: &Arc<dyn Database>,
    contract_id: Uuid,
    observation_kind: &str,
    polarity: &str,
    weight: f64,
    summary: Option<&str>,
    evidence: serde_json::Value,
    fingerprint: &str,
    observed_at: DateTime<Utc>,
) -> Result<(), String> {
    let observation = outcome_policy::build_observation(
        contract_id,
        observation_kind,
        polarity,
        weight,
        summary,
        evidence,
        fingerprint,
        observed_at,
    );
    store
        .insert_outcome_observation(&observation)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
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

async fn resolve_learning_event_for_manual_review(
    store: &Arc<dyn Database>,
    contract: &OutcomeContract,
    decision: &str,
    observations: &[OutcomeObservation],
) -> Result<Uuid, String> {
    if contract.source_kind == SOURCE_LEARNING_EVENT {
        return Uuid::parse_str(&contract.source_id).map_err(|err| err.to_string());
    }
    if let Some(existing) = outcome_policy::ledger_learning_event_id(contract) {
        return Ok(existing);
    }

    let event = outcome_policy::manual_review_learning_event(contract, decision, observations);
    let event_id = store
        .insert_learning_event(&event)
        .await
        .map_err(|err| err.to_string())?;
    if event_id.is_nil() {
        Ok(event.id)
    } else {
        Ok(event_id)
    }
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
            source_kind: SOURCE_LEARNING_EVENT.to_string(),
            source_id: Uuid::new_v4().to_string(),
            contract_type: CONTRACT_TURN.to_string(),
            status: STATUS_OPEN.to_string(),
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
        let score = outcome_policy::deterministic_score(&contract(), &[]);
        assert_eq!(score.verdict, VERDICT_NEUTRAL);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn deterministic_scoring_flags_strong_negative() {
        let score = outcome_policy::deterministic_score(
            &contract(),
            &[observation("rollback", VERDICT_NEGATIVE, 1.0)],
        );
        assert_eq!(score.verdict, VERDICT_NEGATIVE);
        assert!(score.score <= -1.0 || score.score < 0.0);
    }

    #[test]
    fn deterministic_scoring_detects_positive_follow_up() {
        let score = outcome_policy::deterministic_score(
            &contract(),
            &[observation("next_step_continuation", VERDICT_POSITIVE, 0.6)],
        );
        assert_eq!(score.verdict, VERDICT_POSITIVE);
    }

    #[test]
    fn deterministic_scoring_rewards_tool_durability_survival() {
        let mut contract = contract();
        contract.contract_type = CONTRACT_TOOL.to_string();
        let score = outcome_policy::deterministic_score(&contract, &[]);
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
            outcome_policy::candidate_class_for_contract(&contract),
            outcome_policy::OutcomeCandidateClass::Unknown
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::USER,
        });
        assert_eq!(
            outcome_policy::candidate_class_for_contract(&contract),
            outcome_policy::OutcomeCandidateClass::Prompt
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::actor_user("alice"),
        });
        assert_eq!(
            outcome_policy::candidate_class_for_contract(&contract),
            outcome_policy::OutcomeCandidateClass::Prompt
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

        let target = outcome_policy::latest_turn_observation_target_id(
            &[older, newer],
            base_time + chrono::Duration::seconds(10),
        );
        assert_eq!(target, Some(newer_id));
    }

    #[test]
    fn evaluator_health_flags_stale_due_work() {
        let now = Utc::now();
        let healthy = crate::history::OutcomeEvaluatorHealth {
            oldest_due_at: Some(now - chrono::Duration::seconds(30)),
            oldest_evaluating_claimed_at: None,
        };
        assert!(outcome_policy::evaluator_health_status(&healthy, 60, now));

        let stale = crate::history::OutcomeEvaluatorHealth {
            oldest_due_at: Some(now - chrono::Duration::seconds(121)),
            oldest_evaluating_claimed_at: None,
        };
        assert!(!outcome_policy::evaluator_health_status(&stale, 60, now));
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
        let patch = outcome_policy::routine_candidate_patch(
            &contract,
            &[observation("routine_muted", VERDICT_NEGATIVE, 1.0)],
        );
        assert_eq!(
            patch.get("type").and_then(|value| value.as_str()),
            Some("notification_noise_reduction")
        );
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
        let turn_event = outcome_policy::synthetic_learning_event(&turn_contract, &score, &[]);
        assert_eq!(
            turn_event
                .payload
                .get("trajectory_target_id")
                .and_then(|value| value.as_str()),
            Some("session:thread:7")
        );

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
        let tool_event = outcome_policy::synthetic_learning_event(&tool_contract, &score, &[]);
        assert!(
            tool_event.payload.get("trajectory_target_id").is_none(),
            "non-turn synthetic events should stay out of trajectory hydration"
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
        assert!(
            outcome_policy::code_candidate_payload(&code_contract).is_none(),
            "empty diffs should suppress outcome-driven code proposals"
        );

        code_contract.metadata["diff"] =
            json!("diff --git a/src/agent/outcomes.rs b/src/agent/outcomes.rs");
        let payload = outcome_policy::code_candidate_payload(&code_contract).expect("code payload");
        assert_eq!(
            payload.get("title").and_then(|value| value.as_str()),
            Some("Fix contract drift")
        );
        assert_eq!(
            payload.get("diff").and_then(|value| value.as_str()),
            Some("diff --git a/src/agent/outcomes.rs b/src/agent/outcomes.rs")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn prompt_candidate_payload_includes_materialized_content() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "outcome-prompt-payload-user";
        let workspace = std::sync::Arc::new(crate::workspace::Workspace::new_with_db(
            user_id,
            std::sync::Arc::clone(&db),
        ));
        workspace
            .write(
                paths::USER,
                "# USER.md\n\n## Preferences\n- prefer concise implementation notes\n",
            )
            .await
            .expect("seed USER.md");

        let service = OutcomeService::new(
            std::sync::Arc::clone(&db),
            None,
            std::sync::Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig::default(),
            )),
        )
        .with_learning_context(
            Some(workspace),
            None::<std::sync::Arc<tokio::sync::RwLock<crate::skills::SkillRegistry>>>,
            None::<std::sync::Arc<crate::agent::routine_engine::RoutineEngine>>,
        );

        let mut prompt_contract = contract();
        prompt_contract.user_id = user_id.to_string();
        prompt_contract.metadata = json!({
            "pattern_key": "turn:actor:thread",
        });
        let payload = service
            .prompt_candidate_payload(
                &prompt_contract,
                &[observation("explicit_correction", VERDICT_NEGATIVE, 1.0)],
                Some(paths::USER),
            )
            .await
            .expect("payload generation should succeed")
            .expect("prompt payload");

        assert_eq!(
            payload.get("target").and_then(|value| value.as_str()),
            Some(paths::USER)
        );
        assert_eq!(
            payload
                .get("prompt_patch")
                .and_then(|value| value.get("operation"))
                .and_then(|value| value.as_str()),
            Some("upsert_section")
        );
        let content = payload
            .get("content")
            .and_then(|value| value.as_str())
            .expect("materialized content");
        assert!(content.contains("## Preferences"));
        assert!(content.contains("## Outcome-Backed Guidance"));
        assert!(content.contains("finish the requested work before concluding"));
    }
}

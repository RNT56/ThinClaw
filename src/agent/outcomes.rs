//! Outcome-backed learning helpers and evaluator.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::learning::{ImprovementClass, LearningOrchestrator, RiskTier};
use crate::agent::routine::{Routine, RoutineAction, RoutineRun, RunStatus};
use crate::db::Database;
use crate::history::{
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, LearningFeedbackRecord, LearningRollbackRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeObservation,
};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
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

const VERDICT_POSITIVE: &str = "positive";
const VERDICT_NEUTRAL: &str = "neutral";
const VERDICT_NEGATIVE: &str = "negative";

const SOURCE_LEARNING_EVENT: &str = "learning_event";
const SOURCE_ARTIFACT_VERSION: &str = "artifact_version";
const SOURCE_CODE_PROPOSAL: &str = "learning_code_proposal";
const SOURCE_ROUTINE_RUN: &str = "routine_run";

const LEDGER_EVENT_ID_KEY: &str = "ledger_learning_event_id";
const EVALUATOR_OUTCOME: &str = "outcome_evaluator_v1";
const EVALUATOR_MANUAL_REVIEW: &str = "outcome_manual_review_v1";

const DEFAULT_EVALUATION_INTERVAL_SECS: u64 = 600;
const MIN_EVALUATION_INTERVAL_SECS: u64 = 1;

#[derive(Debug, Clone)]
struct OutcomeScore {
    verdict: String,
    score: f64,
    details: serde_json::Value,
}

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
        let mut score = deterministic_score(&contract, &observations);
        if settings.outcomes.llm_assist_enabled
            && has_mixed_observations(&observations)
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
        annotate_contract_with_last_evaluator(&mut contract, EVALUATOR_OUTCOME);
        contract.evaluated_at = Some(Utc::now());
        contract.updated_at = Utc::now();
        self.store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;

        let learning_event_id = self
            .persist_learning_evaluation(&contract, &score, &observations)
            .await?;
        annotate_contract_with_ledger_event_id(&mut contract, learning_event_id);
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
        upsert_json_string(
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
        let request = CompletionRequest::new(vec![
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
        .with_max_tokens(250);

        let reasoning = Reasoning::new(llm, self.safety.clone());
        let (content, _) = reasoning
            .complete(request)
            .await
            .map_err(|err| err.to_string())?;
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
        Ok(Some(OutcomeScore {
            verdict,
            score,
            details: json!({
                "strategy": "llm_assisted",
                "llm_result": parsed,
                "observations": observations,
            }),
        }))
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
            let event = synthetic_learning_event(contract, score, observations);
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

        let class = candidate_class_for_contract(contract);
        if class == ImprovementClass::Unknown {
            return Ok(None);
        }
        let risk = candidate_risk_for_class(class);
        let target_name = candidate_target_name(contract);
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
            "target_type": candidate_target_type(contract),
            "routine_patch": routine_candidate_patch(contract, observations),
        });

        let recent_candidates = self
            .store
            .list_learning_candidates(&contract.user_id, Some(class.as_str()), None, 50)
            .await
            .map_err(|err| err.to_string())?;
        let dedupe = stable_key(&[
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
            routine_candidate_patch(contract, observations),
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
                let Some(code_payload) = code_candidate_payload(contract) else {
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
            target_type: Some(candidate_target_type(contract)),
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
        if !is_prompt_candidate_target_name_allowed(target) {
            return Ok(None);
        }
        let heading = "Outcome-Backed Guidance";
        let section_content = prompt_guidance_section(contract, observations);
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
        let content = apply_prompt_patch_content(&existing, &patch, target)?;
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
    annotate_contract_with_ledger_event_id(contract, learning_event_id);
    annotate_contract_with_last_evaluator(contract, EVALUATOR_MANUAL_REVIEW);

    let evaluation = LearningEvaluation {
        id: Uuid::new_v4(),
        learning_event_id,
        user_id: contract.user_id.clone(),
        evaluator: EVALUATOR_MANUAL_REVIEW.to_string(),
        status: manual_review_status(contract, &normalized_decision),
        score: Some(manual_review_score(contract, &normalized_decision)),
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
    let contract = OutcomeContract {
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
        expires_at: event.created_at
            + chrono::Duration::hours(settings.outcomes.default_ttl_hours as i64),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
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
    let observation = user_turn_observation(event, content);
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

    let observation_target_id = latest_turn_observation_target_id(&contracts, event.created_at);

    for mut contract in contracts
        .into_iter()
        .filter(|entry| entry.created_at <= event.created_at)
    {
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
                json!(event.id.to_string()),
            );
        }
        if follow_up_turns >= 2 {
            contract.due_at = Utc::now();
        }
        contract.updated_at = Utc::now();
        store
            .update_outcome_contract(&contract)
            .await
            .map_err(|err| err.to_string())?;

        if Some(contract.id) == observation_target_id
            && let Some((kind, polarity, weight, summary)) = observation.clone()
        {
            insert_observation(
                store,
                contract.id,
                &kind,
                &polarity,
                weight,
                summary.as_deref(),
                json!({
                    "event_id": event.id,
                    "content_preview": content,
                }),
                &stable_key(&[&contract.id.to_string(), &event.id.to_string(), &kind]),
                event.created_at,
            )
            .await?;
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
    let actor_id = json_string(&version.provenance, "actor_id");
    let channel = json_string(&version.provenance, "channel");
    let thread_id = json_string(&version.provenance, "thread_id");
    let pattern_key = format!(
        "artifact:{}:{}",
        version.artifact_type, version.artifact_name
    );
    let contract = OutcomeContract {
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
        expires_at: version.created_at
            + chrono::Duration::hours(settings.outcomes.default_ttl_hours as i64),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
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
    let contract = OutcomeContract {
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
        due_at: Utc::now() + chrono::Duration::hours(24),
        expires_at: Utc::now()
            + chrono::Duration::hours(settings.outcomes.default_ttl_hours as i64),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
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
        let (polarity, weight) = feedback_polarity(&feedback.verdict);
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
            &stable_key(&[
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
            &stable_key(&[
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
            &stable_key(&[
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
    if !is_user_visible_routine_run(routine, run) {
        return Ok(None);
    }
    let settings = load_learning_settings(&**store, &routine.user_id).await;
    if !outcomes_enabled(&settings) {
        return Ok(None);
    }
    let contract = OutcomeContract {
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
        due_at: Utc::now() + chrono::Duration::days(7),
        expires_at: Utc::now() + chrono::Duration::days(7),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
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
        &stable_key(&[
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
    Ok(evaluator_health_status(
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

fn evaluator_health_status(
    health: &crate::history::OutcomeEvaluatorHealth,
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

fn deterministic_score(
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

fn has_mixed_observations(observations: &[OutcomeObservation]) -> bool {
    let has_positive = observations
        .iter()
        .any(|obs| obs.polarity == VERDICT_POSITIVE);
    let has_negative = observations
        .iter()
        .any(|obs| obs.polarity == VERDICT_NEGATIVE);
    has_positive && has_negative
}

fn user_turn_observation(
    event: &LearningEvent,
    content: &str,
) -> Option<(String, String, f64, Option<String>)> {
    let correction_count = event
        .payload
        .get("correction_count")
        .and_then(|value| value.as_u64())
        .unwrap_or_default();
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

fn detect_repeated_request_signal(content: &str) -> bool {
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

fn detect_thanks_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    ["thanks", "thank you", "looks good", "perfect", "great"]
        .iter()
        .any(|needle| normalized.contains(needle))
}

fn latest_turn_observation_target_id(
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

fn candidate_class_for_contract(contract: &OutcomeContract) -> ImprovementClass {
    match contract.contract_type.as_str() {
        CONTRACT_ROUTINE => ImprovementClass::Routine,
        CONTRACT_TURN => ImprovementClass::Prompt,
        CONTRACT_TOOL => match contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
        {
            Some("skill") => ImprovementClass::Skill,
            Some("prompt") if is_outcome_prompt_target_allowed(contract) => {
                ImprovementClass::Prompt
            }
            Some("prompt") => ImprovementClass::Unknown,
            Some("memory") => ImprovementClass::Memory,
            _ if contract.source_kind == SOURCE_CODE_PROPOSAL => ImprovementClass::Code,
            _ => ImprovementClass::Unknown,
        },
        _ => ImprovementClass::Unknown,
    }
}

fn candidate_risk_for_class(class: ImprovementClass) -> RiskTier {
    match class {
        ImprovementClass::Memory | ImprovementClass::Skill => RiskTier::Low,
        ImprovementClass::Prompt | ImprovementClass::Routine | ImprovementClass::Unknown => {
            RiskTier::Medium
        }
        ImprovementClass::Code => RiskTier::Critical,
    }
}

fn candidate_target_type(contract: &OutcomeContract) -> String {
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

fn candidate_target_name(contract: &OutcomeContract) -> Option<String> {
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

fn code_candidate_payload(contract: &OutcomeContract) -> Option<serde_json::Value> {
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

fn routine_candidate_patch(
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

fn is_outcome_prompt_target_allowed(contract: &OutcomeContract) -> bool {
    contract
        .metadata
        .get("artifact_name")
        .and_then(|value| value.as_str())
        .is_some_and(is_prompt_candidate_target_name_allowed)
}

fn is_prompt_candidate_target_name_allowed(name: &str) -> bool {
    name.eq_ignore_ascii_case(paths::USER)
        || name
            .to_ascii_lowercase()
            .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()))
}

fn feedback_polarity(verdict: &str) -> (&'static str, f64) {
    match verdict.to_ascii_lowercase().as_str() {
        "helpful" | "approve" => (VERDICT_POSITIVE, 0.8),
        "harmful" | "revert" | "dont_learn" | "reject" => (VERDICT_NEGATIVE, 1.0),
        _ => (VERDICT_NEUTRAL, 0.0),
    }
}

fn is_user_visible_routine_run(routine: &Routine, run: &RoutineRun) -> bool {
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

fn synthetic_learning_event(
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
    let mut metadata = json!({
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
        copy_turn_trajectory_metadata(contract, &mut metadata);
    }
    let event = crate::agent::learning::LearningEvent::new(
        format!("outcome::{}", contract.source_kind),
        class,
        risk,
        summary,
    )
    .with_metadata(metadata);
    event.into_persisted(
        contract.user_id.clone(),
        contract.actor_id.clone(),
        contract.channel.clone(),
        contract.thread_id.clone(),
        None,
        None,
        None,
    )
}

fn manual_review_learning_event(
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
    let mut metadata = json!({
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
        copy_turn_trajectory_metadata(contract, &mut metadata);
    }
    let event = crate::agent::learning::LearningEvent::new(
        format!("outcome_review::{}", contract.source_kind),
        class,
        risk,
        summary,
    )
    .with_metadata(metadata);
    event.into_persisted(
        contract.user_id.clone(),
        contract.actor_id.clone(),
        contract.channel.clone(),
        contract.thread_id.clone(),
        None,
        None,
        None,
    )
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
    let observation = OutcomeObservation {
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
    };
    store
        .insert_outcome_observation(&observation)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn turn_pattern_key(event: &LearningEvent) -> String {
    format!(
        "turn:{}:{}",
        event.actor_id.as_deref().unwrap_or(event.user_id.as_str()),
        event.thread_id.as_deref().unwrap_or("no-thread")
    )
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

fn stable_key(parts: &[&str]) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
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

fn prompt_guidance_section(
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

fn apply_prompt_patch_content(
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

fn ensure_prompt_document_root(current: &str, target: &str) -> String {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        return ensure_prompt_trailing_newline(trimmed);
    }
    if target.ends_with(paths::SOUL_LOCAL) {
        let mut sections = std::collections::BTreeMap::new();
        for section in crate::identity::soul::LOCAL_SECTIONS {
            sections.insert((*section).to_string(), String::new());
        }
        return crate::identity::soul::render_local_soul_overlay(
            &crate::identity::soul::LocalSoulOverlay { sections },
        );
    }
    if target.ends_with(paths::SOUL) {
        return crate::identity::soul::compose_seeded_soul("balanced").unwrap_or_else(|_| {
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

async fn resolve_learning_event_for_manual_review(
    store: &Arc<dyn Database>,
    contract: &OutcomeContract,
    decision: &str,
    observations: &[OutcomeObservation],
) -> Result<Uuid, String> {
    if contract.source_kind == SOURCE_LEARNING_EVENT {
        return Uuid::parse_str(&contract.source_id).map_err(|err| err.to_string());
    }
    if let Some(existing) = ledger_learning_event_id(contract) {
        return Ok(existing);
    }

    let event = manual_review_learning_event(contract, decision, observations);
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

pub fn ledger_learning_event_id(contract: &OutcomeContract) -> Option<Uuid> {
    contract
        .metadata
        .get(LEDGER_EVENT_ID_KEY)
        .or_else(|| contract.evaluation_details.get(LEDGER_EVENT_ID_KEY))
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn annotate_contract_with_ledger_event_id(contract: &mut OutcomeContract, learning_event_id: Uuid) {
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

pub fn contract_last_evaluator(contract: &OutcomeContract) -> Option<String> {
    contract
        .evaluation_details
        .get("last_evaluator")
        .or_else(|| contract.metadata.get("last_evaluator"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn annotate_contract_with_last_evaluator(contract: &mut OutcomeContract, evaluator: &str) {
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

fn manual_review_status(contract: &OutcomeContract, decision: &str) -> String {
    match decision {
        "confirm" => contract
            .final_verdict
            .clone()
            .unwrap_or_else(|| VERDICT_NEUTRAL.to_string()),
        "dismiss" | "requeue" => "review".to_string(),
        _ => VERDICT_NEUTRAL.to_string(),
    }
}

fn manual_review_score(contract: &OutcomeContract, decision: &str) -> f64 {
    match decision {
        "confirm" => contract.final_score.unwrap_or_else(|| {
            verdict_score(contract.final_verdict.as_deref().unwrap_or(VERDICT_NEUTRAL))
        }),
        "dismiss" | "requeue" => 0.0,
        _ => 0.0,
    }
}

fn verdict_score(verdict: &str) -> f64 {
    match verdict {
        VERDICT_POSITIVE => 1.0,
        VERDICT_NEGATIVE => -1.0,
        _ => 0.0,
    }
}

fn upsert_json_string(target: &mut serde_json::Value, key: &str, value: String) {
    if !target.is_object() {
        *target = json!({});
    }
    if let Some(map) = target.as_object_mut() {
        map.insert(key.to_string(), json!(value));
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
            ImprovementClass::Unknown
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::USER,
        });
        assert_eq!(
            candidate_class_for_contract(&contract),
            ImprovementClass::Prompt
        );

        contract.metadata = json!({
            "artifact_type": "prompt",
            "artifact_name": paths::actor_user("alice"),
        });
        assert_eq!(
            candidate_class_for_contract(&contract),
            ImprovementClass::Prompt
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
    fn evaluator_health_flags_stale_due_work() {
        let now = Utc::now();
        let healthy = crate::history::OutcomeEvaluatorHealth {
            oldest_due_at: Some(now - chrono::Duration::seconds(30)),
            oldest_evaluating_claimed_at: None,
        };
        assert!(evaluator_health_status(&healthy, 60, now));

        let stale = crate::history::OutcomeEvaluatorHealth {
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
            code_candidate_payload(&code_contract).is_none(),
            "empty diffs should suppress outcome-driven code proposals"
        );

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

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

fn runtime_settings(
    learning_enabled: bool,
    outcomes_enabled: bool,
    evaluation_interval_secs: u64,
) -> OutcomeRuntimeSettings {
    OutcomeRuntimeSettings {
        learning_enabled,
        outcomes_enabled,
        evaluation_interval_secs,
        max_due_per_tick: 50,
        default_ttl_hours: 72,
        llm_assist_enabled: true,
        heartbeat_summary_enabled: true,
    }
}

#[test]
fn scheduler_plan_filters_disabled_users_and_uses_shortest_interval() {
    let plan = scheduler_plan_for_pending_users([
        OutcomePendingUserSettings {
            user_id: "enabled-slow".to_string(),
            settings: runtime_settings(true, true, 600),
        },
        OutcomePendingUserSettings {
            user_id: "learning-disabled".to_string(),
            settings: runtime_settings(false, true, 30),
        },
        OutcomePendingUserSettings {
            user_id: "outcomes-disabled".to_string(),
            settings: runtime_settings(true, false, 20),
        },
        OutcomePendingUserSettings {
            user_id: "enabled-fast".to_string(),
            settings: runtime_settings(true, true, 10),
        },
    ]);

    assert_eq!(plan.user_ids, ["enabled-slow", "enabled-fast"]);
    assert_eq!(plan.sleep_interval_secs, 10);
}

#[test]
fn evaluation_plan_sets_status_and_evaluator_metadata() {
    let mut contract = contract();
    let now = Utc::now();
    let score = OutcomeScore {
        verdict: VERDICT_NEGATIVE.to_string(),
        score: -0.8,
        details: json!({"strategy":"deterministic"}),
    };
    let plan = evaluated_contract_plan(&score, EVALUATOR_OUTCOME, now);
    apply_evaluated_contract_plan(&mut contract, &plan);

    assert_eq!(contract.status, STATUS_EVALUATED);
    assert_eq!(contract.final_verdict.as_deref(), Some(VERDICT_NEGATIVE));
    assert_eq!(contract.final_score, Some(-0.8));
    assert_eq!(contract.evaluated_at, Some(now));
    assert_eq!(
        contract_last_evaluator(&contract).as_deref(),
        Some(EVALUATOR_OUTCOME)
    );
}

#[test]
fn requeue_plan_only_applies_to_unfinished_claimed_contracts() {
    let mut contract = contract();
    contract.status = STATUS_EVALUATING.to_string();
    contract.claimed_at = Some(Utc::now());
    let now = Utc::now();
    let plan =
        failed_contract_requeue_plan(&contract, "temporary failure", now).expect("requeue plan");
    apply_failed_contract_requeue_plan(&mut contract, &plan);

    assert_eq!(contract.status, STATUS_OPEN);
    assert!(contract.claimed_at.is_none());
    assert_eq!(contract.updated_at, now);
    assert_eq!(
        contract
            .evaluation_details
            .get("last_error")
            .and_then(|value| value.as_str()),
        Some("temporary failure")
    );

    contract.status = STATUS_EVALUATED.to_string();
    contract.evaluated_at = Some(now);
    assert!(failed_contract_requeue_plan(&contract, "late failure", now).is_none());
}

#[test]
fn source_mapping_and_status_policy_match_root_observation_rules() {
    assert_eq!(
        feedback_target_source_kind("learning_event"),
        Some(SOURCE_LEARNING_EVENT)
    );
    assert_eq!(
        feedback_target_source_kind("artifact_version"),
        Some(SOURCE_ARTIFACT_VERSION)
    );
    assert_eq!(
        feedback_target_source_kind("code_proposal"),
        Some(SOURCE_CODE_PROPOSAL)
    );
    assert_eq!(feedback_target_source_kind("unknown"), None);

    let mut contract = contract();
    assert!(contract_accepts_observation(&contract));
    contract.status = STATUS_EVALUATED.to_string();
    assert!(!contract_accepts_observation(&contract));
    assert!(should_due_contract_for_observation_polarity(
        VERDICT_NEGATIVE
    ));
    assert!(!should_due_contract_for_observation_polarity(
        VERDICT_POSITIVE
    ));
}

#[test]
fn candidate_seed_and_builder_shape_repeated_negative_payload() {
    let mut contract = contract();
    contract.contract_type = CONTRACT_TOOL.to_string();
    contract.metadata = json!({
        "artifact_type": "memory",
        "artifact_name": paths::MEMORY,
        "pattern_key": "artifact:memory:MEMORY.md",
    });
    let score = OutcomeScore {
        verdict: VERDICT_NEGATIVE.to_string(),
        score: -0.9,
        details: json!({"strategy":"deterministic"}),
    };
    let seed = outcome_candidate_seed(&contract, &score, 2).expect("candidate seed");
    assert_eq!(seed.candidate_type, "memory");
    assert_eq!(seed.risk_tier, "low");
    assert_eq!(seed.supplement_kind, OutcomeCandidateSupplementKind::None);

    let observations = [observation("explicit_correction", VERDICT_NEGATIVE, 1.0)];
    let candidate = build_outcome_candidate(BuildOutcomeCandidateInput {
        id: Uuid::new_v4(),
        learning_event_id: Uuid::new_v4(),
        contract: &contract,
        score: &score,
        observations: &observations,
        seed: &seed,
        prompt_payload: None,
        created_at: Utc::now(),
    })
    .expect("candidate");
    assert_eq!(candidate.candidate_type, "memory");
    assert_eq!(
        candidate
            .proposal
            .get("dedupe_key")
            .and_then(|value| value.as_str()),
        Some(seed.dedupe_key.as_str())
    );
    assert_eq!(
        candidate
            .proposal
            .get("evidence")
            .and_then(|value| value.get("pattern_count"))
            .and_then(|value| value.as_u64()),
        Some(2)
    );
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

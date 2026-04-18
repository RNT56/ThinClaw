use std::sync::Arc;

use thinclaw::agent::outcomes::{self, OutcomeService};
use thinclaw::agent::routine::RunStatus;
use thinclaw::api::learning as learning_api;
use thinclaw::db::Database;
use thinclaw::history::ConversationKind;
use thinclaw::safety::SafetyLayer;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

async fn enable_outcomes(db: &Arc<dyn Database>, user_id: &str) {
    db.set_setting(user_id, "learning.enabled", &serde_json::json!(true))
        .await
        .expect("learning.enabled should be set");
    db.set_setting(
        user_id,
        "learning.outcomes.enabled",
        &serde_json::json!(true),
    )
    .await
    .expect("learning.outcomes.enabled should be set");
    db.set_setting(
        user_id,
        "learning.outcomes.evaluation_interval_secs",
        &serde_json::json!(1),
    )
    .await
    .expect("learning.outcomes.evaluation_interval_secs should be set");
    db.set_setting(
        user_id,
        "learning.outcomes.max_due_per_tick",
        &serde_json::json!(10),
    )
    .await
    .expect("learning.outcomes.max_due_per_tick should be set");
}

fn outcome_service(db: &Arc<dyn Database>) -> OutcomeService {
    OutcomeService::new(
        Arc::clone(db),
        None,
        Arc::new(SafetyLayer::new(&thinclaw::config::SafetyConfig::default())),
    )
}

#[tokio::test]
async fn conversation_message_flow_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("conversation_user");
    let channel = "repl";

    let conversation_id = ctx
        .db
        .create_conversation(channel, &user, Some("thread-contract"))
        .await
        .expect("create_conversation should succeed");
    assert_ne!(conversation_id, Uuid::nil());

    ctx.db
        .add_conversation_message(conversation_id, "user", "hello contract world")
        .await
        .expect("add_conversation_message should succeed");
    ctx.db
        .add_conversation_message_with_attribution(
            conversation_id,
            "assistant",
            "contract response",
            Some("assistant-1"),
            Some("Assistant"),
            Some("raw-id"),
            Some(&serde_json::json!({"source":"contract"})),
        )
        .await
        .expect("add_conversation_message_with_attribution should succeed");

    let messages = ctx
        .db
        .list_conversation_messages(conversation_id)
        .await
        .expect("list_conversation_messages should succeed");
    assert_eq!(messages.len(), 2);

    let (page, has_more) = ctx
        .db
        .list_conversation_messages_paginated(conversation_id, None, 1)
        .await
        .expect("list_conversation_messages_paginated should succeed");
    assert_eq!(page.len(), 1);
    assert!(has_more);

    let previews = ctx
        .db
        .list_conversations_with_preview(&user, channel, 10)
        .await
        .expect("list_conversations_with_preview should succeed");
    assert!(!previews.is_empty());

    let search_hits = ctx
        .db
        .search_conversation_messages(&user, "contract", None, Some(channel), None, 10)
        .await
        .expect("search_conversation_messages should succeed");
    assert!(!search_hits.is_empty(), "expected at least one search hit");

    ctx.db
        .update_conversation_identity(
            conversation_id,
            Some(&user),
            Some("actor-1"),
            Some(Uuid::new_v4()),
            ConversationKind::Direct,
            Some("repl:contract"),
        )
        .await
        .expect("update_conversation_identity should succeed");
}

#[tokio::test]
async fn legacy_direct_conversations_without_actor_are_listed_for_principal_actor() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("legacy_actor_user");
    let conversation_id = ctx
        .db
        .create_conversation("gateway", &user, Some("legacy-thread"))
        .await
        .expect("create_conversation should succeed");

    ctx.db
        .update_conversation_identity(
            conversation_id,
            Some(&user),
            None,
            Some(Uuid::new_v4()),
            ConversationKind::Direct,
            Some("gateway://direct/legacy"),
        )
        .await
        .expect("update_conversation_identity should succeed");

    let summaries = ctx
        .db
        .list_actor_conversations_for_recall(&user, &user, false, 10)
        .await
        .expect("list_actor_conversations_for_recall should succeed");

    assert!(
        summaries
            .iter()
            .any(|summary| summary.id == conversation_id)
    );
}

#[tokio::test]
async fn conversation_identity_repair_updates_principal_owner() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let placeholder_user = fixtures::user("placeholder_owner");
    let real_user = fixtures::user("repaired_owner");
    let conversation_id = ctx
        .db
        .create_conversation("gateway", &placeholder_user, Some("repair-thread"))
        .await
        .expect("create_conversation should succeed");

    ctx.db
        .update_conversation_identity(
            conversation_id,
            Some(&real_user),
            Some(&real_user),
            Some(Uuid::new_v4()),
            ConversationKind::Direct,
            Some("gateway://direct/repaired"),
        )
        .await
        .expect("update_conversation_identity should succeed");

    let summaries = ctx
        .db
        .list_actor_conversations_for_recall(&real_user, &real_user, false, 10)
        .await
        .expect("list_actor_conversations_for_recall should succeed");
    let belongs = ctx
        .db
        .conversation_belongs_to_actor(conversation_id, &real_user, &real_user)
        .await
        .expect("conversation_belongs_to_actor should succeed");

    assert!(belongs);
    assert!(
        summaries
            .iter()
            .any(|summary| summary.id == conversation_id)
    );
}

#[tokio::test]
async fn conversation_learning_flow_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("conversation_learning_user");
    let conversation_id = ctx
        .db
        .create_conversation("repl", &user, Some("learning-thread"))
        .await
        .expect("create_conversation should succeed");
    let message_id = ctx
        .db
        .add_conversation_message(conversation_id, "user", "learning candidate")
        .await
        .expect("add_conversation_message should succeed");

    let mut event = fixtures::learning_event(&user, Some(conversation_id), Some(message_id));
    event.thread_id = Some("learning-thread".to_string());
    let event_id = ctx
        .db
        .insert_learning_event(&event)
        .await
        .expect("insert_learning_event should succeed");

    let eval = fixtures::learning_evaluation(&user, event_id);
    ctx.db
        .insert_learning_evaluation(&eval)
        .await
        .expect("insert_learning_evaluation should succeed");

    let candidate = fixtures::learning_candidate(&user, event_id);
    let candidate_id = ctx
        .db
        .insert_learning_candidate(&candidate)
        .await
        .expect("insert_learning_candidate should succeed");

    let artifact = fixtures::learning_artifact_version(&user, candidate_id);
    ctx.db
        .insert_learning_artifact_version(&artifact)
        .await
        .expect("insert_learning_artifact_version should succeed");

    let proposal = fixtures::learning_code_proposal(&user, event_id);
    ctx.db
        .insert_learning_code_proposal(&proposal)
        .await
        .expect("insert_learning_code_proposal should succeed");

    let events = ctx
        .db
        .list_learning_events(&user, None, Some("repl"), Some("learning-thread"), 20)
        .await
        .expect("list_learning_events should succeed");
    assert!(events.iter().any(|entry| entry.id == event_id));

    let evals = ctx
        .db
        .list_learning_evaluations(&user, 20)
        .await
        .expect("list_learning_evaluations should succeed");
    assert!(
        evals
            .iter()
            .any(|entry| entry.learning_event_id == event_id)
    );

    let proposals = ctx
        .db
        .list_learning_code_proposals(&user, Some("proposed"), 20)
        .await
        .expect("list_learning_code_proposals should succeed");
    assert!(
        proposals
            .iter()
            .any(|entry| entry.learning_event_id == Some(event_id))
    );
}

#[tokio::test]
async fn outcome_contract_flow_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("outcome_contract_user");
    let contract = fixtures::outcome_contract(&user);
    let contract_id = ctx
        .db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");
    assert_ne!(contract_id, Uuid::nil());

    let fetched = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(fetched.contract_type, "turn_usefulness");

    let listed = ctx
        .db
        .list_outcome_contracts(&thinclaw::history::OutcomeContractQuery {
            user_id: user.clone(),
            actor_id: contract.actor_id.clone(),
            status: Some("open".to_string()),
            contract_type: Some("turn_usefulness".to_string()),
            source_kind: Some("learning_event".to_string()),
            source_id: Some(contract.source_id.clone()),
            thread_id: contract.thread_id.clone(),
            limit: 10,
        })
        .await
        .expect("list_outcome_contracts should succeed");
    assert_eq!(listed.len(), 1);

    let observation = fixtures::outcome_observation(contract_id);
    ctx.db
        .insert_outcome_observation(&observation)
        .await
        .expect("insert_outcome_observation should succeed");

    let observations = ctx
        .db
        .list_outcome_observations(contract_id)
        .await
        .expect("list_outcome_observations should succeed");
    assert_eq!(observations.len(), 1);

    let claimed = ctx
        .db
        .claim_due_outcome_contracts(10, chrono::Utc::now())
        .await
        .expect("claim_due_outcome_contracts should succeed");
    assert!(
        claimed.iter().any(|entry| entry.id == contract_id),
        "expected claimed due contract"
    );

    let stats = ctx
        .db
        .outcome_summary_stats(&user)
        .await
        .expect("outcome_summary_stats should succeed");
    assert!(stats.open >= 1);

    let pending_users = ctx
        .db
        .list_users_with_pending_outcome_work(chrono::Utc::now())
        .await
        .expect("list_users_with_pending_outcome_work should succeed");
    assert!(
        pending_users.iter().any(|entry| entry.user_id == user),
        "expected user with due outcome work to be listed"
    );

    let health = ctx
        .db
        .outcome_evaluator_health(&user, chrono::Utc::now())
        .await
        .expect("outcome_evaluator_health should succeed");
    assert!(
        health.oldest_due_at.is_some() || health.oldest_evaluating_claimed_at.is_some(),
        "expected outcome evaluator health markers for active outcome work"
    );
}

#[tokio::test]
async fn manual_outcome_review_reuses_learning_event_in_ledger() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("manual_outcome_review_event_user");
    let event = fixtures::learning_event(&user, None, None);
    let event_id = ctx
        .db
        .insert_learning_event(&event)
        .await
        .expect("insert_learning_event should succeed");

    let mut contract = fixtures::outcome_contract(&user);
    contract.source_id = event_id.to_string();
    let contract_id = ctx
        .db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");

    learning_api::review_outcome(&ctx.db, &user, contract_id, "confirm", Some("positive"))
        .await
        .expect("review_outcome should succeed");

    let updated = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    let event_id_string = event_id.to_string();
    assert_eq!(updated.status, "evaluated");
    assert_eq!(updated.final_verdict.as_deref(), Some("positive"));
    assert_eq!(
        updated
            .evaluation_details
            .get("ledger_learning_event_id")
            .and_then(|value| value.as_str()),
        Some(event_id_string.as_str())
    );

    let events = ctx
        .db
        .list_learning_events(&user, None, None, None, 10)
        .await
        .expect("list_learning_events should succeed");
    assert_eq!(
        events.len(),
        1,
        "manual review should reuse the source event"
    );

    let evaluations = ctx
        .db
        .list_learning_evaluations(&user, 10)
        .await
        .expect("list_learning_evaluations should succeed");
    let evaluation = evaluations
        .iter()
        .find(|entry| entry.evaluator == "outcome_manual_review_v1")
        .expect("manual review evaluation should exist");
    assert_eq!(evaluation.learning_event_id, event_id);
    assert_eq!(evaluation.status, "positive");
}

#[tokio::test]
async fn manual_outcome_review_requeue_creates_synthetic_learning_event() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("manual_outcome_review_synth_user");
    let mut contract = fixtures::outcome_contract(&user);
    contract.source_kind = "artifact_version".to_string();
    contract.source_id = Uuid::new_v4().to_string();
    contract.contract_type = "tool_durability".to_string();
    let contract_id = ctx
        .db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");

    learning_api::review_outcome(&ctx.db, &user, contract_id, "requeue", None)
        .await
        .expect("review_outcome should succeed");

    let updated = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(updated.status, "open");
    let ledger_event_id = updated
        .metadata
        .get("ledger_learning_event_id")
        .and_then(|value| value.as_str())
        .expect("manual review should store ledger event id");

    let events = ctx
        .db
        .list_learning_events(&user, None, None, None, 10)
        .await
        .expect("list_learning_events should succeed");
    let manual_event = events
        .iter()
        .find(|entry| entry.id.to_string() == ledger_event_id)
        .expect("manual review should create a synthetic learning event");
    assert_eq!(manual_event.source, "outcome_review::artifact_version");

    let evaluations = ctx
        .db
        .list_learning_evaluations(&user, 10)
        .await
        .expect("list_learning_evaluations should succeed");
    let evaluation = evaluations
        .iter()
        .find(|entry| entry.evaluator == "outcome_manual_review_v1")
        .expect("manual review evaluation should exist");
    assert_eq!(evaluation.learning_event_id.to_string(), ledger_event_id);
    assert_eq!(evaluation.status, "review");
}

#[tokio::test]
async fn repeated_manual_review_reuses_existing_ledger_event() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("manual_outcome_reuse_user");
    let mut contract = fixtures::outcome_contract(&user);
    contract.source_kind = "artifact_version".to_string();
    contract.source_id = Uuid::new_v4().to_string();
    contract.contract_type = "tool_durability".to_string();
    let contract_id = ctx
        .db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");

    learning_api::review_outcome(&ctx.db, &user, contract_id, "requeue", None)
        .await
        .expect("first review_outcome should succeed");
    let first = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    let ledger_event_id = first
        .metadata
        .get("ledger_learning_event_id")
        .and_then(|value| value.as_str())
        .expect("first review should persist ledger event id")
        .to_string();

    learning_api::review_outcome(&ctx.db, &user, contract_id, "dismiss", None)
        .await
        .expect("second review_outcome should succeed");
    let second = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(
        second
            .metadata
            .get("ledger_learning_event_id")
            .and_then(|value| value.as_str()),
        Some(ledger_event_id.as_str())
    );

    let events = ctx
        .db
        .list_learning_events(&user, None, None, None, 10)
        .await
        .expect("list_learning_events should succeed");
    assert_eq!(events.len(), 1, "should reuse the synthetic ledger event");

    let evaluations = ctx
        .db
        .list_learning_evaluations(&user, 10)
        .await
        .expect("list_learning_evaluations should succeed");
    let manual_reviews = evaluations
        .iter()
        .filter(|entry| entry.evaluator == "outcome_manual_review_v1")
        .collect::<Vec<_>>();
    assert_eq!(
        manual_reviews.len(),
        2,
        "each review should still be logged"
    );
    assert!(
        manual_reviews
            .iter()
            .all(|entry| entry.learning_event_id.to_string() == ledger_event_id)
    );
}

#[tokio::test]
async fn evaluate_now_processes_only_requested_user() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user_a = fixtures::user("outcome_eval_user_a");
    let user_b = fixtures::user("outcome_eval_user_b");
    enable_outcomes(&ctx.db, &user_a).await;
    enable_outcomes(&ctx.db, &user_b).await;

    let mut contract_a = fixtures::outcome_contract(&user_a);
    contract_a.source_kind = "artifact_version".to_string();
    contract_a.source_id = Uuid::new_v4().to_string();
    contract_a.contract_type = "tool_durability".to_string();
    contract_a.metadata = serde_json::json!({
        "pattern_key": "artifact:test-a",
        "artifact_type": "memory",
        "artifact_name": "MEMORY.md"
    });
    let mut contract_b = fixtures::outcome_contract(&user_b);
    contract_b.source_kind = "artifact_version".to_string();
    contract_b.source_id = Uuid::new_v4().to_string();
    contract_b.contract_type = "tool_durability".to_string();
    contract_b.metadata = serde_json::json!({
        "pattern_key": "artifact:test-b",
        "artifact_type": "memory",
        "artifact_name": "MEMORY.md"
    });
    let contract_a_id = ctx
        .db
        .insert_outcome_contract(&contract_a)
        .await
        .expect("insert_outcome_contract should succeed");
    let contract_b_id = ctx
        .db
        .insert_outcome_contract(&contract_b)
        .await
        .expect("insert_outcome_contract should succeed");
    ctx.db
        .insert_outcome_observation(&fixtures::outcome_observation(contract_a_id))
        .await
        .expect("insert_outcome_observation should succeed");
    ctx.db
        .insert_outcome_observation(&fixtures::outcome_observation(contract_b_id))
        .await
        .expect("insert_outcome_observation should succeed");

    let processed = outcome_service(&ctx.db)
        .run_once_for_user(&user_a)
        .await
        .expect("run_once_for_user should succeed");
    assert_eq!(processed, 1);

    let updated_a = ctx
        .db
        .get_outcome_contract(&user_a, contract_a_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    let updated_b = ctx
        .db
        .get_outcome_contract(&user_b, contract_b_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(updated_a.status, "evaluated");
    assert_eq!(updated_b.status, "open");
}

#[tokio::test]
async fn rollback_hook_drives_negative_durability_verdict() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("rollback_outcome_user");
    enable_outcomes(&ctx.db, &user).await;

    let event = fixtures::learning_event(&user, None, None);
    let event_id = ctx
        .db
        .insert_learning_event(&event)
        .await
        .expect("insert_learning_event should succeed");
    let candidate = fixtures::learning_candidate(&user, event_id);
    let candidate_id = ctx
        .db
        .insert_learning_candidate(&candidate)
        .await
        .expect("insert_learning_candidate should succeed");
    let mut version = fixtures::learning_artifact_version(&user, candidate_id);
    version.status = "applied".to_string();
    version.artifact_type = "memory".to_string();
    version.artifact_name = "MEMORY.md".to_string();
    ctx.db
        .insert_learning_artifact_version(&version)
        .await
        .expect("insert_learning_artifact_version should succeed");

    let contract_id = outcomes::maybe_create_artifact_contract(&ctx.db, &version)
        .await
        .expect("maybe_create_artifact_contract should succeed")
        .expect("artifact contract should be created");
    let rollback = thinclaw::history::LearningRollbackRecord {
        id: Uuid::new_v4(),
        user_id: user.clone(),
        artifact_type: version.artifact_type.clone(),
        artifact_name: version.artifact_name.clone(),
        artifact_version_id: Some(version.id),
        reason: "operator rollback".to_string(),
        metadata: serde_json::json!({}),
        created_at: chrono::Utc::now(),
    };
    outcomes::observe_rollback(&ctx.db, &rollback)
        .await
        .expect("observe_rollback should succeed");

    let processed = outcome_service(&ctx.db)
        .run_once_for_user(&user)
        .await
        .expect("run_once_for_user should succeed");
    assert_eq!(processed, 1);

    let updated = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(updated.status, "evaluated");
    assert_eq!(updated.final_verdict.as_deref(), Some("negative"));
}

#[tokio::test]
async fn proposal_rejection_hook_drives_negative_durability_verdict() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("proposal_outcome_user");
    enable_outcomes(&ctx.db, &user).await;

    let event = fixtures::learning_event(&user, None, None);
    let event_id = ctx
        .db
        .insert_learning_event(&event)
        .await
        .expect("insert_learning_event should succeed");
    let proposal = fixtures::learning_code_proposal(&user, event_id);
    ctx.db
        .insert_learning_code_proposal(&proposal)
        .await
        .expect("insert_learning_code_proposal should succeed");
    let contract_id = outcomes::maybe_create_proposal_contract(&ctx.db, &proposal)
        .await
        .expect("maybe_create_proposal_contract should succeed")
        .expect("proposal contract should be created");

    outcomes::observe_proposal_rejection(&ctx.db, &proposal, Some("operator rejected"))
        .await
        .expect("observe_proposal_rejection should succeed");

    let processed = outcome_service(&ctx.db)
        .run_once_for_user(&user)
        .await
        .expect("run_once_for_user should succeed");
    assert_eq!(processed, 1);

    let updated = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(updated.status, "evaluated");
    assert_eq!(updated.final_verdict.as_deref(), Some("negative"));
}

#[tokio::test]
async fn routine_state_change_hook_drives_negative_routine_verdict() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("routine_outcome_user");
    let actor = fixtures::actor_name("routine-owner");
    enable_outcomes(&ctx.db, &user).await;

    let mut routine = fixtures::routine(&user, &actor);
    routine.notify.on_success = true;
    ctx.db
        .create_routine(&routine)
        .await
        .expect("create_routine should succeed");

    let mut run = fixtures::routine_run(routine.id, RunStatus::Ok);
    run.result_summary = Some("No findings".to_string());
    ctx.db
        .create_routine_run(&run)
        .await
        .expect("create_routine_run should succeed");

    let contract_id = outcomes::maybe_create_routine_contract(&ctx.db, &routine, &run)
        .await
        .expect("maybe_create_routine_contract should succeed")
        .expect("routine contract should be created");

    outcomes::observe_routine_state_change(&ctx.db, &routine, "routine_muted")
        .await
        .expect("observe_routine_state_change should succeed");

    let processed = outcome_service(&ctx.db)
        .run_once_for_user(&user)
        .await
        .expect("run_once_for_user should succeed");
    assert_eq!(processed, 1);

    let updated = ctx
        .db
        .get_outcome_contract(&user, contract_id)
        .await
        .expect("get_outcome_contract should succeed")
        .expect("contract should exist");
    assert_eq!(updated.status, "evaluated");
    assert_eq!(updated.final_verdict.as_deref(), Some("negative"));
}

#[tokio::test]
async fn heartbeat_summary_only_appears_when_enabled_and_non_empty() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("heartbeat_outcome_user");
    assert_eq!(
        outcomes::heartbeat_review_summary(&ctx.db, &user)
            .await
            .expect("heartbeat_review_summary should succeed"),
        None
    );

    enable_outcomes(&ctx.db, &user).await;
    assert_eq!(
        outcomes::heartbeat_review_summary(&ctx.db, &user)
            .await
            .expect("heartbeat_review_summary should succeed"),
        None
    );

    let contract = fixtures::outcome_contract(&user);
    ctx.db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");
    let summary = outcomes::heartbeat_review_summary(&ctx.db, &user)
        .await
        .expect("heartbeat_review_summary should succeed");
    assert!(
        summary.is_some_and(|value| value.contains("Outcome Review Queue")),
        "expected heartbeat summary once enabled outcome work exists"
    );
}

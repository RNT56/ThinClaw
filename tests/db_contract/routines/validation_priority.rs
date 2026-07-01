//! routines: validation_priority.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use reqwest::StatusCode;
use serde_json::json;
use thinclaw::agent::routine::{
    RoutineAction, RoutineEvent, RoutineEventDecision, RoutineEventEvaluation, RoutineEventStatus,
    RunStatus, Trigger, content_hash,
};
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

use super::*;

#[tokio::test]
async fn routine_event_recent_content_match_honors_window_and_hash() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_dedup_match_user");
    let actor = fixtures::actor_name("routine_dedup_match");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Dedup probe")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("dedup-match-{}", Uuid::new_v4().simple()),
            "description": "Exercise content-hash dedup window query",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Handle the deploy event."
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    let fired_hash = content_hash("deploy now").to_string();
    let event = RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: user.clone(),
        actor_id: actor.clone(),
        channel: "slack".to_string(),
        event_type: String::new(),
        raw_sender_id: actor.clone(),
        conversation_scope_id: Uuid::new_v4().to_string(),
        stable_external_conversation_key: format!("test://slack/{user}/{actor}/dedup"),
        content: "deploy now".to_string(),
        content_hash: fired_hash.clone(),
        metadata: json!({"source": "dedup_match_test"}),
        idempotency_key: format!("dedup-match-{}", Uuid::new_v4().simple()),
        status: RoutineEventStatus::Processed,
        diagnostics: json!({"content_preview": "deploy now"}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: Some(Utc::now()),
        error_message: None,
        matched_routines: 1,
        fired_routines: 1,
        attempt_count: 1,
        created_at: Utc::now(),
    };
    ctx.db
        .create_routine_event(&event)
        .await
        .expect("event should insert");

    let evaluation = RoutineEventEvaluation {
        id: Uuid::new_v4(),
        event_id: event.id,
        routine_id,
        decision: RoutineEventDecision::Fired,
        reason: Some("fired".to_string()),
        details: json!({}),
        sequence_num: 0,
        channel: "slack".to_string(),
        content_preview: "deploy now".to_string(),
        created_at: Utc::now(),
    };
    ctx.db
        .upsert_routine_event_evaluation(&evaluation)
        .await
        .expect("evaluation should insert");

    // Matching hash within the window → true.
    let recent = Utc::now() - ChronoDuration::hours(1);
    assert!(
        ctx.db
            .routine_event_recent_content_match(routine_id, &fired_hash, recent)
            .await
            .expect("content match query should succeed"),
        "matching content within the window should be detected"
    );

    // Window starts after the fire → false.
    let future = Utc::now() + ChronoDuration::hours(1);
    assert!(
        !ctx.db
            .routine_event_recent_content_match(routine_id, &fired_hash, future)
            .await
            .expect("content match query should succeed"),
        "a fire before the window start must not match"
    );

    // Different content hash → false.
    let other_hash = content_hash("something else").to_string();
    assert!(
        !ctx.db
            .routine_event_recent_content_match(routine_id, &other_hash, recent)
            .await
            .expect("content match query should succeed"),
        "non-matching content hash must not match"
    );
}

#[tokio::test]
async fn toggle_and_delete_endpoints_refresh_event_cache() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_toggle_cache_user");
    let actor = fixtures::actor_name("routine_toggle_cache");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("toggle-cache-{}", Uuid::new_v4().simple());

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Cache refresh routine should run.")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));
    let addr =
        start_routine_gateway_server(Arc::clone(&ctx.db), &user, &actor, Arc::clone(&engine)).await;

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Exercise gateway toggle/delete cache refresh",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Handle the deploy event."
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    let client = reqwest::Client::new();
    let routine_url = format!("http://{addr}/api/routines/{routine_id}");

    let disable_response = client
        .post(format!("{routine_url}/toggle"))
        .header("Authorization", "Bearer routine-test-token")
        .json(&json!({ "enabled": false }))
        .send()
        .await
        .expect("toggle request should be sent");
    assert_eq!(disable_response.status(), StatusCode::OK);
    let disable_body = disable_response
        .json::<serde_json::Value>()
        .await
        .expect("toggle response should be JSON");
    assert_eq!(disable_body["status"], json!("disabled"));

    let fired_while_disabled = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired_while_disabled, 0);
    assert!(
        ctx.db
            .list_routine_runs(routine_id, 10)
            .await
            .expect("list_routine_runs should succeed")
            .is_empty()
    );

    let enable_response = client
        .post(format!("{routine_url}/toggle"))
        .header("Authorization", "Bearer routine-test-token")
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .expect("toggle request should be sent");
    assert_eq!(enable_response.status(), StatusCode::OK);
    let enable_body = enable_response
        .json::<serde_json::Value>()
        .await
        .expect("toggle response should be JSON");
    assert_eq!(enable_body["status"], json!("enabled"));

    let fired_after_enable = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired_after_enable, 1);
    let first_run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(first_run.status, RunStatus::Attention);

    let delete_response = client
        .delete(&routine_url)
        .header("Authorization", "Bearer routine-test-token")
        .send()
        .await
        .expect("delete request should be sent");
    assert_eq!(delete_response.status(), StatusCode::OK);
    let delete_body = delete_response
        .json::<serde_json::Value>()
        .await
        .expect("delete response should be JSON");
    assert_eq!(delete_body["status"], json!("deleted"));

    let fired_after_delete = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired_after_delete, 0);

    let stored_after_delete = ctx
        .db
        .get_routine(routine_id)
        .await
        .expect("get_routine should succeed");
    assert!(stored_after_delete.is_none());
}

#[tokio::test]
async fn webhook_routine_endpoint_runs_routine_and_enforces_signature() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_webhook_user");
    let actor = fixtures::actor_name("routine_webhook");
    let routine_name = format!("webhook-audit-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Webhook routine should run.")),
    );
    let addr =
        start_routine_gateway_server(Arc::clone(&ctx.db), &user, &actor, Arc::clone(&engine)).await;

    let client = reqwest::Client::new();
    let secret = "routine-webhook-secret";
    let mut webhook_routine = fixtures::routine(&user, &actor);
    webhook_routine.name = routine_name;
    webhook_routine.trigger = Trigger::Webhook {
        path: Some("/hooks/routine".into()),
        secret: Some(secret.to_string()),
        allow_unsigned_webhook: false,
    };
    webhook_routine.action = RoutineAction::Lightweight {
        prompt: "Summarize the webhook payload.".to_string(),
        context_paths: vec![],
        max_tokens: 256,
    };
    ctx.db
        .create_routine(&webhook_routine)
        .await
        .expect("webhook routine should be inserted");

    let body = serde_json::json!({ "event": "unit-test" , "payload": "tick-tock" });
    let body_bytes = serde_json::to_vec(&body).expect("serialize webhook body");
    let endpoint = format!("http://{}/hooks/routine/{}", addr, webhook_routine.id);

    let response = client
        .post(&endpoint)
        .header(
            "x-webhook-signature",
            webhook_signature(secret, &body_bytes),
        )
        .body(body_bytes.clone())
        .send()
        .await
        .expect("webhook request should be sent");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("webhook response should be JSON");
    assert_eq!(body["status"], "triggered");
    let run_id = parse_uuid(&body, "run_id");

    let run = wait_for_terminal_run(&ctx.db, webhook_routine.id).await;
    assert_eq!(run.id, run_id);
    // A webhook-triggered routine carries a payload, so the engine labels the run
    // "webhook" (distinct from a payload-less "manual" fire) — see
    // routine_engine::fire_manual_with_payload.
    assert_eq!(run.trigger_type, "webhook");
    assert_eq!(run.status, RunStatus::Attention);
    assert_eq!(
        run.result_summary.as_deref(),
        Some("Webhook routine should run.")
    );

    let stored_routine = ctx
        .db
        .get_routine(webhook_routine.id)
        .await
        .expect("get_routine should succeed")
        .expect("webhook routine should still exist");
    assert_eq!(stored_routine.run_count, 1);
    assert_eq!(stored_routine.consecutive_failures, 0);

    let notification = wait_for_notification(&mut notify_rx).await;
    assert_eq!(notification.metadata["source"], json!("routine"));
    assert_eq!(
        notification.metadata["routine_name"],
        serde_json::json!(webhook_routine.name)
    );
    assert_eq!(notification.metadata["status"], json!("attention"));

    let unauthorized = client
        .post(&endpoint)
        .body(body_bytes.clone())
        .send()
        .await
        .expect("webhook request should be sent");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let invalid_signature = client
        .post(&endpoint)
        .header("x-webhook-signature", "sha256=deadbeef")
        .body(body_bytes)
        .send()
        .await
        .expect("webhook request should be sent");
    assert_eq!(invalid_signature.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webhook_routine_rejects_oversized_payload_without_dispatch() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_webhook_user_large");
    let actor = fixtures::actor_name("routine_webhook_large");
    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Should not run.")),
    );
    let addr =
        start_routine_gateway_server(Arc::clone(&ctx.db), &user, &actor, Arc::clone(&engine)).await;

    let client = reqwest::Client::new();
    let mut oversized = fixtures::routine(&user, &actor);
    oversized.name = format!("webhook-oversize-{}", Uuid::new_v4().simple());
    oversized.trigger = Trigger::Webhook {
        path: None,
        secret: None,
        allow_unsigned_webhook: false,
    };
    ctx.db
        .create_routine(&oversized)
        .await
        .expect("oversized routine should be inserted");

    let endpoint = format!("http://{}/hooks/routine/{}", addr, oversized.id);
    let oversized_body = vec![b'a'; 70_000];
    let response = client
        .post(&endpoint)
        .body(oversized_body)
        .send()
        .await
        .expect("oversized webhook request should be sent");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let runs = ctx
        .db
        .list_routine_runs(oversized.id, 10)
        .await
        .expect("list_routine_runs should succeed");
    assert!(runs.is_empty());
}

#[tokio::test]
async fn failed_routine_runs_increment_failure_state_and_notify() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_failure_user");
    let actor = fixtures::actor_name("routine_failure");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("manual-failure-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::fail("simulated provider outage")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Catch provider failures",
            "trigger_type": "manual",
            "prompt": "Attempt the failing routine."
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    engine
        .fire_manual(routine_id)
        .await
        .expect("manual routine should dispatch");

    let failed_run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(failed_run.status, RunStatus::Failed);
    assert!(
        failed_run
            .result_summary
            .as_deref()
            .expect("failure summary should be present")
            .contains("simulated provider outage")
    );

    let stored_routine = ctx
        .db
        .get_routine(routine_id)
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    assert_eq!(stored_routine.run_count, 1);
    assert_eq!(stored_routine.consecutive_failures, 1);

    let history = execute_routine_tool(
        &registry,
        "routine_history",
        json!({
            "name": routine_name,
            "limit": 5
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(history["total_runs"], json!(1));
    assert_eq!(history["runs"][0]["status"], json!("failed"));

    let notification = wait_for_notification(&mut notify_rx).await;
    assert_eq!(notification.metadata["status"], json!("failed"));
    assert!(notification.content.contains("simulated provider outage"));
}

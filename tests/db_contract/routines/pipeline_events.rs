//! routines: pipeline_events.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use reqwest::StatusCode;
use serde_json::json;
use thinclaw::agent::routine::{RoutineEvent, RoutineEventStatus, RunStatus, content_hash};
use thinclaw::agent::routine_engine::spawn_cron_ticker;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

use super::*;

#[tokio::test]
async fn event_routines_are_scoped_to_the_message_owner() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user_a = fixtures::user("routine_owner_scope_a");
    let user_b = fixtures::user("routine_owner_scope_b");
    let owner_ctx_a = routine_test_context(&user_a, &user_a);
    let owner_ctx_b = routine_test_context(&user_b, &user_b);
    let routine_a_name = format!("owner-scope-a-{}", Uuid::new_v4().simple());
    let routine_b_name = format!("owner-scope-b-{}", Uuid::new_v4().simple());

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user_a,
        Arc::new(TestLlm::reply("Owner-scoped event observed.")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created_a = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_a_name,
            "description": "Only user A should match this event",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Inspect the deploy request for user A."
        }),
        &owner_ctx_a,
    )
    .await;
    let routine_a_id = parse_uuid(&created_a, "id");

    let created_b = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_b_name,
            "description": "Only user B should match this event",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Inspect the deploy request for user B."
        }),
        &owner_ctx_b,
    )
    .await;
    let routine_b_id = parse_uuid(&created_b, "id");

    let fired = engine
        .check_event_triggers(&owned_event_message(
            "slack",
            &user_a,
            &user_a,
            "deploy now",
        ))
        .await;
    assert_eq!(fired, 1);

    let routine_a_run = wait_for_terminal_run(&ctx.db, routine_a_id).await;
    assert_eq!(routine_a_run.status, RunStatus::Attention);
    assert_eq!(routine_a_run.trigger_type, "event");

    let routine_b_runs = ctx
        .db
        .list_routine_runs(routine_b_id, 10)
        .await
        .expect("list_routine_runs should succeed");
    assert!(routine_b_runs.is_empty());
}

#[tokio::test]
async fn event_routine_fires_for_bluebubbles_channel_payloads() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_bubbles_user");
    let actor = fixtures::actor_name("routine_event_bubbles");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("bluebubbles-event-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("BlueBubbles event observed")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "BlueBubbles event listener",
            "trigger_type": "event",
            "event_pattern": "bluebubbles ping",
            "event_channel": "bluebubbles",
            "prompt": "Process the incoming BlueBubbles event."
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(created["trigger_type"], json!("event"));
    let routine_id = parse_uuid(&created, "id");

    let fired = engine
        .check_event_triggers(&owned_event_message(
            "bluebubbles",
            &user,
            &actor,
            "bluebubbles ping now",
        ))
        .await;
    assert_eq!(fired, 1);

    let run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(run.status, RunStatus::Attention);
    assert_eq!(run.trigger_type, "event");
    assert_eq!(run.trigger_detail.as_deref(), Some("bluebubbles ping now"));

    let notification = wait_for_notification(&mut notify_rx).await;
    assert_eq!(
        notification.metadata["routine_name"],
        created["name"].clone()
    );
    assert_eq!(notification.metadata["status"], json!("attention"));
}

#[tokio::test]
async fn manual_routine_trigger_endpoint_executes_pipeline_end_to_end() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_manual_api_user");
    let actor = fixtures::actor_name("routine_manual_api");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("manual-api-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Manual API routine should run.")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));
    let addr =
        start_routine_gateway_server(Arc::clone(&ctx.db), &user, &actor, Arc::clone(&engine)).await;

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Trigger the manual routine through the gateway API",
            "trigger_type": "manual",
            "prompt": "Confirm the manual API trigger completed."
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/api/routines/{routine_id}/trigger"))
        .header("Authorization", "Bearer routine-test-token")
        .send()
        .await
        .expect("manual trigger request should be sent");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("manual trigger response should be JSON");
    assert_eq!(body["status"], json!("triggered"));
    assert_eq!(body["routine_id"], json!(routine_id));

    let run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(run.trigger_type, "manual");
    assert_eq!(run.status, RunStatus::Attention);
    assert_eq!(
        run.result_summary.as_deref(),
        Some("Manual API routine should run.")
    );

    let notification = wait_for_notification(&mut notify_rx).await;
    assert_eq!(notification.metadata["routine_name"], json!(routine_name));
    assert_eq!(notification.metadata["status"], json!("attention"));
}

#[tokio::test]
async fn event_routine_filters_by_channel_and_supports_wildcard() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_filter_user");
    let actor = fixtures::actor_name("routine_event_filter");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Event filter verified")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let specific = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("specific-{}", Uuid::new_v4().simple()),
            "description": "Only fire for discord",
            "trigger_type": "event",
            "event_pattern": "discord\\s+target",
            "event_channel": "discord",
            "prompt": "Discord specific routine.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let wildcard = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("wildcard-{}", Uuid::new_v4().simple()),
            "description": "Fire for any channel",
            "trigger_type": "event",
            "event_pattern": "global ping",
            "prompt": "Global routine.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let specific_id = parse_uuid(&specific, "id");
    let wildcard_id = parse_uuid(&wildcard, "id");

    let fired_discord = engine
        .check_event_triggers(&owned_event_message(
            "discord",
            &user,
            &actor,
            "a discord target global ping",
        ))
        .await;
    assert_eq!(fired_discord, 2);

    let specific_first_run = wait_for_terminal_run(&ctx.db, specific_id).await;
    let wildcard_first_run = wait_for_terminal_run(&ctx.db, wildcard_id).await;
    assert_eq!(specific_first_run.status, RunStatus::Attention);
    assert_eq!(wildcard_first_run.status, RunStatus::Attention);

    let fired_telegram = engine
        .check_event_triggers(&owned_event_message(
            "telegram",
            &user,
            &actor,
            "another global ping from telegram",
        ))
        .await;
    assert_eq!(fired_telegram, 1);

    let wildcard_second_run =
        wait_for_new_terminal_run(&ctx.db, wildcard_id, Some(wildcard_first_run.id)).await;
    assert_ne!(wildcard_first_run.id, wildcard_second_run.id);
}

#[tokio::test]
async fn event_routine_matching_covers_multiple_channel_names() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_channel_matrix_user");
    let actor = fixtures::actor_name("routine_event_channel_matrix");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Matrix routine fired")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let matrix_channels = [
        "bluebubbles",
        "discord",
        "gmail",
        "gateway",
        "http",
        "nostr",
        "repl",
        "signal",
    ];

    let mut channel_routines = Vec::with_capacity(matrix_channels.len());
    for channel in matrix_channels {
        let routine = execute_routine_tool(
            &registry,
            "routine_create",
            json!({
                "name": format!("matrix-{channel}-{}", Uuid::new_v4().simple()),
                "description": format!("Matrix routine for {channel}"),
                "trigger_type": "event",
                "event_pattern": "matrix check",
                "event_channel": channel,
                "prompt": format!("Handle matrix event from {channel}."),
                "cooldown_secs": 0
            }),
            &owner_ctx,
        )
        .await;
        channel_routines.push((channel.to_string(), parse_uuid(&routine, "id")));
    }

    let wildcard = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("matrix-wildcard-{}", Uuid::new_v4().simple()),
            "description": "Matrix wildcard routine",
            "trigger_type": "event",
            "event_pattern": "matrix check",
            "prompt": "Handle matrix event from any source.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let wildcard_id = parse_uuid(&wildcard, "id");

    let mut wildcard_last = None;
    for (index, (channel, routine_id)) in channel_routines.iter().enumerate() {
        let fired = engine
            .check_event_triggers(&owned_event_message(
                channel,
                &user,
                &actor,
                "matrix check is active",
            ))
            .await;
        assert_eq!(
            fired, 2,
            "both wildcard and {channel} routine should fire on hit {index}"
        );

        let specific_run = wait_for_terminal_run(&ctx.db, *routine_id).await;
        assert_eq!(specific_run.status, RunStatus::Attention);
        assert_eq!(
            specific_run.trigger_detail.as_deref(),
            Some("matrix check is active")
        );

        let next_wildcard_run = if let Some(previous_id) = wildcard_last {
            wait_for_new_terminal_run(&ctx.db, wildcard_id, Some(previous_id)).await
        } else {
            wait_for_terminal_run(&ctx.db, wildcard_id).await
        };
        assert_eq!(next_wildcard_run.status, RunStatus::Attention);
        assert_eq!(
            next_wildcard_run.trigger_detail.as_deref(),
            Some("matrix check is active")
        );
        wildcard_last = Some(next_wildcard_run.id);
    }

    let unrelated_fired = engine
        .check_event_triggers(&owned_event_message(
            "webhook",
            &user,
            &actor,
            "matrix check is active",
        ))
        .await;
    assert_eq!(unrelated_fired, 1);

    let wildcard_run_after_unrelated = if let Some(previous_id) = wildcard_last {
        wait_for_new_terminal_run(&ctx.db, wildcard_id, Some(previous_id)).await
    } else {
        wait_for_terminal_run(&ctx.db, wildcard_id).await
    };
    assert_eq!(wildcard_run_after_unrelated.status, RunStatus::Attention);
    assert_eq!(
        wildcard_run_after_unrelated.trigger_detail.as_deref(),
        Some("matrix check is active")
    );
}

#[tokio::test]
async fn routine_event_gateway_exposes_recent_checks_and_activity() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_gateway_user");
    let actor = fixtures::actor_name("routine_event_gateway");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("gateway-event-{}", Uuid::new_v4().simple());

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Gateway event observed")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));
    let addr =
        start_routine_gateway_server(Arc::clone(&ctx.db), &user, &actor, Arc::clone(&engine)).await;

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Expose event diagnostics through the gateway",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "event_priority": 25,
            "prompt": "Handle the gateway event.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    let fired = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired, 1);
    let run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(run.status, RunStatus::Attention);

    let missed = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "status ping"))
        .await;
    assert_eq!(missed, 0);

    let client = reqwest::Client::new();
    let detail = client
        .get(format!("http://{addr}/api/routines/{routine_id}"))
        .header("Authorization", "Bearer routine-test-token")
        .send()
        .await
        .expect("detail request should be sent");
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_body = detail
        .json::<serde_json::Value>()
        .await
        .expect("detail response should be JSON");
    let checks = detail_body["recent_event_checks"]
        .as_array()
        .expect("recent_event_checks should be present");
    assert!(checks.iter().any(|check| check["decision"] == "fired"));
    assert!(
        checks
            .iter()
            .any(|check| check["decision"] == "ignored_pattern")
    );

    let activity = client
        .get(format!("http://{addr}/api/routines/events"))
        .header("Authorization", "Bearer routine-test-token")
        .send()
        .await
        .expect("activity request should be sent");
    assert_eq!(activity.status(), StatusCode::OK);
    let activity_body = activity
        .json::<serde_json::Value>()
        .await
        .expect("activity response should be JSON");
    let events = activity_body["events"]
        .as_array()
        .expect("events should be an array");
    assert!(events.iter().any(|event| event["fired_routines"] == 1));
    assert!(events.iter().any(|event| event["fired_routines"] == 0));
}

#[tokio::test]
async fn pending_event_queue_is_drained_on_startup_ticker() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_replay_user");
    let actor = fixtures::actor_name("routine_event_replay");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Replay event observed")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("replay-{}", Uuid::new_v4().simple()),
            "description": "Replay pending event inbox work on startup",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Handle replayed event.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    let queued_event = RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: user.clone(),
        actor_id: actor.clone(),
        channel: "slack".to_string(),
        event_type: String::new(),
        raw_sender_id: actor.clone(),
        conversation_scope_id: Uuid::new_v4().to_string(),
        stable_external_conversation_key: format!("test://slack/{user}/{actor}/replay"),
        content: "deploy now".to_string(),
        content_hash: content_hash("deploy now").to_string(),
        metadata: json!({"source": "startup_replay_test"}),
        idempotency_key: "startup-replay-test".to_string(),
        status: RoutineEventStatus::Pending,
        diagnostics: json!({"content_preview": "deploy now"}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: Utc::now(),
    };
    ctx.db
        .create_routine_event(&queued_event)
        .await
        .expect("pending routine event should be inserted");

    let handle = spawn_cron_ticker(Arc::clone(&engine), Duration::from_secs(60));
    let run = wait_for_terminal_run(&ctx.db, routine_id).await;
    handle.abort();

    assert_eq!(run.trigger_type, "event");
    assert_eq!(run.status, RunStatus::Attention);
    assert_eq!(run.result_summary.as_deref(), Some("Replay event observed"));

    let replayed_event = ctx
        .db
        .list_routine_events_for_actor(&user, &actor, 10)
        .await
        .expect("replayed event should be queryable")
        .into_iter()
        .find(|event| event.id == queued_event.id)
        .expect("queued event should still exist");
    assert_eq!(replayed_event.status.to_string(), "processed");
    assert_eq!(replayed_event.matched_routines, 1);
    assert_eq!(replayed_event.fired_routines, 1);
}

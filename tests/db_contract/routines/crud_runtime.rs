//! routines: crud_runtime.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use thinclaw::agent::routine::{RoutineAction, RunStatus, Trigger};
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

use super::*;

#[tokio::test]
async fn routine_crud_and_runtime_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_user");
    let actor = fixtures::actor_name("routine");
    let routine = fixtures::routine(&user, &actor);

    ctx.db
        .create_routine(&routine)
        .await
        .expect("create_routine should succeed");

    let loaded = ctx
        .db
        .get_routine(routine.id)
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    assert_eq!(loaded.name, routine.name);

    let by_name = ctx
        .db
        .get_routine_by_name(&user, &routine.name)
        .await
        .expect("get_routine_by_name should succeed");
    assert!(by_name.is_some());

    // Default helper methods for actor filtering should work.
    let by_name_actor = ctx
        .db
        .get_routine_by_name_for_actor(&user, &actor, &routine.name)
        .await
        .expect("get_routine_by_name_for_actor should succeed");
    assert!(by_name_actor.is_some());

    let actor_list = ctx
        .db
        .list_routines_for_actor(&user, &actor)
        .await
        .expect("list_routines_for_actor should succeed");
    assert_eq!(actor_list.len(), 1);

    let now = Utc::now();
    ctx.db
        .update_routine_runtime(
            routine.id,
            now,
            Some(now),
            1,
            0,
            &serde_json::json!({"ok":true}),
        )
        .await
        .expect("update_routine_runtime should succeed");
}

#[tokio::test]
async fn routine_runs_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("routine_runs_user");
    let actor = fixtures::actor_name("routine_runs");
    let routine = fixtures::routine(&user, &actor);

    ctx.db
        .create_routine(&routine)
        .await
        .expect("create_routine should succeed");

    let run = fixtures::routine_run(routine.id, RunStatus::Running);
    ctx.db
        .create_routine_run(&run)
        .await
        .expect("create_routine_run should succeed");

    let running_before = ctx
        .db
        .count_running_routine_runs(routine.id)
        .await
        .expect("count_running_routine_runs should succeed");
    assert_eq!(running_before, 1);

    ctx.db
        .complete_routine_run(run.id, RunStatus::Ok, Some("done"), Some(123))
        .await
        .expect("complete_routine_run should succeed");

    let runs = ctx
        .db
        .list_routine_runs(routine.id, 10)
        .await
        .expect("list_routine_runs should succeed");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Ok);

    let running_after = ctx
        .db
        .count_running_routine_runs(routine.id)
        .await
        .expect("count_running_routine_runs should succeed");
    assert_eq!(running_after, 0);

    let deleted = ctx
        .db
        .delete_routine(routine.id)
        .await
        .expect("delete_routine should succeed");
    assert!(deleted);
}

#[tokio::test]
async fn routine_tool_lifecycle_preserves_actor_isolation() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_pipeline_user");
    let actor = fixtures::actor_name("routine_pipeline");
    let other_actor = fixtures::actor_name("routine_pipeline_other");
    let owner_ctx = routine_test_context(&user, &actor);
    let other_ctx = routine_test_context(&user, &other_actor);
    let routine_name = format!("user-generated-{}", Uuid::new_v4().simple());

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("ROUTINE_OK")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), engine);

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Track inbound requests",
            "trigger_type": "manual",
            "prompt": "Summarize the queue."
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(created["status"], json!("created"));
    assert_eq!(created["trigger_type"], json!("manual"));

    let owner_list = execute_routine_tool(&registry, "routine_list", json!({}), &owner_ctx).await;
    assert_eq!(owner_list["count"], json!(1));
    assert_eq!(owner_list["routines"][0]["name"], json!(routine_name));

    let other_list = execute_routine_tool(&registry, "routine_list", json!({}), &other_ctx).await;
    assert_eq!(other_list["count"], json!(0));

    let initial_history = execute_routine_tool(
        &registry,
        "routine_history",
        json!({
            "name": routine_name,
            "limit": 5
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(initial_history["total_runs"], json!(0));
    assert_eq!(
        initial_history["runs"]
            .as_array()
            .expect("runs should be an array")
            .len(),
        0
    );

    let updated = execute_routine_tool(
        &registry,
        "routine_update",
        json!({
            "name": routine_name,
            "description": "Track inbound requests with a schedule",
            "prompt": "Check for overdue requests.",
            "schedule": "*/15 * * * *"
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(updated["status"], json!("updated"));
    assert_eq!(updated["trigger_type"], json!("cron"));
    assert!(updated["next_fire_at"].as_str().is_some());

    let stored_routine = ctx
        .db
        .get_routine(parse_uuid(&created, "id"))
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    assert_eq!(stored_routine.owner_actor_id(), actor);
    assert_eq!(
        stored_routine.description,
        "Track inbound requests with a schedule"
    );
    match &stored_routine.action {
        RoutineAction::Lightweight { prompt, .. } => {
            assert_eq!(prompt, "Check for overdue requests.");
        }
        other => panic!("expected lightweight routine, got {other:?}"),
    }
    match &stored_routine.trigger {
        Trigger::Cron { schedule } => assert_eq!(schedule, "0 */15 * * * * *"),
        other => panic!("expected cron trigger, got {other:?}"),
    }

    let interval_updated = execute_routine_tool(
        &registry,
        "routine_update",
        json!({
            "name": routine_name,
            "schedule": "0 */213 * * * * *"
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(interval_updated["status"], json!("updated"));
    assert_eq!(interval_updated["trigger_type"], json!("cron"));
    assert!(interval_updated["next_fire_at"].as_str().is_some());

    let interval_routine = ctx
        .db
        .get_routine(parse_uuid(&created, "id"))
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    match &interval_routine.trigger {
        Trigger::Cron { schedule } => assert_eq!(schedule, "every 213m"),
        other => panic!("expected scheduled trigger, got {other:?}"),
    }

    let deleted = execute_routine_tool(
        &registry,
        "routine_delete",
        json!({ "name": routine_name }),
        &owner_ctx,
    )
    .await;
    assert_eq!(deleted["deleted"], json!(true));

    let final_list = execute_routine_tool(&registry, "routine_list", json!({}), &owner_ctx).await;
    assert_eq!(final_list["count"], json!(0));
}

#[tokio::test]
async fn manual_routine_pipeline_records_history_and_notifications() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_manual_user");
    let actor = fixtures::actor_name("routine_manual");
    let owner_ctx = routine_test_context(&user, &actor);
    let routine_name = format!("manual-audit-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Investigate the stuck deployment.")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": routine_name,
            "description": "Escalate deployment drift",
            "trigger_type": "manual",
            "prompt": "Review the latest deployment state."
        }),
        &owner_ctx,
    )
    .await;
    let routine_id = parse_uuid(&created, "id");

    engine
        .fire_manual(routine_id)
        .await
        .expect("manual routine should dispatch");

    let completed_run = wait_for_terminal_run(&ctx.db, routine_id).await;
    assert_eq!(completed_run.status, RunStatus::Attention);
    assert_eq!(
        completed_run.result_summary.as_deref(),
        Some("Investigate the stuck deployment.")
    );
    assert_eq!(completed_run.tokens_used, Some(15));

    let stored_routine = ctx
        .db
        .get_routine(routine_id)
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    assert_eq!(stored_routine.run_count, 1);
    assert_eq!(stored_routine.consecutive_failures, 0);
    assert!(stored_routine.last_run_at.is_some());

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
    assert_eq!(history["runs"][0]["status"], json!("attention"));
    assert_eq!(
        history["runs"][0]["result_summary"],
        json!("Investigate the stuck deployment.")
    );
    assert_eq!(history["runs"][0]["tokens_used"], json!(15));

    let notification = wait_for_notification(&mut notify_rx).await;
    assert!(notification.content.contains(&routine_name));
    assert!(
        notification
            .content
            .contains("Investigate the stuck deployment.")
    );
    assert_eq!(notification.metadata["source"], json!("routine"));
    assert_eq!(notification.metadata["routine_name"], json!(routine_name));
    assert_eq!(notification.metadata["status"], json!("attention"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn event_and_cron_routines_execute_end_to_end() {
    // Cron-triggered fires go through IC-CRON-STAGGER jitter (up to 30s by
    // default), which would starve this test's polling window. Pin it to 0;
    // `lock_env` serializes against other env-mutating tests in the binary.
    let _env_guard = thinclaw_config::helpers::lock_env();
    unsafe {
        std::env::set_var("CRON_STAGGER_SECS", "0");
    }

    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_trigger_user");
    let actor = fixtures::actor_name("routine_trigger");
    let owner_ctx = routine_test_context(&user, &actor);
    let event_name = format!("event-watch-{}", Uuid::new_v4().simple());
    let cron_name = format!("cron-watch-{}", Uuid::new_v4().simple());

    let (engine, mut notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Review the deployment logs.")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let event_created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": event_name,
            "description": "Catch deploy messages",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "prompt": "Inspect the deployment request."
        }),
        &owner_ctx,
    )
    .await;
    let event_id = parse_uuid(&event_created, "id");

    let cron_created = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": cron_name,
            "description": "Run every fifteen minutes",
            "trigger_type": "cron",
            "schedule": "*/15 * * * *",
            "prompt": "Inspect the scheduled deployment check."
        }),
        &owner_ctx,
    )
    .await;
    let cron_id = parse_uuid(&cron_created, "id");

    let mut cron_routine = ctx
        .db
        .get_routine(cron_id)
        .await
        .expect("get_routine should succeed")
        .expect("cron routine should exist");
    cron_routine.next_fire_at = Some(Utc::now() - ChronoDuration::minutes(1));
    ctx.db
        .update_routine(&cron_routine)
        .await
        .expect("update_routine should succeed");

    let fired = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired, 1);

    let event_run = wait_for_terminal_run(&ctx.db, event_id).await;
    assert_eq!(event_run.status, RunStatus::Attention);
    assert_eq!(event_run.trigger_type, "event");
    assert_eq!(
        event_run.result_summary.as_deref(),
        Some("Review the deployment logs.")
    );
    assert_eq!(event_run.trigger_detail.as_deref(), Some("deploy now"));

    engine.check_cron_triggers().await;

    let cron_run = wait_for_terminal_run(&ctx.db, cron_id).await;
    assert_eq!(cron_run.status, RunStatus::Attention);
    assert_eq!(cron_run.trigger_type, "cron");
    assert_eq!(
        cron_run.result_summary.as_deref(),
        Some("Review the deployment logs.")
    );
    assert_eq!(cron_run.trigger_detail.as_deref(), Some("0 */15 * * * * *"));

    let cron_after = ctx
        .db
        .get_routine(cron_id)
        .await
        .expect("get_routine should succeed")
        .expect("cron routine should still exist");
    assert_eq!(cron_after.run_count, 1);
    assert!(
        cron_after
            .next_fire_at
            .expect("next_fire_at should be updated")
            > Utc::now() - ChronoDuration::seconds(1)
    );

    let event_history = execute_routine_tool(
        &registry,
        "routine_history",
        json!({
            "name": event_name,
            "limit": 5
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(event_history["runs"][0]["trigger_type"], json!("event"));

    let cron_history = execute_routine_tool(
        &registry,
        "routine_history",
        json!({
            "name": cron_name,
            "limit": 5
        }),
        &owner_ctx,
    )
    .await;
    assert_eq!(cron_history["runs"][0]["trigger_type"], json!("cron"));

    let mut notified_names = vec![
        wait_for_notification(&mut notify_rx).await.metadata["routine_name"]
            .as_str()
            .expect("routine_name should be present")
            .to_string(),
        wait_for_notification(&mut notify_rx).await.metadata["routine_name"]
            .as_str()
            .expect("routine_name should be present")
            .to_string(),
    ];
    notified_names.sort();
    assert_eq!(notified_names, vec![cron_name, event_name]);
    unsafe {
        std::env::remove_var("CRON_STAGGER_SECS");
    }
}

#[tokio::test]
async fn event_routine_validation_and_warnings_flow_end_to_end() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_validation_user");
    let actor = fixtures::actor_name("routine_event_validation");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Validation warnings recorded")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let warning_result = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("warning-{}", Uuid::new_v4().simple()),
            "description": "Broad event routine for warning coverage",
            "trigger_type": "event",
            "event_pattern": ".*",
            "prompt": "Observe all messages."
        }),
        &owner_ctx,
    )
    .await;

    let warnings = warning_result["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert!(
        warnings
            .iter()
            .any(|warning| warning == "matches all channels")
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning == "pattern is extremely broad and may fire on most messages")
    );

    let invalid_pattern = "a".repeat(600);
    let error = execute_routine_tool_error(
        &registry,
        "routine_create",
        json!({
            "name": format!("invalid-{}", Uuid::new_v4().simple()),
            "description": "Invalid event routine",
            "trigger_type": "event",
            "event_pattern": invalid_pattern,
            "prompt": "Should fail."
        }),
        &owner_ctx,
    )
    .await;
    assert!(error.contains("pattern exceeds"));
}

#[tokio::test]
async fn event_routine_priority_and_diagnostics_are_persisted() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_event_priority_user");
    let actor = fixtures::actor_name("routine_event_priority");
    let owner_ctx = routine_test_context(&user, &actor);

    let (engine, _notify_rx) = build_routine_engine(
        Arc::clone(&ctx.db),
        &user,
        Arc::new(TestLlm::reply("Priority pipeline observed")),
    );
    let registry = build_registry(Arc::clone(&ctx.db), Arc::clone(&engine));

    let high = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("priority-high-{}", Uuid::new_v4().simple()),
            "description": "High priority match",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "event_priority": 100,
            "prompt": "Handle the high priority deploy event.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let middle = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("priority-middle-{}", Uuid::new_v4().simple()),
            "description": "Middle priority non-match",
            "trigger_type": "event",
            "event_pattern": "deploy later",
            "event_channel": "slack",
            "event_priority": 50,
            "prompt": "Handle the delayed deploy event.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;
    let low = execute_routine_tool(
        &registry,
        "routine_create",
        json!({
            "name": format!("priority-low-{}", Uuid::new_v4().simple()),
            "description": "Low priority match",
            "trigger_type": "event",
            "event_pattern": "deploy now",
            "event_channel": "slack",
            "event_priority": 0,
            "prompt": "Handle the low priority deploy event.",
            "cooldown_secs": 0
        }),
        &owner_ctx,
    )
    .await;

    let high_id = parse_uuid(&high, "id");
    let middle_id = parse_uuid(&middle, "id");
    let low_id = parse_uuid(&low, "id");

    let fired = engine
        .check_event_triggers(&owned_event_message("slack", &user, &actor, "deploy now"))
        .await;
    assert_eq!(fired, 2);

    let high_run = wait_for_terminal_run(&ctx.db, high_id).await;
    let low_run = wait_for_terminal_run(&ctx.db, low_id).await;
    assert_eq!(high_run.status, RunStatus::Attention);
    assert_eq!(low_run.status, RunStatus::Attention);

    let high_eval = ctx
        .db
        .list_routine_event_evaluations(high_id, 5)
        .await
        .expect("high priority evaluations should load")
        .into_iter()
        .next()
        .expect("high priority evaluation should exist");
    let middle_eval = ctx
        .db
        .list_routine_event_evaluations(middle_id, 5)
        .await
        .expect("middle priority evaluations should load")
        .into_iter()
        .next()
        .expect("middle priority evaluation should exist");
    let low_eval = ctx
        .db
        .list_routine_event_evaluations(low_id, 5)
        .await
        .expect("low priority evaluations should load")
        .into_iter()
        .next()
        .expect("low priority evaluation should exist");

    assert_eq!(high_eval.decision.to_string(), "fired");
    assert_eq!(middle_eval.decision.to_string(), "ignored_pattern");
    assert_eq!(low_eval.decision.to_string(), "fired");
    assert!(high_eval.sequence_num < middle_eval.sequence_num);
    assert!(middle_eval.sequence_num < low_eval.sequence_num);

    let event = ctx
        .db
        .list_routine_events_for_actor(&user, &actor, 10)
        .await
        .expect("event inbox should load")
        .into_iter()
        .next()
        .expect("event inbox row should exist");
    assert_eq!(event.status.to_string(), "processed");
    assert_eq!(event.matched_routines, 2);
    assert_eq!(event.fired_routines, 2);
    assert_eq!(event.diagnostics["decision_counts"]["fired"], json!(2));
    assert_eq!(
        event.diagnostics["decision_counts"]["ignored_pattern"],
        json!(1)
    );
    assert_eq!(event.diagnostics["identity_mismatch_count"], json!(0));
}

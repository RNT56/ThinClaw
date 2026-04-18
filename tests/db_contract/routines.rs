use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use sha2::Sha256;
use thinclaw::agent::routine::{RoutineAction, RunStatus, Trigger};
use thinclaw::agent::routine_engine::RoutineEngine;
use thinclaw::channels::web::server::{GatewayState, start_server};
use thinclaw::channels::web::sse::SseManager;
use thinclaw::channels::web::ws::WsConnectionTracker;
use thinclaw::channels::{IncomingMessage, OutgoingResponse};
use thinclaw::config::RoutineConfig;
use thinclaw::context::JobContext;
use thinclaw::db::Database;
use thinclaw::error::LlmError;
use thinclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use thinclaw::tools::ToolRegistry;
use thinclaw::workspace::Workspace;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

enum TestLlmMode {
    Reply(String),
    Fail(String),
}

struct TestLlm {
    mode: TestLlmMode,
}

impl TestLlm {
    fn reply(text: impl Into<String>) -> Self {
        Self {
            mode: TestLlmMode::Reply(text.into()),
        }
    }

    fn fail(reason: impl Into<String>) -> Self {
        Self {
            mode: TestLlmMode::Fail(reason.into()),
        }
    }
}

#[async_trait]
impl LlmProvider for TestLlm {
    fn model_name(&self) -> &str {
        "routine-test-llm"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        match &self.mode {
            TestLlmMode::Reply(text) => Ok(CompletionResponse {
                content: text.clone(),
                provider_model: None,
                cost_usd: None,
                thinking_content: None,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
            }),
            TestLlmMode::Fail(reason) => Err(LlmError::RequestFailed {
                provider: self.model_name().to_string(),
                reason: reason.clone(),
            }),
        }
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        match &self.mode {
            TestLlmMode::Reply(text) => Ok(ToolCompletionResponse {
                content: Some(text.clone()),
                provider_model: None,
                cost_usd: None,
                tool_calls: vec![],
                thinking_content: None,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
            }),
            TestLlmMode::Fail(reason) => Err(LlmError::RequestFailed {
                provider: self.model_name().to_string(),
                reason: reason.clone(),
            }),
        }
    }
}

fn routine_test_context(user_id: &str, actor_id: &str) -> JobContext {
    JobContext::with_user_and_actor(
        user_id.to_string(),
        actor_id.to_string(),
        "routine contract test",
        "exercise the routine pipeline",
    )
}

fn build_routine_engine(
    db: Arc<dyn Database>,
    user_id: &str,
    llm: Arc<dyn LlmProvider>,
) -> (Arc<RoutineEngine>, mpsc::Receiver<OutgoingResponse>) {
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let (notify_tx, notify_rx) = mpsc::channel(8);
    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        db,
        llm,
        workspace,
        notify_tx,
        None,
    ));
    (engine, notify_rx)
}

fn build_registry(db: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Arc<ToolRegistry> {
    let registry = Arc::new(ToolRegistry::new());
    registry.register_routine_tools(db, engine);
    registry
}

async fn execute_routine_tool(
    registry: &Arc<ToolRegistry>,
    tool_name: &str,
    params: serde_json::Value,
    ctx: &JobContext,
) -> serde_json::Value {
    let tool = registry
        .get(tool_name)
        .await
        .unwrap_or_else(|| panic!("tool should be registered: {tool_name}"));
    tool.execute(params, ctx)
        .await
        .unwrap_or_else(|err| panic!("tool execution should succeed for {tool_name}: {err}"))
        .result
}

fn parse_uuid(value: &serde_json::Value, field: &str) -> Uuid {
    Uuid::parse_str(
        value
            .get(field)
            .and_then(|entry| entry.as_str())
            .unwrap_or_else(|| panic!("missing string field: {field}")),
    )
    .unwrap_or_else(|err| panic!("field {field} should be a UUID: {err}"))
}

async fn wait_for_terminal_run(
    db: &Arc<dyn Database>,
    routine_id: Uuid,
) -> thinclaw::agent::routine::RoutineRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let runs = db
            .list_routine_runs(routine_id, 10)
            .await
            .expect("list_routine_runs should succeed");
        if let Some(run) = runs.into_iter().next()
            && run.status != RunStatus::Running
        {
            return run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for routine run {routine_id} to finish"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_new_terminal_run(
    db: &Arc<dyn Database>,
    routine_id: Uuid,
    previous_run_id: Option<Uuid>,
) -> thinclaw::agent::routine::RoutineRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let runs = db
            .list_routine_runs(routine_id, 20)
            .await
            .expect("list_routine_runs should succeed");

        for run in runs {
            if Some(run.id) == previous_run_id || run.status == RunStatus::Running {
                continue;
            }
            return run;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for routine {routine_id} to emit a new terminal run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_notification(rx: &mut mpsc::Receiver<OutgoingResponse>) -> OutgoingResponse {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("notification should arrive before timeout")
        .expect("notification channel should stay open")
}

fn webhook_signature(secret: &str, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac secret should be accepted");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

async fn start_routine_gateway_server(
    db: Arc<dyn Database>,
    user_id: &str,
    actor_id: &str,
    engine: Arc<RoutineEngine>,
) -> SocketAddr {
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(db),
        job_manager: None,
        prompt_queue: None,
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        routine_engine: Some(engine),
        secrets_store: None,
        channel_manager: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    start_server(addr, state, "routine-test-token".to_string(), vec![])
        .await
        .expect("routine webhook server should start")
}

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
async fn event_and_cron_routines_execute_end_to_end() {
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
        .check_event_triggers(&IncomingMessage::new("slack", &user, "deploy now"))
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
        .check_event_triggers(&IncomingMessage::new(
            "bluebubbles",
            &user,
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
        .check_event_triggers(&IncomingMessage::new(
            "discord",
            &user,
            "a discord target global ping",
        ))
        .await;
    assert_eq!(fired_discord, 2);

    let specific_first_run = wait_for_terminal_run(&ctx.db, specific_id).await;
    let wildcard_first_run = wait_for_terminal_run(&ctx.db, wildcard_id).await;
    assert_eq!(specific_first_run.status, RunStatus::Attention);
    assert_eq!(wildcard_first_run.status, RunStatus::Attention);

    let fired_telegram = engine
        .check_event_triggers(&IncomingMessage::new(
            "telegram",
            &user,
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
            .check_event_triggers(&IncomingMessage::new(
                channel,
                &user,
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
        .check_event_triggers(&IncomingMessage::new(
            "webhook",
            &user,
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
    assert_eq!(run.trigger_type, "manual");
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

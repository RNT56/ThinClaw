#[cfg(feature = "libsql")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "libsql")]
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
#[cfg(feature = "libsql")]
use rust_decimal::Decimal;
#[cfg(feature = "libsql")]
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::agent::routine::{
    NotifyConfig, RoutineEventDecision, RoutineEventStatus, RoutineTriggerStatus, RunStatus,
    content_hash,
};
#[cfg(feature = "libsql")]
use crate::error::LlmError;
#[cfg(feature = "libsql")]
use crate::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
#[cfg(feature = "libsql")]
use crate::testing::StubLlm;
#[cfg(feature = "libsql")]
use crate::testing::test_db;

#[test]
fn trigger_retry_attempts_are_persisted_and_saturating() {
    let diagnostics = serde_json::json!({
        "defer_attempt_count": 2,
        "failure_attempt_count": u64::MAX,
    });
    assert_eq!(
        next_trigger_retry_attempt(&diagnostics, "defer_attempt_count"),
        3
    );
    assert_eq!(
        next_trigger_retry_attempt(&diagnostics, "failure_attempt_count"),
        u32::MAX
    );
    assert_eq!(next_trigger_retry_attempt(&diagnostics, "missing"), 1);
}

#[cfg(feature = "libsql")]
struct BlockingLlm {
    started_tx: Mutex<Option<oneshot::Sender<()>>>,
    dropped_tx: Mutex<Option<oneshot::Sender<()>>>,
}

#[cfg(feature = "libsql")]
impl BlockingLlm {
    fn new(started_tx: oneshot::Sender<()>, dropped_tx: oneshot::Sender<()>) -> Self {
        Self {
            started_tx: Mutex::new(Some(started_tx)),
            dropped_tx: Mutex::new(Some(dropped_tx)),
        }
    }
}

#[cfg(feature = "libsql")]
struct DropSignal(Option<oneshot::Sender<()>>);

#[cfg(feature = "libsql")]
impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl LlmProvider for BlockingLlm {
    fn model_name(&self) -> &str {
        "blocking-llm"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        if let Some(tx) = self.started_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        let _drop_signal = DropSignal(self.dropped_tx.lock().unwrap().take());
        std::future::pending::<()>().await;
        unreachable!()
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        if let Some(tx) = self.started_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        let _drop_signal = DropSignal(self.dropped_tx.lock().unwrap().take());
        std::future::pending::<()>().await;
        unreachable!()
    }
}

#[cfg(feature = "libsql")]
fn make_test_routine(name: &str, trigger: Trigger, action: RoutineAction) -> Routine {
    Routine {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: "test routine".to_string(),
        user_id: "default".to_string(),
        actor_id: "default".to_string(),
        enabled: true,
        trigger,
        action,
        guardrails: crate::agent::routine::RoutineGuardrails::default(),
        notify: NotifyConfig::default(),
        policy: Default::default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        config_version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn test_notification_gating() {
    let config = NotifyConfig {
        on_success: false,
        on_failure: true,
        on_attention: true,
        ..Default::default()
    };

    // on_success = false means Ok status should not notify
    assert!(!config.on_success);
    assert!(config.on_failure);
    assert!(config.on_attention);
}

#[test]
fn test_run_status_icons() {
    // Just verify the mapping doesn't panic
    for status in [
        RunStatus::Ok,
        RunStatus::Attention,
        RunStatus::Failed,
        RunStatus::Running,
    ] {
        let _ = status.to_string();
    }
}

#[test]
fn root_event_policy_adapter_builds_dispatch_details() {
    let now = Utc::now();
    let event = RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: "default".to_string(),
        actor_id: "default".to_string(),
        channel: "slack".to_string(),
        event_type: "message".to_string(),
        raw_sender_id: "default".to_string(),
        conversation_scope_id: Uuid::new_v4().to_string(),
        stable_external_conversation_key: "test://root-event-policy".to_string(),
        idempotency_key: "event:slack:default:default:message:root".to_string(),
        content: "deploy".to_string(),
        content_hash: content_hash("deploy").to_string(),
        metadata: serde_json::json!({}),
        status: RoutineEventStatus::Pending,
        diagnostics: serde_json::json!({}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: now - ChronoDuration::seconds(42),
    };

    let details = routine_event_evaluation_details("worker-l", &event, now, Some("event-key"));
    assert_eq!(details["claimed_by"], "worker-l");
    assert_eq!(details["event_age_secs"], serde_json::json!(42));
    assert_eq!(details["trigger_key"], "event-key");

    let dispatch = decide_routine_event_dispatch(false, true, true, true);
    assert_eq!(dispatch.decision, RoutineEventDecision::Fired);
    assert!(dispatch.should_fire);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn fire_manual_tasks_are_tracked_for_abort_all() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let llm = Arc::new(BlockingLlm::new(started_tx, dropped_tx));

    let engine = RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        llm,
        workspace,
        notify_tx,
        None,
    );

    let routine = make_test_routine(
        "manual-abort",
        Trigger::Manual,
        RoutineAction::Lightweight {
            prompt: "wait forever".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    db.create_routine(&routine).await.unwrap();

    engine.fire_manual(routine.id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("manual run should start")
        .unwrap();

    engine.abort_all().await;

    tokio::time::timeout(Duration::from_secs(2), dropped_rx)
        .await
        .expect("abort_all should cancel tracked manual routine")
        .unwrap();
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn system_event_does_not_advance_runtime_when_enqueue_fails() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);
    let (system_event_tx, system_event_rx) = mpsc::channel(1);
    drop(system_event_rx);

    let engine = RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("ok")),
        workspace,
        notify_tx,
        None,
    )
    .with_system_event_tx(system_event_tx);

    let due_at = Utc::now() - ChronoDuration::minutes(1);
    let mut routine = make_test_routine(
        "system-event-fail",
        Trigger::SystemEvent {
            message: "run heartbeat".to_string(),
            schedule: Some("*/5 * * * *".to_string()),
        },
        RoutineAction::Lightweight {
            prompt: "unused".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.next_fire_at = Some(due_at);
    db.create_routine(&routine).await.unwrap();

    engine.check_cron_triggers().await;
    let first_attempt = db
        .list_routine_triggers(routine.id, 10)
        .await
        .expect("routine triggers should be queryable")
        .into_iter()
        .next()
        .expect("failed system event should remain durably queued");
    assert_eq!(first_attempt.status, RoutineTriggerStatus::Pending);
    assert_eq!(
        first_attempt.diagnostics["failure_attempt_count"],
        serde_json::json!(1)
    );

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    engine.check_cron_triggers().await;
    let second_attempt = db
        .list_routine_triggers(routine.id, 10)
        .await
        .expect("routine triggers should be queryable")
        .into_iter()
        .find(|trigger| trigger.id == first_attempt.id)
        .expect("the same active trigger should be retried");
    assert_eq!(second_attempt.status, RoutineTriggerStatus::Pending);
    assert_eq!(
        second_attempt.diagnostics["failure_attempt_count"],
        serde_json::json!(2)
    );

    tokio::time::sleep(Duration::from_millis(2_100)).await;
    engine.check_cron_triggers().await;
    let terminal_attempt = db
        .list_routine_triggers(routine.id, 10)
        .await
        .expect("routine triggers should be queryable")
        .into_iter()
        .find(|trigger| trigger.id == first_attempt.id)
        .expect("terminal trigger evidence should remain queryable");
    assert_eq!(terminal_attempt.status, RoutineTriggerStatus::Failed);
    assert!(
        terminal_attempt
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("retry budget exhausted after 3 attempts"))
    );

    let refreshed = db.get_routine(routine.id).await.unwrap().unwrap();
    assert_eq!(refreshed.run_count, 0);
    assert_eq!(refreshed.last_run_at, None);
    let refreshed_next = refreshed
        .next_fire_at
        .expect("next_fire_at should stay due");
    assert!(refreshed_next <= Utc::now());
    assert!(
        (refreshed_next - due_at).num_milliseconds().abs() < 1_000,
        "next_fire_at should remain effectively unchanged"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn main_session_heartbeat_run_completes_ok_after_injection() {
    // Regression test for the "every main-session heartbeat run is
    // eventually reaped as a failure" bug: execute_heartbeat's
    // light_context=false path used to inject the prompt into the main
    // session and return RunStatus::Running with a comment claiming
    // "the dispatcher handles completion" — but nothing ever called
    // complete_routine_run for these runs, so the zombie reaper marked
    // every one of them as failed once its TTL elapsed. The fix
    // completes the run immediately as Ok (delivery of the prompt is
    // this run's job), independent of however the dispatcher later
    // processes that injected message.
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    // Seed a non-empty HEARTBEAT.md so execute_heartbeat doesn't take
    // the "checklist empty — skip" early-return path.
    workspace
        .write("HEARTBEAT.md", "- Check server status\n")
        .await
        .unwrap();

    let (notify_tx, _notify_rx) = mpsc::channel(4);
    let (system_event_tx, mut system_event_rx) = mpsc::channel(4);

    let engine = RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new(
            "unused — main-session injection short-circuits",
        )),
        workspace,
        notify_tx,
        None,
    )
    .with_system_event_tx(system_event_tx);

    let routine = make_test_routine(
        "main-session-heartbeat",
        Trigger::Manual,
        RoutineAction::Heartbeat {
            light_context: false,
            prompt: None,
            include_reasoning: false,
            active_start_hour: None,
            active_end_hour: None,
            target: "chat".to_string(),
            max_iterations: 5,
            interval_secs: None,
        },
    );
    db.create_routine(&routine).await.unwrap();

    let run_id = engine.fire_manual(routine.id).await.unwrap();

    // The heartbeat prompt should have been injected into the main
    // session via system_event_tx.
    let injected = tokio::time::timeout(Duration::from_secs(2), system_event_rx.recv())
        .await
        .expect("heartbeat message should be injected")
        .expect("system_event_tx should not be closed");
    assert_eq!(
        injected
            .metadata
            .get("run_id")
            .and_then(|v| v.as_str())
            .unwrap(),
        run_id.to_string()
    );

    // The routine run must reach a terminal `Ok` state on its own —
    // nothing else (no dispatcher turn) completes it in this test.
    let mut completed = None;
    for _ in 0..20 {
        let runs = db.list_routine_runs(routine.id, 5).await.unwrap();
        if let Some(run) = runs.into_iter().find(|r| r.id == run_id)
            && run.status != RunStatus::Running
        {
            completed = Some(run);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let completed = completed.expect("heartbeat run should reach a terminal state");
    assert_eq!(completed.status, RunStatus::Ok);
    assert_eq!(
        completed.result_summary.as_deref(),
        Some("Injected into main session")
    );
    assert!(completed.completed_at.is_some());
}

#[cfg(feature = "libsql")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn cron_ticker_checks_due_routines_immediately_on_startup() {
    // Cron-triggered fires now go through IC-CRON-STAGGER jitter
    // (`StaggerConfig::from_env().jitter_delay()`), which defaults to up
    // to 30s. Pin it to 0 for this test so the short polling window
    // below stays meaningful — `lock_env` serializes against any other
    // test in the process that also mutates env vars.
    let _env_guard = thinclaw_config::helpers::lock_env();
    unsafe {
        std::env::set_var("CRON_STAGGER_SECS", "0");
    }

    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);

    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("Review the deployment logs.")),
        workspace,
        notify_tx,
        None,
    ));

    let mut routine = make_test_routine(
        "startup-cron-catchup",
        Trigger::Cron {
            schedule: "0 */15 * * * * *".to_string(),
        },
        RoutineAction::Lightweight {
            prompt: "Inspect deployment state".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.next_fire_at = Some(Utc::now() - ChronoDuration::minutes(1));
    db.create_routine(&routine).await.unwrap();

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle =
        spawn_cron_ticker_with_shutdown(Arc::clone(&engine), Duration::from_secs(60), shutdown_rx);

    let mut fired = false;
    for _ in 0..20 {
        let refreshed = db.get_routine(routine.id).await.unwrap().unwrap();
        if refreshed.run_count > 0 {
            fired = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    shutdown_tx.send(()).expect("cron ticker should be running");
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("cron ticker should stop after shutdown signal")
        .expect("cron ticker task should join cleanly");
    unsafe {
        std::env::remove_var("CRON_STAGGER_SECS");
    }
    assert!(
        fired,
        "due cron routine should be checked immediately without waiting for the first interval"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn skip_catch_up_collapses_overdue_cron_backlog_without_running() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);

    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("unused")),
        workspace,
        notify_tx,
        None,
    ));

    let mut routine = make_test_routine(
        "skip-catch-up",
        Trigger::Cron {
            schedule: "every 1h".to_string(),
        },
        RoutineAction::Lightweight {
            prompt: "Should not run".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.policy.catch_up_mode = crate::agent::routine::RoutineCatchUpMode::Skip;
    routine.next_fire_at = Some(Utc::now() - ChronoDuration::days(90));
    db.create_routine(&routine).await.unwrap();

    engine.check_cron_triggers().await;

    let refreshed = db.get_routine(routine.id).await.unwrap().unwrap();
    assert_eq!(refreshed.run_count, 0);
    assert!(refreshed.next_fire_at.is_some_and(|next| next > Utc::now()));
    assert!(
        db.list_routine_runs(routine.id, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let trigger = db
        .list_routine_triggers(routine.id, 10)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("scheduled trigger audit should be recorded");
    assert_eq!(
        trigger.decision,
        Some(RoutineTriggerDecision::SkippedCatchUp)
    );
    assert!(trigger.backlog_collapsed);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn stale_durable_events_expire_without_firing_routines() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);
    let mut config = RoutineConfig::default();
    config.default_event_max_age_secs = 60;

    let engine = Arc::new(RoutineEngine::new(
        config,
        Arc::clone(&db),
        Arc::new(StubLlm::new("unused")),
        workspace,
        notify_tx,
        None,
    ));

    let mut routine = make_test_routine(
        "stale-event",
        Trigger::Event {
            channel: Some("slack".to_string()),
            event_type: Some("message".to_string()),
            actor: None,
            metadata: None,
            pattern: "deploy".to_string(),
            priority: 0,
        },
        RoutineAction::Lightweight {
            prompt: "Should not run".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.policy.max_event_age_secs = Some(60);
    db.create_routine(&routine).await.unwrap();
    engine.refresh_event_cache().await;

    let event = RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: "default".to_string(),
        actor_id: "default".to_string(),
        channel: "slack".to_string(),
        event_type: "message".to_string(),
        raw_sender_id: "default".to_string(),
        conversation_scope_id: Uuid::new_v4().to_string(),
        stable_external_conversation_key: "test://stale-event".to_string(),
        idempotency_key: "stale-event-idempotency".to_string(),
        content: "deploy".to_string(),
        content_hash: content_hash("deploy").to_string(),
        metadata: serde_json::json!({}),
        status: RoutineEventStatus::Pending,
        diagnostics: serde_json::json!({"content_preview": "deploy"}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: Utc::now() - ChronoDuration::days(90),
    };
    db.create_routine_event(&event).await.unwrap();

    let fired = engine.drain_pending_event_queue().await;
    assert_eq!(fired, 0);
    assert!(
        db.list_routine_runs(routine.id, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let refreshed_event = db
        .list_routine_events_for_actor("default", "default", 10)
        .await
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.idempotency_key == "stale-event-idempotency")
        .expect("durable event should remain queryable");
    assert_eq!(refreshed_event.status, RoutineEventStatus::Processed);

    let evaluation = db
        .list_routine_event_evaluations_for_event(refreshed_event.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("event evaluation should be recorded");
    assert_eq!(evaluation.decision, RoutineEventDecision::SkippedExpired);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn structured_event_filters_and_idempotency_suppress_duplicates() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);

    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("structured event fired")),
        workspace,
        notify_tx,
        None,
    ));

    let mut routine = make_test_routine(
        "structured-event",
        Trigger::Event {
            channel: Some("slack".to_string()),
            event_type: Some("reaction_added".to_string()),
            actor: Some("actor-a".to_string()),
            metadata: Some(serde_json::json!({"tag": "deploy", "flags": ["urgent"]})),
            pattern: "".to_string(),
            priority: 50,
        },
        RoutineAction::Lightweight {
            prompt: "Inspect the structured event".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.actor_id = "actor-a".to_string();
    db.create_routine(&routine).await.unwrap();
    engine.refresh_event_cache().await;

    let identity = crate::identity::ResolvedIdentity {
        principal_id: "default".to_string(),
        actor_id: "actor-a".to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: crate::identity::ConversationKind::Direct,
        raw_sender_id: "actor-a".to_string(),
        stable_external_conversation_key: "test://structured-event".to_string(),
    };
    let first = IncomingMessage::new("slack", "default", "ignored")
        .with_identity(identity.clone())
        .with_metadata(serde_json::json!({
            "event_type": "reaction_added",
            "message_id": "structured-1",
            "tag": "deploy",
            "flags": ["urgent", "audit"],
        }));
    let second = IncomingMessage::new("slack", "default", "ignored")
        .with_identity(identity)
        .with_metadata(serde_json::json!({
            "event_type": "reaction_added",
            "message_id": "structured-1",
            "tag": "deploy",
            "flags": ["urgent", "audit"],
        }));

    assert_eq!(engine.check_event_triggers(&first).await, 1);
    assert_eq!(engine.check_event_triggers(&second).await, 0);

    let runs = db.list_routine_runs(routine.id, 10).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].trigger_key.as_deref(),
        Some("event:event:slack:default:actor-a:reaction_added:structured-1")
    );
}

/// `dedup_window` suppresses a second *distinct* event with identical
/// content within the window; outside the window it fires again.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn dedup_window_suppresses_duplicate_content_within_window() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);

    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("deploy noticed")),
        workspace,
        notify_tx,
        None,
    ));

    let mut routine = make_test_routine(
        "dedup-window",
        Trigger::Event {
            channel: Some("slack".to_string()),
            event_type: Some("message".to_string()),
            actor: None,
            metadata: None,
            pattern: "deploy".to_string(),
            priority: 0,
        },
        RoutineAction::Lightweight {
            prompt: "Inspect deploy".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
    );
    routine.guardrails.dedup_window = Some(std::time::Duration::from_secs(3600));
    // Disable cooldown so suppression is attributable to content dedup.
    routine.guardrails.cooldown = std::time::Duration::from_secs(0);
    db.create_routine(&routine).await.unwrap();
    engine.refresh_event_cache().await;

    let identity = crate::identity::ResolvedIdentity {
        principal_id: "default".to_string(),
        actor_id: "default".to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: crate::identity::ConversationKind::Direct,
        raw_sender_id: "default".to_string(),
        stable_external_conversation_key: "test://dedup-window".to_string(),
    };
    // Two distinct messages (different message ids => different trigger keys)
    // with identical content.
    let first = IncomingMessage::new("slack", "default", "deploy prod now")
        .with_identity(identity.clone())
        .with_metadata(serde_json::json!({ "message_id": "dedup-1" }));
    let second = IncomingMessage::new("slack", "default", "deploy prod now")
        .with_identity(identity)
        .with_metadata(serde_json::json!({ "message_id": "dedup-2" }));

    assert_eq!(engine.check_event_triggers(&first).await, 1);
    assert_eq!(
        engine.check_event_triggers(&second).await,
        0,
        "identical content within dedup_window must be suppressed"
    );

    let runs = db.list_routine_runs(routine.id, 10).await.unwrap();
    assert_eq!(runs.len(), 1, "only the first event should fire");

    // The second event records a SkippedDuplicate evaluation.
    let second_event = db
        .list_routine_events_for_actor("default", "default", 10)
        .await
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.idempotency_key.contains("dedup-2"))
        .expect("second durable event should be queryable");
    let evaluation = db
        .list_routine_event_evaluations_for_event(second_event.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("second event evaluation should be recorded");
    assert_eq!(evaluation.decision, RoutineEventDecision::SkippedDuplicate);
}

/// Two distinct routines matching the same event must both fire in a single
/// drain — the dispatch loop isolates per-routine failures and never aborts
/// siblings (T2: `continue` instead of `break`).
#[cfg(feature = "libsql")]
#[tokio::test]
async fn sibling_routines_both_fire_for_same_event() {
    let (db, _tmp) = test_db().await;
    let workspace = Arc::new(crate::workspace::Workspace::new_with_db(
        "default",
        Arc::clone(&db),
    ));
    let (notify_tx, _notify_rx) = mpsc::channel(4);

    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("handled")),
        workspace,
        notify_tx,
        None,
    ));

    for name in ["sibling-a", "sibling-b"] {
        let routine = make_test_routine(
            name,
            Trigger::Event {
                channel: Some("slack".to_string()),
                event_type: Some("message".to_string()),
                actor: None,
                metadata: None,
                pattern: "deploy".to_string(),
                priority: 0,
            },
            RoutineAction::Lightweight {
                prompt: format!("Inspect deploy for {name}"),
                context_paths: Vec::new(),
                max_tokens: 32,
            },
        );
        db.create_routine(&routine).await.unwrap();
    }
    engine.refresh_event_cache().await;

    let identity = crate::identity::ResolvedIdentity {
        principal_id: "default".to_string(),
        actor_id: "default".to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: crate::identity::ConversationKind::Direct,
        raw_sender_id: "default".to_string(),
        stable_external_conversation_key: "test://siblings".to_string(),
    };
    let message = IncomingMessage::new("slack", "default", "deploy prod")
        .with_identity(identity)
        .with_metadata(serde_json::json!({ "message_id": "siblings-1" }));

    assert_eq!(
        engine.check_event_triggers(&message).await,
        2,
        "both sibling routines should fire from one event"
    );
}

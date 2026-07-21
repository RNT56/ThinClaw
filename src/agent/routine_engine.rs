//! Routine execution engine.
//!
//! Handles loading routines, checking triggers, enforcing guardrails,
//! and executing both lightweight (single LLM call) and full-job routines.
//!
//! The engine runs two independent loops:
//! - A **cron ticker** that polls the DB every N seconds for due cron routines
//! - An **event matcher** called synchronously from the agent main loop
//!
//! Lightweight routines execute inline (single LLM call, no scheduler slot).
//! Full-job routines are delegated to the existing `Scheduler`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Timelike, Utc};
use futures::{FutureExt, future::join_all};
use regex::Regex;
use thinclaw_agent::loop_control::{LoopKind, LoopStopReason};
use thinclaw_agent::routine_engine::{
    ClaimedScheduledTriggerDecisionInput, EVENT_CONTENT_PREVIEW_LIMIT, FullJobRuntimeMetadata,
    HeartbeatTarget, RoutineEventEvaluationPlan, RoutineEventFilterOutcome, ScheduledTriggerAction,
    active_hour_allows, build_heartbeat_prompt, build_routine_event_from_message,
    build_routine_notification, build_scheduled_routine_triggers,
    classify_lightweight_routine_response, compare_event_cache_routines,
    decide_claimed_scheduled_trigger, decide_missing_scheduled_trigger_routine,
    decide_routine_event_dispatch, effective_lightweight_max_tokens,
    evaluate_routine_event_filters, fair_interleave_routine_events, full_job_metadata,
    heartbeat_critique_setting_key, heartbeat_job_metadata, increment_decision_count,
    lightweight_routine_evidence, lightweight_routine_fixed_messages, render_trigger_payload_block,
    routine_cooldown_allows, routine_event_attempts_exhausted, routine_event_evaluation_details,
    routine_event_fairness_key, routine_event_owner_matches, routine_failure_backoff,
    routine_queue_retry_delay, routine_requests_desktop_capabilities, routine_should_auto_disable,
    sanitize_routine_name, scheduled_run_trigger_key, should_continue_queue_drain,
    should_jitter_trigger_type, should_refresh_event_cache, summarize_runtime_capabilities,
    truncate,
};
use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::agent::cron_stagger::StaggerConfig;
use crate::agent::outcomes;
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineEvent, RoutineEventDecision,
    RoutineEventEvaluation, RoutineRun, RoutineTrigger, RoutineTriggerDecision, RoutineTriggerKind,
    RunStatus, Trigger, compile_event_trigger_pattern, next_fire_for_routine,
    routine_timezone_setting,
};
use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::agent::{AgentRunArtifact, AgentRunStatus};
use crate::api::experiments as experiments_api;
use crate::channels::web::types::SseEvent;
use crate::channels::{IncomingMessage, OutgoingResponse};
use crate::config::RoutineConfig;
use crate::db::{Database, RoutineRunAdmission, RoutineRunCompletion};
use crate::error::{DatabaseError, RoutineError};
use crate::llm::{CompletionRequest, LlmProvider};
use crate::observability::{LoopMetricGuard, NoopObserver, Observer};
use crate::tools::ToolProfile;
use crate::tools::execution_backend::routine_engine_runtime_descriptor;
use crate::workspace::{AuthorizedWorkspace, Workspace};

mod execution;
use execution::execute_routine;

fn routine_identity(routine: &Routine) -> crate::identity::ResolvedIdentity {
    let actor_id = routine.owner_actor_id().to_string();
    let stable_external_conversation_key =
        thinclaw_identity::direct_conversation_key(&routine.user_id, &actor_id);
    crate::identity::ResolvedIdentity {
        principal_id: routine.user_id.clone(),
        actor_id: actor_id.clone(),
        conversation_scope_id: thinclaw_identity::direct_scope_id(&routine.user_id, &actor_id),
        conversation_kind: crate::identity::ConversationKind::Direct,
        raw_sender_id: actor_id,
        stable_external_conversation_key,
    }
}

/// The routine execution engine.
pub struct RoutineEngine {
    config: RoutineConfig,
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    /// Sender for notifications (routed to channel manager).
    notify_tx: mpsc::Sender<OutgoingResponse>,
    /// Compiled event regex cache: routine_id -> compiled regex.
    event_cache: Arc<RwLock<Vec<CachedEventRoutine>>>,
    /// Last observed global event-cache version in the database.
    event_cache_version: Arc<RwLock<i64>>,
    /// When the in-memory event cache was last refreshed.
    event_cache_refreshed_at: Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
    /// Scheduler for dispatching jobs (FullJob mode).
    scheduler: Option<Arc<Scheduler>>,
    /// Runtime-scoped desktop autonomy state. Routine scheduling must not
    /// depend on a manager installed by an unrelated application instance.
    desktop_autonomy_manager: Option<Arc<crate::desktop_autonomy::DesktopAutonomyManager>>,
    /// Optional SSE broadcast sender for emitting routine lifecycle events.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Optional sender for injecting messages into the main session.
    /// Used by Heartbeat action with `light_context: false` to run inside
    /// the main conversational context with full session history.
    system_event_tx: Option<mpsc::Sender<IncomingMessage>>,
    /// Optional subagent executor for running non-heartbeat automations as subagents.
    subagent_executor: Option<Arc<SubagentExecutor>>,
    /// IC-018: Tracked handles for all running routine tasks.
    /// Uses `std::sync::Mutex` intentionally: `JoinSet::spawn()` is synchronous
    /// and we must never hold an `async` lock across a non-async call (Bug 14 fix).
    active_tasks: Arc<std::sync::Mutex<tokio::task::JoinSet<()>>>,
    /// Admission gate serialized with `active_tasks` registration. Closing it
    /// during shutdown prevents an in-flight fire from spawning into a fresh
    /// task set after the shutdown drain has already taken the old one.
    task_admission_open: std::sync::Mutex<bool>,
    /// Durable run/routine IDs for inline routine tasks. A drop guard removes
    /// entries on every normal, panic, or abort path; shutdown snapshots any
    /// entries that remain immediately before a forced abort.
    active_task_runs: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Uuid>>>,
    /// User timezone (IANA) for active-hours checks. Populated from Settings.
    user_timezone: Option<String>,
    /// Stable worker id used when claiming persisted event inbox items.
    worker_id: String,
    /// Shared observability sink for routine event/trigger loop metrics.
    observer: Arc<dyn Observer>,
}

const EVENT_QUEUE_BATCH_LIMIT: i64 = 64;
const TRIGGER_QUEUE_BATCH_LIMIT: i64 = 64;
const QUEUE_MAX_BATCHES_PER_TICK: usize = 4;
const ROUTINE_EVENT_MAX_ATTEMPTS: u32 = 3;
const ROUTINE_TRIGGER_MAX_FAILURE_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimedTriggerOutcome {
    Fired,
    Completed,
    Released,
}

fn next_trigger_retry_attempt(diagnostics: &serde_json::Value, field: &str) -> u32 {
    diagnostics
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
        .unwrap_or(0)
        .saturating_add(1)
}

/// Initial DB lease window (seconds) set on a routine run at spawn time,
/// before the worker/subagent (or lightweight inline execution) has had a
/// chance to renew it. Generous enough to cover engine scheduling jitter
/// and job-dispatch latency without masking a genuinely dead run for long.
const INITIAL_ROUTINE_RUN_LEASE_SECS: i64 = 300;
const ROUTINE_TASK_SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
const ROUTINE_DB_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

struct ActiveRoutineTaskRegistration {
    run_id: Uuid,
    active_runs: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Uuid>>>,
}

impl Drop for ActiveRoutineTaskRegistration {
    fn drop(&mut self) {
        let mut active_runs = self
            .active_runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active_runs.remove(&self.run_id);
    }
}

#[derive(Clone)]
struct CachedEventRoutine {
    routine: Routine,
    regex: Option<Regex>,
}

fn routine_event_batch_source_count(events: &[RoutineEvent]) -> usize {
    events
        .iter()
        .map(routine_event_fairness_key)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

impl RoutineEngine {
    async fn effective_timezone_for_routine(&self, routine: &Routine) -> Option<String> {
        if let Some(timezone) = routine
            .state
            .get("user_timezone")
            .and_then(|value| value.as_str())
        {
            return Some(timezone.to_string());
        }

        let workspace = self
            .workspace
            .scoped_clone(routine.user_id.clone(), self.workspace.agent_id());
        if let Some(timezone) = workspace
            .extract_user_timezone_from_path(&crate::workspace::paths::actor_user(
                routine.owner_actor_id(),
            ))
            .await
        {
            return Some(timezone);
        }
        if let Some(timezone) = workspace.extract_user_timezone().await {
            return Some(timezone);
        }
        if routine.user_id == self.workspace.user_id()
            && let Some(timezone) = self.user_timezone.as_ref()
        {
            return Some(timezone.clone());
        }
        Some(
            workspace
                .effective_timezone_for_identity(&routine_identity(routine))
                .await
                .name()
                .to_string(),
        )
    }

    pub fn new(
        config: RoutineConfig,
        store: Arc<dyn Database>,
        llm: Arc<dyn LlmProvider>,
        workspace: Arc<Workspace>,
        notify_tx: mpsc::Sender<OutgoingResponse>,
        scheduler: Option<Arc<Scheduler>>,
    ) -> Self {
        Self {
            config,
            store,
            llm,
            workspace,
            notify_tx,
            event_cache: Arc::new(RwLock::new(Vec::new())),
            event_cache_version: Arc::new(RwLock::new(0)),
            event_cache_refreshed_at: Arc::new(RwLock::new(None)),
            scheduler,
            desktop_autonomy_manager: None,
            sse_tx: None,
            system_event_tx: None,
            subagent_executor: None,
            active_tasks: Arc::new(std::sync::Mutex::new(tokio::task::JoinSet::new())),
            task_admission_open: std::sync::Mutex::new(true),
            active_task_runs: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            user_timezone: None,
            worker_id: Uuid::new_v4().to_string(),
            observer: Arc::new(NoopObserver),
        }
    }

    /// Set the SSE broadcast sender for emitting routine lifecycle events.
    pub fn with_sse_sender(mut self, tx: tokio::sync::broadcast::Sender<SseEvent>) -> Self {
        self.sse_tx = Some(tx);
        self
    }

    /// Set the shared observer for routine queue loop metrics.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = observer;
        self
    }

    /// Bind desktop-autonomy state to this engine's runtime.
    pub fn with_desktop_autonomy_manager(
        mut self,
        manager: Option<Arc<crate::desktop_autonomy::DesktopAutonomyManager>>,
    ) -> Self {
        self.desktop_autonomy_manager = manager;
        self
    }

    /// Set the system event sender for main-session heartbeat injection.
    pub fn with_system_event_tx(mut self, tx: mpsc::Sender<IncomingMessage>) -> Self {
        self.system_event_tx = Some(tx);
        self
    }

    /// Set the subagent executor for running non-heartbeat automations.
    pub fn with_subagent_executor(mut self, executor: Arc<SubagentExecutor>) -> Self {
        self.subagent_executor = Some(executor);
        self
    }

    /// Set the user timezone (IANA name). Active-hours checks use this
    /// instead of `chrono::Local` so cloud/VPS deployments work correctly.
    pub fn with_user_timezone(mut self, tz: Option<String>) -> Self {
        self.user_timezone = tz;
        self
    }

    /// Refresh the in-memory event trigger cache from DB.
    pub async fn refresh_event_cache(&self) {
        let cache_version = self
            .store
            .get_routine_event_cache_version()
            .await
            .unwrap_or(0);
        match self.store.list_event_routines().await {
            Ok(routines) => {
                let mut cache = Vec::new();
                for routine in routines {
                    if let Trigger::Event { ref pattern, .. } = routine.trigger {
                        let regex = if pattern.trim().is_empty() {
                            None
                        } else {
                            match compile_event_trigger_pattern(pattern) {
                                Ok(re) => Some(re),
                                Err(e) => {
                                    tracing::warn!(
                                        routine = %routine.name,
                                        "Invalid event regex '{}': {}",
                                        pattern, e
                                    );
                                    continue;
                                }
                            }
                        };
                        cache.push(CachedEventRoutine {
                            routine: routine.clone(),
                            regex,
                        });
                    }
                }
                cache.sort_by(|left, right| {
                    compare_event_cache_routines(&left.routine, &right.routine)
                });
                let count = cache.len();
                *self.event_cache.write().await = cache;
                *self.event_cache_version.write().await = cache_version;
                *self.event_cache_refreshed_at.write().await = Some(Utc::now());
                tracing::debug!(cache_version, "Refreshed event cache: {} routines", count);
            }
            Err(e) => {
                tracing::error!("Failed to refresh event cache: {}", e);
            }
        }
    }

    /// Check incoming message against event triggers. Returns number of routines fired.
    ///
    /// Called synchronously from the main loop after handle_message(). The actual
    /// execution is spawned async so this returns quickly.
    pub async fn check_event_triggers(&self, message: &IncomingMessage) -> usize {
        let mut loop_metrics =
            LoopMetricGuard::start(Arc::clone(&self.observer), LoopKind::RoutineEventQueue);
        self.ensure_event_cache_loaded().await;
        let event = match self.enqueue_routine_event(message).await {
            Ok(event) => event,
            Err(error) => {
                loop_metrics.stop_with(LoopStopReason::FatalError);
                tracing::error!("Failed to enqueue routine event: {}", error);
                return 0;
            }
        };
        loop_metrics.set_iterations(1);

        if let Some(manager) = self.desktop_autonomy_manager.as_ref()
            && manager.emergency_stop_active()
        {
            loop_metrics.stop_with(LoopStopReason::Cancelled);
            tracing::warn!(
                event_id = %event.id,
                "Desktop autonomy emergency stop is active; leaving event queued"
            );
            return 0;
        }

        match self.try_process_routine_event(event.id).await {
            Ok(Some(fired)) => {
                loop_metrics.stop_with(LoopStopReason::Completed);
                fired
            }
            Ok(None) => {
                loop_metrics.stop_with(LoopStopReason::NoWork);
                0
            }
            Err(error) => {
                loop_metrics.stop_with(LoopStopReason::FatalError);
                tracing::error!(
                    event_id = %event.id,
                    "Failed to process routine event: {}",
                    error
                );
                0
            }
        }
    }

    /// Check all due cron/system routines and fire them. Called by the cron ticker.
    pub async fn check_cron_triggers(&self) -> usize {
        if let Some(manager) = self.desktop_autonomy_manager.as_ref()
            && manager.emergency_stop_active()
        {
            tracing::warn!("Desktop autonomy emergency stop is active; skipping cron routines");
            return 0;
        }

        if let Err(error) = self.enqueue_due_cron_triggers().await {
            tracing::error!("Failed to enqueue due cron routines: {}", error);
            return 0;
        }

        self.drain_pending_trigger_queue().await
    }

    /// Enqueue a prebuilt trigger and process the durable trigger queue.
    pub async fn enqueue_trigger_and_drain(
        &self,
        trigger: RoutineTrigger,
    ) -> Result<usize, RoutineError> {
        self.store
            .enqueue_routine_trigger(&trigger)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?;
        Ok(self.drain_pending_trigger_queue().await)
    }

    /// Fire a routine through the same background execution path used by event/cron triggers.
    pub async fn fire_routine_run_request(
        &self,
        routine: Routine,
        trigger_key: String,
    ) -> Result<Uuid, RoutineError> {
        if !routine.enabled {
            return Err(RoutineError::Disabled {
                name: routine.name.clone(),
            });
        }
        self.spawn_fire(routine, "port", None, Some(trigger_key))
            .await
    }

    async fn ensure_event_cache_loaded(&self) {
        let cache_empty = self.event_cache.read().await.is_empty();
        let last_refreshed_at = *self.event_cache_refreshed_at.read().await;
        let current_version = *self.event_cache_version.read().await;
        let observed_version = self.store.get_routine_event_cache_version().await.ok();

        if should_refresh_event_cache(
            cache_empty,
            last_refreshed_at,
            current_version,
            observed_version,
            self.config.event_cache_ttl_secs,
            Utc::now(),
        ) {
            self.refresh_event_cache().await;
        }
    }

    fn claim_stale_before(&self) -> chrono::DateTime<Utc> {
        Utc::now() - ChronoDuration::seconds(self.config.claim_lease_secs as i64)
    }

    async fn enqueue_routine_event(
        &self,
        message: &IncomingMessage,
    ) -> Result<RoutineEvent, RoutineError> {
        let event = build_routine_event_from_message(message);

        self.store
            .create_routine_event(&event)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })
    }

    /// Fire a routine manually (from tool call or CLI).
    pub async fn fire_manual(&self, routine_id: Uuid) -> Result<Uuid, RoutineError> {
        self.fire_manual_with_payload(routine_id, None).await
    }

    /// Fire a routine manually with an optional trigger payload.
    ///
    /// Used by the webhook trigger path to forward the (validated, size-capped)
    /// request body into the routine's effective prompt via
    /// [`RoutineRun::trigger_detail`]. Non-webhook callers pass `None`.
    pub async fn fire_manual_with_payload(
        &self,
        routine_id: Uuid,
        payload: Option<String>,
    ) -> Result<Uuid, RoutineError> {
        let routine = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|e| RoutineError::Database {
                reason: e.to_string(),
            })?
            .ok_or(RoutineError::NotFound { id: routine_id })?;

        if !routine.enabled {
            return Err(RoutineError::Disabled {
                name: routine.name.clone(),
            });
        }

        let trigger_type = if payload.is_some() {
            "webhook"
        } else {
            "manual"
        };
        self.spawn_fire(routine, trigger_type, payload, None).await
    }

    async fn enqueue_due_cron_triggers(&self) -> Result<(), RoutineError> {
        let due_routines =
            self.store
                .list_due_cron_routines()
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
        let now = Utc::now();

        for routine in due_routines {
            self.enqueue_due_routine_triggers(&routine, now).await?;
        }

        Ok(())
    }

    async fn enqueue_due_routine_triggers(
        &self,
        routine: &Routine,
        now: chrono::DateTime<Utc>,
    ) -> Result<(), RoutineError> {
        let user_timezone = self.effective_timezone_for_routine(routine).await;
        for trigger in build_scheduled_routine_triggers(
            routine,
            user_timezone.as_deref(),
            now,
            TRIGGER_QUEUE_BATCH_LIMIT as usize,
        )? {
            self.store
                .enqueue_routine_trigger(&trigger)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
        }

        Ok(())
    }

    async fn drain_pending_trigger_queue(&self) -> usize {
        let mut loop_metrics =
            LoopMetricGuard::start(Arc::clone(&self.observer), LoopKind::RoutineTriggerQueue);
        let mut total_fired = 0usize;
        let mut processed_triggers = 0usize;
        let mut batches_processed = 0usize;
        let mut retried_triggers = 0u32;
        let mut saw_work = false;
        let mut stop_reason = None;

        loop {
            let claimed = match self
                .store
                .claim_routine_triggers(
                    &self.worker_id,
                    self.claim_stale_before(),
                    TRIGGER_QUEUE_BATCH_LIMIT,
                )
                .await
            {
                Ok(items) => items,
                Err(error) => {
                    stop_reason = Some(LoopStopReason::FatalError);
                    tracing::error!("Failed to claim scheduled routine triggers: {}", error);
                    break;
                }
            };

            if claimed.is_empty() {
                break;
            }

            saw_work = true;
            batches_processed += 1;
            let batch_len = claimed.len();
            for trigger in claimed {
                processed_triggers += 1;
                match self.process_claimed_trigger(trigger.clone()).await {
                    Ok(ClaimedTriggerOutcome::Fired) => total_fired += 1,
                    Ok(ClaimedTriggerOutcome::Released) => retried_triggers += 1,
                    Ok(ClaimedTriggerOutcome::Completed) => {}
                    Err(error) => {
                        match self.record_routine_trigger_failure(&trigger, &error).await {
                            Ok(terminal) => {
                                if terminal {
                                    stop_reason = Some(LoopStopReason::RetryBudgetExceeded);
                                } else {
                                    retried_triggers += 1;
                                    if stop_reason.is_none() {
                                        stop_reason = Some(LoopStopReason::FatalError);
                                    }
                                }
                            }
                            Err(store_error) => {
                                stop_reason = Some(LoopStopReason::FatalError);
                                tracing::error!(
                                    trigger_id = %trigger.id,
                                    error = %store_error,
                                    "Failed to persist scheduled routine trigger failure"
                                );
                            }
                        }
                        tracing::error!("Failed to process scheduled routine trigger: {}", error);
                    }
                }
            }

            if !should_continue_queue_drain(
                batch_len,
                TRIGGER_QUEUE_BATCH_LIMIT as usize,
                batches_processed,
                QUEUE_MAX_BATCHES_PER_TICK,
            ) {
                if batch_len >= TRIGGER_QUEUE_BATCH_LIMIT as usize
                    && batches_processed >= QUEUE_MAX_BATCHES_PER_TICK
                    && stop_reason.is_none()
                {
                    stop_reason = Some(LoopStopReason::IterationBudgetExceeded);
                }
                break;
            }
        }

        loop_metrics.set_iterations(processed_triggers);
        loop_metrics.set_retries(retried_triggers);
        loop_metrics.stop_with(stop_reason.unwrap_or(if saw_work {
            LoopStopReason::Completed
        } else {
            LoopStopReason::NoWork
        }));

        total_fired
    }

    async fn process_claimed_trigger(
        &self,
        trigger: RoutineTrigger,
    ) -> Result<ClaimedTriggerOutcome, RoutineError> {
        let Some(routine) = self
            .store
            .get_routine(trigger.routine_id)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?
        else {
            let plan = decide_missing_scheduled_trigger_routine();
            let diagnostics = serde_json::json!({
                "decision": plan.decision.to_string(),
                "reason": plan.reason,
                "claimed_by": self.worker_id,
            });
            self.store
                .complete_routine_trigger(trigger.id, Utc::now(), plan.decision, &diagnostics)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(ClaimedTriggerOutcome::Completed);
        };

        let trigger_key = scheduled_run_trigger_key(&trigger);
        let duplicate_exists = if routine.enabled {
            self.store
                .routine_run_exists_for_trigger_key(routine.id, &trigger_key)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?
        } else {
            false
        };
        let cooldown_allowed = duplicate_exists || self.check_cooldown(&routine);
        let routine_capacity_available = duplicate_exists
            || !cooldown_allowed
            || matches!(trigger.trigger_kind, RoutineTriggerKind::SystemEvent)
            || self.check_concurrent(&routine).await;
        let global_capacity_available = if duplicate_exists
            || !cooldown_allowed
            || !routine_capacity_available
            || matches!(trigger.trigger_kind, RoutineTriggerKind::SystemEvent)
        {
            true
        } else {
            let global_running = self.count_global_running().await?;
            global_running < self.config.max_concurrent_routines as i64
        };

        let user_timezone = self.effective_timezone_for_routine(&routine).await;
        let plan = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
            routine: &routine,
            trigger: &trigger,
            duplicate_exists,
            cooldown_allowed,
            routine_capacity_available,
            global_capacity_available,
            user_timezone: user_timezone.as_deref(),
            now: Utc::now(),
        })?;

        match plan.action {
            ScheduledTriggerAction::Complete if plan.decision != RoutineTriggerDecision::Fired => {
                if plan.decision == RoutineTriggerDecision::SkippedCatchUp {
                    self.reschedule_without_run(&routine, plan.next_fire_at)
                        .await?;
                }
                let diagnostics = serde_json::json!({
                    "decision": plan.decision.to_string(),
                    "reason": plan.reason,
                    "claimed_by": self.worker_id,
                    "idempotency_key": trigger.idempotency_key,
                    "backlog_collapsed": trigger.backlog_collapsed,
                    "next_fire_at": plan.next_fire_at.map(|value| value.to_rfc3339()),
                });
                self.store
                    .complete_routine_trigger(trigger.id, Utc::now(), plan.decision, &diagnostics)
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
                return Ok(ClaimedTriggerOutcome::Completed);
            }
            ScheduledTriggerAction::Release => {
                let defer_attempt =
                    next_trigger_retry_attempt(&trigger.diagnostics, "defer_attempt_count");
                let retry_delay = routine_queue_retry_delay(defer_attempt);
                let next_attempt_at = Utc::now()
                    + ChronoDuration::from_std(retry_delay)
                        .unwrap_or_else(|_| ChronoDuration::seconds(1));
                let diagnostics = serde_json::json!({
                    "decision": plan.decision.to_string(),
                    "reason": plan.reason,
                    "claimed_by": self.worker_id,
                    "due_at": trigger.due_at.to_rfc3339(),
                    "defer_attempt_count": defer_attempt,
                    "next_attempt_at": next_attempt_at.to_rfc3339(),
                });
                self.store
                    .release_routine_trigger(trigger.id, next_attempt_at, &diagnostics)
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
                return Ok(ClaimedTriggerOutcome::Released);
            }
            ScheduledTriggerAction::Dispatch => {}
            ScheduledTriggerAction::Complete => {}
        }

        match trigger.trigger_kind {
            RoutineTriggerKind::Cron => {
                let trigger_detail = trigger
                    .trigger_label
                    .clone()
                    .unwrap_or_else(|| trigger.due_at.to_rfc3339());
                let run_id = self
                    .spawn_fire(
                        routine.clone(),
                        "cron",
                        Some(trigger_detail),
                        Some(trigger_key),
                    )
                    .await?;
                let diagnostics = serde_json::json!({
                    "decision": RoutineTriggerDecision::Fired.to_string(),
                    "claimed_by": self.worker_id,
                    "due_at": trigger.due_at.to_rfc3339(),
                    "run_id": run_id.to_string(),
                    "backlog_collapsed": trigger.backlog_collapsed,
                    "coalesced_count": trigger.coalesced_count,
                });
                self.store
                    .complete_routine_trigger(
                        trigger.id,
                        Utc::now(),
                        RoutineTriggerDecision::Fired,
                        &diagnostics,
                    )
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
                Ok(ClaimedTriggerOutcome::Fired)
            }
            RoutineTriggerKind::SystemEvent => {
                self.dispatch_system_event(&routine, &trigger, &trigger_key)
                    .await?;
                let diagnostics = serde_json::json!({
                    "decision": RoutineTriggerDecision::Fired.to_string(),
                    "claimed_by": self.worker_id,
                    "due_at": trigger.due_at.to_rfc3339(),
                    "idempotency_key": trigger.idempotency_key,
                });
                self.store
                    .complete_routine_trigger(
                        trigger.id,
                        Utc::now(),
                        RoutineTriggerDecision::Fired,
                        &diagnostics,
                    )
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
                Ok(ClaimedTriggerOutcome::Fired)
            }
        }
    }

    async fn record_routine_trigger_failure(
        &self,
        trigger: &RoutineTrigger,
        error: &RoutineError,
    ) -> Result<bool, RoutineError> {
        let attempt_count =
            next_trigger_retry_attempt(&trigger.diagnostics, "failure_attempt_count");
        let error_message = error.to_string();
        if routine_event_attempts_exhausted(attempt_count, ROUTINE_TRIGGER_MAX_FAILURE_ATTEMPTS) {
            let terminal_error =
                format!("retry budget exhausted after {attempt_count} attempts: {error_message}");
            self.store
                .fail_routine_trigger(trigger.id, Utc::now(), &terminal_error)
                .await
                .map_err(|store_error| RoutineError::Database {
                    reason: store_error.to_string(),
                })?;
            return Ok(true);
        }

        let retry_delay = routine_queue_retry_delay(attempt_count);
        let next_attempt_at = Utc::now()
            + ChronoDuration::from_std(retry_delay).unwrap_or_else(|_| ChronoDuration::seconds(1));
        let diagnostics = serde_json::json!({
            "decision": "retry_processing_error",
            "claimed_by": self.worker_id,
            "reason": error_message,
            "due_at": trigger.due_at.to_rfc3339(),
            "failure_attempt_count": attempt_count,
            "next_attempt_at": next_attempt_at.to_rfc3339(),
        });
        self.store
            .release_routine_trigger(trigger.id, next_attempt_at, &diagnostics)
            .await
            .map_err(|store_error| RoutineError::Database {
                reason: store_error.to_string(),
            })?;
        Ok(false)
    }

    async fn dispatch_system_event(
        &self,
        routine: &Routine,
        trigger: &RoutineTrigger,
        trigger_key: &str,
    ) -> Result<(), RoutineError> {
        let Trigger::SystemEvent { message, .. } = &routine.trigger else {
            return Err(RoutineError::ExecutionFailed {
                reason:
                    "scheduled system event queue item did not resolve to a system_event routine"
                        .to_string(),
            });
        };
        let tx = self
            .system_event_tx
            .as_ref()
            .ok_or_else(|| RoutineError::ExecutionFailed {
                reason: "system event queue is not available".to_string(),
            })?;

        let identity = routine_identity(routine);
        let user_timezone = self.effective_timezone_for_routine(routine).await;
        let event_message = IncomingMessage::new("heartbeat", "system", message.clone())
            .with_metadata(serde_json::json!({
                "source": "system_event",
                "conversation_kind": "direct",
                "conversation_scope_id": identity.conversation_scope_id.to_string(),
                "stable_external_conversation_key": identity.stable_external_conversation_key.clone(),
                "user_timezone": user_timezone.clone(),
                "routine_id": routine.id.to_string(),
                "routine_name": routine.name,
                "trigger_id": trigger.id.to_string(),
                "trigger_key": trigger_key,
                "scheduled_due_at": trigger.due_at.to_rfc3339(),
            }))
            .with_identity(identity);
        tx.send(event_message)
            .await
            .map_err(|error| RoutineError::ExecutionFailed {
                reason: format!("failed to enqueue system event: {error}"),
            })?;

        let now = Utc::now();
        let next_fire = next_fire_for_routine(routine, user_timezone.as_deref(), now)?;
        self.store
            .advance_routine_runtime(routine.id, now, next_fire)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?;

        Ok(())
    }

    async fn reschedule_without_run(
        &self,
        routine: &Routine,
        next_fire_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<(), RoutineError> {
        self.store
            .set_routine_next_fire_at(routine.id, next_fire_at)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })
    }

    async fn try_process_routine_event(
        &self,
        event_id: Uuid,
    ) -> Result<Option<usize>, RoutineError> {
        let claimed = self
            .store
            .claim_routine_event(event_id, &self.worker_id, self.claim_stale_before())
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?;

        let Some(event) = claimed else {
            return Ok(None);
        };

        match self.process_claimed_event(event.clone()).await {
            Ok(fired) => Ok(Some(fired)),
            Err(error) => {
                self.record_routine_event_failure(&event, &error).await;
                Err(error)
            }
        }
    }

    async fn record_routine_event_failure(&self, event: &RoutineEvent, error: &RoutineError) {
        let error_message = error.to_string();
        let terminal =
            routine_event_attempts_exhausted(event.attempt_count, ROUTINE_EVENT_MAX_ATTEMPTS);
        let diagnostics = serde_json::json!({
            "content_preview": truncate(&event.content, EVENT_CONTENT_PREVIEW_LIMIT),
            "claimed_by": self.worker_id,
            "attempt_count": event.attempt_count,
            "max_attempts": ROUTINE_EVENT_MAX_ATTEMPTS,
            "last_error": error_message.clone(),
            "dispatch_error": error_message.clone(),
            "dispatch_errors": [error_message.clone()],
            "dead_lettered": terminal,
        });

        let result = if terminal {
            self.store
                .dead_letter_routine_event(event.id, Utc::now(), &error_message, &diagnostics)
                .await
        } else {
            let retry_delay = routine_queue_retry_delay(event.attempt_count);
            let next_attempt_at = Utc::now()
                + ChronoDuration::from_std(retry_delay)
                    .unwrap_or_else(|_| ChronoDuration::seconds(1));
            self.store
                .release_routine_event(event.id, next_attempt_at, &diagnostics)
                .await
        };

        if let Err(store_error) = result {
            tracing::error!(
                event_id = %event.id,
                terminal,
                "Failed to record routine event failure: {}",
                store_error
            );
        } else if terminal {
            tracing::warn!(
                event_id = %event.id,
                attempts = event.attempt_count,
                max_attempts = ROUTINE_EVENT_MAX_ATTEMPTS,
                "Routine event dead-lettered after bounded retries"
            );
        }
    }

    pub async fn drain_pending_event_queue(&self) -> usize {
        let mut loop_metrics =
            LoopMetricGuard::start(Arc::clone(&self.observer), LoopKind::RoutineEventQueue);
        if let Some(manager) = self.desktop_autonomy_manager.as_ref()
            && manager.emergency_stop_active()
        {
            loop_metrics.stop_with(LoopStopReason::Cancelled);
            tracing::warn!("Desktop autonomy emergency stop is active; skipping event inbox drain");
            return 0;
        }

        self.ensure_event_cache_loaded().await;
        let mut total_fired = 0usize;
        let mut processed_events = 0usize;
        let mut batches_processed = 0usize;
        let mut retried_events = 0u32;
        let mut saw_work = false;
        let mut stop_reason = None;

        loop {
            let pending = match self
                .store
                .list_pending_routine_events(self.claim_stale_before(), EVENT_QUEUE_BATCH_LIMIT)
                .await
            {
                Ok(events) => events,
                Err(error) => {
                    stop_reason = Some(LoopStopReason::FatalError);
                    tracing::error!("Failed to load pending routine events: {}", error);
                    break;
                }
            };

            if pending.is_empty() {
                break;
            }

            saw_work = true;
            batches_processed += 1;
            let batch_len = pending.len();
            let batch_source_count = routine_event_batch_source_count(&pending);
            let pending = fair_interleave_routine_events(pending);
            tracing::debug!(
                batch_len,
                batch_source_count,
                "Draining routine event batch with fair source interleaving"
            );

            for (batch_index, event) in pending.into_iter().enumerate() {
                processed_events += 1;
                if event.attempt_count > 0 {
                    retried_events = retried_events.saturating_add(1);
                }
                tracing::trace!(
                    event_id = %event.id,
                    batch_index,
                    batch_len,
                    batch_source_count,
                    source_key = %routine_event_fairness_key(&event),
                    attempt_count = event.attempt_count,
                    "Processing routine event from fair interleaved batch"
                );
                match self.try_process_routine_event(event.id).await {
                    Ok(Some(fired)) => total_fired += fired,
                    Ok(None) => {}
                    Err(error) => {
                        if stop_reason.is_none() {
                            stop_reason = Some(LoopStopReason::FatalError);
                        }
                        tracing::error!(
                            event_id = %event.id,
                            "Failed to drain pending routine event: {}",
                            error
                        );
                    }
                }
            }

            if !should_continue_queue_drain(
                batch_len,
                EVENT_QUEUE_BATCH_LIMIT as usize,
                batches_processed,
                QUEUE_MAX_BATCHES_PER_TICK,
            ) {
                if batch_len >= EVENT_QUEUE_BATCH_LIMIT as usize
                    && batches_processed >= QUEUE_MAX_BATCHES_PER_TICK
                {
                    stop_reason = Some(LoopStopReason::IterationBudgetExceeded);
                }
                break;
            }
        }

        loop_metrics.set_iterations(processed_events);
        loop_metrics.set_retries(retried_events);
        loop_metrics.stop_with(stop_reason.unwrap_or(if saw_work {
            LoopStopReason::Completed
        } else {
            LoopStopReason::NoWork
        }));

        total_fired
    }

    async fn process_claimed_event(&self, event: RoutineEvent) -> Result<usize, RoutineError> {
        self.ensure_event_cache_loaded().await;

        let cache = self.event_cache.read().await.clone();
        let total_event_routines = cache.len() as u32;
        let global_running = self.count_global_running().await?;

        let now = Utc::now();
        let content_preview = truncate(&event.content, EVENT_CONTENT_PREVIEW_LIMIT);
        let mut owner_candidate_routines = 0u32;
        let mut matched_routines = 0u32;
        let mut fired_routines = 0u32;
        let mut decision_counts = serde_json::Map::new();
        let mut plans = Vec::new();
        let mut has_deferred = false;

        for cached in &cache {
            let routine = &cached.routine;
            if !routine_event_owner_matches(routine, &event) {
                continue;
            }

            owner_candidate_routines += 1;
            let sequence_num = plans.len() as u32;
            let pattern_matches = cached
                .regex
                .as_ref()
                .map(|regex| regex.is_match(&event.content))
                .unwrap_or(true);

            let (decision, reason, should_fire, trigger_key) = match evaluate_routine_event_filters(
                routine,
                &event,
                pattern_matches,
                now,
                self.config.default_event_max_age_secs,
            ) {
                RoutineEventFilterOutcome::Ignored { decision, reason } => {
                    (decision, Some(reason), false, None)
                }
                RoutineEventFilterOutcome::Matched {
                    trigger_key: candidate_trigger_key,
                } => {
                    matched_routines += 1;

                    // Content-hash window dedup (RoutineGuardrails.dedup_window):
                    // suppress semantically duplicate *distinct* events that
                    // already fired this routine within the window. The extra
                    // query only runs when a window is configured, so the
                    // common `dedup_window = None` path is unchanged.
                    let dedup_skip = match routine.guardrails.dedup_window {
                        Some(window) => {
                            let since = now
                                - ChronoDuration::from_std(window).unwrap_or_else(|_| {
                                    ChronoDuration::seconds(window.as_secs() as i64)
                                });
                            self.store
                                .routine_event_recent_content_match(
                                    routine.id,
                                    &event.content_hash,
                                    since,
                                )
                                .await
                                .map_err(|error| RoutineError::Database {
                                    reason: error.to_string(),
                                })?
                        }
                        None => false,
                    };
                    if dedup_skip {
                        increment_decision_count(
                            &mut decision_counts,
                            RoutineEventDecision::SkippedDuplicate,
                        );
                        plans.push(RoutineEventEvaluationPlan {
                            routine: routine.clone(),
                            decision: RoutineEventDecision::SkippedDuplicate,
                            reason: Some(
                                "content matched a recent fire within dedup_window".to_string(),
                            ),
                            details: routine_event_evaluation_details(
                                &self.worker_id,
                                &event,
                                now,
                                None,
                            ),
                            should_fire: false,
                            sequence_num,
                            trigger_key: None,
                        });
                        continue;
                    }

                    let duplicate_exists = self
                        .store
                        .routine_run_exists_for_trigger_key(routine.id, &candidate_trigger_key)
                        .await
                        .map_err(|error| RoutineError::Database {
                            reason: error.to_string(),
                        })?;
                    let cooldown_allowed = self.check_cooldown(routine);
                    let routine_capacity_available = if duplicate_exists || !cooldown_allowed {
                        true
                    } else {
                        self.check_concurrent(routine).await
                    };
                    let global_capacity_available = duplicate_exists
                        || !cooldown_allowed
                        || !routine_capacity_available
                        || (global_running + fired_routines as i64)
                            < self.config.max_concurrent_routines as i64;
                    let dispatch = decide_routine_event_dispatch(
                        duplicate_exists,
                        cooldown_allowed,
                        routine_capacity_available,
                        global_capacity_available,
                    );
                    has_deferred |= dispatch.deferred;
                    if dispatch.should_fire {
                        fired_routines += 1;
                    }
                    (
                        dispatch.decision,
                        Some(dispatch.reason),
                        dispatch.should_fire,
                        dispatch.should_fire.then_some(candidate_trigger_key),
                    )
                }
            };

            increment_decision_count(&mut decision_counts, decision);
            plans.push(RoutineEventEvaluationPlan {
                routine: routine.clone(),
                decision,
                reason,
                details: routine_event_evaluation_details(
                    &self.worker_id,
                    &event,
                    now,
                    trigger_key.as_deref(),
                ),
                should_fire,
                sequence_num,
                trigger_key,
            });
        }
        for plan in &plans {
            let evaluation = RoutineEventEvaluation {
                id: Uuid::new_v4(),
                event_id: event.id,
                routine_id: plan.routine.id,
                decision: plan.decision,
                reason: plan.reason.clone(),
                details: plan.details.clone(),
                sequence_num: plan.sequence_num,
                channel: event.channel.clone(),
                content_preview: content_preview.clone(),
                created_at: Utc::now(),
            };
            self.store
                .upsert_routine_event_evaluation(&evaluation)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
        }

        // Per-event error isolation: a single routine that fails to spawn must
        // not defer its sibling routines on the same event. Each spawn is
        // independently idempotent via `routine_run_exists_for_trigger_key`, so
        // failed routines are safely retried on the next drain while successful
        // siblings fire now. Accumulate every error for diagnostics.
        let mut dispatch_errors: Vec<String> = Vec::new();
        for plan in plans.iter().filter(|plan| plan.should_fire) {
            let Some(trigger_key) = plan.trigger_key.clone() else {
                continue;
            };
            if let Err(error) = self
                .spawn_fire(
                    plan.routine.clone(),
                    "event",
                    Some(content_preview.clone()),
                    Some(trigger_key),
                )
                .await
            {
                tracing::warn!(
                    routine = %plan.routine.name,
                    event_id = %event.id,
                    "Failed to spawn routine for event — continuing with siblings: {}",
                    error
                );
                dispatch_errors.push(format!("{}: {}", plan.routine.name, error));
            }
        }

        if !dispatch_errors.is_empty() {
            return Err(RoutineError::ExecutionFailed {
                reason: format!(
                    "{} routine event dispatch(es) failed: {}",
                    dispatch_errors.len(),
                    dispatch_errors.join("; ")
                ),
            });
        }

        // Keep the legacy dispatch-error keys in successful diagnostics. Actual
        // errors are routed through the bounded failure/dead-letter path above.
        let dispatch_error = dispatch_errors.first().cloned();
        let diagnostics = serde_json::json!({
            "channel": event.channel,
            "event_type": event.event_type,
            "content_preview": content_preview,
            "owner_candidate_routines": owner_candidate_routines,
            "identity_mismatch_count": total_event_routines.saturating_sub(owner_candidate_routines),
            "evaluated_routines": plans.len(),
            "matched_routines": matched_routines,
            "fired_routines": fired_routines,
            "decision_counts": decision_counts,
            "claimed_by": self.worker_id,
            "deferred": has_deferred,
            "dispatch_error": dispatch_error,
            "dispatch_errors": dispatch_errors,
        });

        if has_deferred {
            let retry_delay = routine_queue_retry_delay(event.attempt_count);
            let next_attempt_at = Utc::now()
                + ChronoDuration::from_std(retry_delay)
                    .unwrap_or_else(|_| ChronoDuration::seconds(1));
            self.store
                .release_routine_event(event.id, next_attempt_at, &diagnostics)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(fired_routines as usize);
        }

        self.store
            .complete_routine_event(
                event.id,
                Utc::now(),
                matched_routines,
                fired_routines,
                &diagnostics,
            )
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?;

        Ok(fired_routines as usize)
    }

    /// Spawn a fire in a background task.
    async fn spawn_fire(
        &self,
        routine: Routine,
        trigger_type: &str,
        trigger_detail: Option<String>,
        trigger_key: Option<String>,
    ) -> Result<Uuid, RoutineError> {
        // Resolve all fallible runtime inputs before the durable admission.
        // Once the transaction commits there is deliberately no await point
        // before task registration, minimizing the cancellation window that
        // would otherwise leave a leased but unowned Running row.
        let user_timezone = self.effective_timezone_for_routine(&routine).await;
        let started_at = Utc::now();
        let next_fire_at = next_fire_for_routine(&routine, user_timezone.as_deref(), started_at)?;
        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            scheduler: self.scheduler.clone(),
            sse_tx: self.sse_tx.clone(),
            system_event_tx: self.system_event_tx.clone(),
            subagent_executor: self.subagent_executor.clone(),
            user_timezone,
            desktop_autonomy_manager: self.desktop_autonomy_manager.clone(),
        };

        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: trigger_type.to_string(),
            trigger_detail,
            trigger_key,
            started_at,
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: started_at,
        };
        let initial_lease_expires_at =
            started_at + ChronoDuration::seconds(INITIAL_ROUTINE_RUN_LEASE_SECS);
        let global_limit = i64::try_from(self.config.max_concurrent_routines).unwrap_or(i64::MAX);
        match self
            .store
            .try_admit_routine_run(
                &run,
                i64::from(routine.guardrails.max_concurrent),
                global_limit,
                initial_lease_expires_at,
                next_fire_at,
            )
            .await
            .map_err(|error| RoutineError::Database {
                reason: format!("failed to admit routine run: {error}"),
            })? {
            RoutineRunAdmission::Admitted => {}
            RoutineRunAdmission::Duplicate(existing_run_id) => return Ok(existing_run_id),
            RoutineRunAdmission::RoutineCapacity | RoutineRunAdmission::GlobalCapacity => {
                return Err(RoutineError::MaxConcurrent {
                    name: routine.name.clone(),
                });
            }
        }

        let routine_name = routine.name.clone();
        let run_id = run.id;
        let run_for_task = run.clone();
        // IC-CRON-STAGGER: add random jitter before cron-triggered fires so a
        // post-downtime backlog of due cron routines doesn't thundering-herd
        // the LLM backend the moment the engine catches up. Other trigger
        // kinds (event, manual, system_event) fire immediately as before.
        let cron_jitter = if should_jitter_trigger_type(trigger_type) {
            StaggerConfig::from_env().jitter_delay()
        } else {
            std::time::Duration::ZERO
        };
        if !self.spawn_tracked_task(&routine_name, run.id, routine.id, async move {
            if !cron_jitter.is_zero() {
                tokio::time::sleep(cron_jitter).await;
            }
            execute_routine(engine, routine, run_for_task).await;
        }) {
            let reason = "Runtime shutdown closed routine task admission";
            if let Err(error) = finalize_routine_run_record(
                &self.store,
                run.id,
                RunStatus::Failed,
                Some(reason),
                None,
            )
            .await
            {
                tracing::error!(run_id = %run.id, %error, "Failed to close routine run rejected during shutdown");
            }
            return Err(RoutineError::ExecutionFailed {
                reason: reason.to_string(),
            });
        }

        Ok(run_id)
    }

    /// IC-018: Gracefully drain running inline routine tasks, then abort and
    /// durably fail only rows that are still Running. Called on engine shutdown.
    pub async fn abort_all(&self) {
        let mut tasks = {
            // Lock order is admission -> task set, matching spawn_tracked_task.
            let mut admission = self
                .task_admission_open
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *admission = false;
            let mut guard = self.active_tasks.lock().unwrap_or_else(|poisoned| {
                tracing::warn!("Recovering poisoned routine task registry during shutdown");
                poisoned.into_inner()
            });
            std::mem::take(&mut *guard)
        };

        let graceful = async {
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result {
                    tracing::warn!(%error, "Routine task failed during shutdown drain");
                }
            }
        };
        if tokio::time::timeout(ROUTINE_TASK_SHUTDOWN_GRACE, graceful)
            .await
            .is_ok()
        {
            tracing::info!("Drained all running routine tasks");
            return;
        }

        let interrupted = self
            .active_task_runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        tasks.abort_all();
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Routine task failed while aborting during shutdown");
            }
        }

        let reason = "Runtime shutdown interrupted routine execution";
        join_all(interrupted.into_keys().map(|run_id| async move {
            match tokio::time::timeout(
                ROUTINE_DB_OPERATION_TIMEOUT,
                finalize_routine_run_record(
                    &self.store,
                    run_id,
                    RunStatus::Failed,
                    Some(reason),
                    None,
                ),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::error!(%run_id, %error, "Failed to close interrupted routine run");
                }
                Err(_) => tracing::error!(
                    %run_id,
                    timeout_secs = ROUTINE_DB_OPERATION_TIMEOUT.as_secs(),
                    "Timed out closing interrupted routine run"
                ),
            }
        }))
        .await;
        tracing::info!("Aborted remaining routine tasks after shutdown grace period");
    }

    /// IC-006: Reap zombie routine runs whose lease has expired.
    ///
    /// Pure DB cleanup — marks stale `running` rows as `failed`. No in-memory
    /// counter manipulation needed because the DB is the single source of
    /// truth for global concurrency gating.
    ///
    /// Runs with a live lease are never reaped regardless of age — workers
    /// and subagents renew the lease while actively executing. Legacy rows
    /// with no lease at all fall back to
    /// [`crate::db::DEFAULT_LEGACY_ROUTINE_RUN_TTL_SECS`] instead of the old
    /// hardcoded 10-minute cutoff.
    pub async fn reap_zombie_runs(&self) {
        match self
            .store
            .cleanup_stale_routine_runs(crate::db::DEFAULT_LEGACY_ROUTINE_RUN_TTL_SECS)
            .await
        {
            Ok(result) => {
                for (routine_id, consecutive_failures) in &result.failure_streaks {
                    apply_routine_failure_streak_policy(
                        &self.store,
                        *routine_id,
                        *consecutive_failures,
                    )
                    .await;
                }
                if result.reaped > 0 {
                    tracing::info!("IC-006: Reaped {} zombie routine runs", result.reaped);
                }
            }
            Err(e) => {
                tracing::error!("Failed to reap zombie routine runs: {}", e);
            }
        }
    }

    fn check_cooldown(&self, routine: &Routine) -> bool {
        routine_cooldown_allows(routine, Utc::now())
    }

    async fn check_concurrent(&self, routine: &Routine) -> bool {
        match tokio::time::timeout(
            ROUTINE_DB_OPERATION_TIMEOUT,
            self.store.count_running_routine_runs(routine.id),
        )
        .await
        {
            Ok(Ok(count)) => count < i64::from(routine.guardrails.max_concurrent),
            Ok(Err(e)) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to check concurrent runs: {}", e
                );
                false
            }
            Err(_) => {
                tracing::error!(
                    routine = %routine.name,
                    timeout_secs = ROUTINE_DB_OPERATION_TIMEOUT.as_secs(),
                    "Timed out checking concurrent routine runs"
                );
                false
            }
        }
    }

    async fn count_global_running(&self) -> Result<i64, RoutineError> {
        match tokio::time::timeout(
            ROUTINE_DB_OPERATION_TIMEOUT,
            self.store.count_all_running_routine_runs(),
        )
        .await
        {
            Ok(Ok(count)) => Ok(count),
            Ok(Err(error)) => Err(RoutineError::Database {
                reason: error.to_string(),
            }),
            Err(_) => Err(RoutineError::Database {
                reason: format!(
                    "timed out counting running routines after {} seconds",
                    ROUTINE_DB_OPERATION_TIMEOUT.as_secs()
                ),
            }),
        }
    }

    fn spawn_tracked_task<F>(
        &self,
        routine_name: &str,
        run_id: Uuid,
        routine_id: Uuid,
        task: F,
    ) -> bool
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let admission = self
            .task_admission_open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*admission {
            return false;
        }
        let mut guard = self.active_tasks.lock().unwrap_or_else(|poisoned| {
            tracing::warn!(
                routine = routine_name,
                "Recovering poisoned routine task registry before spawning"
            );
            poisoned.into_inner()
        });
        // Drain finished entries first: a JoinSet retains completed-task
        // slots until joined, so an always-on engine would otherwise
        // accumulate one entry per run for the process lifetime. Joining is
        // also the only place a panicked routine task becomes visible —
        // surface it instead of silently discarding the JoinError.
        while let Some(joined) = guard.try_join_next() {
            if let Err(join_error) = joined
                && join_error.is_panic()
            {
                // The JoinSet is shared by all routines, so the panicked
                // task's own routine name is unknown at drain time — do NOT
                // attribute it to `routine_name` (the routine currently
                // spawning), which would misdirect debugging.
                tracing::error!(
                    "A previously-spawned routine task panicked (drained while spawning '{}'): {}",
                    routine_name,
                    join_error
                );
            }
        }
        self.active_task_runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(run_id, routine_id);
        let registration = ActiveRoutineTaskRegistration {
            run_id,
            active_runs: Arc::clone(&self.active_task_runs),
        };
        let store = Arc::clone(&self.store);
        let routine_name = routine_name.to_string();
        guard.spawn(async move {
            let _registration = registration;
            if let Err(panic) = std::panic::AssertUnwindSafe(task).catch_unwind().await {
                let panic_message = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                let reason = format!("Routine task panicked: {panic_message}");
                tracing::error!(
                    routine = %routine_name,
                    %run_id,
                    %panic_message,
                    "Routine execution task panicked"
                );
                match tokio::time::timeout(
                    ROUTINE_DB_OPERATION_TIMEOUT,
                    finalize_routine_run_record(
                        &store,
                        run_id,
                        RunStatus::Failed,
                        Some(&reason),
                        None,
                    ),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => tracing::error!(
                        routine = %routine_name,
                        %run_id,
                        %error,
                        "Failed to close panicked routine run"
                    ),
                    Err(_) => tracing::error!(
                        routine = %routine_name,
                        %run_id,
                        "Timed out closing panicked routine run"
                    ),
                }
            }
        });
        true
    }
}

/// Shared context passed to the execution function.
struct EngineContext {
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    notify_tx: mpsc::Sender<OutgoingResponse>,
    scheduler: Option<Arc<Scheduler>>,
    /// Optional SSE broadcast sender for routine lifecycle events.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Optional sender for injecting messages into the main session.
    system_event_tx: Option<mpsc::Sender<IncomingMessage>>,
    /// Optional subagent executor for non-heartbeat automations.
    subagent_executor: Option<Arc<SubagentExecutor>>,
    /// User timezone (IANA) for active-hours checks.
    user_timezone: Option<String>,
    /// Runtime-scoped desktop autonomy state inherited from the engine.
    desktop_autonomy_manager: Option<Arc<crate::desktop_autonomy::DesktopAutonomyManager>>,
}

impl EngineContext {
    /// Broadcast an SSE event if the sender is available.
    fn broadcast_sse(&self, event: SseEvent) {
        if let Some(ref tx) = self.sse_tx {
            let _ = tx.send(event);
        }
    }
}

/// Win a routine run's terminal compare-and-set and apply failure policy
/// against the exact failure streak produced by that transition.
///
/// Returns `Ok(false)` when another finalizer already committed the terminal
/// result. Callers must then skip duplicate notifications/artifacts.
pub(crate) async fn finalize_routine_run_record(
    store: &Arc<dyn Database>,
    run_id: Uuid,
    status: RunStatus,
    result_summary: Option<&str>,
    tokens_used: Option<i32>,
) -> Result<bool, DatabaseError> {
    let completion = store
        .complete_routine_run(run_id, status, result_summary, tokens_used)
        .await?;
    let RoutineRunCompletion::Completed {
        routine_id,
        consecutive_failures,
    } = completion
    else {
        return Ok(false);
    };

    apply_routine_failure_streak_policy(store, routine_id, consecutive_failures).await;

    Ok(true)
}

pub(crate) async fn apply_routine_failure_streak_policy(
    store: &Arc<dyn Database>,
    routine_id: Uuid,
    consecutive_failures: u32,
) {
    let Some(backoff) = routine_failure_backoff(consecutive_failures) else {
        return;
    };
    let disable = routine_should_auto_disable(consecutive_failures);
    let not_before = Utc::now() + backoff;
    match store
        .apply_routine_failure_policy(routine_id, consecutive_failures, not_before, disable)
        .await
    {
        Ok(true) if disable => tracing::warn!(
            %routine_id,
            consecutive_failures,
            "Routine auto-disabled after repeated consecutive failures"
        ),
        Ok(_) => {}
        Err(error) => tracing::error!(
            %routine_id,
            consecutive_failures,
            %error,
            "Failed to apply routine failure backoff/disable policy"
        ),
    }
}

/// Renew a routine run's reaper lease if enough of the window has elapsed.
///
/// Execution-loop iterations can be sub-second, and a DB write per iteration
/// is wasted I/O for a lease measured in minutes — renewal is gated to once
/// per third of the lease window. The lease is padded past `timeout_secs` so
/// a slow-but-alive iteration never races the reaper. Shared by the worker
/// and subagent execution loops so the formula and gating cannot drift.
pub(crate) async fn renew_routine_run_lease_if_due(
    store: &Arc<dyn Database>,
    run_id: Uuid,
    timeout_secs: u64,
    last_renewed: &std::sync::Mutex<Option<std::time::Instant>>,
) {
    let lease_secs = (timeout_secs as i64).saturating_add(120).max(120);
    let renew_every = Duration::from_secs(lease_secs as u64 / 3);
    {
        let last = last_renewed.lock().unwrap_or_else(|p| p.into_inner());
        if last.is_some_and(|at| at.elapsed() < renew_every) {
            return;
        }
    }
    match store.renew_routine_run_lease(run_id, lease_secs).await {
        Ok(()) => {
            *last_renewed.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(std::time::Instant::now());
        }
        Err(e) => {
            tracing::debug!(run_id = %run_id, "Failed to renew routine run lease: {}", e);
        }
    }
}

/// IC-006: Spawn a periodic zombie reaper for routine runs.
/// Checks every 2 minutes for runs whose renewable lease has expired
/// (legacy NULL-lease rows fall back to a fixed TTL — see
/// `DEFAULT_LEGACY_ROUTINE_RUN_TTL_SECS`).
pub fn spawn_zombie_reaper(engine: Arc<RoutineEngine>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(120));
        interval.tick().await; // skip immediate tick
        loop {
            interval.tick().await;
            engine.reap_zombie_runs().await;
        }
    })
}

pub fn spawn_zombie_reaper_with_shutdown(
    engine: Arc<RoutineEngine>,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(120));
        interval.tick().await; // skip immediate tick
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("routine zombie reaper shutting down");
                    return;
                }
                _ = interval.tick() => {
                    engine.reap_zombie_runs().await;
                }
            }
        }
    })
}

/// Spawn the cron ticker background task.
pub fn spawn_cron_ticker(
    engine: Arc<RoutineEngine>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        engine.refresh_event_cache().await;
        engine.drain_pending_event_queue().await;
        engine.check_cron_triggers().await;

        let mut ticker = tokio::time::interval(interval);
        // Align the first timed tick to `interval` after the startup catch-up.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            engine.refresh_event_cache().await;
            engine.drain_pending_event_queue().await;
            engine.check_cron_triggers().await;
            // IC-006: Zombie reaping is handled by the dedicated spawn_zombie_reaper
            // task (every 120s). The cron ticker only checks triggers.
        }
    })
}

pub fn spawn_cron_ticker_with_shutdown(
    engine: Arc<RoutineEngine>,
    interval: Duration,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        engine.refresh_event_cache().await;
        engine.drain_pending_event_queue().await;
        engine.check_cron_triggers().await;

        let mut ticker = tokio::time::interval(interval);
        // Align the first timed tick to `interval` after the startup catch-up.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("routine cron ticker shutting down");
                    return;
                }
                _ = ticker.tick() => {
                    engine.refresh_event_cache().await;
                    engine.drain_pending_event_queue().await;
                    engine.check_cron_triggers().await;
                    // IC-006: Zombie reaping is handled by the dedicated
                    // spawn_zombie_reaper task. The cron ticker only checks triggers.
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;

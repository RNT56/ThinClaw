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
use regex::Regex;
use thinclaw_agent::routine_engine::{
    EVENT_CONTENT_PREVIEW_LIMIT, FullJobRuntimeMetadata, active_hour_allows,
    build_heartbeat_prompt, build_lightweight_routine_prompt, build_routine_event_from_message,
    build_routine_notification, build_scheduled_routine_triggers,
    classify_lightweight_routine_response, effective_lightweight_max_tokens, event_run_trigger_key,
    full_job_metadata, heartbeat_job_metadata, increment_decision_count,
    lightweight_routine_messages, metadata_contains_subset, routine_cooldown_allows,
    routine_requests_desktop_capabilities, sanitize_routine_name, scheduled_run_trigger_key,
    should_refresh_event_cache, summarize_runtime_capabilities, truncate,
};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::agent::outcomes;
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineCatchUpMode, RoutineEvent, RoutineEventDecision,
    RoutineEventEvaluation, RoutineRun, RoutineTrigger, RoutineTriggerDecision, RoutineTriggerKind,
    RunStatus, Trigger, compile_event_trigger_pattern, next_fire_for_routine,
    routine_state_with_runtime_advance,
};
use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::agent::{AgentRunArtifact, AgentRunStatus};
use crate::api::experiments as experiments_api;
use crate::channels::web::types::SseEvent;
use crate::channels::{IncomingMessage, OutgoingResponse};
use crate::config::RoutineConfig;
use crate::db::Database;
use crate::error::{DatabaseError, RoutineError};
use crate::llm::{CompletionRequest, LlmProvider};
use crate::tools::ToolProfile;
use crate::tools::execution_backend::routine_engine_runtime_descriptor;
use crate::workspace::Workspace;

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
    /// User timezone (IANA) for active-hours checks. Populated from Settings.
    user_timezone: Option<String>,
    /// Stable worker id used when claiming persisted event inbox items.
    worker_id: String,
}

const EVENT_QUEUE_BATCH_LIMIT: i64 = 64;
const TRIGGER_QUEUE_BATCH_LIMIT: i64 = 64;

#[derive(Clone)]
struct CachedEventRoutine {
    routine: Routine,
    regex: Option<Regex>,
}

impl RoutineEngine {
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
            sse_tx: None,
            system_event_tx: None,
            subagent_executor: None,
            active_tasks: Arc::new(std::sync::Mutex::new(tokio::task::JoinSet::new())),
            user_timezone: None,
            worker_id: Uuid::new_v4().to_string(),
        }
    }

    /// Set the SSE broadcast sender for emitting routine lifecycle events.
    pub fn with_sse_sender(mut self, tx: tokio::sync::broadcast::Sender<SseEvent>) -> Self {
        self.sse_tx = Some(tx);
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
                    right
                        .routine
                        .event_priority()
                        .cmp(&left.routine.event_priority())
                        .then_with(|| left.routine.created_at.cmp(&right.routine.created_at))
                        .then_with(|| left.routine.name.cmp(&right.routine.name))
                        .then_with(|| left.routine.id.cmp(&right.routine.id))
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
        self.ensure_event_cache_loaded().await;
        let event = match self.enqueue_routine_event(message).await {
            Ok(event) => event,
            Err(error) => {
                tracing::error!("Failed to enqueue routine event: {}", error);
                return 0;
            }
        };

        if let Some(manager) = crate::desktop_autonomy::desktop_autonomy_manager()
            && manager.emergency_stop_active()
        {
            tracing::warn!(
                event_id = %event.id,
                "Desktop autonomy emergency stop is active; leaving event queued"
            );
            return 0;
        }

        match self.try_process_routine_event(event.id).await {
            Ok(Some(fired)) => fired,
            Ok(None) => 0,
            Err(error) => {
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
        if let Some(manager) = crate::desktop_autonomy::desktop_autonomy_manager()
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
        if !self.check_concurrent(&routine).await {
            return Err(RoutineError::MaxConcurrent {
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

        if !self.check_concurrent(&routine).await {
            return Err(RoutineError::MaxConcurrent {
                name: routine.name.clone(),
            });
        }

        self.spawn_fire(routine, "manual", None, None).await
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
        for trigger in build_scheduled_routine_triggers(
            routine,
            self.user_timezone.as_deref(),
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
        let mut total_fired = 0usize;

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
                    tracing::error!("Failed to claim scheduled routine triggers: {}", error);
                    break;
                }
            };

            if claimed.is_empty() {
                break;
            }

            let batch_len = claimed.len();
            for trigger in claimed {
                match self.process_claimed_trigger(trigger).await {
                    Ok(fired) => total_fired += usize::from(fired),
                    Err(error) => {
                        tracing::error!("Failed to process scheduled routine trigger: {}", error);
                    }
                }
            }

            if batch_len < TRIGGER_QUEUE_BATCH_LIMIT as usize {
                break;
            }
        }

        total_fired
    }

    async fn process_claimed_trigger(&self, trigger: RoutineTrigger) -> Result<bool, RoutineError> {
        let Some(routine) = self
            .store
            .get_routine(trigger.routine_id)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?
        else {
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::SkippedDisabled.to_string(),
                "reason": "routine no longer exists",
                "claimed_by": self.worker_id,
            });
            self.store
                .complete_routine_trigger(
                    trigger.id,
                    Utc::now(),
                    RoutineTriggerDecision::SkippedDisabled,
                    &diagnostics,
                )
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        };

        if !routine.enabled {
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::SkippedDisabled.to_string(),
                "reason": "routine is disabled",
                "claimed_by": self.worker_id,
            });
            self.store
                .complete_routine_trigger(
                    trigger.id,
                    Utc::now(),
                    RoutineTriggerDecision::SkippedDisabled,
                    &diagnostics,
                )
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        }

        let trigger_key = scheduled_run_trigger_key(&trigger);
        if self
            .store
            .routine_run_exists_for_trigger_key(routine.id, &trigger_key)
            .await
            .map_err(|error| RoutineError::Database {
                reason: error.to_string(),
            })?
        {
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::SkippedDuplicate.to_string(),
                "reason": "a run already exists for this logical scheduled trigger",
                "claimed_by": self.worker_id,
                "idempotency_key": trigger.idempotency_key,
            });
            self.store
                .complete_routine_trigger(
                    trigger.id,
                    Utc::now(),
                    RoutineTriggerDecision::SkippedDuplicate,
                    &diagnostics,
                )
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        }

        if matches!(routine.policy.catch_up_mode, RoutineCatchUpMode::Skip) {
            let next_fire =
                next_fire_for_routine(&routine, self.user_timezone.as_deref(), Utc::now())?;
            self.reschedule_without_run(&routine, next_fire).await?;
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::SkippedCatchUp.to_string(),
                "reason": "catch-up mode is skip; backlog was collapsed without execution",
                "claimed_by": self.worker_id,
                "backlog_collapsed": trigger.backlog_collapsed,
                "next_fire_at": next_fire.map(|value| value.to_rfc3339()),
            });
            self.store
                .complete_routine_trigger(
                    trigger.id,
                    Utc::now(),
                    RoutineTriggerDecision::SkippedCatchUp,
                    &diagnostics,
                )
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        }

        if !self.check_cooldown(&routine) {
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::DeferredCooldown.to_string(),
                "reason": "routine cooldown is still active",
                "claimed_by": self.worker_id,
                "due_at": trigger.due_at.to_rfc3339(),
            });
            self.store
                .release_routine_trigger(trigger.id, &diagnostics)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        }

        if !matches!(trigger.trigger_kind, RoutineTriggerKind::SystemEvent)
            && !self.check_concurrent(&routine).await
        {
            let diagnostics = serde_json::json!({
                "decision": RoutineTriggerDecision::DeferredConcurrency.to_string(),
                "reason": "routine is already at max concurrent runs",
                "claimed_by": self.worker_id,
                "due_at": trigger.due_at.to_rfc3339(),
            });
            self.store
                .release_routine_trigger(trigger.id, &diagnostics)
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;
            return Ok(false);
        }

        if !matches!(trigger.trigger_kind, RoutineTriggerKind::SystemEvent) {
            let global_running =
                self.store
                    .count_all_running_routine_runs()
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
            if global_running >= self.config.max_concurrent_routines as i64 {
                let diagnostics = serde_json::json!({
                    "decision": RoutineTriggerDecision::DeferredGlobalCapacity.to_string(),
                    "reason": "global routine capacity is currently full",
                    "claimed_by": self.worker_id,
                    "due_at": trigger.due_at.to_rfc3339(),
                });
                self.store
                    .release_routine_trigger(trigger.id, &diagnostics)
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?;
                return Ok(false);
            }
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
                Ok(true)
            }
            RoutineTriggerKind::SystemEvent => {
                if let Err(error) = self
                    .dispatch_system_event(&routine, &trigger, &trigger_key)
                    .await
                {
                    let diagnostics = serde_json::json!({
                        "decision": RoutineTriggerDecision::DeferredGlobalCapacity.to_string(),
                        "claimed_by": self.worker_id,
                        "reason": error.to_string(),
                        "due_at": trigger.due_at.to_rfc3339(),
                    });
                    self.store
                        .release_routine_trigger(trigger.id, &diagnostics)
                        .await
                        .map_err(|store_error| RoutineError::Database {
                            reason: store_error.to_string(),
                        })?;
                    return Ok(false);
                }
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
                Ok(true)
            }
        }
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

        let event_message = IncomingMessage::new("heartbeat", "system", message.clone())
            .with_metadata(serde_json::json!({
                "source": "system_event",
                "routine_id": routine.id.to_string(),
                "routine_name": routine.name,
                "trigger_id": trigger.id.to_string(),
                "trigger_key": trigger_key,
                "scheduled_due_at": trigger.due_at.to_rfc3339(),
            }));
        tx.send(event_message)
            .await
            .map_err(|error| RoutineError::ExecutionFailed {
                reason: format!("failed to enqueue system event: {error}"),
            })?;

        let now = Utc::now();
        let next_fire = next_fire_for_routine(routine, self.user_timezone.as_deref(), now)?;
        persist_routine_runtime_update(
            &self.store,
            routine.id,
            now,
            next_fire,
            routine.run_count + 1,
            0,
            &routine.state,
        )
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
        let mut updated = routine.clone();
        updated.next_fire_at = next_fire_at;
        self.store
            .update_routine(&updated)
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
                if let Err(store_error) = self
                    .store
                    .fail_routine_event(event.id, Utc::now(), &error.to_string())
                    .await
                {
                    tracing::error!(
                        event_id = %event.id,
                        "Failed to mark routine event as failed: {}",
                        store_error
                    );
                }
                Err(error)
            }
        }
    }

    pub async fn drain_pending_event_queue(&self) -> usize {
        if let Some(manager) = crate::desktop_autonomy::desktop_autonomy_manager()
            && manager.emergency_stop_active()
        {
            tracing::warn!("Desktop autonomy emergency stop is active; skipping event inbox drain");
            return 0;
        }

        self.ensure_event_cache_loaded().await;
        let mut total_fired = 0usize;

        loop {
            let pending = match self
                .store
                .list_pending_routine_events(self.claim_stale_before(), EVENT_QUEUE_BATCH_LIMIT)
                .await
            {
                Ok(events) => events,
                Err(error) => {
                    tracing::error!("Failed to load pending routine events: {}", error);
                    break;
                }
            };

            if pending.is_empty() {
                break;
            }

            let batch_len = pending.len();
            for event in pending {
                match self.try_process_routine_event(event.id).await {
                    Ok(Some(fired)) => total_fired += fired,
                    Ok(None) => {}
                    Err(error) => {
                        tracing::error!(
                            event_id = %event.id,
                            "Failed to drain pending routine event: {}",
                            error
                        );
                    }
                }
            }

            if batch_len < EVENT_QUEUE_BATCH_LIMIT as usize {
                break;
            }
        }

        total_fired
    }

    async fn process_claimed_event(&self, event: RoutineEvent) -> Result<usize, RoutineError> {
        self.ensure_event_cache_loaded().await;

        let cache = self.event_cache.read().await;
        let total_event_routines = cache.len() as u32;
        let global_running =
            self.store
                .count_all_running_routine_runs()
                .await
                .map_err(|error| RoutineError::Database {
                    reason: error.to_string(),
                })?;

        let now = Utc::now();
        let content_preview = truncate(&event.content, EVENT_CONTENT_PREVIEW_LIMIT);
        let mut owner_candidate_routines = 0u32;
        let mut matched_routines = 0u32;
        let mut fired_routines = 0u32;
        let mut decision_counts = serde_json::Map::new();
        let mut plans = Vec::new();
        let mut has_deferred = false;

        for cached in cache.iter() {
            let routine = &cached.routine;
            if routine.user_id != event.principal_id || routine.owner_actor_id() != event.actor_id {
                continue;
            }

            owner_candidate_routines += 1;
            let sequence_num = plans.len() as u32;
            let age_secs = now
                .signed_duration_since(event.created_at)
                .num_seconds()
                .max(0) as u64;
            let mut details = serde_json::json!({
                "claimed_by": self.worker_id,
                "event_age_secs": age_secs,
                "event_type": event.event_type,
                "content_hash": event.content_hash,
            });

            let Trigger::Event {
                channel,
                event_type,
                actor,
                metadata,
                ..
            } = &routine.trigger
            else {
                continue;
            };

            let (decision, reason, should_fire, trigger_key) = if age_secs
                > routine.effective_event_max_age_secs(self.config.default_event_max_age_secs)
            {
                (
                    RoutineEventDecision::SkippedExpired,
                    Some("durably queued event exceeded the routine max age".to_string()),
                    false,
                    None,
                )
            } else if channel
                .as_ref()
                .is_some_and(|value| value != &event.channel)
            {
                (
                    RoutineEventDecision::IgnoredChannel,
                    Some(format!(
                        "event channel '{}' does not match routine channel '{}'",
                        event.channel,
                        channel.as_deref().unwrap_or_default()
                    )),
                    false,
                    None,
                )
            } else if event_type
                .as_ref()
                .is_some_and(|value| value != &event.event_type)
            {
                (
                    RoutineEventDecision::IgnoredEventType,
                    Some(format!(
                        "event type '{}' does not match routine event type '{}'",
                        event.event_type,
                        event_type.as_deref().unwrap_or_default()
                    )),
                    false,
                    None,
                )
            } else if actor
                .as_ref()
                .is_some_and(|value| value != &event.actor_id && value != &event.raw_sender_id)
            {
                (
                    RoutineEventDecision::IgnoredActor,
                    Some("event actor did not match the routine actor filter".to_string()),
                    false,
                    None,
                )
            } else if metadata
                .as_ref()
                .is_some_and(|expected| !metadata_contains_subset(expected, &event.metadata))
            {
                (
                    RoutineEventDecision::IgnoredMetadata,
                    Some("event metadata did not match the routine metadata filter".to_string()),
                    false,
                    None,
                )
            } else if cached
                .regex
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&event.content))
            {
                (
                    RoutineEventDecision::IgnoredPattern,
                    Some("pattern did not match event content".to_string()),
                    false,
                    None,
                )
            } else {
                matched_routines += 1;
                let candidate_trigger_key = event_run_trigger_key(&event);
                if self
                    .store
                    .routine_run_exists_for_trigger_key(routine.id, &candidate_trigger_key)
                    .await
                    .map_err(|error| RoutineError::Database {
                        reason: error.to_string(),
                    })?
                {
                    (
                        RoutineEventDecision::SkippedDuplicate,
                        Some(
                            "this event already produced a logical run for the routine".to_string(),
                        ),
                        false,
                        None,
                    )
                } else if !self.check_cooldown(routine) {
                    (
                        RoutineEventDecision::SkippedCooldown,
                        Some("routine cooldown is still active".to_string()),
                        false,
                        None,
                    )
                } else if !self.check_concurrent(routine).await {
                    has_deferred = true;
                    (
                        RoutineEventDecision::DeferredConcurrency,
                        Some("routine is already at max concurrent runs".to_string()),
                        false,
                        None,
                    )
                } else if (global_running + fired_routines as i64)
                    >= self.config.max_concurrent_routines as i64
                {
                    has_deferred = true;
                    (
                        RoutineEventDecision::DeferredGlobalCapacity,
                        Some("global routine capacity is currently full".to_string()),
                        false,
                        None,
                    )
                } else {
                    fired_routines += 1;
                    (
                        RoutineEventDecision::Fired,
                        Some("event matched and the routine was dispatched".to_string()),
                        true,
                        Some(candidate_trigger_key),
                    )
                }
            };

            increment_decision_count(&mut decision_counts, decision);
            if let Some(trigger_key) = &trigger_key {
                details["trigger_key"] = serde_json::json!(trigger_key);
            }
            plans.push(EventEvaluationPlan {
                routine: routine.clone(),
                decision,
                reason,
                details,
                should_fire,
                sequence_num,
                trigger_key,
            });
        }
        drop(cache);

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

        let mut dispatch_error = None;
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
                dispatch_error = Some(error.to_string());
                has_deferred = true;
                break;
            }
        }

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
        });

        if has_deferred {
            self.store
                .release_routine_event(event.id, &diagnostics)
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
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: trigger_type.to_string(),
            trigger_detail,
            trigger_key,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        self.store
            .create_routine_run(&run)
            .await
            .map_err(|error| RoutineError::Database {
                reason: format!("failed to create run record: {error}"),
            })?;

        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            scheduler: self.scheduler.clone(),
            sse_tx: self.sse_tx.clone(),
            system_event_tx: self.system_event_tx.clone(),
            subagent_executor: self.subagent_executor.clone(),
            user_timezone: self.user_timezone.clone(),
        };

        let routine_name = routine.name.clone();
        let run_id = run.id;
        let run_for_task = run.clone();
        self.spawn_tracked_task(&routine_name, async move {
            execute_routine(engine, routine, run_for_task).await;
        })
        .map_err(|reason| RoutineError::ExecutionFailed { reason })?;

        Ok(run_id)
    }

    /// IC-018: Abort all running routine tasks. Called on engine shutdown.
    pub async fn abort_all(&self) {
        // std::sync::Mutex — lock is sync, no await inside the guard scope.
        if let Ok(mut guard) = self.active_tasks.lock() {
            guard.abort_all();
        }
        tracing::info!("Aborted all running routine tasks");
    }

    /// IC-006: Reap zombie routine runs that have exceeded the 10-minute TTL.
    ///
    /// Pure DB cleanup — marks stale `running` rows as `failed`. No in-memory
    /// counter manipulation needed because the DB is the single source of
    /// truth for global concurrency gating.
    pub async fn reap_zombie_runs(&self) {
        match self.store.cleanup_stale_routine_runs().await {
            Ok(reaped) => {
                if reaped > 0 {
                    tracing::info!("IC-006: Reaped {} zombie routine runs", reaped);
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
        match self.store.count_running_routine_runs(routine.id).await {
            Ok(count) => count < routine.guardrails.max_concurrent as i64,
            Err(e) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to check concurrent runs: {}", e
                );
                false
            }
        }
    }

    fn spawn_tracked_task<F>(&self, routine_name: &str, task: F) -> Result<(), String>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut guard = self.active_tasks.lock().map_err(|_| {
            format!(
                "active_tasks mutex poisoned — routine '{}' not spawned",
                routine_name
            )
        })?;
        guard.spawn(task);
        Ok(())
    }
}

struct EventEvaluationPlan {
    routine: Routine,
    decision: RoutineEventDecision,
    reason: Option<String>,
    details: serde_json::Value,
    should_fire: bool,
    sequence_num: u32,
    trigger_key: Option<String>,
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
}

impl EngineContext {
    /// Broadcast an SSE event if the sender is available.
    fn broadcast_sse(&self, event: SseEvent) {
        if let Some(ref tx) = self.sse_tx {
            let _ = tx.send(event);
        }
    }
}

pub(crate) async fn persist_routine_runtime_update(
    store: &Arc<dyn Database>,
    routine_id: Uuid,
    last_run_at: chrono::DateTime<Utc>,
    next_fire_at: Option<chrono::DateTime<Utc>>,
    run_count: u64,
    consecutive_failures: u32,
    state: &serde_json::Value,
) -> Result<(), DatabaseError> {
    let mut last_error = None;
    for attempt in 1..=3 {
        match store
            .update_routine_runtime(
                routine_id,
                last_run_at,
                next_fire_at,
                run_count,
                consecutive_failures,
                state,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 3 {
                    tracing::warn!(
                        routine_id = %routine_id,
                        attempt,
                        "Failed to persist routine runtime update; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                }
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| DatabaseError::Query("unknown runtime update failure".to_string())))
}

/// IC-006: Spawn a periodic zombie reaper for routine runs.
/// Checks every 2 minutes for runs that have exceeded the 10-minute TTL.
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

/// Execute a routine run. Handles both lightweight and full_job modes.
async fn execute_routine(ctx: EngineContext, routine: Routine, run: RoutineRun) {
    // Broadcast routine start event
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "started".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: None,
    });

    let result = match &routine.action {
        RoutineAction::Lightweight {
            prompt,
            context_paths,
            max_tokens,
        } => execute_lightweight(&ctx, &routine, prompt, context_paths, *max_tokens).await,
        RoutineAction::FullJob {
            title,
            description,
            max_iterations,
            allowed_tools,
            allowed_skills,
            tool_profile,
        } => {
            if ctx.subagent_executor.is_some() {
                execute_as_subagent(
                    &ctx,
                    &routine,
                    &run,
                    title,
                    description,
                    allowed_tools.as_deref(),
                    allowed_skills.as_deref(),
                    *tool_profile,
                )
                .await
            } else {
                execute_full_job(
                    &ctx,
                    &routine,
                    &run,
                    title,
                    description,
                    *max_iterations,
                    allowed_tools.as_deref(),
                    allowed_skills.as_deref(),
                    *tool_profile,
                )
                .await
            }
        }
        RoutineAction::Heartbeat {
            light_context,
            prompt,
            include_reasoning,
            active_start_hour,
            active_end_hour,
            target,
            max_iterations,
            ..
        } => {
            execute_heartbeat(
                &ctx,
                &routine,
                &run,
                *light_context,
                prompt.as_deref(),
                *include_reasoning,
                *active_start_hour,
                *active_end_hour,
                target,
                *max_iterations,
            )
            .await
        }
        RoutineAction::ExperimentCampaign {
            project_id,
            runner_profile_id,
            max_trials_override,
        } => Ok(
            match experiments_api::start_campaign(
                &ctx.store,
                "default",
                *project_id,
                experiments_api::StartExperimentCampaignRequest {
                    runner_profile_id: *runner_profile_id,
                    max_trials_override: *max_trials_override,
                    gateway_url: None,
                },
            )
            .await
            {
                Ok(response) => (
                    RunStatus::Attention,
                    Some(format!(
                        "Experiment campaign {} started: {}",
                        response.campaign.id, response.message
                    )),
                    None,
                ),
                Err(error) => (RunStatus::Failed, Some(error.to_string()), None),
            },
        ),
    };

    // Process result
    let (status, summary, tokens) = match result {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(routine = %routine.name, "Execution failed: {}", e);
            (RunStatus::Failed, Some(e.to_string()), None)
        }
    };

    // RunStatus::Running means the job was dispatched to a worker or subagent.
    // The worker/subagent handles its own DB completion + SSE lifecycle event,
    // so skip all post-processing here to avoid conflicts.
    if status == RunStatus::Running {
        // Still update the routine schedule so next_fire_at advances
        let now = Utc::now();
        let next_fire =
            next_fire_for_routine(&routine, ctx.user_timezone.as_deref(), now).unwrap_or(None);
        let state = routine_state_with_runtime_advance(&routine.state, run.id, now);
        if let Err(error) = persist_routine_runtime_update(
            &ctx.store,
            routine.id,
            now,
            next_fire,
            routine.run_count + 1,
            routine.consecutive_failures,
            &state,
        )
        .await
        {
            tracing::error!(
                routine = %routine.name,
                run_id = %run.id,
                "Failed to persist dispatched routine runtime state: {}",
                error
            );
        }
        return;
    }

    let now = Utc::now();
    let next_fire =
        next_fire_for_routine(&routine, ctx.user_timezone.as_deref(), now).unwrap_or(None);

    let new_failures = if status == RunStatus::Failed {
        routine.consecutive_failures + 1
    } else {
        0
    };

    if let Err(e) = persist_routine_runtime_update(
        &ctx.store,
        routine.id,
        now,
        next_fire,
        routine.run_count + 1,
        new_failures,
        &routine.state,
    )
    .await
    {
        tracing::error!(routine = %routine.name, "Failed to update runtime state: {}", e);
    }

    // Complete the run record after advancing the parent routine state so a
    // visible terminal run also has consistent runtime metadata.
    if let Err(e) = ctx
        .store
        .complete_routine_run(run.id, status, summary.as_deref(), tokens)
        .await
    {
        tracing::error!(routine = %routine.name, "Failed to complete run record: {}", e);
    }

    let mut completed_run = run.clone();
    completed_run.status = status;
    completed_run.result_summary = summary.clone();
    completed_run.tokens_used = tokens;
    completed_run.completed_at = Some(Utc::now());
    if let Err(err) =
        outcomes::maybe_create_routine_contract(&ctx.store, &routine, &completed_run).await
    {
        tracing::debug!(routine = %routine.name, error = %err, "Outcome routine contract hook skipped");
    }
    let run_artifact = AgentRunArtifact::new(
        "routine_run",
        match status {
            RunStatus::Failed => AgentRunStatus::Failed,
            RunStatus::Ok | RunStatus::Attention | RunStatus::Running => AgentRunStatus::Completed,
        },
        run.started_at,
        completed_run.completed_at,
    )
    .with_failure_reason(
        summary
            .as_ref()
            .filter(|_| status == RunStatus::Failed)
            .cloned(),
    )
    .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
        &routine_engine_runtime_descriptor(),
    )))
    .with_metadata(serde_json::json!({
        "event": "routine_run_completed",
        "routine_id": routine.id,
        "routine_name": routine.name.clone(),
        "run_id": completed_run.id,
        "status": status.to_string(),
        "result_summary": completed_run.result_summary.clone(),
        "tokens_used": completed_run.tokens_used,
    }));
    let routine_user_id = routine.user_id.clone();
    let provider_store = Arc::clone(&ctx.store);
    let mut run_artifact = run_artifact;
    run_artifact.user_id = Some(routine.user_id.clone());
    run_artifact.actor_id = Some(routine.owner_actor_id().to_string());
    tokio::spawn(async move {
        let harness = crate::agent::AgentRunHarness::new(None);
        if let Err(err) = harness.append_artifact(&run_artifact).await {
            tracing::debug!(error = %err, "Failed to append routine run artifact");
        }
        let manager = crate::agent::learning::MemoryProviderManager::new(provider_store);
        manager
            .session_end_extract(&routine_user_id, &run_artifact)
            .await;
    });

    // Send notifications based on config
    send_notification(
        &ctx.notify_tx,
        &routine.notify,
        &routine.name,
        status,
        summary.as_deref(),
    )
    .await;

    let event_type = match status {
        RunStatus::Ok => "completed",
        RunStatus::Attention => "attention",
        RunStatus::Failed => "failed",
        RunStatus::Running => unreachable!(), // handled above
    };
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: event_type.to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: summary.clone(),
    });
}

/// Execute a non-heartbeat automation as a subagent.
///
/// Routes through the SubagentExecutor for UI isolation (dedicated split pane),
/// fresh context per run, and proper cancellation support. The subagent executor
/// handles its own SSE lifecycle events via SubagentSpawned / SubagentProgress /
/// SubagentCompleted status updates. Returns `RunStatus::Running` so the calling
/// `execute_routine` skips premature `complete_routine_run`.
async fn execute_as_subagent(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    title: &str,
    description: &str,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    tool_profile: Option<ToolProfile>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let executor = ctx
        .subagent_executor
        .as_ref()
        .ok_or_else(|| RoutineError::ExecutionFailed {
            reason: "SubagentExecutor not available".into(),
        })?;

    let request = SubagentSpawnRequest {
        name: format!("Automation: {}", routine.name),
        task: description.to_string(),
        system_prompt: Some(format!(
            "You are executing the automation '{}'. \
             Complete the task thoroughly and report results via `emit_user_message`. \
             Use tools as needed. When finished, return a clear summary.\n\n\
             Title: {}\n\nDescription: {}",
            routine.name, title, description
        )),
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile,
        allowed_tools: allowed_tools.map(|tools| tools.to_vec()),
        allowed_skills: allowed_skills.map(|skills| skills.to_vec()),
        principal_id: Some(routine.user_id.clone()),
        actor_id: Some(routine.owner_actor_id().to_string()),
        agent_workspace_id: None,
        timeout_secs: Some(300),
        wait: false,
    };

    // Pass routine metadata through channel_metadata so SubagentExecutor
    // can finalize the routine_run on completion.
    let channel_metadata = serde_json::json!({
        "thread_id": "agent:main",
        "routine_id": routine.id.to_string(),
        "routine_name": routine.name,
        "routine_run_id": run.id.to_string(),
        "reinject_result": false,
    });

    match executor
        .spawn(
            request,
            "tauri",
            &channel_metadata,
            routine.owner_actor_id(),
            None,
            Some("agent:main"),
        )
        .await
    {
        Ok(result) => {
            // Broadcast "dispatched" SSE so the UI shows the subagent panel
            ctx.broadcast_sse(SseEvent::RoutineLifecycle {
                routine_name: routine.name.clone(),
                event: "dispatched".to_string(),
                run_id: Some(run.id.to_string()),
                result_summary: Some(format!(
                    "Subagent spawned (id: {}) — {}",
                    result.agent_id,
                    summarize_runtime_capabilities(
                        tool_profile.unwrap_or(ToolProfile::ExplicitOnly),
                        allowed_tools,
                        allowed_skills,
                    )
                )),
            });

            Ok((
                RunStatus::Running,
                Some(format!("Subagent spawned (id: {})", result.agent_id)),
                None,
            ))
        }
        Err(e) => Err(RoutineError::ExecutionFailed {
            reason: format!("Failed to spawn subagent: {}", e),
        }),
    }
}

/// Execute a full-job routine by dispatching to the scheduler.
///
/// Uses `dispatch_job_for_routine` so the spawned worker carries routine
/// metadata and can emit a real `RoutineLifecycle` SSE event on actual
/// completion — not just on dispatch. Returns `RunStatus::Running` so
/// `execute_routine` knows NOT to emit a premature "completed" event.
async fn execute_full_job(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    title: &str,
    description: &str,
    max_iterations: u32,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    tool_profile: Option<ToolProfile>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    if let Some(manager) = crate::desktop_autonomy::desktop_autonomy_manager() {
        if routine_requests_desktop_capabilities(allowed_tools) {
            manager
                .ensure_can_run()
                .await
                .map_err(|reason| RoutineError::ExecutionFailed { reason })?;
        } else if manager.emergency_stop_active() {
            return Err(RoutineError::ExecutionFailed {
                reason: "desktop autonomy emergency stop is active".to_string(),
            });
        }
    }

    let desktop = crate::desktop_autonomy::desktop_autonomy_manager().map(|manager| {
        serde_json::json!({
            "desktop_session": manager.default_session_id(),
            "deployment_mode": manager.config().deployment_mode.as_str(),
            "desktop_run_id": run.id.to_string(),
            "recovery_count": 0,
            "last_verified_snapshot": serde_json::Value::Null,
            "managed_build_id": manager.current_build_id(),
            "autonomy_profile": manager.config().profile.as_str(),
        })
    });
    let metadata = full_job_metadata(
        routine,
        run.id,
        max_iterations,
        FullJobRuntimeMetadata {
            allowed_tools: allowed_tools.map(|tools| tools.to_vec()),
            allowed_skills: allowed_skills.map(|skills| skills.to_vec()),
            tool_profile,
            desktop,
        },
    );

    let job_id = scheduler
        .dispatch_job_for_routine(
            &routine.user_id,
            routine.owner_actor_id(),
            title,
            description,
            Some(metadata),
            routine.id,
            routine.name.clone(),
            run.id.to_string(),
        )
        .await
        .map_err(|e| RoutineError::JobDispatchFailed {
            reason: format!("failed to dispatch job: {e}"),
        })?;

    // Link the routine run to the dispatched job
    if let Err(e) = ctx.store.link_routine_run_to_job(run.id, job_id).await {
        tracing::error!(
            routine = %routine.name,
            "Failed to link run to job: {}", e
        );
    }

    // Broadcast "dispatched" SSE so the UI shows a queued state, NOT success
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "dispatched".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: Some(format!(
            "Job {job_id} queued — {}",
            summarize_runtime_capabilities(
                tool_profile.unwrap_or(ToolProfile::Restricted),
                allowed_tools,
                allowed_skills,
            )
        )),
    });

    // Also broadcast the generic job started event for job view
    ctx.broadcast_sse(SseEvent::JobStarted {
        job_id: job_id.to_string(),
        title: format!("Routine '{}': {}", routine.name, title),
        browse_url: String::new(),
    });

    tracing::info!(
        routine = %routine.name,
        job_id = %job_id,
        max_iterations = max_iterations,
        "Dispatched full job for routine — worker will emit completion SSE"
    );

    let summary = format!(
        "Dispatched job {job_id} for full execution ({}, max_iterations: {max_iterations})",
        summarize_runtime_capabilities(
            tool_profile.unwrap_or(ToolProfile::Restricted),
            allowed_tools,
            allowed_skills,
        )
    );
    // Return RunStatus::Running — execute_routine will skip emitting "completed"
    // for this case; the worker emits the real event via WorkerDeps::sse_tx.
    Ok((RunStatus::Running, Some(summary), None))
}

/// Execute a heartbeat routine.
///
/// In `light_context` mode (default), dispatches as a full worker job with
/// HEARTBEAT.md + daily logs as the prompt — isolated from the main session
/// but with full tool access.
///
/// When `light_context` is false, injects the heartbeat prompt into the main
/// session via `system_event_tx` for full conversational context.
async fn execute_heartbeat(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    light_context: bool,
    custom_prompt: Option<&str>,
    _include_reasoning: bool,
    active_start_hour: Option<u8>,
    active_end_hour: Option<u8>,
    _target: &str,
    max_iterations: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    // 0. Active hours check
    if let (Some(s), Some(e)) = (active_start_hour, active_end_hour) {
        let tz = crate::timezone::resolve_effective_timezone(
            Some(&routine.user_id),
            ctx.user_timezone.as_deref(),
        );
        let now_hour = crate::timezone::now_in_tz(tz).hour() as u8;
        if !active_hour_allows(now_hour, s, e) {
            tracing::debug!(
                routine = %routine.name,
                hour = now_hour,
                active = %format!("{:02}:00-{:02}:00", s, e),
                "Heartbeat outside active hours — skipping"
            );
            return Ok((
                RunStatus::Ok,
                Some("Skipped — outside active hours".to_string()),
                None,
            ));
        }
    }

    // 1. Read HEARTBEAT.md
    let checklist = match ctx.workspace.heartbeat_checklist().await {
        Ok(Some(content)) if !crate::agent::heartbeat::is_effectively_empty(&content) => content,
        Ok(_) => {
            tracing::debug!(routine = %routine.name, "HEARTBEAT.md is empty or missing — skipping");
            return Ok((
                RunStatus::Ok,
                Some("HEARTBEAT_OK — checklist empty".to_string()),
                None,
            ));
        }
        Err(e) => {
            return Err(RoutineError::ExecutionFailed {
                reason: format!("Failed to read HEARTBEAT.md: {}", e),
            });
        }
    };

    // IC-013: Use shared function to build daily log context
    let daily_context = crate::agent::heartbeat::build_daily_context(&ctx.workspace).await;

    // ── Self-critique feedback: inject previous run's evaluation ─────
    // If the previous heartbeat was flagged by the post-completion
    // evaluator, inject that feedback so the agent can learn from it.
    let critique_context = match ctx
        .store
        .get_setting("system", "heartbeat.last_critique")
        .await
    {
        Ok(Some(critique)) if !critique.is_null() => {
            let reasoning = critique
                .get("reasoning")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown issue");
            let quality = critique
                .get("quality")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!(
                "\n\n## Previous Heartbeat Feedback (Self-Critique)\n\n\
                 ⚠️ The previous heartbeat run scored {}/100. \
                 Evaluator feedback: {}\n\n\
                 Take this into account and avoid repeating the same mistake.",
                quality, reasoning
            )
        }
        _ => String::new(),
    };

    // 3. Build the full prompt
    let outcome_summary = match crate::agent::outcomes::heartbeat_review_summary(
        &ctx.store,
        &routine.user_id,
    )
    .await
    {
        Ok(Some(summary)) => Some(summary),
        _ => None,
    };
    let full_prompt = build_heartbeat_prompt(
        custom_prompt,
        &checklist,
        &daily_context,
        &critique_context,
        outcome_summary.as_deref(),
    );

    if !light_context {
        // ── Main-session injection mode ──────────────────────────────
        // Inject the heartbeat prompt into the main session via system_event_tx.
        // The dispatcher processes it as a normal turn with full session history
        // and tool access. The response flows through normal SSE → chat.
        if let Some(ref tx) = ctx.system_event_tx {
            let message = IncomingMessage::new("heartbeat", "system", &full_prompt).with_metadata(
                serde_json::json!({
                    "source": "heartbeat",
                    "routine_name": routine.name,
                    "run_id": run.id.to_string(),
                }),
            );

            if let Err(e) = tx.send(message).await {
                return Err(RoutineError::ExecutionFailed {
                    reason: format!("Failed to inject heartbeat into main session: {}", e),
                });
            }

            tracing::info!(
                routine = %routine.name,
                "Injected heartbeat into main session — dispatcher will process with full context"
            );

            // Return Running — the dispatcher handles completion.
            // The main session will produce the response (HEARTBEAT_OK or findings).
            return Ok((
                RunStatus::Running,
                Some("Injected into main session — awaiting agent response".to_string()),
                None,
            ));
        } else {
            tracing::warn!(
                routine = %routine.name,
                "No system_event_tx available — falling back to light_context mode"
            );
            // Fall through to light_context mode below
        }
    }

    // ── Light-context mode: dispatch as isolated worker job ──────────
    // Uses the reserved overflow slot so heartbeats never get blocked
    // by "Maximum parallel jobs exceeded" when user jobs fill all slots.
    let title = format!("Heartbeat: {}", routine.name);
    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    let metadata = heartbeat_job_metadata(routine, max_iterations);

    let job_id = scheduler
        .dispatch_job_reserved_for_routine(
            &routine.user_id,
            routine.owner_actor_id(),
            &title,
            &full_prompt,
            Some(metadata),
            routine.id,
            routine.name.clone(),
            run.id.to_string(),
        )
        .await
        .map_err(|e| RoutineError::JobDispatchFailed {
            reason: format!("failed to dispatch heartbeat job: {e}"),
        })?;

    // Link the routine run to the dispatched job
    if let Err(e) = ctx.store.link_routine_run_to_job(run.id, job_id).await {
        tracing::error!(
            routine = %routine.name,
            "Failed to link heartbeat run to job: {}", e
        );
    }

    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "dispatched".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: Some(format!("Heartbeat job {job_id} dispatched (reserved slot)")),
    });

    tracing::info!(
        routine = %routine.name,
        job_id = %job_id,
        "Dispatched heartbeat via reserved slot"
    );

    Ok((
        RunStatus::Running,
        Some(format!("Dispatched heartbeat job {job_id} (reserved slot)")),
        None,
    ))
}

/// Execute a lightweight routine (single LLM call).
async fn execute_lightweight(
    ctx: &EngineContext,
    routine: &Routine,
    prompt: &str,
    context_paths: &[String],
    max_tokens: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    // Load context from workspace
    let mut context_parts = Vec::new();
    for path in context_paths {
        match ctx.workspace.read(path).await {
            Ok(doc) => {
                context_parts.push(format!("## {}\n\n{}", path, doc.content));
            }
            Err(e) => {
                tracing::debug!(
                    routine = %routine.name,
                    "Failed to read context path {}: {}", path, e
                );
            }
        }
    }

    // Load routine state from workspace (name sanitized to prevent path traversal)
    let safe_name = sanitize_routine_name(&routine.name);
    let state_path = format!("routines/{safe_name}/state.md");
    let state_content = match ctx.workspace.read(&state_path).await {
        Ok(doc) => Some(doc.content),
        Err(_) => None,
    };

    let full_prompt =
        build_lightweight_routine_prompt(prompt, &context_parts, state_content.as_deref());

    // Get system prompt
    let system_prompt = match ctx.workspace.system_prompt().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(routine = %routine.name, "Failed to get system prompt: {}", e);
            String::new()
        }
    };

    let messages = lightweight_routine_messages(&system_prompt, &full_prompt);

    // Determine max_tokens from model metadata with fallback
    let effective_max_tokens = effective_lightweight_max_tokens(
        max_tokens,
        ctx.llm
            .model_metadata()
            .await
            .ok()
            .and_then(|meta| meta.context_length),
    );

    let request = CompletionRequest::new(messages)
        .with_max_tokens(effective_max_tokens)
        .with_temperature(0.3);

    let response = ctx
        .llm
        .complete(request)
        .await
        .map_err(|e| RoutineError::LlmFailed {
            reason: e.to_string(),
        })?;

    classify_lightweight_routine_response(
        &response.content,
        response.finish_reason,
        response.input_tokens,
        response.output_tokens,
    )
}

/// Send a notification based on the routine's notify config and run status.
async fn send_notification(
    tx: &mpsc::Sender<OutgoingResponse>,
    notify: &NotifyConfig,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
) {
    let Some(notification) = build_routine_notification(notify, routine_name, status, summary)
    else {
        return;
    };

    let response = OutgoingResponse {
        content: notification.content,
        thread_id: None,
        metadata: notification.metadata,
        attachments: Vec::new(),
    };

    if let Err(e) = tx.send(response).await {
        tracing::error!(routine = %routine_name, "Failed to send notification: {}", e);
    }
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{Duration as ChronoDuration, Utc};
    use rust_decimal::Decimal;
    use tokio::sync::{mpsc, oneshot};

    use super::*;
    use crate::agent::routine::{NotifyConfig, RoutineEventStatus, RunStatus, content_hash};
    use crate::error::LlmError;
    use crate::llm::{
        CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::testing::StubLlm;
    #[cfg(feature = "libsql")]
    use crate::testing::test_db;

    struct BlockingLlm {
        started_tx: Mutex<Option<oneshot::Sender<()>>>,
        dropped_tx: Mutex<Option<oneshot::Sender<()>>>,
    }

    impl BlockingLlm {
        fn new(started_tx: oneshot::Sender<()>, dropped_tx: oneshot::Sender<()>) -> Self {
            Self {
                started_tx: Mutex::new(Some(started_tx)),
                dropped_tx: Mutex::new(Some(dropped_tx)),
            }
        }
    }

    struct DropSignal(Option<oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    #[async_trait]
    impl LlmProvider for BlockingLlm {
        fn model_name(&self) -> &str {
            "blocking-llm"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
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
    async fn cron_ticker_checks_due_routines_immediately_on_startup() {
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

        let handle = spawn_cron_ticker(Arc::clone(&engine), Duration::from_secs(60));

        let mut fired = false;
        for _ in 0..20 {
            let refreshed = db.get_routine(routine.id).await.unwrap().unwrap();
            if refreshed.run_count > 0 {
                fired = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        handle.abort();
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
        routine.policy.catch_up_mode = RoutineCatchUpMode::Skip;
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
}

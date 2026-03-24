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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{Timelike, Utc};
use regex::Regex;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineRun, RunStatus, Trigger, next_cron_fire,
};
use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::channels::web::types::SseEvent;
use crate::channels::{IncomingMessage, OutgoingResponse};
use crate::config::RoutineConfig;
use crate::db::Database;
use crate::error::RoutineError;
use crate::llm::{ChatMessage, CompletionRequest, FinishReason, LlmProvider};
use crate::workspace::Workspace;

/// The routine execution engine.
pub struct RoutineEngine {
    config: RoutineConfig,
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    /// Sender for notifications (routed to channel manager).
    notify_tx: mpsc::Sender<OutgoingResponse>,
    /// Currently running routine count (across all routines).
    running_count: Arc<AtomicUsize>,
    /// Compiled event regex cache: routine_id -> compiled regex.
    event_cache: Arc<RwLock<Vec<(Uuid, Routine, Regex)>>>,
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
            running_count: Arc::new(AtomicUsize::new(0)),
            event_cache: Arc::new(RwLock::new(Vec::new())),
            scheduler,
            sse_tx: None,
            system_event_tx: None,
            subagent_executor: None,
            active_tasks: Arc::new(std::sync::Mutex::new(tokio::task::JoinSet::new())),
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

    /// Refresh the in-memory event trigger cache from DB.
    pub async fn refresh_event_cache(&self) {
        match self.store.list_event_routines().await {
            Ok(routines) => {
                let mut cache = Vec::new();
                for routine in routines {
                    if let Trigger::Event { ref pattern, .. } = routine.trigger {
                        match Regex::new(pattern) {
                            Ok(re) => cache.push((routine.id, routine.clone(), re)),
                            Err(e) => {
                                tracing::warn!(
                                    routine = %routine.name,
                                    "Invalid event regex '{}': {}",
                                    pattern, e
                                );
                            }
                        }
                    }
                }
                let count = cache.len();
                *self.event_cache.write().await = cache;
                tracing::debug!("Refreshed event cache: {} routines", count);
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
        let cache = self.event_cache.read().await;
        let mut fired = 0;

        for (_, routine, re) in cache.iter() {
            // Channel filter
            if let Trigger::Event {
                channel: Some(ch), ..
            } = &routine.trigger
                && ch != &message.channel
            {
                continue;
            }

            // Regex match
            if !re.is_match(&message.content) {
                continue;
            }

            // Cooldown check
            if !self.check_cooldown(routine) {
                tracing::debug!(routine = %routine.name, "Skipped: cooldown active");
                continue;
            }

            // Concurrent run check
            if !self.check_concurrent(routine).await {
                tracing::debug!(routine = %routine.name, "Skipped: max concurrent reached");
                continue;
            }

            // Global capacity check
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!(routine = %routine.name, "Skipped: global max concurrent reached");
                continue;
            }

            let detail = truncate(&message.content, 200);
            self.spawn_fire(routine.clone(), "event", Some(detail))
                .await;
            fired += 1;
        }

        fired
    }

    /// Check all due cron routines and fire them. Called by the cron ticker.
    pub async fn check_cron_triggers(&self) {
        let routines = match self.store.list_due_cron_routines().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to load due cron routines: {}", e);
                return;
            }
        };

        for routine in routines {
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!("Global max concurrent routines reached, skipping remaining");
                break;
            }

            if !self.check_cooldown(&routine) {
                continue;
            }

            if !self.check_concurrent(&routine).await {
                continue;
            }

            let detail = match &routine.trigger {
                Trigger::Cron { schedule } => Some(schedule.clone()),
                Trigger::SystemEvent { schedule, .. } => schedule.clone(),
                _ => None,
            };

            // SystemEvent trigger: inject message into heartbeat queue instead of
            // running the routine's action. This is the cron→heartbeat bridge.
            if let Trigger::SystemEvent { message, .. } = &routine.trigger {
                if let Some(ref tx) = self.system_event_tx {
                    let msg = IncomingMessage::new("heartbeat", "system_event", message)
                        .with_metadata(serde_json::json!({
                            "source": "system_event",
                            "routine_name": routine.name,
                        }));
                    if let Err(e) = tx.send(msg).await {
                        tracing::error!(
                            routine = %routine.name,
                            "Failed to inject system event into heartbeat queue: {}", e
                        );
                    } else {
                        tracing::info!(
                            routine = %routine.name,
                            "Injected system event into heartbeat queue"
                        );
                    }
                    // Update runtime state: advance next_fire_at, bump run_count
                    let next = detail
                        .as_ref()
                        .and_then(|s| next_cron_fire(s).unwrap_or(None));
                    let _ = self
                        .store
                        .update_routine_runtime(
                            routine.id,
                            Utc::now(),
                            next,
                            routine.run_count + 1,
                            routine.consecutive_failures,
                            &routine.state,
                        )
                        .await;
                } else {
                    tracing::warn!(
                        routine = %routine.name,
                        "SystemEvent trigger but no system_event_tx — ignoring"
                    );
                }
                continue;
            }

            self.spawn_fire(routine, "cron", detail).await;
        }
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

        let run_id = Uuid::new_v4();
        let run = RoutineRun {
            id: run_id,
            routine_id: routine.id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        if let Err(e) = self.store.create_routine_run(&run).await {
            return Err(RoutineError::Database {
                reason: format!("failed to create run record: {e}"),
            });
        }

        // Execute inline for manual triggers (caller wants to wait)
        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            running_count: self.running_count.clone(),
            scheduler: self.scheduler.clone(),
            sse_tx: self.sse_tx.clone(),
            system_event_tx: self.system_event_tx.clone(),
            subagent_executor: self.subagent_executor.clone(),
        };

        tokio::spawn(async move {
            execute_routine(engine, routine, run).await;
        });

        Ok(run_id)
    }

    /// Spawn a fire in a background task.
    async fn spawn_fire(
        &self,
        routine: Routine,
        trigger_type: &str,
        trigger_detail: Option<String>,
    ) {
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: trigger_type.to_string(),
            trigger_detail,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            running_count: self.running_count.clone(),
            scheduler: self.scheduler.clone(),
            sse_tx: self.sse_tx.clone(),
            system_event_tx: self.system_event_tx.clone(),
            subagent_executor: self.subagent_executor.clone(),
        };

        // Record the run in DB, then spawn execution (IC-018: tracked via JoinSet)
        let store = self.store.clone();
        let tasks = self.active_tasks.clone();
        // Bug 14 fix: use std::sync::Mutex (sync lock) so we never hold an async
        // lock across the synchronous JoinSet::spawn() call.
        if let Ok(mut guard) = tasks.lock() {
            guard.spawn(async move {
                if let Err(e) = store.create_routine_run(&run).await {
                    tracing::error!(routine = %routine.name, "Failed to record run: {}", e);
                    return;
                }
                execute_routine(engine, routine, run).await;
            });
        } else {
            tracing::error!(routine = %routine.name, "active_tasks mutex poisoned — routine not spawned");
        }
    }

    /// IC-018: Abort all running routine tasks. Called on engine shutdown.
    pub async fn abort_all(&self) {
        // std::sync::Mutex — lock is sync, no await inside the guard scope.
        if let Ok(mut guard) = self.active_tasks.lock() {
            guard.abort_all();
        }
        tracing::info!("Aborted all running routine tasks");
    }

    /// IC-006: Reap zombie routine runs that are still in `Running` status.
    ///
    /// Uses the existing `cleanup_stale_routine_runs()` DB method which marks
    /// all Running runs as Failed. This prevents slot exhaustion when the process
    /// crashes mid-run or a routine hangs beyond the check interval.
    pub async fn reap_zombie_runs(&self) {
        match self.store.cleanup_stale_routine_runs().await {
            Ok(reaped) => {
                if reaped > 0 {
                    // Bug 4 fix: use per-item fetch_sub instead of non-atomic load→store.
                    // A bulk load→store races with concurrent fetch_sub(1) calls from
                    // normally-completing routines, causing double-decrements that drive
                    // running_count to 0 and permanently block new routines.
                    for _ in 0..reaped {
                        // saturating_sub via compare-exchange loop prevents underflow.
                        let prev = self.running_count.fetch_update(
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                            |c| Some(c.saturating_sub(1)),
                        );
                        let _ = prev; // always succeeds; result is informational only
                    }
                    tracing::info!(
                        "IC-006: Reaped {} zombie routine runs, running_count now {}",
                        reaped,
                        self.running_count.load(Ordering::Relaxed)
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to reap zombie routine runs: {}", e);
            }
        }
    }

    fn check_cooldown(&self, routine: &Routine) -> bool {
        if let Some(last_run) = routine.last_run_at {
            let elapsed = Utc::now().signed_duration_since(last_run);
            let cooldown = chrono::Duration::from_std(routine.guardrails.cooldown)
                .unwrap_or(chrono::Duration::seconds(300));
            if elapsed < cooldown {
                return false;
            }
        }
        true
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
}

/// Shared context passed to the execution function.
struct EngineContext {
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    notify_tx: mpsc::Sender<OutgoingResponse>,
    running_count: Arc<AtomicUsize>,
    scheduler: Option<Arc<Scheduler>>,
    /// Optional SSE broadcast sender for routine lifecycle events.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Optional sender for injecting messages into the main session.
    system_event_tx: Option<mpsc::Sender<IncomingMessage>>,
    /// Optional subagent executor for non-heartbeat automations.
    subagent_executor: Option<Arc<SubagentExecutor>>,
}

impl EngineContext {
    /// Broadcast an SSE event if the sender is available.
    fn broadcast_sse(&self, event: SseEvent) {
        if let Some(ref tx) = self.sse_tx {
            let _ = tx.send(event);
        }
    }
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
    // Increment running count (atomic: survives panics in the execution below)
    ctx.running_count.fetch_add(1, Ordering::Relaxed);

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
        } => {
            if ctx.subagent_executor.is_some() {
                execute_as_subagent(&ctx, &routine, &run, title, description).await
            } else {
                execute_full_job(&ctx, &routine, &run, title, description, *max_iterations).await
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
    };

    // Decrement running count
    ctx.running_count.fetch_sub(1, Ordering::Relaxed);

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
        // Still update the routine's cron schedule so next_fire_at advances
        let now = Utc::now();
        let next_fire = if let Trigger::Cron { ref schedule } = routine.trigger {
            next_cron_fire(schedule).unwrap_or(None)
        } else {
            None
        };
        let _ = ctx
            .store
            .update_routine_runtime(
                routine.id,
                now,
                next_fire,
                routine.run_count + 1,
                routine.consecutive_failures,
                &routine.state,
            )
            .await;
        return;
    }

    // Complete the run record
    if let Err(e) = ctx
        .store
        .complete_routine_run(run.id, status, summary.as_deref(), tokens)
        .await
    {
        tracing::error!(routine = %routine.name, "Failed to complete run record: {}", e);
    }

    // Update routine runtime state
    let now = Utc::now();
    let next_fire = if let Trigger::Cron { ref schedule } = routine.trigger {
        next_cron_fire(schedule).unwrap_or(None)
    } else {
        None
    };

    let new_failures = if status == RunStatus::Failed {
        routine.consecutive_failures + 1
    } else {
        0
    };

    if let Err(e) = ctx
        .store
        .update_routine_runtime(
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

/// Sanitize a routine name for use in workspace paths.
/// Only keeps alphanumeric, dash, and underscore characters; replaces everything else.
fn sanitize_routine_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
        allowed_tools: None,
        timeout_secs: Some(300),
        wait: false,
    };

    // Pass routine metadata through channel_metadata so SubagentExecutor
    // can finalize the routine_run on completion.
    let channel_metadata = serde_json::json!({
        "thread_id": "agent:main",
        "routine_name": routine.name,
        "routine_run_id": run.id.to_string(),
    });

    match executor.spawn(request, "tauri", &channel_metadata).await {
        Ok(result) => {
            // Broadcast "dispatched" SSE so the UI shows the subagent panel
            ctx.broadcast_sse(SseEvent::RoutineLifecycle {
                routine_name: routine.name.clone(),
                event: "dispatched".to_string(),
                run_id: Some(run.id.to_string()),
                result_summary: Some(format!(
                    "Subagent spawned (id: {}) — running with full tool access",
                    result.agent_id
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
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    let metadata = serde_json::json!({ "max_iterations": max_iterations });

    let job_id = scheduler
        .dispatch_job_for_routine(
            &routine.user_id,
            title,
            description,
            Some(metadata),
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
            "Job {job_id} queued — worker running with full tool access"
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
        "Dispatched job {job_id} for full execution with tool access (max_iterations: {max_iterations})"
    );
    // Return RunStatus::Running — execute_routine will skip emitting "completed"
    // for this case; the worker emits the real event via WorkerDeps::sse_tx.
    Ok((RunStatus::Running, Some(summary), None))
}

/// Default heartbeat prompt body.
const DEFAULT_HEARTBEAT_PROMPT: &str = "\
Read the HEARTBEAT.md checklist below and follow it strictly. \
Do not infer or repeat old tasks from prior chats. Check each item and report findings.\n\
\n\
If nothing needs attention, reply EXACTLY with: HEARTBEAT_OK\n\
\n\
If something needs attention, provide a short, specific summary of what needs action. \
Do NOT echo these instructions back — give real findings only. \
Use `emit_user_message` to deliver your findings to the user.\n\
\n\
You may edit HEARTBEAT.md to add, remove, or update checklist items as needed.";

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
        let now_hour = chrono::Local::now().hour() as u8;
        let in_window = if s <= e {
            now_hour >= s && now_hour < e
        } else {
            now_hour >= s || now_hour < e
        };
        if !in_window {
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
    let prompt_body = custom_prompt.unwrap_or(DEFAULT_HEARTBEAT_PROMPT);
    let logs_note = if daily_context.is_empty() {
        "\n\nNote: No daily logs exist yet (no conversations recorded). \
         Any checklist items that reference daily logs are automatically satisfied. \
         If all items depend on daily logs, reply HEARTBEAT_OK."
    } else {
        ""
    };
    let full_prompt = format!(
        "{}\n\n## HEARTBEAT.md\n\n{}{}{}{}",
        prompt_body, checklist, daily_context, critique_context, logs_note
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

    let metadata = serde_json::json!({ "max_iterations": max_iterations, "heartbeat": true });

    let job_id = scheduler
        .dispatch_job_reserved_for_routine(
            &routine.user_id,
            &title,
            &full_prompt,
            Some(metadata),
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

    // Build the prompt
    let mut full_prompt = String::new();
    full_prompt.push_str(prompt);

    if !context_parts.is_empty() {
        full_prompt.push_str("\n\n---\n\n# Context\n\n");
        full_prompt.push_str(&context_parts.join("\n\n"));
    }

    if let Some(state) = &state_content {
        full_prompt.push_str("\n\n---\n\n# Previous State\n\n");
        full_prompt.push_str(state);
    }

    full_prompt.push_str(
        "\n\n---\n\nIf nothing needs attention, reply EXACTLY with: ROUTINE_OK\n\
         If something needs attention, provide a concise summary.",
    );

    // Get system prompt
    let system_prompt = match ctx.workspace.system_prompt().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(routine = %routine.name, "Failed to get system prompt: {}", e);
            String::new()
        }
    };

    let messages = if system_prompt.is_empty() {
        vec![ChatMessage::user(&full_prompt)]
    } else {
        vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&full_prompt),
        ]
    };

    // Determine max_tokens from model metadata with fallback
    let effective_max_tokens = match ctx.llm.model_metadata().await {
        Ok(meta) => {
            let from_api = meta.context_length.map(|ctx| ctx / 2).unwrap_or(max_tokens);
            from_api.max(max_tokens)
        }
        Err(_) => max_tokens,
    };

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

    let content = response.content.trim();
    let tokens_used = Some((response.input_tokens + response.output_tokens) as i32);

    // Empty content guard (same as heartbeat)
    if content.is_empty() {
        return if response.finish_reason == FinishReason::Length {
            Err(RoutineError::TruncatedResponse)
        } else {
            Err(RoutineError::EmptyResponse)
        };
    }

    // Check for the "nothing to do" sentinel
    if content == "ROUTINE_OK" || content.contains("ROUTINE_OK") {
        return Ok((RunStatus::Ok, None, tokens_used));
    }

    Ok((RunStatus::Attention, Some(content.to_string()), tokens_used))
}

/// Send a notification based on the routine's notify config and run status.
async fn send_notification(
    tx: &mpsc::Sender<OutgoingResponse>,
    notify: &NotifyConfig,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
) {
    let should_notify = match status {
        RunStatus::Ok => notify.on_success,
        RunStatus::Attention => notify.on_attention,
        RunStatus::Failed => notify.on_failure,
        RunStatus::Running => false,
    };

    if !should_notify {
        return;
    }

    let icon = match status {
        RunStatus::Ok => "✅",
        RunStatus::Attention => "🔔",
        RunStatus::Failed => "❌",
        RunStatus::Running => "⏳",
    };

    let message = match summary {
        Some(s) => format!("{} *Routine '{}'*: {}\n\n{}", icon, routine_name, status, s),
        None => format!("{} *Routine '{}'*: {}", icon, routine_name, status),
    };

    let response = OutgoingResponse {
        content: message,
        thread_id: None,
        metadata: serde_json::json!({
            "source": "routine",
            "routine_name": routine_name,
            "status": status.to_string(),
            "notify_user": notify.user,
            "notify_channel": notify.channel,
        }),
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
        let mut ticker = tokio::time::interval(interval);
        // Skip immediate first tick
        ticker.tick().await;

        loop {
            ticker.tick().await;
            engine.check_cron_triggers().await;
            // IC-006: Reap zombie runs on each cron interval
            engine.reap_zombie_runs().await;
        }
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::routine::{NotifyConfig, RunStatus};

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
}

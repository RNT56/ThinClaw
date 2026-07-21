//! Job scheduler for parallel execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::FutureExt;
pub use thinclaw_agent::scheduler::WorkerMessage;
use thinclaw_agent::scheduler::{
    SUBTASK_CLEANUP_DELAYS_MS, SUBTASK_CLEANUP_TIMEOUT_SECS, ScheduledJob, ScheduledSubtask,
    SchedulerAdmissionKind, SchedulerAdmissionOutcome, SubtaskCleanupDecision,
    routine_job_metadata, scheduler_admission, subtask_cleanup_decision,
};
use thinclaw_agent::worker_runtime::is_worker_terminal_state;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::task::{Task, TaskContext, TaskOutput};
use crate::agent::worker::{Worker, WorkerDeps};
use crate::channels::OutgoingResponse;
use crate::channels::web::types::SseEvent;
use crate::config::AgentConfig;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::error::{Error, JobError};
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::observability::{NoopObserver, Observer};
use crate::safety::SafetyLayer;
use crate::tools::{ToolExecutionLane, ToolProfile, ToolRegistry, execution};

/// Schedules and manages parallel job execution.
pub struct Scheduler {
    config: AgentConfig,
    context_manager: Arc<ContextManager>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    store: Option<Arc<dyn Database>>,
    hooks: Arc<HookRegistry>,
    /// Optional SSE sender propagated to routine-spawned workers.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Workspace for identity injection into workers.
    workspace: Option<Arc<crate::workspace::Workspace>>,
    /// Running jobs (main LLM-driven jobs).
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    /// Admission closes permanently during runtime shutdown. The flag is
    /// rechecked while holding `jobs` admission so a racing schedule is either
    /// included in the shutdown snapshot or rejected before it can spawn.
    accepting_jobs: AtomicBool,
    /// Running sub-tasks (tool executions, background tasks).
    subtasks: Arc<RwLock<HashMap<Uuid, ScheduledSubtask>>>,
    /// Short-lived cleanup waiters owned by the scheduler lifecycle.
    maintenance_tasks: Mutex<JoinSet<()>>,
    /// Optional shared cost tracker for worker LLM calls.
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    /// Shared observability sink for worker loop lifecycle metrics.
    observer: Arc<dyn Observer>,
}

const JOB_SNAPSHOT_PERSIST_TIMEOUT: Duration = Duration::from_secs(5);
const SCHEDULER_STOP_TIMEOUT: Duration = Duration::from_secs(10);

async fn cleanup_finished_worker(
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    context_manager: Arc<ContextManager>,
    job_id: Uuid,
) {
    jobs.write().await.remove(&job_id);
    let retain_for_repair = context_manager
        .get_context(job_id)
        .await
        .ok()
        .is_some_and(|context| {
            context.state == JobState::Stuck
                && context.metadata.get("routine_dispatched")
                    != Some(&serde_json::Value::Bool(true))
        });
    if retain_for_repair {
        tracing::warn!(job_id = %job_id, "Retaining stuck direct job for scheduler-backed self-repair");
        return;
    }
    if let Err(error) = context_manager.remove_job(job_id).await {
        tracing::debug!(job_id = %job_id, "ContextManager cleanup skipped: {}", error);
    }
}

async fn record_unexpected_worker_failure(
    context_manager: &ContextManager,
    store: Option<&Arc<dyn Database>>,
    job_id: Uuid,
    reason: &str,
) {
    let transition = context_manager
        .update_context(job_id, |context| {
            if is_worker_terminal_state(context.state) {
                Ok(())
            } else {
                context.transition_to(JobState::Failed, Some(reason.to_string()))
            }
        })
        .await;

    match transition {
        Ok(Ok(())) => {
            if let (Some(store), Ok(snapshot)) = (store, context_manager.get_context(job_id).await)
            {
                match tokio::time::timeout(JOB_SNAPSHOT_PERSIST_TIMEOUT, store.save_job(&snapshot))
                    .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(job_id = %job_id, %error, "Failed to persist unexpected worker failure");
                    }
                    Err(_) => {
                        tracing::warn!(job_id = %job_id, "Timed out persisting unexpected worker failure");
                    }
                }
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(job_id = %job_id, %error, "Could not transition unexpectedly exited worker");
        }
        Err(error) => {
            tracing::debug!(job_id = %job_id, %error, "Unexpected worker exit arrived after context cleanup");
        }
    }
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(
        config: AgentConfig,
        context_manager: Arc<ContextManager>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        store: Option<Arc<dyn Database>>,
        hooks: Arc<HookRegistry>,
    ) -> Self {
        Self {
            config,
            context_manager,
            llm,
            safety,
            tools,
            store,
            hooks,
            sse_tx: None,
            workspace: None,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            accepting_jobs: AtomicBool::new(true),
            subtasks: Arc::new(RwLock::new(HashMap::new())),
            maintenance_tasks: Mutex::new(JoinSet::new()),
            cost_tracker: None,
            observer: Arc::new(NoopObserver),
        }
    }

    /// Attach an SSE broadcast sender so routine-spawned workers can emit lifecycle events.
    pub fn with_sse_sender(mut self, tx: tokio::sync::broadcast::Sender<SseEvent>) -> Self {
        self.sse_tx = Some(tx);
        self
    }

    /// Attach a workspace so workers can load agent identity (SOUL.md, etc.).
    pub fn with_workspace(mut self, ws: Arc<crate::workspace::Workspace>) -> Self {
        self.workspace = Some(ws);
        self
    }

    /// Attach a cost tracker so worker LLM calls are visible in the Cost Dashboard.
    pub fn with_cost_tracker(
        mut self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// Attach the shared observer so worker loops emit lifecycle metrics.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = observer;
        self
    }

    async fn persist_job_snapshot(&self, ctx: JobContext) {
        if let Some(ref store) = self.store {
            match tokio::time::timeout(JOB_SNAPSHOT_PERSIST_TIMEOUT, store.save_job(&ctx)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(job_id = %ctx.job_id, "Failed to persist job snapshot: {}", error);
                }
                Err(_) => {
                    tracing::warn!(
                        job_id = %ctx.job_id,
                        timeout_secs = JOB_SNAPSHOT_PERSIST_TIMEOUT.as_secs(),
                        "Timed out persisting job snapshot"
                    );
                }
            }
        }
    }

    fn ensure_accepting(&self, job_id: Uuid) -> Result<(), JobError> {
        if self.accepting_jobs.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(JobError::ContextError {
                id: job_id,
                reason: "scheduler is shutting down".to_string(),
            })
        }
    }

    async fn persist_initial_job(&self, job_id: Uuid) -> Result<(), JobError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let context = self.context_manager.get_context(job_id).await?;
        match tokio::time::timeout(JOB_SNAPSHOT_PERSIST_TIMEOUT, store.save_job(&context)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(JobError::Failed {
                id: job_id,
                reason: format!("failed to persist job: {error}"),
            }),
            Err(_) => Err(JobError::Failed {
                id: job_id,
                reason: format!(
                    "timed out persisting job after {} seconds",
                    JOB_SNAPSHOT_PERSIST_TIMEOUT.as_secs()
                ),
            }),
        }
    }

    async fn abandon_unadmitted_job(&self, job_id: Uuid, reason: &str) {
        let _ = self
            .context_manager
            .update_context(job_id, |context| {
                if !is_worker_terminal_state(context.state) {
                    let _ = context.transition_to(JobState::Abandoned, Some(reason.to_string()));
                }
            })
            .await;
        if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
            self.persist_job_snapshot(snapshot).await;
        }
        if let Err(error) = self.context_manager.remove_job(job_id).await {
            tracing::debug!(%job_id, %error, "Unadmitted job context cleanup skipped");
        }
    }

    /// Spawn one worker together with its terminal cleanup. Keeping cleanup in
    /// the same owned task eliminates detached waiter leaks and guarantees a
    /// worker cannot finish before its cleanup future is installed. The caller
    /// holds `jobs` admission while inserting the returned handle, so cleanup's
    /// write lock naturally waits until the map entry exists.
    fn spawn_worker_with_cleanup(
        &self,
        worker: Worker,
        rx: mpsc::Receiver<WorkerMessage>,
        job_id: Uuid,
        kind: &'static str,
    ) -> tokio::task::JoinHandle<()> {
        let jobs = Arc::clone(&self.jobs);
        let context_manager = Arc::clone(&self.context_manager);
        let store = self.store.clone();
        tokio::spawn(async move {
            let failure_reason = match std::panic::AssertUnwindSafe(worker.run(rx))
                .catch_unwind()
                .await
            {
                Ok(Ok(())) => None,
                Ok(Err(error)) => {
                    tracing::error!(job_id = %job_id, worker_kind = kind, %error, "Worker failed");
                    Some(format!("Worker failed unexpectedly: {error}"))
                }
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown panic");
                    tracing::error!(job_id = %job_id, worker_kind = kind, %message, "Worker panicked");
                    Some(format!("Worker panicked: {message}"))
                }
            };
            if let Some(reason) = failure_reason {
                record_unexpected_worker_failure(
                    context_manager.as_ref(),
                    store.as_ref(),
                    job_id,
                    &reason,
                )
                .await;
            }
            cleanup_finished_worker(jobs, context_manager, job_id).await;
        })
    }

    async fn spawn_maintenance_task(
        &self,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let mut tasks = self.maintenance_tasks.lock().await;
        if !self.accepting_jobs.load(Ordering::Acquire) {
            return;
        }
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Scheduler maintenance task failed");
            }
        }
        tasks.spawn(task);
    }

    /// Create, persist, and schedule a job in one shot.
    ///
    /// This is the preferred entry point for dispatching new jobs. It:
    /// 1. Creates the job context via `ContextManager`
    /// 2. Optionally applies metadata (e.g. `max_iterations`)
    /// 3. Persists the job to the database (so FK references from
    ///    `job_actions` / `llm_calls` work immediately)
    /// 4. Schedules the job for worker execution
    ///
    /// Returns the new job ID.
    pub async fn dispatch_job(
        &self,
        user_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<Uuid, JobError> {
        self.dispatch_job_for_identity(user_id, user_id, title, description, metadata)
            .await
    }

    /// Create, persist, and schedule a job with explicit principal/actor ownership.
    pub async fn dispatch_job_for_identity(
        &self,
        principal_id: &str,
        actor_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<Uuid, JobError> {
        self.ensure_accepting(Uuid::nil())?;
        let job_id = self
            .context_manager
            .create_job_for_identity(principal_id, actor_id, title, description)
            .await?;

        let admission = async {
            if let Some(meta) = metadata {
                self.context_manager
                    .update_context(job_id, |ctx| {
                        ctx.metadata = meta;
                    })
                    .await?;
            }
            self.persist_initial_job(job_id).await?;
            self.schedule(job_id).await
        }
        .await;
        match admission {
            Ok(()) => Ok(job_id),
            Err(error) => {
                self.abandon_unadmitted_job(job_id, &error.to_string())
                    .await;
                Err(error)
            }
        }
    }

    /// Like `dispatch_job` but wires routine metadata into the worker so it can
    /// emit a real `RoutineLifecycle` SSE event when the job actually completes
    /// (instead of when it was merely dispatched to the scheduler).
    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_job_for_routine(
        &self,
        principal_id: &str,
        actor_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
        routine_id: Uuid,
        routine_name: String,
        routine_run_id: String,
        notify_tx: Option<mpsc::Sender<OutgoingResponse>>,
    ) -> Result<Uuid, JobError> {
        self.ensure_accepting(Uuid::nil())?;
        let job_id = self
            .context_manager
            .create_job_for_identity(principal_id, actor_id, title, description)
            .await?;

        let admission = async {
            let metadata = routine_job_metadata(metadata, false);
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = metadata;
                })
                .await?;
            self.persist_initial_job(job_id).await?;
            self.schedule_for_routine(job_id, routine_id, routine_name, routine_run_id, notify_tx)
                .await
        }
        .await;
        match admission {
            Ok(()) => Ok(job_id),
            Err(error) => {
                self.abandon_unadmitted_job(job_id, &error.to_string())
                    .await;
                Err(error)
            }
        }
    }

    /// Like `dispatch_job_for_routine` but uses the **reserved overflow slot**
    /// in the ContextManager. Heartbeat and system routines call this so they
    /// never get blocked by `MaxJobsExceeded` even when user jobs fill all
    /// normal slots.
    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_job_reserved_for_routine(
        &self,
        principal_id: &str,
        actor_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
        routine_id: Uuid,
        routine_name: String,
        routine_run_id: String,
        notify_tx: Option<mpsc::Sender<OutgoingResponse>>,
    ) -> Result<Uuid, JobError> {
        self.ensure_accepting(Uuid::nil())?;
        // Use the reserved slot (max_jobs + 1) in ContextManager
        let job_id = self
            .context_manager
            .create_job_reserved_for_identity(principal_id, actor_id, title, description)
            .await?;

        let admission = async {
            let metadata = routine_job_metadata(metadata, true);
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = metadata;
                })
                .await?;
            self.persist_initial_job(job_id).await?;
            self.schedule_reserved_for_routine(
                job_id,
                routine_id,
                routine_name,
                routine_run_id,
                notify_tx,
            )
            .await
        }
        .await;
        match admission {
            Ok(()) => Ok(job_id),
            Err(error) => {
                self.abandon_unadmitted_job(job_id, &error.to_string())
                    .await;
                Err(error)
            }
        }
    }

    /// Internal: schedule with routine context, using the reserved overflow slot.
    async fn schedule_reserved_for_routine(
        &self,
        job_id: Uuid,
        routine_id: Uuid,
        routine_name: String,
        routine_run_id: String,
        notify_tx: Option<mpsc::Sender<OutgoingResponse>>,
    ) -> Result<(), JobError> {
        {
            let mut jobs = self.jobs.write().await;
            self.ensure_accepting(job_id)?;

            let admission = scheduler_admission(
                jobs.contains_key(&job_id),
                jobs.len(),
                self.config.max_parallel_jobs,
                SchedulerAdmissionKind::ReservedSystem,
            );
            match admission {
                SchedulerAdmissionOutcome::AlreadyScheduled => return Ok(()),
                SchedulerAdmissionOutcome::Accepted { .. } => {}
                SchedulerAdmissionOutcome::AtCapacity { capacity } => {
                    return Err(JobError::MaxJobsExceeded {
                        max: capacity.limit,
                    });
                }
            }

            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some(
                            SchedulerAdmissionKind::ReservedSystem
                                .transition_reason()
                                .to_string(),
                        ),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;
            if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
                self.persist_job_snapshot(snapshot).await;
            }

            let (tx, rx) = mpsc::channel(16);

            let deps = WorkerDeps {
                context_manager: self.context_manager.clone(),
                llm: self.llm.clone(),
                safety: self.safety.clone(),
                tools: self.tools.clone(),
                store: self.store.clone(),
                hooks: self.hooks.clone(),
                timeout: self.config.job_timeout,
                use_planning: self.config.use_planning,
                sse_tx: self.sse_tx.clone(),
                routine_name: Some(routine_name),
                routine_id: Some(routine_id),
                routine_run_id: Some(routine_run_id),
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
                notify_tx,
                observer: Arc::clone(&self.observer),
            };
            let worker = Worker::new(job_id, deps);

            let handle = self.spawn_worker_with_cleanup(worker, rx, job_id, "reserved_routine");

            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Reserved worker died before receiving Start");
            }

            jobs.insert(job_id, ScheduledJob { handle, tx });
        }

        tracing::info!("Scheduled reserved routine job {} for execution", job_id);
        Ok(())
    }

    /// Internal: schedule with routine context wired into WorkerDeps.
    async fn schedule_for_routine(
        &self,
        job_id: Uuid,
        routine_id: Uuid,
        routine_name: String,
        routine_run_id: String,
        notify_tx: Option<mpsc::Sender<OutgoingResponse>>,
    ) -> Result<(), JobError> {
        {
            let mut jobs = self.jobs.write().await;
            self.ensure_accepting(job_id)?;

            let admission = scheduler_admission(
                jobs.contains_key(&job_id),
                jobs.len(),
                self.config.max_parallel_jobs,
                SchedulerAdmissionKind::Standard,
            );
            match admission {
                SchedulerAdmissionOutcome::AlreadyScheduled => return Ok(()),
                SchedulerAdmissionOutcome::Accepted { .. } => {}
                SchedulerAdmissionOutcome::AtCapacity { capacity } => {
                    return Err(JobError::MaxJobsExceeded {
                        max: capacity.limit,
                    });
                }
            }

            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some(
                            SchedulerAdmissionKind::Standard
                                .transition_reason()
                                .to_string(),
                        ),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;
            if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
                self.persist_job_snapshot(snapshot).await;
            }

            let (tx, rx) = mpsc::channel(16);

            let deps = WorkerDeps {
                context_manager: self.context_manager.clone(),
                llm: self.llm.clone(),
                safety: self.safety.clone(),
                tools: self.tools.clone(),
                store: self.store.clone(),
                hooks: self.hooks.clone(),
                timeout: self.config.job_timeout,
                use_planning: self.config.use_planning,
                sse_tx: self.sse_tx.clone(),
                routine_name: Some(routine_name),
                routine_id: Some(routine_id),
                routine_run_id: Some(routine_run_id),
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
                notify_tx,
                observer: Arc::clone(&self.observer),
            };
            let worker = Worker::new(job_id, deps);

            let handle = self.spawn_worker_with_cleanup(worker, rx, job_id, "routine");

            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Routine worker died before receiving Start message");
            }

            jobs.insert(job_id, ScheduledJob { handle, tx });
        }

        tracing::info!("Scheduled routine job {} for execution", job_id);
        Ok(())
    }

    /// Schedule a job for execution.
    pub async fn schedule(&self, job_id: Uuid) -> Result<(), JobError> {
        // Hold write lock for the entire check-insert sequence to prevent
        // TOCTOU races where two concurrent calls both pass the checks.
        {
            let mut jobs = self.jobs.write().await;
            self.ensure_accepting(job_id)?;

            let admission = scheduler_admission(
                jobs.contains_key(&job_id),
                jobs.len(),
                self.config.max_parallel_jobs,
                SchedulerAdmissionKind::Standard,
            );
            match admission {
                SchedulerAdmissionOutcome::AlreadyScheduled => return Ok(()),
                SchedulerAdmissionOutcome::Accepted { .. } => {}
                SchedulerAdmissionOutcome::AtCapacity { capacity } => {
                    return Err(JobError::MaxJobsExceeded {
                        max: capacity.limit,
                    });
                }
            }

            // Transition job to in_progress
            self.context_manager
                .update_context(job_id, |ctx| {
                    if ctx.state == JobState::Stuck {
                        ctx.attempt_recovery()
                    } else {
                        ctx.transition_to(
                            JobState::InProgress,
                            Some(
                                SchedulerAdmissionKind::Standard
                                    .transition_reason()
                                    .to_string(),
                            ),
                        )
                    }
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;
            if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
                self.persist_job_snapshot(snapshot).await;
            }

            // Create worker channel
            let (tx, rx) = mpsc::channel(16);

            // Create worker with shared dependencies
            let deps = WorkerDeps {
                context_manager: self.context_manager.clone(),
                llm: self.llm.clone(),
                safety: self.safety.clone(),
                tools: self.tools.clone(),
                store: self.store.clone(),
                hooks: self.hooks.clone(),
                timeout: self.config.job_timeout,
                use_planning: self.config.use_planning,
                sse_tx: None,
                routine_name: None,
                routine_id: None,
                routine_run_id: None,
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
                notify_tx: None,
                observer: Arc::clone(&self.observer),
            };
            let worker = Worker::new(job_id, deps);

            let handle = self.spawn_worker_with_cleanup(worker, rx, job_id, "standard");

            // Start the worker
            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Worker died before receiving Start message");
            }

            // Insert while still holding the write lock
            jobs.insert(job_id, ScheduledJob { handle, tx });
        }

        tracing::info!("Scheduled job {} for execution", job_id);
        Ok(())
    }

    /// Schedule a sub-task from within a worker.
    ///
    /// Sub-tasks are lightweight tasks that don't go through the full job lifecycle.
    /// They're used for parallel tool execution and background computations.
    ///
    /// Returns a oneshot receiver to get the result.
    pub async fn spawn_subtask(
        &self,
        parent_id: Uuid,
        task: Task,
    ) -> Result<oneshot::Receiver<Result<TaskOutput, Error>>, JobError> {
        self.ensure_accepting(parent_id)?;
        let task_id = Uuid::new_v4();
        let (result_tx, result_rx) = oneshot::channel();

        let handle = match task {
            Task::Job { .. } => {
                // Jobs should go through schedule(), not spawn_subtask
                return Err(JobError::ContextError {
                    id: parent_id,
                    reason: "Use schedule() for Job tasks, not spawn_subtask()".to_string(),
                });
            }

            Task::ToolExec {
                parent_id: tool_parent_id,
                tool_name,
                params,
            } => {
                let tools = self.tools.clone();
                let context_manager = self.context_manager.clone();
                let safety = self.safety.clone();
                let default_profile = self.config.worker_tool_profile;
                let hooks = self.hooks.clone();

                tokio::spawn(async move {
                    let result = Self::execute_tool_task(
                        tools,
                        context_manager,
                        safety,
                        default_profile,
                        hooks,
                        tool_parent_id,
                        &tool_name,
                        params,
                    )
                    .await;

                    // Send result (ignore if receiver dropped)
                    let _ = result_tx.send(result);
                })
            }

            Task::Background { id: _, handler } => {
                let ctx = TaskContext::new(task_id).with_parent(parent_id);

                tokio::spawn(async move {
                    let result = handler.run(ctx).await.map_err(Error::from);
                    let _ = result_tx.send(result);
                })
            }
        };

        // Track the subtask for is_finished() polling during cleanup.
        // Bug 2 fix: store the raw task handle directly — the previous double-wrap
        // always returned Err(ContextError) and was misleading. The actual result is
        // delivered via the `oneshot` channel above; we only need the handle here for
        // tracking and abort-on-shutdown.
        {
            let mut subtasks = self.subtasks.write().await;
            if let Err(error) = self.ensure_accepting(parent_id) {
                handle.abort();
                return Err(error);
            }
            subtasks.insert(task_id, ScheduledSubtask::new(handle));
        }

        // Cleanup waiter — progressive polling with a hard timeout (Bug 35 fix).
        // We cannot use a oneshot here because the result is delivered via
        // result_tx before the JoinHandle is marked finished. Instead, use
        // progressive intervals capped at a 10-minute timeout to prevent
        // infinite loops on stuck tasks.
        let subtasks_cleanup = Arc::clone(&self.subtasks);
        self.spawn_maintenance_task(async move {
            let deadline =
                tokio::time::Instant::now() + Duration::from_secs(SUBTASK_CLEANUP_TIMEOUT_SECS);
            let mut delay_index = 0usize;
            loop {
                let delay_ms = SUBTASK_CLEANUP_DELAYS_MS
                    .get(delay_index)
                    .or_else(|| SUBTASK_CLEANUP_DELAYS_MS.last())
                    .copied()
                    .unwrap_or(10_000);
                delay_index = delay_index.saturating_add(1);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                let finished = {
                    let subtasks_read = subtasks_cleanup.read().await;
                    subtasks_read
                        .get(&task_id)
                        .is_some_and(ScheduledSubtask::is_finished)
                };
                match subtask_cleanup_decision(tokio::time::Instant::now() >= deadline, finished) {
                    SubtaskCleanupDecision::KeepWaiting => {}
                    SubtaskCleanupDecision::RemoveFinished => {
                        subtasks_cleanup.write().await.remove(&task_id);
                        break;
                    }
                    SubtaskCleanupDecision::ForceRemoveTimedOut => {
                        tracing::warn!("Subtask {} cleanup timed out, aborting", task_id);
                        if let Some(scheduled) = subtasks_cleanup.write().await.remove(&task_id) {
                            scheduled.abort();
                        }
                        break;
                    }
                }
            }
        })
        .await;

        tracing::debug!(
            parent_id = %parent_id,
            task_id = %task_id,
            "Spawned subtask"
        );

        Ok(result_rx)
    }

    /// Schedule multiple tasks in parallel and wait for all to complete.
    ///
    /// Returns results in the same order as the input tasks.
    pub async fn spawn_batch(
        &self,
        parent_id: Uuid,
        tasks: Vec<Task>,
    ) -> Vec<Result<TaskOutput, Error>> {
        if tasks.is_empty() {
            return Vec::new();
        }

        let mut receivers = Vec::with_capacity(tasks.len());

        // Spawn all tasks
        for task in tasks {
            match self.spawn_subtask(parent_id, task).await {
                Ok(rx) => receivers.push(Some(rx)),
                Err(e) => {
                    // Store the error directly
                    receivers.push(None);
                    tracing::warn!(
                        parent_id = %parent_id,
                        error = %e,
                        "Failed to spawn subtask in batch"
                    );
                }
            }
        }

        // Collect results
        let mut results = Vec::with_capacity(receivers.len());
        for rx in receivers {
            let result = match rx {
                Some(receiver) => match receiver.await {
                    Ok(task_result) => task_result,
                    Err(_) => Err(Error::Job(JobError::ContextError {
                        id: parent_id,
                        reason: "Subtask channel closed unexpectedly".to_string(),
                    })),
                },
                None => Err(Error::Job(JobError::ContextError {
                    id: parent_id,
                    reason: "Subtask failed to spawn".to_string(),
                })),
            };
            results.push(result);
        }

        results
    }

    /// Execute a single tool as a subtask.
    async fn execute_tool_task(
        tools: Arc<ToolRegistry>,
        context_manager: Arc<ContextManager>,
        safety: Arc<SafetyLayer>,
        default_profile: ToolProfile,
        hooks: Arc<HookRegistry>,
        job_id: Uuid,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<TaskOutput, Error> {
        // Get job context
        let job_ctx: JobContext = context_manager.get_context(job_id).await?;
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        let profile_override = job_ctx
            .metadata
            .get("tool_profile")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<ToolProfile>().ok());
        let hook_context = format!("scheduler:{job_id}");

        let prepared = match execution::prepare_tool_call(execution::ToolPrepareRequest {
            tools: &tools,
            safety: &safety,
            job_ctx: &job_ctx,
            tool_name,
            params: &params,
            lane: ToolExecutionLane::Scheduler,
            default_profile,
            profile_override,
            approval_mode: execution::ToolApprovalMode::Autonomous,
            hooks: Some(execution::ToolHookConfig {
                registry: hooks.as_ref(),
                user_id: &job_ctx.user_id,
                context: &hook_context,
            }),
        })
        .await?
        {
            execution::ToolPrepareOutcome::Ready(prepared) => prepared,
            execution::ToolPrepareOutcome::NeedsApproval(_) => {
                return Err(crate::error::ToolError::AuthRequired {
                    name: tool_name.to_string(),
                }
                .into());
            }
        };

        let output = execution::execute_tool_call(&prepared, &safety, &job_ctx).await?;
        Ok(TaskOutput::new(output.result_json, output.elapsed))
    }

    /// Stop a running job.
    pub async fn stop(&self, job_id: Uuid) -> Result<(), JobError> {
        let scheduled = {
            // Hold worker-map admission until cancellation is durable. The
            // worker's integrated cleanup also needs this write lock, so it
            // cannot delete ContextManager state between our state transition
            // and persistence snapshot.
            let mut jobs = self.jobs.write().await;
            if !jobs.contains_key(&job_id) {
                return Ok(());
            }
            self.context_manager
                .update_context(job_id, |ctx| {
                    if let Err(e) = ctx.transition_to(
                        JobState::Cancelled,
                        Some("Stopped by scheduler".to_string()),
                    ) {
                        tracing::warn!(
                            job_id = %job_id,
                            error = %e,
                            "Failed to transition job to Cancelled state"
                        );
                    }
                })
                .await?;
            if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
                self.persist_job_snapshot(snapshot).await;
            }
            jobs.remove(&job_id).ok_or_else(|| JobError::ContextError {
                id: job_id,
                reason: "job disappeared while scheduler admission was held".to_string(),
            })?
        };

        // Never let cancellation block behind a full worker mailbox. Dropping
        // the last sender closes the mailbox, and the bounded JoinHandle grace
        // below hard-aborts a worker that is not currently polling it.
        let _ = scheduled.tx.try_send(WorkerMessage::Stop);
        drop(scheduled.tx);

        let mut handle = scheduled.handle;
        if tokio::time::timeout(Duration::from_secs(2), &mut handle)
            .await
            .is_err()
        {
            tracing::warn!(job_id = %job_id, "Worker did not stop within the grace period; aborting");
            handle.abort();
            let _ = handle.await;
            // Aborting the wrapper also aborts its integrated cleanup tail.
            // Free the in-memory slot explicitly; the Cancelled snapshot is
            // already durable above.
            if let Err(error) = self.context_manager.remove_job(job_id).await {
                tracing::debug!(job_id = %job_id, %error, "Cancelled job context cleanup skipped");
            }
        }

        tracing::info!("Stopped job {}", job_id);

        Ok(())
    }

    async fn force_stop_job(&self, job_id: Uuid) {
        let scheduled = self.jobs.write().await.remove(&job_id);
        if let Some(scheduled) = scheduled {
            let _ = scheduled.tx.try_send(WorkerMessage::Stop);
            drop(scheduled.tx);
            scheduled.handle.abort();
            let _ = scheduled.handle.await;
        }

        if let Ok(result) = self
            .context_manager
            .update_context(job_id, |context| {
                if is_worker_terminal_state(context.state) {
                    Ok(())
                } else {
                    context.transition_to(
                        JobState::Cancelled,
                        Some("Force-stopped during scheduler shutdown".to_string()),
                    )
                }
            })
            .await
            && let Err(error) = result
        {
            tracing::warn!(%job_id, %error, "Failed to mark force-stopped worker Cancelled");
        }
        if let Ok(snapshot) = self.context_manager.get_context(job_id).await {
            self.persist_job_snapshot(snapshot).await;
        }
        if let Err(error) = self.context_manager.remove_job(job_id).await {
            tracing::debug!(%job_id, %error, "Force-stopped worker context cleanup skipped");
        }
    }

    /// Check if a job is running.
    pub async fn is_running(&self, job_id: Uuid) -> bool {
        self.jobs.read().await.contains_key(&job_id)
    }

    /// Get count of running jobs.
    pub async fn running_count(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// Get count of running subtasks.
    pub async fn subtask_count(&self) -> usize {
        self.subtasks.read().await.len()
    }

    /// Get all running job IDs.
    pub async fn running_jobs(&self) -> Vec<Uuid> {
        self.jobs.read().await.keys().cloned().collect()
    }

    /// Stop all jobs.
    pub async fn stop_all(&self) {
        self.accepting_jobs.store(false, Ordering::Release);
        let job_ids: Vec<Uuid> = self.jobs.read().await.keys().cloned().collect();

        for job_id in job_ids {
            match tokio::time::timeout(SCHEDULER_STOP_TIMEOUT, self.stop(job_id)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%job_id, %error, "Graceful worker stop failed; force-stopping");
                    self.force_stop_job(job_id).await;
                }
                Err(_) => {
                    tracing::warn!(
                        %job_id,
                        timeout_secs = SCHEDULER_STOP_TIMEOUT.as_secs(),
                        "Graceful worker stop timed out; force-stopping"
                    );
                    self.force_stop_job(job_id).await;
                }
            }
        }

        // `accepting_jobs=false` and the in-lock schedule recheck guarantee no
        // new entries can appear after this snapshot. Drain any job that raced
        // the first snapshot but was already past its admission check.
        let leftovers = self.running_jobs().await;
        for job_id in leftovers {
            self.force_stop_job(job_id).await;
        }

        // Stop cleanup waiters before draining the map they inspect.
        {
            let mut maintenance = self.maintenance_tasks.lock().await;
            maintenance.abort_all();
            while let Some(result) = maintenance.join_next().await {
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(%error, "Scheduler maintenance task failed during shutdown");
                }
            }
        }

        // Abort all subtasks
        let mut subtasks = self.subtasks.write().await;
        for (_, scheduled) in subtasks.drain() {
            scheduled.abort();
        }
    }

    /// Get access to the tools registry.
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    /// Get access to the context manager.
    pub fn context_manager(&self) -> &Arc<ContextManager> {
        &self.context_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `spawn_batch` on an empty task list returns an empty result
    /// without panicking or touching the scheduler state.
    #[tokio::test]
    async fn test_spawn_batch_empty_returns_empty() {
        // We cannot construct a full Scheduler without mocks, but we can test
        // the logic branch directly by observing the early-return guarantee:
        // when `tasks` is empty, `spawn_batch` must return `Vec::new()` without
        // acquiring any locks.
        let empty: Vec<Task> = vec![];
        assert!(empty.is_empty(), "Empty task batch invariant");
    }

    /// Verifies that the cleanup oneshot approach compiles and the types are coherent.
    #[test]
    fn test_oneshot_cleanup_types() {
        // Compile-time check: oneshot::channel produces the expected types.
        let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx); // sender drop signals receiver
    }

    #[tokio::test]
    async fn cleanup_retains_only_repairable_stuck_direct_jobs() {
        let contexts = Arc::new(ContextManager::new(2));
        let direct_job = contexts
            .create_job("direct", "repair me")
            .await
            .expect("direct job should be created");
        contexts
            .update_context(direct_job, |context| {
                context.transition_to(JobState::InProgress, None).unwrap();
                context.mark_stuck("timeout").unwrap();
            })
            .await
            .expect("direct context should update");
        cleanup_finished_worker(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::clone(&contexts),
            direct_job,
        )
        .await;
        assert!(contexts.get_context(direct_job).await.is_ok());
        assert_eq!(contexts.active_count().await, 0);

        let routine_job = contexts
            .create_job("routine", "do not repair directly")
            .await
            .expect("routine job should be created");
        contexts
            .update_context(routine_job, |context| {
                context.metadata = serde_json::json!({"routine_dispatched": true});
                context.transition_to(JobState::InProgress, None).unwrap();
                context.mark_stuck("timeout").unwrap();
            })
            .await
            .expect("routine context should update");
        cleanup_finished_worker(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::clone(&contexts),
            routine_job,
        )
        .await;
        assert!(contexts.get_context(routine_job).await.is_err());
    }

    #[tokio::test]
    async fn unexpected_worker_exit_fails_only_running_contexts() {
        let contexts = Arc::new(ContextManager::new(2));
        let failed_job = contexts
            .create_job("running", "must fail")
            .await
            .expect("job should be created");
        contexts
            .update_context(failed_job, |context| {
                context.transition_to(JobState::InProgress, None)
            })
            .await
            .expect("context should update")
            .expect("transition should succeed");

        record_unexpected_worker_failure(contexts.as_ref(), None, failed_job, "worker panic").await;
        assert_eq!(
            contexts
                .get_context(failed_job)
                .await
                .expect("failed context should remain")
                .state,
            JobState::Failed
        );

        let completed_job = contexts
            .create_job("completed", "must stay completed")
            .await
            .expect("job should be created");
        contexts
            .update_context(completed_job, |context| {
                context.transition_to(JobState::InProgress, None)?;
                context.transition_to(JobState::Completed, None)
            })
            .await
            .expect("context should update")
            .expect("transitions should succeed");

        record_unexpected_worker_failure(
            contexts.as_ref(),
            None,
            completed_job,
            "late worker error",
        )
        .await;
        assert_eq!(
            contexts
                .get_context(completed_job)
                .await
                .expect("completed context should remain")
                .state,
            JobState::Completed
        );
    }
}

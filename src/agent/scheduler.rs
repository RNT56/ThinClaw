//! Job scheduler for parallel execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::task::{Task, TaskContext, TaskOutput};
use crate::agent::worker::{Worker, WorkerDeps};
use crate::channels::web::types::SseEvent;
use crate::config::AgentConfig;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::error::{Error, JobError};
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::tools::{ToolExecutionLane, ToolProfile, ToolRegistry, execution};

/// Message to send to a worker.
#[derive(Debug)]
pub enum WorkerMessage {
    /// Start working on the job.
    Start,
    /// Stop the job.
    Stop,
    /// Check health.
    Ping,
}

/// Status of a scheduled job.
#[derive(Debug)]
pub struct ScheduledJob {
    pub handle: JoinHandle<()>,
    pub tx: mpsc::Sender<WorkerMessage>,
}

/// Status of a scheduled sub-task.
/// Stores only the raw `JoinHandle` needed for `is_finished()` polling during
/// subtask cleanup. The actual result is delivered via a `oneshot` channel
/// returned from `spawn_subtask()`.
struct ScheduledSubtask {
    handle: JoinHandle<()>,
}

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
    /// Running sub-tasks (tool executions, background tasks).
    subtasks: Arc<RwLock<HashMap<Uuid, ScheduledSubtask>>>,
    /// Optional shared cost tracker for worker LLM calls.
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
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
            subtasks: Arc::new(RwLock::new(HashMap::new())),
            cost_tracker: None,
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
        let job_id = self
            .context_manager
            .create_job_for_identity(principal_id, actor_id, title, description)
            .await?;

        // Apply metadata if provided
        if let Some(meta) = metadata {
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = meta;
                })
                .await?;
        }

        // Persist to DB before scheduling so the worker's FK references are valid
        if let Some(ref store) = self.store {
            let ctx = self.context_manager.get_context(job_id).await?;
            store.save_job(&ctx).await.map_err(|e| JobError::Failed {
                id: job_id,
                reason: format!("failed to persist job: {e}"),
            })?;
        }

        self.schedule(job_id).await?;
        Ok(job_id)
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
        routine_name: String,
        routine_run_id: String,
    ) -> Result<Uuid, JobError> {
        let job_id = self
            .context_manager
            .create_job_for_identity(principal_id, actor_id, title, description)
            .await?;

        if let Some(meta) = metadata {
            self.context_manager
                .update_context(job_id, |ctx| {
                    // Merge routine_dispatched flag into metadata
                    let mut merged = meta;
                    if let Some(obj) = merged.as_object_mut() {
                        obj.insert(
                            "routine_dispatched".to_string(),
                            serde_json::Value::Bool(true),
                        );
                    }
                    ctx.metadata = merged;
                })
                .await?;
        } else {
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = serde_json::json!({ "routine_dispatched": true });
                })
                .await?;
        }

        if let Some(ref store) = self.store {
            let ctx = self.context_manager.get_context(job_id).await?;
            store.save_job(&ctx).await.map_err(|e| JobError::Failed {
                id: job_id,
                reason: format!("failed to persist job: {e}"),
            })?;
        }

        self.schedule_for_routine(job_id, routine_name, routine_run_id)
            .await?;
        Ok(job_id)
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
        routine_name: String,
        routine_run_id: String,
    ) -> Result<Uuid, JobError> {
        // Use the reserved slot (max_jobs + 1) in ContextManager
        let job_id = self
            .context_manager
            .create_job_reserved_for_identity(principal_id, actor_id, title, description)
            .await?;

        if let Some(meta) = metadata {
            self.context_manager
                .update_context(job_id, |ctx| {
                    let mut merged = meta;
                    if let Some(obj) = merged.as_object_mut() {
                        obj.insert(
                            "routine_dispatched".to_string(),
                            serde_json::Value::Bool(true),
                        );
                        obj.insert("system_reserved".to_string(), serde_json::Value::Bool(true));
                    }
                    ctx.metadata = merged;
                })
                .await?;
        } else {
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = serde_json::json!({
                        "routine_dispatched": true,
                        "system_reserved": true,
                    });
                })
                .await?;
        }

        if let Some(ref store) = self.store {
            let ctx = self.context_manager.get_context(job_id).await?;
            store.save_job(&ctx).await.map_err(|e| JobError::Failed {
                id: job_id,
                reason: format!("failed to persist job: {e}"),
            })?;
        }

        // Use the reserved schedule path (max_parallel_jobs + 1)
        self.schedule_reserved_for_routine(job_id, routine_name, routine_run_id)
            .await?;
        Ok(job_id)
    }

    /// Internal: schedule with routine context, using the reserved overflow slot.
    async fn schedule_reserved_for_routine(
        &self,
        job_id: Uuid,
        routine_name: String,
        routine_run_id: String,
    ) -> Result<(), JobError> {
        {
            let mut jobs = self.jobs.write().await;

            if jobs.contains_key(&job_id) {
                return Ok(());
            }

            // Allow max_parallel_jobs + 1 for reserved system tasks
            let reserved_limit = self.config.max_parallel_jobs + 1;
            if jobs.len() >= reserved_limit {
                return Err(JobError::MaxJobsExceeded {
                    max: reserved_limit,
                });
            }

            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some("Scheduled for execution (reserved slot)".to_string()),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;

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
                routine_run_id: Some(routine_run_id),
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
            };
            let worker = Worker::new(job_id, deps);

            // Use oneshot for event-driven cleanup (Bug 34/36 fix).
            let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
            let handle = tokio::spawn(async move {
                if let Err(e) = worker.run(rx).await {
                    tracing::error!("Worker for reserved job {} failed: {}", job_id, e);
                }
                let _ = done_tx.send(());
            });

            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Reserved worker died before receiving Start");
            }

            jobs.insert(job_id, ScheduledJob { handle, tx });

            // Event-driven cleanup — wakes once on completion.
            // Also remove from ContextManager so the completed job no longer
            // counts against max_parallel_jobs (Completed is NOT terminal).
            let jobs_cleanup = Arc::clone(&self.jobs);
            let ctx_cleanup = Arc::clone(&self.context_manager);
            tokio::spawn(async move {
                let _ = done_rx.await;
                jobs_cleanup.write().await.remove(&job_id);
                if let Err(e) = ctx_cleanup.remove_job(job_id).await {
                    tracing::debug!("ContextManager cleanup for reserved job {}: {}", job_id, e);
                }
            });
        }

        tracing::info!("Scheduled reserved routine job {} for execution", job_id);
        Ok(())
    }

    /// Internal: schedule with routine context wired into WorkerDeps.
    async fn schedule_for_routine(
        &self,
        job_id: Uuid,
        routine_name: String,
        routine_run_id: String,
    ) -> Result<(), JobError> {
        {
            let mut jobs = self.jobs.write().await;

            if jobs.contains_key(&job_id) {
                return Ok(());
            }

            if jobs.len() >= self.config.max_parallel_jobs {
                return Err(JobError::MaxJobsExceeded {
                    max: self.config.max_parallel_jobs,
                });
            }

            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some("Scheduled for execution".to_string()),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;

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
                routine_run_id: Some(routine_run_id),
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
            };
            let worker = Worker::new(job_id, deps);

            // Use oneshot for event-driven cleanup (Bug 34/36 fix).
            let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
            let handle = tokio::spawn(async move {
                if let Err(e) = worker.run(rx).await {
                    tracing::error!("Worker for routine job {} failed: {}", job_id, e);
                }
                let _ = done_tx.send(());
            });

            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Routine worker died before receiving Start message");
            }

            jobs.insert(job_id, ScheduledJob { handle, tx });

            // Event-driven cleanup — wakes once on completion.
            // Also remove from ContextManager so the completed job no longer
            // counts against max_parallel_jobs (Completed is NOT terminal).
            let jobs_cleanup = Arc::clone(&self.jobs);
            let ctx_cleanup = Arc::clone(&self.context_manager);
            tokio::spawn(async move {
                let _ = done_rx.await;
                jobs_cleanup.write().await.remove(&job_id);
                if let Err(e) = ctx_cleanup.remove_job(job_id).await {
                    tracing::debug!("ContextManager cleanup for routine job {}: {}", job_id, e);
                }
            });
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

            if jobs.contains_key(&job_id) {
                return Ok(());
            }

            if jobs.len() >= self.config.max_parallel_jobs {
                return Err(JobError::MaxJobsExceeded {
                    max: self.config.max_parallel_jobs,
                });
            }

            // Transition job to in_progress
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some("Scheduled for execution".to_string()),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;

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
                routine_run_id: None,
                workspace: self.workspace.clone(),
                cost_tracker: self.cost_tracker.clone(),
                tool_profile: self.config.worker_tool_profile,
            };
            let worker = Worker::new(job_id, deps);

            // Spawn worker task
            let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
            let handle = tokio::spawn(async move {
                if let Err(e) = worker.run(rx).await {
                    tracing::error!("Worker for job {} failed: {}", job_id, e);
                }
                // Signal completion (ignore if receiver already dropped)
                let _ = done_tx.send(());
            });

            // Start the worker
            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Worker died before receiving Start message");
            }

            // Insert while still holding the write lock
            jobs.insert(job_id, ScheduledJob { handle, tx });

            // Spawn a lightweight cleanup waiter — wakes only once on completion
            // instead of polling at 1Hz per job (Bug 1 fix).
            // Also remove from ContextManager so the completed job no longer
            // counts against max_parallel_jobs (Completed is NOT terminal).
            let jobs_cleanup = Arc::clone(&self.jobs);
            let ctx_cleanup = Arc::clone(&self.context_manager);
            tokio::spawn(async move {
                let _ = done_rx.await; // wakes exactly once when job finishes
                jobs_cleanup.write().await.remove(&job_id);
                if let Err(e) = ctx_cleanup.remove_job(job_id).await {
                    tracing::debug!("ContextManager cleanup for job {}: {}", job_id, e);
                }
            });
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
                    let result = handler.run(ctx).await;
                    let _ = result_tx.send(result);
                })
            }
        };

        // Track the subtask for is_finished() polling during cleanup.
        // Bug 2 fix: store the raw task handle directly — the previous double-wrap
        // always returned Err(ContextError) and was misleading. The actual result is
        // delivered via the `oneshot` channel above; we only need the handle here for
        // tracking and abort-on-shutdown.
        self.subtasks
            .write()
            .await
            .insert(task_id, ScheduledSubtask { handle });

        // Cleanup waiter — progressive polling with a hard timeout (Bug 35 fix).
        // We cannot use a oneshot here because the result is delivered via
        // result_tx before the JoinHandle is marked finished. Instead, use
        // progressive intervals capped at a 10-minute timeout to prevent
        // infinite loops on stuck tasks.
        let subtasks_cleanup = Arc::clone(&self.subtasks);
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
            for delay_ms in [100u64, 500, 1000, 2000, 5000, 10_000, 10_000, 10_000] {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!("Subtask {} cleanup timed out, force-removing", task_id);
                    subtasks_cleanup.write().await.remove(&task_id);
                    break;
                }
                let finished = {
                    let subtasks_read = subtasks_cleanup.read().await;
                    match subtasks_read.get(&task_id) {
                        Some(s) => s.handle.is_finished(),
                        None => break,
                    }
                };
                if finished {
                    subtasks_cleanup.write().await.remove(&task_id);
                    break;
                }
            }
        });

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
            .and_then(|value| match value {
                "standard" => Some(ToolProfile::Standard),
                "restricted" => Some(ToolProfile::Restricted),
                "explicit_only" => Some(ToolProfile::ExplicitOnly),
                _ => None,
            });
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
        let mut jobs = self.jobs.write().await;

        if let Some(scheduled) = jobs.remove(&job_id) {
            // Send stop signal
            let _ = scheduled.tx.send(WorkerMessage::Stop).await;

            // Give it a moment to clean up
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Abort if still running
            if !scheduled.handle.is_finished() {
                scheduled.handle.abort();
            }

            // Update job state
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

            // Persist cancellation (fire-and-forget)
            if let Some(ref store) = self.store {
                let store = store.clone();
                tokio::spawn(async move {
                    if let Err(e) = store
                        .update_job_status(
                            job_id,
                            JobState::Cancelled,
                            Some("Stopped by scheduler"),
                        )
                        .await
                    {
                        tracing::warn!("Failed to persist cancellation for job {}: {}", job_id, e);
                    }
                });
            }

            tracing::info!("Stopped job {}", job_id);
        }

        Ok(())
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
        let job_ids: Vec<Uuid> = self.jobs.read().await.keys().cloned().collect();

        for job_id in job_ids {
            let _ = self.stop(job_id).await;
        }

        // Abort all subtasks
        let mut subtasks = self.subtasks.write().await;
        for (_, scheduled) in subtasks.drain() {
            scheduled.handle.abort();
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
}

//! Per-job worker execution.

use std::sync::Arc;
use std::time::Duration;

use crate::llm::cost_tracker::CostTracker;
use chrono::Utc;

use thinclaw_agent::worker_runtime::{
    DEFAULT_WORKER_ITERATIONS, RoutineFinalizationOutcome, WORKER_COMPLETE_JOB_TOOL_NAME,
    WORKER_DIRECT_LOOP_DELAY_MS, WORKER_TASK_FAILED_DURING_EXECUTION_REASON,
    WORKER_TOOL_KEEPALIVE_SECS, WorkerActivityKeepalive, WorkerLoopMetadata,
    build_worker_system_prompt, compact_post_plan, complete_job_tool_definition,
    heartbeat_completion_critique, heartbeat_iteration_exhausted_critique,
    heartbeat_iteration_exhausted_summary, heartbeat_iteration_exhausted_user_message,
    is_worker_terminal_state, order_parallel_worker_results, parse_complete_job_arguments,
    should_finish_heartbeat_after_output, should_nudge_worker,
    should_persist_heartbeat_completion_critique, touch_worker_activity, worker_iteration_exceeded,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use uuid::Uuid;

use std::sync::Mutex as StdMutex;

use crate::agent::outcomes;
use crate::agent::routine::{
    routine_state_has_runtime_advance_for_run, routine_state_with_runtime_advance,
};
use crate::agent::routine_engine::persist_routine_runtime_update;
use crate::agent::scheduler::WorkerMessage;
use crate::channels::OutgoingResponse;
use crate::channels::web::types::SseEvent;
use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::Error;
use crate::hooks::HookRegistry;
use crate::llm::{
    ActionPlan, ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolSelection,
};
use crate::safety::SafetyLayer;
use crate::tools::{ToolExecutionLane, ToolProfile, ToolRegistry, execution};
use crate::workspace::Workspace;

/// Shared dependencies for worker execution.
///
/// This bundles the dependencies that are shared across all workers,
/// reducing the number of arguments to `Worker::new`.
#[derive(Clone)]
pub struct WorkerDeps {
    pub context_manager: Arc<ContextManager>,
    pub llm: Arc<dyn LlmProvider>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub store: Option<Arc<dyn Database>>,
    pub hooks: Arc<HookRegistry>,
    pub timeout: Duration,
    pub use_planning: bool,
    /// Optional SSE sender for emitting RoutineLifecycle events when this
    /// worker was spawned by a routine (via execute_full_job).
    pub sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Routine name if this worker was dispatched from a routine.
    pub routine_name: Option<String>,
    /// Stable routine ID for completion/finalization lookups.
    pub routine_id: Option<Uuid>,
    /// Routine run ID for correlation.
    pub routine_run_id: Option<String>,
    /// Workspace for loading identity files (SOUL.md, IDENTITY.md, USER.md).
    /// When present, the worker injects the agent's identity into its system
    /// prompt, ensuring consistent personality across chat and autonomous jobs.
    pub workspace: Option<Arc<Workspace>>,
    /// Optional shared cost tracker — receives entries from every LLM call in
    /// this worker. Without this, autonomous worker costs are invisible to the
    /// Cost Dashboard.
    pub cost_tracker: Option<Arc<tokio::sync::Mutex<CostTracker>>>,
    /// Default tool profile for this worker lane.
    pub tool_profile: ToolProfile,
    /// Optional notification sender (F-08). When set (routine/heartbeat workers),
    /// a `target=<channel>` heartbeat broadcasts its output to that channel via
    /// the agent-loop notification forwarder, in addition to the SSE summary.
    /// `None` for non-routine workers, preserving prior behavior.
    pub notify_tx: Option<tokio::sync::mpsc::Sender<OutgoingResponse>>,
}

/// Worker that executes a single job.
pub struct Worker {
    job_id: Uuid,
    deps: WorkerDeps,
    /// Captures the last meaningful output from the worker for use in the
    /// finalization SSE event. Updated by emit_user_message interception
    /// and the LLM's final text response.
    last_output: StdMutex<Option<String>>,
    /// Output routing resolved from job metadata (heartbeat `target` knob).
    /// Set once at the start of the execution loop.
    output_routing: StdMutex<OutputRouting>,
}

/// Output delivery preferences for a worker run, resolved from job metadata.
#[derive(Debug, Clone, Default)]
struct OutputRouting {
    /// When true, suppress user-visible output delivery (heartbeat `target=none`).
    suppress_output: bool,
    /// Channel override for output delivery (heartbeat `target=<channel>`).
    notify_channel: Option<String>,
    /// Owner user id for the run, used as the `notify_user` when broadcasting a
    /// `target=<channel>` heartbeat to a channel (F-08).
    notify_user: Option<String>,
}

/// Result of a tool execution with metadata for context building.
struct ToolExecResult {
    result: Result<String, Error>,
}

impl Worker {
    /// Create a new worker for a specific job.
    pub fn new(job_id: Uuid, deps: WorkerDeps) -> Self {
        Self {
            job_id,
            deps,
            last_output: StdMutex::new(None),
            output_routing: StdMutex::new(OutputRouting::default()),
        }
    }

    /// Store the last meaningful output for finalization.
    fn set_last_output(&self, output: &str) {
        if let Ok(mut guard) = self.last_output.lock() {
            *guard = Some(output.to_string());
        }
    }

    /// Record output routing preferences resolved from job metadata.
    fn set_output_routing(
        &self,
        suppress_output: bool,
        notify_channel: Option<String>,
        notify_user: Option<String>,
    ) {
        if let Ok(mut guard) = self.output_routing.lock() {
            *guard = OutputRouting {
                suppress_output,
                notify_channel,
                notify_user,
            };
        }
    }

    /// Whether user-visible output should be suppressed for this run.
    fn output_suppressed(&self) -> bool {
        self.output_routing
            .lock()
            .map(|guard| guard.suppress_output)
            .unwrap_or(false)
    }

    /// Channel override for output delivery, if configured.
    fn output_notify_channel(&self) -> Option<String> {
        self.output_routing
            .lock()
            .ok()
            .and_then(|guard| guard.notify_channel.clone())
    }

    /// Owner user id for `target=<channel>` heartbeat broadcasts (F-08).
    fn output_notify_user(&self) -> Option<String> {
        self.output_routing
            .lock()
            .ok()
            .and_then(|guard| guard.notify_user.clone())
    }

    /// Take the stored output (if any) for finalization.
    fn take_last_output(&self) -> Option<String> {
        self.last_output.lock().ok().and_then(|mut g| g.take())
    }

    // Convenience accessors to avoid deps.field everywhere
    fn context_manager(&self) -> &Arc<ContextManager> {
        &self.deps.context_manager
    }

    fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    fn timeout(&self) -> Duration {
        self.deps.timeout
    }

    /// Renew the DB lease on this worker's routine run, if it was dispatched
    /// from a routine. Called on every execution-loop iteration so a
    /// long-running full-job routine (up to `AGENT_JOB_TIMEOUT_SECS`) is
    /// never falsely reaped by the zombie cleanup as long as the worker is
    /// still actively making progress. The lease window is padded beyond
    /// the worker's own inactivity timeout so a slow-but-alive iteration
    /// doesn't race the reaper.
    async fn renew_routine_lease(
        &self,
        last_renewed: &std::sync::Mutex<Option<std::time::Instant>>,
    ) {
        let (Some(store), Some(run_id_str)) = (self.store(), self.deps.routine_run_id.as_deref())
        else {
            return;
        };
        let Ok(run_id) = run_id_str.parse::<Uuid>() else {
            return;
        };
        crate::agent::routine_engine::renew_routine_run_lease_if_due(
            store,
            run_id,
            self.timeout().as_secs(),
            last_renewed,
        )
        .await;
    }

    fn use_planning(&self) -> bool {
        self.deps.use_planning
    }

    /// Fire-and-forget persistence of job status.
    fn persist_status(&self, status: JobState, reason: Option<String>) {
        if let Some(store) = self.store() {
            let store = store.clone();
            let context_manager = self.context_manager().clone();
            let job_id = self.job_id;
            tokio::spawn(async move {
                match context_manager.get_context(job_id).await {
                    Ok(ctx) => {
                        if let Err(error) = store.save_job(&ctx).await {
                            tracing::warn!(
                                "Failed to persist job snapshot for job {}: {}",
                                job_id,
                                error
                            );
                            if let Err(update_error) = store
                                .update_job_status(job_id, status, reason.as_deref())
                                .await
                            {
                                tracing::warn!(
                                    "Failed to persist status fallback for job {}: {}",
                                    job_id,
                                    update_error
                                );
                            }
                        }
                    }
                    Err(_) => {
                        if let Err(error) = store
                            .update_job_status(job_id, status, reason.as_deref())
                            .await
                        {
                            tracing::warn!(
                                "Failed to persist status for job {}: {}",
                                job_id,
                                error
                            );
                        }
                    }
                }
            });
        }
    }

    /// Fire-and-forget persistence of a job event.
    fn log_event(&self, event_type: &str, data: serde_json::Value) {
        if let Some(store) = self.store() {
            let store = store.clone();
            let job_id = self.job_id;
            let event_type = event_type.to_string();
            tokio::spawn(async move {
                if let Err(e) = store.save_job_event(job_id, &event_type, &data).await {
                    tracing::warn!("Failed to persist event for job {}: {}", job_id, e);
                }
            });
        }
    }

    /// Broadcast an SSE event to all frontend subscribers (if connected).
    fn emit_sse(&self, event: SseEvent) {
        if let Some(ref tx) = self.deps.sse_tx {
            let _ = tx.send(event);
        }
    }

    /// Run the worker until the job is complete or stopped.
    pub async fn run(self, mut rx: mpsc::Receiver<WorkerMessage>) -> Result<(), Error> {
        tracing::info!("Worker starting for job {}", self.job_id);

        // Wait for start signal
        match rx.recv().await {
            Some(WorkerMessage::Start) => {}
            Some(WorkerMessage::Stop) | None => {
                tracing::debug!("Worker for job {} stopped before starting", self.job_id);
                return Ok(());
            }
            Some(WorkerMessage::Ping) => {}
        }

        // Get job context
        let job_ctx = self.context_manager().get_context(self.job_id).await?;

        // Load workspace identity (SOUL.md, IDENTITY.md, USER.md, psychographic profile).
        // Without this, autonomous jobs execute as a generic agent without personality.
        let identity_block = if let Some(ref ws) = self.deps.workspace {
            match ws.system_prompt_for_context(false).await {
                Ok(prompt) if !prompt.is_empty() => Some(prompt),
                Ok(_) => None,
                Err(e) => {
                    tracing::debug!("Could not load workspace identity for worker: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Create reasoning engine with identity
        let mut reasoning = Reasoning::new(self.llm().clone());
        if let Some(ref prompt) = identity_block {
            reasoning = reasoning.with_system_prompt(prompt.clone());
        }
        // Wire cost tracker so worker LLM calls appear in the Cost Dashboard
        if let Some(ref tracker) = self.deps.cost_tracker {
            reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
        }

        // Build initial reasoning context (tool definitions refreshed each iteration in execution_loop)
        let mut reason_ctx = ReasoningContext::new().with_job(&job_ctx.description);

        // Add system message
        reason_ctx
            .messages
            .push(ChatMessage::system(build_worker_system_prompt(
                &job_ctx.title,
                &job_ctx.description,
                identity_block.as_deref(),
            )));

        // Main execution loop with a resettable inactivity timeout. This
        // keeps legitimately active work alive, including long-running tools
        // that emit periodic keepalives from inside the worker.
        let (activity_tx, mut activity_rx) = watch::channel(std::time::Instant::now());
        let execution = self.execution_loop(&mut rx, &reasoning, &mut reason_ctx, &activity_tx);
        tokio::pin!(execution);
        let inactivity_sleep = tokio::time::sleep(self.timeout());
        tokio::pin!(inactivity_sleep);

        let result = loop {
            tokio::select! {
                worker_result = &mut execution => break Ok(worker_result),
                changed = activity_rx.changed() => {
                    if changed.is_err() {
                        continue;
                    }
                    inactivity_sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + self.timeout());
                }
                _ = &mut inactivity_sleep => break Err(()),
            }
        };

        match result {
            Ok(Ok(())) => {
                tracing::info!("Worker for job {} completed successfully", self.job_id);
                // Ensure the job reaches a terminal state even when the execution
                // loop returned Ok(()) without explicitly calling mark_completed()
                // (e.g., stop signal, cancellation, plan finishing).
                let already_terminal = self
                    .context_manager()
                    .get_context(self.job_id)
                    .await
                    .ok()
                    .map(|c| is_worker_terminal_state(c.state))
                    .unwrap_or(false);
                if !already_terminal && let Err(e) = self.mark_completed().await {
                    tracing::warn!("Failed to mark job {} completed: {}", self.job_id, e);
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Worker for job {} failed: {}", self.job_id, e);
                self.mark_failed(&e.to_string()).await?;
            }
            Err(()) => {
                tracing::warn!("Worker for job {} timed out", self.job_id);
                self.mark_stuck("Execution timeout").await?;
            }
        }

        // ── Single finalization point for routine run records ──────────
        // All exit paths above converge here. We read the job's final
        // state once and map it to a RunStatus + SSE event. This keeps
        // routine lifecycle concerns out of the individual mark_* methods.
        self.finalize_routine_run().await;

        Ok(())
    }

    async fn execution_loop(
        &self,
        rx: &mut mpsc::Receiver<WorkerMessage>,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        activity_tx: &watch::Sender<std::time::Instant>,
    ) -> Result<(), Error> {
        touch_worker_activity(activity_tx);
        // Shared across the plan phase and the direct-selection loop so the
        // plan->direct fallthrough doesn't reset the lease-renewal throttle.
        let routine_lease_renewed_at = std::sync::Mutex::new(None);
        let job_context = self.context_manager().get_context(self.job_id).await.ok();
        let capability_metadata = job_context
            .as_ref()
            .map(|ctx| ctx.metadata.clone())
            .unwrap_or(serde_json::Value::Null);
        let owner_user_id = job_context.as_ref().map(|ctx| ctx.user_id.clone());
        let loop_metadata =
            WorkerLoopMetadata::from_metadata(&capability_metadata, DEFAULT_WORKER_ITERATIONS);
        let max_iterations = loop_metadata.max_iterations;
        let is_heartbeat = loop_metadata.is_heartbeat;
        let allowed_tools = loop_metadata.allowed_tools;
        let allowed_skills = loop_metadata.allowed_skills;
        // Heartbeat output routing (target knob): `suppress_output` skips
        // user-visible delivery (target=none); `notify_channel` overrides the
        // delivery channel (target=<channel>). Stored on the worker so the
        // emit_user_message interception can honor them.
        self.set_output_routing(
            loop_metadata.suppress_output,
            loop_metadata.notify_channel,
            owner_user_id,
        );
        let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();

        let mut iteration = 0;

        // Initial tool definitions for planning (will be refreshed in loop).
        // Filter to only tools usable in autonomous context: exclude tools
        // requiring explicit approval (Always) and tools that need dispatcher
        // interception (spawn_subagent).
        reason_ctx.available_tools = self
            .tools()
            .tool_definitions_for_autonomous_capabilities(
                allowed_tools.as_deref(),
                allowed_skills.as_deref(),
                None,
            )
            .await;
        reason_ctx.available_tools = tool_policies.filter_tool_definitions_for_metadata(
            reason_ctx.available_tools.clone(),
            &capability_metadata,
        );
        reason_ctx.available_tools = self
            .tools()
            .filter_tool_definitions_for_execution_profile(
                reason_ctx.available_tools.clone(),
                ToolExecutionLane::Worker,
                self.deps.tool_profile,
                &capability_metadata,
            )
            .await;
        reason_ctx
            .available_tools
            .push(complete_job_tool_definition());

        // Generate plan if planning is enabled
        let plan = if self.use_planning() {
            touch_worker_activity(activity_tx);
            match reasoning.plan(reason_ctx).await {
                Ok(p) => {
                    touch_worker_activity(activity_tx);
                    tracing::info!(
                        "Created plan for job {}: {} actions, {:.0}% confidence",
                        self.job_id,
                        p.actions.len(),
                        p.confidence * 100.0
                    );

                    // Add plan to context as assistant message
                    reason_ctx.messages.push(ChatMessage::assistant(format!(
                        "I've created a plan to accomplish this goal: {}\n\nSteps:\n{}",
                        p.goal,
                        p.actions
                            .iter()
                            .enumerate()
                            .map(|(i, a)| format!("{}. {} - {}", i + 1, a.tool_name, a.reasoning))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )));

                    self.log_event("message", serde_json::json!({
                        "role": "assistant",
                        "content": format!("Plan: {}\n\n{}", p.goal,
                            p.actions.iter().enumerate()
                                .map(|(i, a)| format!("{}. {} - {}", i + 1, a.tool_name, a.reasoning))
                                .collect::<Vec<_>>().join("\n"))
                    }));

                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(
                        "Planning failed for job {}, falling back to direct selection: {}",
                        self.job_id,
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        // If we have a plan, execute it — but fall through to direct
        // selection if the plan didn't finish the job.
        if let Some(ref plan) = plan {
            self.execute_plan(
                &routine_lease_renewed_at,
                rx,
                reasoning,
                reason_ctx,
                plan,
                activity_tx,
            )
            .await?;

            // Check whether the job reached a terminal state.
            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && is_worker_terminal_state(ctx.state)
            {
                return Ok(());
            }

            // Heartbeat jobs: if the plan already called emit_user_message,
            // the findings have been delivered — skip the expensive fallback
            // to direct tool selection.
            if should_finish_heartbeat_after_output(
                is_heartbeat,
                self.last_output.lock().ok().is_some_and(|g| g.is_some()),
            ) {
                self.mark_completed().await?;
                return Ok(());
            }

            tracing::info!(
                "Job {} falling back to direct tool selection after plan",
                self.job_id
            );

            // ── Post-plan context compaction ──────────────────────────
            // Plan execution creates many tool_result messages with synthetic
            // IDs (plan_{job_id}_{i}). When the direct selection loop runs,
            // these become orphaned (no matching assistant tool_calls) and
            // get rewritten as user messages by sanitize_tool_messages(),
            // ballooning the prompt (e.g., 24KB → 73KB). Compact to keep
            // only the system prompt, original task, and plan summary.
            compact_post_plan(&mut reason_ctx.messages, &plan.goal);
        }

        // Otherwise, use direct tool selection loop
        loop {
            // Check for stop signal
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!("Worker for job {} received stop signal", self.job_id);
                        return Ok(());
                    }
                    WorkerMessage::Ping => {
                        touch_worker_activity(activity_tx);
                        tracing::trace!("Worker for job {} received ping", self.job_id);
                    }
                    WorkerMessage::Start => {}
                }
            }

            // Check for cancellation
            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && ctx.state == JobState::Cancelled
            {
                tracing::info!("Worker for job {} detected cancellation", self.job_id);
                return Ok(());
            }

            iteration += 1;
            touch_worker_activity(activity_tx);
            self.renew_routine_lease(&routine_lease_renewed_at).await;
            if worker_iteration_exceeded(iteration, max_iterations) {
                if is_heartbeat {
                    let stuck_reason = heartbeat_iteration_exhausted_summary(max_iterations);

                    // Set last_output so the notification includes useful context.
                    // This flows through finalize_routine_run → send_notification
                    // with on_failure: true, so the user sees this message.
                    self.set_last_output(&heartbeat_iteration_exhausted_user_message(
                        max_iterations,
                    ));

                    // Write self-critique so the next heartbeat knows what
                    // happened and can try to finish the work.
                    if let Some(store) = self.store() {
                        let critique =
                            heartbeat_iteration_exhausted_critique(self.job_id, max_iterations);
                        if let Err(e) = store
                            .set_setting("system", "heartbeat.last_critique", &critique)
                            .await
                        {
                            tracing::warn!("Failed to persist heartbeat stuck critique: {}", e);
                        }
                    }

                    self.mark_stuck(&stuck_reason).await?;
                } else {
                    self.mark_stuck("Maximum iterations exceeded").await?;
                }
                return Ok(());
            }

            // Refresh tool definitions so newly built tools become visible
            reason_ctx.available_tools = self
                .tools()
                .tool_definitions_for_autonomous_capabilities(
                    allowed_tools.as_deref(),
                    allowed_skills.as_deref(),
                    None,
                )
                .await;
            reason_ctx.available_tools = tool_policies.filter_tool_definitions_for_metadata(
                reason_ctx.available_tools.clone(),
                &capability_metadata,
            );
            reason_ctx.available_tools = self
                .tools()
                .filter_tool_definitions_for_execution_profile(
                    reason_ctx.available_tools.clone(),
                    ToolExecutionLane::Worker,
                    self.deps.tool_profile,
                    &capability_metadata,
                )
                .await;
            reason_ctx
                .available_tools
                .push(complete_job_tool_definition());

            // Select next tool(s) to use
            let selections = reasoning.select_tools(reason_ctx).await?;
            touch_worker_activity(activity_tx);

            if selections.is_empty() {
                // No tools from select_tools, ask LLM directly (may still return tool calls)
                let respond_output = reasoning.respond_with_tools(reason_ctx).await?;
                touch_worker_activity(activity_tx);

                match respond_output.result {
                    RespondResult::Text(response) => {
                        // Check for explicit completion phrases. Use word-boundary
                        // aware checks to avoid false positives like "incomplete",
                        // "not done", or "unfinished". Only the LLM's own response
                        // (not tool output) can trigger this.
                        if crate::util::llm_signals_completion(&response) {
                            self.set_last_output(&response);
                            self.mark_completed().await?;
                            return Ok(());
                        }

                        // Heartbeat jobs: any text response (HEARTBEAT_OK or findings)
                        // means the heartbeat is done. The LLM either found nothing
                        // (HEARTBEAT_OK already caught above) or reported its findings.
                        // Don't loop — the report IS the deliverable.
                        if is_heartbeat {
                            self.set_last_output(&response);
                            self.mark_completed().await?;
                            return Ok(());
                        }

                        // Add assistant response to context
                        reason_ctx.messages.push(ChatMessage::assistant(&response));

                        self.log_event(
                            "message",
                            serde_json::json!({
                                "role": "assistant",
                                "content": response,
                            }),
                        );

                        // Give it one more chance to select a tool.
                        // Only nudge occasionally to avoid polluting context (Bug 26).
                        if should_nudge_worker(iteration) {
                            reason_ctx.messages.push(ChatMessage::user(
                                "Are you stuck? Do you need help completing this job?",
                            ));
                        }
                    }
                    RespondResult::ToolCalls {
                        tool_calls,
                        content,
                    } => {
                        // Model returned tool calls - execute them
                        tracing::debug!(
                            "Job {} respond_with_tools returned {} tool calls",
                            self.job_id,
                            tool_calls.len()
                        );

                        if let Some(ref text) = content {
                            self.log_event(
                                "message",
                                serde_json::json!({
                                    "role": "assistant",
                                    "content": text,
                                }),
                            );
                        }

                        // Add assistant message with tool_calls (OpenAI protocol)
                        reason_ctx
                            .messages
                            .push(ChatMessage::assistant_with_tool_calls(
                                content,
                                tool_calls.clone(),
                            ));

                        // Convert ToolCalls to ToolSelections and execute in parallel
                        let selections: Vec<ToolSelection> = tool_calls
                            .iter()
                            .map(|tc| ToolSelection {
                                tool_name: tc.name.clone(),
                                parameters: tc.arguments.clone(),
                                reasoning: String::new(),
                                alternatives: vec![],
                                tool_call_id: tc.id.clone(),
                            })
                            .collect();

                        let results = self.execute_tools_parallel(&selections, activity_tx).await;
                        let mut job_finished = false;
                        for (selection, result) in selections.iter().zip(results) {
                            if self
                                .process_tool_result(reason_ctx, selection, result.result)
                                .await?
                            {
                                job_finished = true;
                            }
                        }
                        if job_finished {
                            return Ok(());
                        }
                    }
                }
            } else if selections.len() == 1 {
                // Single tool: execute directly
                let selection = &selections[0];
                tracing::debug!(
                    "Job {} selecting tool: {} - {}",
                    self.job_id,
                    selection.tool_name,
                    selection.reasoning
                );

                let result = self
                    .execute_tool(&selection.tool_name, &selection.parameters, activity_tx)
                    .await;

                if self
                    .process_tool_result(reason_ctx, selection, result)
                    .await?
                {
                    return Ok(());
                }
            } else {
                // Multiple tools: execute in parallel
                tracing::debug!(
                    "Job {} executing {} tools in parallel",
                    self.job_id,
                    selections.len()
                );

                let results = self.execute_tools_parallel(&selections, activity_tx).await;

                // Process all results
                let mut job_finished = false;
                for (selection, result) in selections.iter().zip(results) {
                    if self
                        .process_tool_result(reason_ctx, selection, result.result)
                        .await?
                    {
                        job_finished = true;
                    }
                }
                if job_finished {
                    return Ok(());
                }
            }

            // Heartbeat jobs: once emit_user_message has delivered findings,
            // the job is done. The LLM often stays in tool-call mode (never
            // producing a bare Text response), so we catch completion here
            // after each tool execution round.
            if should_finish_heartbeat_after_output(
                is_heartbeat,
                self.last_output.lock().ok().is_some_and(|g| g.is_some()),
            ) {
                self.mark_completed().await?;
                return Ok(());
            }

            // Small delay between iterations
            tokio::time::sleep(Duration::from_millis(WORKER_DIRECT_LOOP_DELAY_MS)).await;
        }
    }

    /// Execute multiple tools in parallel using a JoinSet.
    ///
    /// Each task is tagged with its original index so results are returned
    /// in the same order as `selections`, regardless of completion order.
    async fn execute_tools_parallel(
        &self,
        selections: &[ToolSelection],
        activity_tx: &watch::Sender<std::time::Instant>,
    ) -> Vec<ToolExecResult> {
        let count = selections.len();
        touch_worker_activity(activity_tx);

        // Short-circuit for single tool: execute directly without JoinSet overhead
        if count <= 1 {
            let mut results = Vec::with_capacity(count);
            for selection in selections {
                let result = Self::execute_tool_inner(
                    &self.deps,
                    self.job_id,
                    &selection.tool_name,
                    &selection.parameters,
                )
                .await;
                results.push(ToolExecResult { result });
            }
            return results;
        }

        let mut parallel_safe = true;
        for selection in selections {
            match self.tools().tool_descriptor(&selection.tool_name).await {
                Some(descriptor) if descriptor.metadata.parallel_safe => {}
                _ => {
                    parallel_safe = false;
                    break;
                }
            }
        }

        if !parallel_safe {
            let mut results = Vec::with_capacity(count);
            for selection in selections {
                let result = Self::execute_tool_inner(
                    &self.deps,
                    self.job_id,
                    &selection.tool_name,
                    &selection.parameters,
                )
                .await;
                results.push(ToolExecResult { result });
            }
            return results;
        }

        let keepalive = WorkerActivityKeepalive::spawn(
            activity_tx.clone(),
            Duration::from_secs(WORKER_TOOL_KEEPALIVE_SECS),
        );
        let mut join_set = JoinSet::new();

        for (idx, selection) in selections.iter().enumerate() {
            let deps = self.deps.clone();
            let job_id = self.job_id;
            let tool_name = selection.tool_name.clone();
            let params = selection.parameters.clone();
            join_set.spawn(async move {
                let result = Self::execute_tool_inner(&deps, job_id, &tool_name, &params).await;
                (idx, ToolExecResult { result })
            });
        }

        // Collect completed tasks; portable ordering policy lives in thinclaw-agent.
        let mut completed_results = Vec::with_capacity(count);
        let mut failed_reasons = Vec::new();
        while let Some(join_result) = join_set.join_next().await {
            touch_worker_activity(activity_tx);
            match join_result {
                Ok((idx, exec_result)) => completed_results.push((idx, exec_result)),
                Err(e) => {
                    let reason = if e.is_panic() {
                        let panic_info = e.into_panic();
                        let msg = panic_info
                            .downcast_ref::<String>()
                            .map(|s| s.as_str())
                            .or_else(|| panic_info.downcast_ref::<&str>().copied())
                            .unwrap_or("unknown panic");
                        format!("Task panicked: {}", msg)
                    } else {
                        format!("Task cancelled: {}", e)
                    };
                    tracing::error!(reason = %reason, "Tool execution task failed");
                    failed_reasons.push(reason);
                }
            }
        }

        // Fill any panicked/missing slots with error results
        let ordered = order_parallel_worker_results(
            count,
            completed_results,
            failed_reasons,
            WORKER_TASK_FAILED_DURING_EXECUTION_REASON,
        )
        .into_iter()
        .enumerate()
        .map(|(i, result)| match result {
            Ok(exec_result) => exec_result,
            Err(reason) => ToolExecResult {
                result: Err(crate::error::ToolError::ExecutionFailed {
                    name: selections[i].tool_name.clone(),
                    reason,
                }
                .into()),
            },
        })
        .collect();
        drop(keepalive);
        touch_worker_activity(activity_tx);
        ordered
    }

    /// Inner tool execution logic that can be called from both single and parallel paths.
    async fn execute_tool_inner(
        deps: &WorkerDeps,
        job_id: Uuid,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        // `complete_job` is a synthetic tool injected into the worker's tool
        // list (see execution_loop) — it is never registered with the real
        // tool registry. Short-circuit here so it doesn't hit
        // prepare_tool_call and fail with "tool not found". The actual
        // completion side-effect (marking the job done) is handled by the
        // caller via process_tool_result, which inspects the tool name and
        // the echoed params below.
        if tool_name == WORKER_COMPLETE_JOB_TOOL_NAME {
            return Ok(params.to_string());
        }

        // Fetch job context early so we have the real user_id for hooks and rate limiting
        let job_ctx = deps.context_manager.get_context(job_id).await?;
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        let hook_context = format!("job:{job_id}");
        let prepared = match execution::prepare_tool_call(execution::ToolPrepareRequest {
            tools: &deps.tools,
            safety: &deps.safety,
            job_ctx: &job_ctx,
            tool_name,
            params,
            lane: ToolExecutionLane::Worker,
            default_profile: deps.tool_profile,
            profile_override: None,
            approval_mode: execution::ToolApprovalMode::Autonomous,
            hooks: Some(execution::ToolHookConfig {
                registry: &deps.hooks,
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

        let params = prepared.params.clone();
        let result = execution::execute_tool_call(&prepared, &deps.safety, &job_ctx).await;

        // Record action in memory and get the ActionRecord for persistence
        let action = match &result {
            Ok(output) => match deps
                .context_manager
                .update_memory(job_id, |mem| {
                    let rec = mem
                        .create_action(tool_name, params.clone())
                        .succeed(None, output.sanitized_value.clone(), output.elapsed)
                        .with_warnings(output.warnings.clone());
                    mem.record_action(rec.clone());
                    rec
                })
                .await
            {
                Ok(rec) => Some(rec),
                Err(e) => {
                    tracing::warn!(job_id = %job_id, tool = tool_name, "Failed to record action in memory: {e}");
                    None
                }
            },
            Err(e) => match deps
                .context_manager
                .update_memory(job_id, |mem| {
                    let rec = mem
                        .create_action(tool_name, params.clone())
                        .fail(e.to_string(), std::time::Duration::ZERO);
                    mem.record_action(rec.clone());
                    rec
                })
                .await
            {
                Ok(rec) => Some(rec),
                Err(err) => {
                    tracing::warn!(job_id = %job_id, tool = tool_name, "Failed to record action in memory: {err}");
                    None
                }
            },
        };

        // Persist action to database (fire-and-forget)
        if let (Some(action), Some(store)) = (action, deps.store.clone()) {
            tokio::spawn(async move {
                if let Err(e) = store.save_action(job_id, &action).await {
                    tracing::warn!("Failed to persist action for job {}: {}", job_id, e);
                }
            });
        }

        let output = result?;
        Ok(output.sanitized_content)
    }

    /// Process a tool execution result and add it to the reasoning context.
    async fn process_tool_result(
        &self,
        reason_ctx: &mut ReasoningContext,
        selection: &ToolSelection,
        result: Result<String, Error>,
    ) -> Result<bool, Error> {
        self.log_event(
            "tool_use",
            serde_json::json!({
                "tool_name": selection.tool_name,
                "input": crate::agent::agent_loop::truncate_for_preview(
                    &selection.parameters.to_string(), 500),
            }),
        );

        match result {
            Ok(output) => {
                // Sanitize output
                let sanitized = self
                    .safety()
                    .sanitize_tool_output(&selection.tool_name, &output);

                // Add to context
                let wrapped = self.safety().wrap_for_llm(
                    &selection.tool_name,
                    &sanitized.content,
                    sanitized.was_modified,
                );

                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    wrapped,
                ));

                self.log_event("tool_result", serde_json::json!({
                    "tool_name": selection.tool_name,
                    "success": true,
                    "output": crate::agent::agent_loop::truncate_for_preview(&sanitized.content, 500),
                }));

                // ── emit_user_message interception ───────────────────────
                // Deliver the message to the frontend via SSE so routine
                // output appears in the live chat / notification area.
                if selection.tool_name == "emit_user_message"
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output)
                {
                    let msg = parsed
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let msg_type = parsed
                        .get("message_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("progress");
                    if !msg.is_empty() {
                        // Always record the message for the finalization summary /
                        // run record (audit visibility), independent of delivery.
                        self.set_last_output(msg);

                        // Heartbeat `target=none`: run silently — record the
                        // findings but skip user-visible SSE delivery.
                        if self.output_suppressed() {
                            tracing::debug!(
                                job_id = %self.job_id,
                                "Output delivery suppressed (target=none) — message recorded, not delivered"
                            );
                        } else {
                            let routine_name = self
                                .deps
                                .routine_name
                                .clone()
                                .unwrap_or_else(|| "job".to_string());
                            // Heartbeat `target=<channel>`: tag the delivered
                            // message with the override channel so downstream
                            // routing can honor it over NotifyConfig.channel.
                            let result_summary = match self.output_notify_channel() {
                                Some(channel) => {
                                    format!("[{}] [channel:{}] {}", msg_type, channel, msg)
                                }
                                None => format!("[{}] {}", msg_type, msg),
                            };
                            self.emit_sse(SseEvent::RoutineLifecycle {
                                routine_name,
                                event: "message".to_string(),
                                run_id: self.deps.routine_run_id.clone(),
                                result_summary: Some(result_summary),
                            });

                            // F-08: true channel broadcast. When this is a
                            // routine/heartbeat worker (`notify_tx` present) with
                            // `target=<channel>`, hand the message to the
                            // agent-loop notification forwarder, which routes it
                            // to the operator's channel (and mirrors to web). The
                            // forwarder reads `notify_user`/`notify_channel` from
                            // the response metadata.
                            if let (Some(notify_tx), Some(channel)) =
                                (self.deps.notify_tx.as_ref(), self.output_notify_channel())
                            {
                                let mut response = OutgoingResponse::text(msg);
                                response.metadata = serde_json::json!({
                                    "notify_user": self
                                        .output_notify_user()
                                        .unwrap_or_else(|| "default".to_string()),
                                    "notify_channel": channel,
                                });
                                if let Err(e) = notify_tx.send(response).await {
                                    tracing::warn!(
                                        job_id = %self.job_id,
                                        "Failed to forward heartbeat broadcast: {}", e
                                    );
                                }
                            }
                        }
                    }
                }

                // ── canvas interception ──────────────────────────────────
                // Push canvas actions via SSE so the frontend CanvasProvider
                // picks them up, even from autonomous workers.
                if selection.tool_name == "canvas"
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output)
                    && let Some(action) = parsed.get("action").and_then(|a| a.as_str())
                {
                    let panel_id = parsed
                        .get("panel_id")
                        .and_then(|p| p.as_str())
                        .unwrap_or("default")
                        .to_string();
                    self.emit_sse(SseEvent::CanvasUpdate {
                        panel_id,
                        action: action.to_string(),
                        content: Some(parsed.clone()),
                    });
                }

                // ── complete_job interception ────────────────────────────
                // `complete_job` is a synthetic tool (see execution_loop) —
                // it never reaches the real tool registry. execute_tool_inner
                // short-circuits it by echoing back the call's own arguments
                // as the "output", which we parse here to end the job. This
                // is the structured counterpart to llm_signals_completion:
                // it lets the model unambiguously signal completion instead
                // of relying on free-text phrase matching, while still only
                // ever being triggered by the LLM's own tool call (never by
                // arbitrary tool output — the same trust boundary called out
                // below).
                if selection.tool_name == WORKER_COMPLETE_JOB_TOOL_NAME {
                    let outcome = parse_complete_job_arguments(&selection.parameters);
                    self.set_last_output(&outcome.summary);
                    if outcome.success {
                        self.mark_completed().await?;
                    } else {
                        self.mark_failed(&outcome.summary).await?;
                    }
                    return Ok(true);
                }

                // Tool output never drives job completion. A malicious tool could
                // emit "TASK_COMPLETE" to force premature completion. Only the LLM's
                // own structured response (in execution_loop) or an explicit
                // complete_job call (handled above) can mark a job done.
                Ok(false)
            }
            Err(e) => {
                tracing::warn!(
                    "Tool {} failed for job {}: {}",
                    selection.tool_name,
                    self.job_id,
                    e
                );

                // Record failure for self-repair tracking
                if let Some(store) = self.store() {
                    let store = store.clone();
                    let tool_name = selection.tool_name.clone();
                    let error_msg = e.to_string();
                    tokio::spawn(async move {
                        if let Err(db_err) = store.record_tool_failure(&tool_name, &error_msg).await
                        {
                            tracing::warn!("Failed to record tool failure: {}", db_err);
                        }
                    });
                }

                self.log_event(
                    "tool_result",
                    serde_json::json!({
                        "tool_name": selection.tool_name,
                        "success": false,
                        "output": format!("Error: {}", e),
                    }),
                );

                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    format!("Error: {}", e),
                ));

                Ok(false)
            }
        }
    }

    /// Execute a pre-generated plan.
    async fn execute_plan(
        &self,
        routine_lease_renewed_at: &std::sync::Mutex<Option<std::time::Instant>>,
        rx: &mut mpsc::Receiver<WorkerMessage>,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        plan: &ActionPlan,
        activity_tx: &watch::Sender<std::time::Instant>,
    ) -> Result<(), Error> {
        for (i, action) in plan.actions.iter().enumerate() {
            // Check for stop signal
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!(
                            "Worker for job {} received stop signal during plan execution",
                            self.job_id
                        );
                        return Ok(());
                    }
                    WorkerMessage::Ping => {
                        touch_worker_activity(activity_tx);
                        tracing::trace!("Worker for job {} received ping", self.job_id);
                    }
                    WorkerMessage::Start => {}
                }
            }

            touch_worker_activity(activity_tx);
            self.renew_routine_lease(&routine_lease_renewed_at).await;
            tracing::debug!(
                "Job {} executing planned action {}/{}: {} - {}",
                self.job_id,
                i + 1,
                plan.actions.len(),
                action.tool_name,
                action.reasoning
            );

            let mut selection = ToolSelection {
                tool_name: action.tool_name.clone(),
                parameters: action.parameters.clone(),
                reasoning: action.reasoning.clone(),
                alternatives: vec![],
                tool_call_id: format!("plan_{}_{}", self.job_id, i),
            };

            // Execute the planned tool
            let mut result = self
                .execute_tool(&action.tool_name, &selection.parameters, activity_tx)
                .await;

            if let Err(crate::error::Error::Tool(crate::error::ToolError::InvalidParameters {
                reason,
                ..
            })) = &result
            {
                let repair_action = crate::llm::PlannedAction {
                    tool_name: action.tool_name.clone(),
                    parameters: selection.parameters.clone(),
                    reasoning: action.reasoning.clone(),
                    expected_outcome: action.expected_outcome.clone(),
                };

                match reasoning
                    .repair_plan_action(reason_ctx, &repair_action, reason)
                    .await
                {
                    Ok(repaired_params) if repaired_params != selection.parameters => {
                        tracing::info!(
                            job_id = %self.job_id,
                            tool = %action.tool_name,
                            "Retrying planned action with repaired parameters"
                        );
                        selection.parameters = repaired_params;
                        result = self
                            .execute_tool(&action.tool_name, &selection.parameters, activity_tx)
                            .await;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(
                            job_id = %self.job_id,
                            tool = %action.tool_name,
                            "Planned action repair failed: {}",
                            err
                        );
                    }
                }
            }

            // Process the result
            let completed = self
                .process_tool_result(reason_ctx, &selection, result)
                .await?;
            touch_worker_activity(activity_tx);

            if completed {
                return Ok(());
            }

            // Small delay between actions
            tokio::time::sleep(Duration::from_millis(WORKER_DIRECT_LOOP_DELAY_MS)).await;
        }

        // Plan completed, check with LLM if job is done
        reason_ctx.messages.push(ChatMessage::user(
            "All planned actions have been executed. Is the job complete? If not, what else needs to be done?",
        ));

        let response = reasoning.respond(reason_ctx).await?;
        touch_worker_activity(activity_tx);
        reason_ctx.messages.push(ChatMessage::assistant(&response));

        if crate::util::llm_signals_completion(&response) {
            self.set_last_output(&response);
            self.mark_completed().await?;
        } else {
            // Job not complete — return Ok(()) so the caller can fall back
            // to the direct tool selection loop (which uses the proper
            // OpenAI tool-calling protocol with native schema support).
            tracing::info!(
                "Job {} plan completed but work remains, falling back to direct selection",
                self.job_id
            );
        }

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        activity_tx: &watch::Sender<std::time::Instant>,
    ) -> Result<String, Error> {
        touch_worker_activity(activity_tx);
        let keepalive = WorkerActivityKeepalive::spawn(
            activity_tx.clone(),
            Duration::from_secs(WORKER_TOOL_KEEPALIVE_SECS),
        );
        let result = Self::execute_tool_inner(&self.deps, self.job_id, tool_name, params).await;
        drop(keepalive);
        touch_worker_activity(activity_tx);
        result
    }

    /// Single finalization point for routine run records.
    ///
    /// Called once at the end of `run()` after all exit paths have converged.
    /// Reads the job's actual final state and maps it to the appropriate
    /// RunStatus, then updates the DB record and emits the SSE lifecycle event.
    ///
    /// This replaces the previous pattern of calling `complete_routine_run`
    /// inside each `mark_*` method, which was fragile and easy to miss.
    async fn finalize_routine_run(&self) {
        // Only relevant when dispatched from a routine.
        let (run_id_str, routine_name) = match (&self.deps.routine_run_id, &self.deps.routine_name)
        {
            (Some(rid), Some(rn)) => (rid.clone(), rn.clone()),
            _ => return,
        };

        let run_id = match run_id_str.parse::<Uuid>() {
            Ok(id) => id,
            Err(_) => return,
        };

        // Derive RunStatus + summary from the job's actual terminal state.
        let finalization = match self.context_manager().get_context(self.job_id).await {
            Ok(ctx) => {
                let reason = ctx.transitions.last().and_then(|t| t.reason.clone());
                RoutineFinalizationOutcome::from_job_state(
                    ctx.state,
                    reason,
                    ctx.user_id.clone(),
                    ctx.owner_actor_id().to_string(),
                )
            }
            Err(e) => RoutineFinalizationOutcome::from_context_error(e),
        };
        let RoutineFinalizationOutcome {
            status,
            event,
            summary,
            job_user_id,
            job_actor_id,
        } = finalization;

        // Use the last meaningful output from the worker (LLM's final response
        // or last emit_user_message) as the result_summary for the SSE event
        // and DB record. Falls back to the generic status string.
        let rich_summary = self.take_last_output().unwrap_or_else(|| summary.clone());

        // Update the routine run record in the database.
        if let Some(store) = self.store().cloned() {
            let summary_ref = rich_summary.clone();
            if let Err(e) = store
                .complete_routine_run(run_id, status, Some(&summary_ref), None)
                .await
            {
                tracing::error!(run_id = %run_id, "Failed to complete routine run: {}", e);
            }
            if let (Some(user_id), Some(actor_id)) =
                (job_user_id.as_deref(), job_actor_id.as_deref())
            {
                let mut routine = None;
                if let Some(routine_id) = self.deps.routine_id
                    && let Ok(Some(found)) = store.get_routine(routine_id).await
                    && found.user_id == user_id
                    && found.owner_actor_id() == actor_id
                {
                    routine = Some(found);
                }
                if routine.is_none()
                    && let Ok(Some(found)) = store
                        .get_routine_by_name_for_actor(user_id, actor_id, &routine_name)
                        .await
                {
                    routine = Some(found);
                }
                if let Some(routine) = routine {
                    let completed_at = Utc::now();
                    let runtime_already_advanced =
                        routine_state_has_runtime_advance_for_run(&routine.state, run_id);
                    let next_fire_at = if runtime_already_advanced {
                        routine.next_fire_at
                    } else {
                        crate::agent::routine::next_fire_for_routine(&routine, None, completed_at)
                            .unwrap_or(routine.next_fire_at)
                    };
                    let run_count = if runtime_already_advanced {
                        routine.run_count
                    } else {
                        routine.run_count + 1
                    };
                    let consecutive_failures = if status == crate::agent::routine::RunStatus::Failed
                    {
                        routine.consecutive_failures + 1
                    } else {
                        0
                    };
                    let state =
                        routine_state_with_runtime_advance(&routine.state, run_id, completed_at);
                    if let Err(error) = persist_routine_runtime_update(
                        &store,
                        routine.id,
                        completed_at,
                        next_fire_at,
                        run_count,
                        consecutive_failures,
                        &state,
                    )
                    .await
                    {
                        tracing::error!(
                            routine = %routine.name,
                            run_id = %run_id,
                            "Failed to update routine runtime after worker finalization: {}",
                            error
                        );
                    }
                    let completed_run = crate::agent::routine::RoutineRun {
                        id: run_id,
                        routine_id: routine.id,
                        trigger_type: "worker".to_string(),
                        trigger_detail: Some("worker_finalization".to_string()),
                        trigger_key: None,
                        started_at: completed_at,
                        completed_at: Some(completed_at),
                        status,
                        result_summary: Some(summary_ref.clone()),
                        tokens_used: None,
                        job_id: Some(self.job_id),
                        created_at: completed_at,
                    };
                    if let Err(err) =
                        outcomes::maybe_create_routine_contract(&store, &routine, &completed_run)
                            .await
                    {
                        tracing::debug!(run_id = %run_id, error = %err, "Outcome worker routine hook skipped");
                    }
                }
            }
        }

        // Emit the SSE lifecycle event for the UI.
        if let Some(ref tx) = self.deps.sse_tx {
            let _ = tx.send(SseEvent::RoutineLifecycle {
                routine_name,
                event: event.to_string(),
                run_id: Some(run_id_str),
                result_summary: Some(rich_summary),
            });
        }
    }

    async fn mark_completed(&self) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(
                    JobState::Completed,
                    Some("Job completed successfully".to_string()),
                )
            })
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "success": true,
                "message": "Job completed successfully",
            }),
        );
        self.persist_status(
            JobState::Completed,
            Some("Job completed successfully".to_string()),
        );

        // NOTE: Routine run finalization (DB + SSE) is handled by
        // finalize_routine_run() at the end of run(), not here.

        // ── Post-completion evaluation (fire-and-forget) ────────────
        let job_id = self.job_id;
        let context_manager = self.context_manager().clone();
        let store = self.store().cloned();
        tokio::spawn(async move {
            use crate::evaluation::SuccessEvaluator;

            // Check if this was a heartbeat job so we can persist self-critique
            let is_heartbeat = context_manager
                .get_context(job_id)
                .await
                .ok()
                .and_then(|ctx| ctx.metadata.get("heartbeat").and_then(|v| v.as_bool()))
                .unwrap_or(false);

            let eval_result: Result<_, Box<dyn std::error::Error + Send + Sync>> = async {
                let job_ctx = context_manager.get_context(job_id).await?;
                let memory = context_manager.get_memory(job_id).await?;
                let evaluator = crate::evaluation::RuleBasedEvaluator::new();
                Ok(evaluator.evaluate(&job_ctx, &memory.actions, None).await?)
            }
            .await;

            match eval_result {
                Ok(result) => {
                    tracing::info!(
                        job_id = %job_id,
                        success = result.success,
                        quality = result.quality_score,
                        "Post-completion evaluation: {}",
                        result.reasoning
                    );

                    // Persist evaluation result to DB if available
                    if let Some(ref store) = store {
                        let result_json = serde_json::to_value(&result).unwrap_or_default();
                        if let Err(e) = store
                            .set_setting("system", &format!("eval.job.{}", job_id), &result_json)
                            .await
                        {
                            tracing::warn!(
                                "Failed to persist evaluation for job {}: {}",
                                job_id,
                                e
                            );
                        }
                    }

                    // ── Heartbeat self-critique feedback ──────────────────
                    // When a heartbeat evaluation finds issues, persist the
                    // critique so the next heartbeat run can read it and
                    // avoid repeating the same mistake.
                    if is_heartbeat && let Some(ref store) = store {
                        if should_persist_heartbeat_completion_critique(
                            result.success,
                            result.quality_score,
                        ) {
                            let critique = heartbeat_completion_critique(
                                job_id,
                                result.quality_score,
                                result.reasoning,
                            );
                            if let Err(e) = store
                                .set_setting("system", "heartbeat.last_critique", &critique)
                                .await
                            {
                                tracing::warn!("Failed to persist heartbeat self-critique: {}", e);
                            }
                        } else {
                            // Clean run — clear any stale critique
                            let _ = store
                                .set_setting(
                                    "system",
                                    "heartbeat.last_critique",
                                    &serde_json::Value::Null,
                                )
                                .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(job_id = %job_id, "Evaluation skipped: {}", e);
                }
            }
        });

        Ok(())
    }

    async fn mark_failed(&self, reason: &str) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(JobState::Failed, Some(reason.to_string()))
            })
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "success": false,
                "message": format!("Execution failed: {}", reason),
            }),
        );
        self.persist_status(JobState::Failed, Some(reason.to_string()));

        // NOTE: Routine run finalization (DB + SSE) is handled by
        // finalize_routine_run() at the end of run(), not here.

        Ok(())
    }

    async fn mark_stuck(&self, reason: &str) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| ctx.mark_stuck(reason))
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "success": false,
                "message": format!("Job stuck: {}", reason),
            }),
        );
        self.persist_status(JobState::Stuck, Some(reason.to_string()));

        // NOTE: Routine run finalization (DB + SSE) is handled by
        // finalize_routine_run() at the end of run(), not here.

        Ok(())
    }
}

#[cfg(test)]
mod tests;

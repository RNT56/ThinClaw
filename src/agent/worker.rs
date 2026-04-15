//! Per-job worker execution.

use std::sync::Arc;
use std::time::Duration;

use crate::llm::cost_tracker::CostTracker;
use chrono::Utc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use uuid::Uuid;

use std::sync::Mutex as StdMutex;

use crate::agent::outcomes;
use crate::agent::scheduler::WorkerMessage;
use crate::agent::task::TaskOutput;
use crate::channels::web::types::SseEvent;
use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::Error;
use crate::hooks::HookRegistry;
use crate::llm::{
    ActionPlan, ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolSelection,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::tools::rate_limiter::RateLimitResult;
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
}

/// Worker that executes a single job.
pub struct Worker {
    job_id: Uuid,
    deps: WorkerDeps,
    /// Captures the last meaningful output from the worker for use in the
    /// finalization SSE event. Updated by emit_user_message interception
    /// and the LLM's final text response.
    last_output: StdMutex<Option<String>>,
}

/// Result of a tool execution with metadata for context building.
struct ToolExecResult {
    result: Result<String, Error>,
}

fn touch_worker_activity(activity_tx: &watch::Sender<std::time::Instant>) {
    let _ = activity_tx.send(std::time::Instant::now());
}

struct WorkerActivityKeepalive {
    cancel_tx: watch::Sender<bool>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl WorkerActivityKeepalive {
    fn spawn(activity_tx: watch::Sender<std::time::Instant>, interval: Duration) -> Self {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let join_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_rx.changed() => break,
                    _ = tokio::time::sleep(interval) => {
                        touch_worker_activity(&activity_tx);
                    }
                }
            }
        });
        Self {
            cancel_tx,
            join_handle,
        }
    }
}

impl Drop for WorkerActivityKeepalive {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        self.join_handle.abort();
    }
}

impl Worker {
    /// Create a new worker for a specific job.
    pub fn new(job_id: Uuid, deps: WorkerDeps) -> Self {
        Self {
            job_id,
            deps,
            last_output: StdMutex::new(None),
        }
    }

    /// Store the last meaningful output for finalization.
    fn set_last_output(&self, output: &str) {
        if let Ok(mut guard) = self.last_output.lock() {
            *guard = Some(output.to_string());
        }
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

    fn use_planning(&self) -> bool {
        self.deps.use_planning
    }

    /// Fire-and-forget persistence of job status.
    fn persist_status(&self, status: JobState, reason: Option<String>) {
        if let Some(store) = self.store() {
            let store = store.clone();
            let job_id = self.job_id;
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_job_status(job_id, status, reason.as_deref())
                    .await
                {
                    tracing::warn!("Failed to persist status for job {}: {}", job_id, e);
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
        let mut reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        if let Some(ref prompt) = identity_block {
            reasoning = reasoning.with_system_prompt(prompt.clone());
        }
        // Wire cost tracker so worker LLM calls appear in the Cost Dashboard
        if let Some(ref tracker) = self.deps.cost_tracker {
            reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
        }

        // Build initial reasoning context (tool definitions refreshed each iteration in execution_loop)
        let mut reason_ctx = ReasoningContext::new().with_job(&job_ctx.description);

        // Build system message with identity context
        let identity_section = identity_block
            .as_deref()
            .map(|id| format!("\n\n---\n\n{}", id))
            .unwrap_or_default();

        // Add system message
        reason_ctx.messages.push(ChatMessage::system(format!(
            r#"You are an autonomous agent working on a job.

Job: {}
Description: {}

You have access to tools to complete this job. Plan your approach and execute tools as needed.
You may request multiple tools at once if they can be executed in parallel.

IMPORTANT: Use `emit_user_message` to send your results and findings to the user. This is \
how you deliver output — the user sees these messages in real-time in their chat interface. \
Use it for interim progress updates (message_type: "progress") and for your final results \
(message_type: "interim_result"). Do NOT just write results to memory files — the user needs \
to see them directly.

You can also use the `canvas` tool to display rich structured content (tables, panels, etc.) \
in the user's UI.

Report when the job is complete or if you encounter issues you cannot resolve.{identity}"#,
            job_ctx.title,
            job_ctx.description,
            identity = identity_section
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
                    .map(|c| {
                        matches!(
                            c.state,
                            JobState::Completed
                                | JobState::Failed
                                | JobState::Stuck
                                | JobState::Cancelled
                        )
                    })
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
        const MAX_WORKER_ITERATIONS: usize = 500;
        let max_iterations = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .and_then(|ctx| ctx.metadata.get("max_iterations").and_then(|v| v.as_u64()))
            .unwrap_or(50) as usize;
        let max_iterations = max_iterations.min(MAX_WORKER_ITERATIONS);

        // Heartbeat jobs set { "heartbeat": true } in metadata. When the LLM
        // produces a text response (HEARTBEAT_OK or findings), the job is done
        // — don't loop looking for a generic completion phrase.
        let is_heartbeat = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .and_then(|ctx| ctx.metadata.get("heartbeat").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let capability_metadata = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .map(|ctx| ctx.metadata)
            .unwrap_or(serde_json::Value::Null);
        let allowed_tools =
            crate::tools::ToolRegistry::metadata_string_list(&capability_metadata, "allowed_tools");
        let allowed_skills = crate::tools::ToolRegistry::metadata_string_list(
            &capability_metadata,
            "allowed_skills",
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
            )
            .await;
        reason_ctx.available_tools = tool_policies.filter_tool_definitions_for_metadata(
            reason_ctx.available_tools.clone(),
            &capability_metadata,
        );

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
            self.execute_plan(rx, reasoning, reason_ctx, plan, activity_tx)
                .await?;

            // Check whether the job reached a terminal state.
            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && matches!(
                    ctx.state,
                    JobState::Completed | JobState::Failed | JobState::Stuck | JobState::Cancelled
                )
            {
                return Ok(());
            }

            // Heartbeat jobs: if the plan already called emit_user_message,
            // the findings have been delivered — skip the expensive fallback
            // to direct tool selection.
            if is_heartbeat && self.last_output.lock().ok().is_some_and(|g| g.is_some()) {
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
            if iteration > max_iterations {
                if is_heartbeat {
                    // ── Heartbeat-specific stuck handling ─────────────────
                    // Build a descriptive message about what the heartbeat
                    // was trying to do when it ran out of iterations.
                    let stuck_reason = format!(
                        "Heartbeat ran out of iterations ({}/{}) before completing all checklist actions. \
                         The agent may need a higher max_iterations setting, or the checklist \
                         may contain tasks too complex for a single heartbeat run.",
                        max_iterations, max_iterations
                    );

                    // Set last_output so the notification includes useful context.
                    // This flows through finalize_routine_run → send_notification
                    // with on_failure: true, so the user sees this message.
                    self.set_last_output(&format!(
                        "⚠️ Heartbeat incomplete — ran out of tool iterations ({}/{}). \
                         Some checklist actions may not have been completed. \
                         You can increase the iteration budget in Settings → Heartbeat → Max iterations, \
                         or help me finish by prompting me directly.",
                        max_iterations, max_iterations
                    ));

                    // Write self-critique so the next heartbeat knows what
                    // happened and can try to finish the work.
                    if let Some(store) = self.store() {
                        let critique = serde_json::json!({
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "job_id": self.job_id.to_string(),
                            "quality": 0,
                            "reasoning": format!(
                                "Heartbeat exhausted all {} iterations without completing. \
                                 Partial work may have been saved. Pick up where the previous \
                                 run left off — check MEMORY.md and daily logs for what was \
                                 already done, then continue.",
                                max_iterations
                            ),
                        });
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
                )
                .await;
            reason_ctx.available_tools = tool_policies.filter_tool_definitions_for_metadata(
                reason_ctx.available_tools.clone(),
                &capability_metadata,
            );

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
                        if iteration > 8 && iteration % 10 == 0 {
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
                        for (selection, result) in selections.iter().zip(results) {
                            self.process_tool_result(reason_ctx, selection, result.result)
                                .await?;
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

                self.process_tool_result(reason_ctx, selection, result)
                    .await?;
            } else {
                // Multiple tools: execute in parallel
                tracing::debug!(
                    "Job {} executing {} tools in parallel",
                    self.job_id,
                    selections.len()
                );

                let results = self.execute_tools_parallel(&selections, activity_tx).await;

                // Process all results
                for (selection, result) in selections.iter().zip(results) {
                    self.process_tool_result(reason_ctx, selection, result.result)
                        .await?;
                }
            }

            // Heartbeat jobs: once emit_user_message has delivered findings,
            // the job is done. The LLM often stays in tool-call mode (never
            // producing a bare Text response), so we catch completion here
            // after each tool execution round.
            if is_heartbeat && self.last_output.lock().ok().is_some_and(|g| g.is_some()) {
                self.mark_completed().await?;
                return Ok(());
            }

            // Small delay between iterations
            tokio::time::sleep(Duration::from_millis(100)).await;
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

        let keepalive =
            WorkerActivityKeepalive::spawn(activity_tx.clone(), Duration::from_secs(15));
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

        // Collect and reorder by original index
        let mut results: Vec<Option<ToolExecResult>> = (0..count).map(|_| None).collect();
        let mut panicked_reasons: Vec<Option<String>> = (0..count).map(|_| None).collect();
        while let Some(join_result) = join_set.join_next().await {
            touch_worker_activity(activity_tx);
            match join_result {
                Ok((idx, exec_result)) => results[idx] = Some(exec_result),
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
                    // We don't know which index panicked, so store for first empty slot
                    if let Some(slot) = panicked_reasons.iter_mut().find(|s| s.is_none()) {
                        *slot = Some(reason);
                    }
                }
            }
        }

        // Fill any panicked/missing slots with error results
        let mut panic_iter = panicked_reasons.into_iter().flatten();
        let ordered = results
            .into_iter()
            .enumerate()
            .map(|(i, opt)| {
                opt.unwrap_or_else(|| {
                    let reason = panic_iter
                        .next()
                        .unwrap_or_else(|| "Task failed during execution".to_string());
                    ToolExecResult {
                        result: Err(crate::error::ToolError::ExecutionFailed {
                            name: selections[i].tool_name.clone(),
                            reason,
                        }
                        .into()),
                    }
                })
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
        let tool =
            deps.tools
                .get(tool_name)
                .await
                .ok_or_else(|| crate::error::ToolError::NotFound {
                    name: tool_name.to_string(),
                })?;

        // In autonomous workers (routines), `UnlessAutoApproved` tools are
        // auto-approved — there is no human to prompt.  Only block tools
        // that unconditionally require explicit approval (`Always`).
        if tool.requires_approval(params) == crate::tools::ApprovalRequirement::Always {
            return Err(crate::error::ToolError::AuthRequired {
                name: tool_name.to_string(),
            }
            .into());
        }

        // Fetch job context early so we have the real user_id for hooks and rate limiting
        let job_ctx = deps.context_manager.get_context(job_id).await?;
        let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();
        if let Some(reason) = tool_policies.denial_reason_for_metadata(tool_name, &job_ctx.metadata)
        {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason,
            }
            .into());
        }
        if !crate::tools::ToolRegistry::tool_name_allowed_by_metadata(&job_ctx.metadata, tool_name)
        {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Tool is not permitted in this agent context".to_string(),
            }
            .into());
        }

        // Check per-tool rate limit before running hooks or executing (cheaper check first)
        if let Some(config) = tool.rate_limit_config()
            && let RateLimitResult::Limited { retry_after, .. } = deps
                .tools
                .rate_limiter()
                .check_and_record(&job_ctx.user_id, tool_name, &config)
                .await
        {
            return Err(crate::error::ToolError::RateLimited {
                name: tool_name.to_string(),
                retry_after: Some(retry_after),
            }
            .into());
        }

        // Run BeforeToolCall hook
        let params = {
            use crate::hooks::{HookError, HookEvent, HookOutcome};
            let event = HookEvent::ToolCall {
                tool_name: tool_name.to_string(),
                parameters: params.clone(),
                user_id: job_ctx.user_id.clone(),
                context: format!("job:{}", job_id),
            };
            match deps.hooks.run(&event).await {
                Err(HookError::Rejected { reason }) => {
                    return Err(crate::error::ToolError::ExecutionFailed {
                        name: tool_name.to_string(),
                        reason: format!("Blocked by hook: {}", reason),
                    }
                    .into());
                }
                Err(err) => {
                    return Err(crate::error::ToolError::ExecutionFailed {
                        name: tool_name.to_string(),
                        reason: format!("Blocked by hook failure mode: {}", err),
                    }
                    .into());
                }
                Ok(HookOutcome::Continue {
                    modified: Some(new_params),
                }) => serde_json::from_str(&new_params).unwrap_or_else(|e| {
                    tracing::warn!(
                        tool = %tool_name,
                        "Hook returned non-JSON modification for ToolCall, ignoring: {}",
                        e
                    );
                    params.clone()
                }),
                _ => params.clone(),
            }
        };
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        // Validate tool parameters
        let validation = deps.safety.validator().validate_tool_params(&params);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(crate::error::ToolError::InvalidParameters {
                name: tool_name.to_string(),
                reason: format!("Invalid tool parameters: {}", details),
            }
            .into());
        }

        tracing::debug!(
            tool = %tool_name,
            params = %params,
            job = %job_id,
            "Tool call started"
        );

        // Execute with per-tool timeout and timing
        let tool_timeout = tool.execution_timeout();
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(tool_timeout, async {
            tool.execute(params.clone(), &job_ctx).await
        })
        .await;
        let elapsed = start.elapsed();

        match &result {
            Ok(Ok(output)) => {
                let result_str = serde_json::to_string(&output.result)
                    .unwrap_or_else(|_| "<serialize error>".to_string());
                tracing::debug!(
                    tool = %tool_name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    result = %result_str,
                    "Tool call succeeded"
                );
            }
            Ok(Err(e)) => {
                tracing::debug!(
                    tool = %tool_name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    error = %e,
                    "Tool call failed"
                );
            }
            Err(_) => {
                tracing::debug!(
                    tool = %tool_name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    timeout_secs = tool_timeout.as_secs(),
                    "Tool call timed out"
                );
            }
        }

        // Record action in memory and get the ActionRecord for persistence
        let action = match &result {
            Ok(Ok(output)) => {
                let output_str = serde_json::to_string_pretty(&output.result)
                    .ok()
                    .map(|s| deps.safety.sanitize_tool_output(tool_name, &s).content);
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem.create_action(tool_name, params.clone()).succeed(
                            output_str.clone(),
                            output.result.clone(),
                            elapsed,
                        );
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
                }
            }
            Ok(Err(e)) => {
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem
                            .create_action(tool_name, params.clone())
                            .fail(e.to_string(), elapsed);
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
                }
            }
            Err(_) => {
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem
                            .create_action(tool_name, params.clone())
                            .fail("Execution timeout", elapsed);
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
                }
            }
        };

        // Persist action to database (fire-and-forget)
        if let (Some(action), Some(store)) = (action, deps.store.clone()) {
            tokio::spawn(async move {
                if let Err(e) = store.save_action(job_id, &action).await {
                    tracing::warn!("Failed to persist action for job {}: {}", job_id, e);
                }
            });
        }

        // Handle the result
        let output = result
            .map_err(|_| crate::error::ToolError::Timeout {
                name: tool_name.to_string(),
                timeout: tool_timeout,
            })?
            .map_err(|e| crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: e.to_string(),
            })?;

        // Return result as string
        serde_json::to_string_pretty(&output.result).map_err(|e| {
            crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: format!("Failed to serialize result: {}", e),
            }
            .into()
        })
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
                        self.set_last_output(msg);
                        self.emit_sse(SseEvent::RoutineLifecycle {
                            routine_name: self
                                .deps
                                .routine_name
                                .clone()
                                .unwrap_or_else(|| "job".to_string()),
                            event: "message".to_string(),
                            run_id: self.deps.routine_run_id.clone(),
                            result_summary: Some(format!("[{}] {}", msg_type, msg)),
                        });
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

                // Tool output never drives job completion. A malicious tool could
                // emit "TASK_COMPLETE" to force premature completion. Only the LLM's
                // own structured response (in execution_loop) can mark a job done.
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
            tracing::debug!(
                "Job {} executing planned action {}/{}: {} - {}",
                self.job_id,
                i + 1,
                plan.actions.len(),
                action.tool_name,
                action.reasoning
            );

            // Execute the planned tool
            let result = self
                .execute_tool(&action.tool_name, &action.parameters, activity_tx)
                .await;

            // Create a synthetic ToolSelection for process_tool_result.
            // Plan actions don't originate from an LLM tool_call response so
            // there is no real tool_call_id; generate a unique one.
            let selection = ToolSelection {
                tool_name: action.tool_name.clone(),
                parameters: action.parameters.clone(),
                reasoning: action.reasoning.clone(),
                alternatives: vec![],
                tool_call_id: format!("plan_{}_{}", self.job_id, i),
            };

            // Process the result
            let completed = self
                .process_tool_result(reason_ctx, &selection, result)
                .await?;
            touch_worker_activity(activity_tx);

            if completed {
                return Ok(());
            }

            // Small delay between actions
            tokio::time::sleep(Duration::from_millis(100)).await;
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
        let keepalive =
            WorkerActivityKeepalive::spawn(activity_tx.clone(), Duration::from_secs(15));
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
        let (status, event, summary, job_user_id, job_actor_id) =
            match self.context_manager().get_context(self.job_id).await {
                Ok(ctx) => {
                    // Extract the reason from the last state transition (if any).
                    let reason = ctx.transitions.last().and_then(|t| t.reason.clone());
                    match ctx.state {
                        JobState::Completed => (
                            crate::agent::routine::RunStatus::Ok,
                            "completed",
                            "Job completed successfully".to_string(),
                            Some(ctx.user_id.clone()),
                            Some(ctx.owner_actor_id().to_string()),
                        ),
                        JobState::Failed => (
                            crate::agent::routine::RunStatus::Failed,
                            "failed",
                            reason.unwrap_or_else(|| "Job failed".to_string()),
                            Some(ctx.user_id.clone()),
                            Some(ctx.owner_actor_id().to_string()),
                        ),
                        JobState::Stuck => (
                            crate::agent::routine::RunStatus::Failed,
                            "failed",
                            reason.unwrap_or_else(|| "Job stuck".to_string()),
                            Some(ctx.user_id.clone()),
                            Some(ctx.owner_actor_id().to_string()),
                        ),
                        JobState::Cancelled => (
                            crate::agent::routine::RunStatus::Failed,
                            "failed",
                            "Job cancelled".to_string(),
                            Some(ctx.user_id.clone()),
                            Some(ctx.owner_actor_id().to_string()),
                        ),
                        other => (
                            crate::agent::routine::RunStatus::Failed,
                            "failed",
                            format!("Job ended in unexpected state: {:?}", other),
                            Some(ctx.user_id.clone()),
                            Some(ctx.owner_actor_id().to_string()),
                        ),
                    }
                }
                Err(e) => (
                    crate::agent::routine::RunStatus::Failed,
                    "failed",
                    format!("Could not read final job state: {}", e),
                    None,
                    None,
                ),
            };

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
                && let Ok(Some(routine)) = store
                    .get_routine_by_name_for_actor(user_id, actor_id, &routine_name)
                    .await
            {
                let completed_run = crate::agent::routine::RoutineRun {
                    id: run_id,
                    routine_id: routine.id,
                    trigger_type: "worker".to_string(),
                    trigger_detail: Some("worker_finalization".to_string()),
                    started_at: Utc::now(),
                    completed_at: Some(Utc::now()),
                    status,
                    result_summary: Some(summary_ref.clone()),
                    tokens_used: None,
                    job_id: Some(self.job_id),
                    created_at: Utc::now(),
                };
                if let Err(err) =
                    outcomes::maybe_create_routine_contract(&store, &routine, &completed_run).await
                {
                    tracing::debug!(run_id = %run_id, error = %err, "Outcome worker routine hook skipped");
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
                        if !result.success || result.quality_score < 100 {
                            let critique = serde_json::json!({
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                                "job_id": job_id.to_string(),
                                "quality": result.quality_score,
                                "reasoning": result.reasoning,
                            });
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

/// Convert a TaskOutput to a string result for tool execution.
impl From<TaskOutput> for Result<String, Error> {
    fn from(output: TaskOutput) -> Self {
        serde_json::to_string_pretty(&output.result).map_err(|e| {
            crate::error::ToolError::ExecutionFailed {
                name: "task".to_string(),
                reason: format!("Failed to serialize result: {}", e),
            }
            .into()
        })
    }
}

/// Compact context messages after plan execution to prevent orphaned tool_result bloat.
///
/// Keeps:
/// - All System messages (system prompt, instructions)
/// - The first User message (the original task)
/// - A synthetic assistant summary of the plan
///
/// Strips:
/// - Plan-era tool_result messages (with synthetic `plan_*` IDs)
/// - Plan-era assistant messages with tool_calls
/// - Intermediate user messages from orphan rewrites
fn compact_post_plan(messages: &mut Vec<ChatMessage>, plan_goal: &str) {
    use crate::llm::Role;

    let pre_count = messages.len();
    let pre_chars: usize = messages.iter().map(|m| m.estimated_chars()).sum();

    let mut compacted = Vec::new();
    let mut first_user_seen = false;

    for msg in messages.iter() {
        match msg.role {
            Role::System => {
                compacted.push(msg.clone());
            }
            Role::User if !first_user_seen => {
                compacted.push(msg.clone());
                first_user_seen = true;
            }
            _ => {} // Skip all plan-era messages
        }
    }

    // Add a summary note about the completed plan
    compacted.push(ChatMessage::assistant(format!(
        "I executed a plan to accomplish: {}. \
         The plan has been completed. Now I'll check for any remaining work \
         or deliver final results.",
        plan_goal,
    )));

    let post_chars: usize = compacted.iter().map(|m| m.estimated_chars()).sum();
    tracing::info!(
        "Post-plan compaction: {} messages ({} chars) → {} messages ({} chars)",
        pre_count,
        pre_chars,
        compacted.len(),
        post_chars
    );

    *messages = compacted;
}

#[cfg(test)]
mod tests {
    use crate::llm::ToolSelection;
    use crate::util::llm_signals_completion;

    use super::*;
    use crate::config::SafetyConfig;
    use crate::context::JobContext;
    use crate::llm::{
        CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::{Tool, ToolError, ToolOutput};

    /// A test tool that sleeps for a configurable duration before returning.
    struct SlowTool {
        tool_name: String,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "Test tool with configurable delay"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let start = std::time::Instant::now();
            tokio::time::sleep(self.delay).await;
            Ok(ToolOutput::text(
                format!("done_{}", self.tool_name),
                start.elapsed(),
            ))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    /// Stub LLM provider (never called in these tests).
    struct StubLlm;

    #[async_trait::async_trait]
    impl LlmProvider for StubLlm {
        fn model_name(&self) -> &str {
            "stub"
        }
        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }
    }

    /// Build a Worker wired to a ToolRegistry containing the given tools.
    async fn make_worker(tools: Vec<Arc<dyn Tool>>) -> Worker {
        let registry = ToolRegistry::new();
        for t in tools {
            registry.register(t).await;
        }

        let cm = Arc::new(crate::context::ContextManager::new(5));
        let job_id = cm.create_job("test", "test job").await.unwrap();

        let deps = WorkerDeps {
            context_manager: cm,
            llm: Arc::new(StubLlm),
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
                redact_pii_in_prompts: true,
                smart_approval_mode: "off".to_string(),
                external_scanner_mode: "off".to_string(),
                external_scanner_path: None,
            })),
            tools: Arc::new(registry),
            store: None,
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            timeout: Duration::from_secs(30),
            use_planning: false,
            sse_tx: None,
            routine_name: None,
            routine_run_id: None,
            workspace: None,
            cost_tracker: None,
        };

        Worker::new(job_id, deps)
    }

    #[test]
    fn test_tool_selection_preserves_call_id() {
        let selection = ToolSelection {
            tool_name: "memory_search".to_string(),
            parameters: serde_json::json!({"query": "test"}),
            reasoning: "Need to search memory".to_string(),
            alternatives: vec![],
            tool_call_id: "call_abc123".to_string(),
        };

        assert_eq!(selection.tool_call_id, "call_abc123");
        assert_ne!(
            selection.tool_call_id, "tool_call_id",
            "tool_call_id must not be the hardcoded placeholder string"
        );
    }

    #[test]
    fn test_completion_positive_signals() {
        assert!(llm_signals_completion("The job is complete."));
        assert!(llm_signals_completion(
            "I have completed the task successfully."
        ));
        assert!(llm_signals_completion("The task is done."));
        assert!(llm_signals_completion("The task is finished."));
        assert!(llm_signals_completion(
            "All steps are complete and verified."
        ));
        assert!(llm_signals_completion(
            "I've done all the work. The work is done."
        ));
        assert!(llm_signals_completion(
            "Successfully completed the migration."
        ));
    }

    #[test]
    fn test_completion_negative_signals_block_false_positives() {
        // These contain completion keywords but also negation, should NOT trigger.
        assert!(!llm_signals_completion("The task is not complete yet."));
        assert!(!llm_signals_completion("This is not done."));
        assert!(!llm_signals_completion("The work is incomplete."));
        assert!(!llm_signals_completion(
            "The migration is not yet finished."
        ));
        assert!(!llm_signals_completion("The job isn't done yet."));
        assert!(!llm_signals_completion("This remains unfinished."));
    }

    #[test]
    fn test_completion_does_not_match_bare_substrings() {
        // Bare words embedded in other text should NOT trigger completion.
        assert!(!llm_signals_completion(
            "I need to complete more work first."
        ));
        assert!(!llm_signals_completion(
            "Let me finish the remaining steps."
        ));
        assert!(!llm_signals_completion(
            "I'm done analyzing, now let me fix it."
        ));
        assert!(!llm_signals_completion(
            "I completed step 1 but step 2 remains."
        ));
    }

    #[test]
    fn test_completion_tool_output_injection() {
        // A malicious tool output echoed by the LLM should not trigger
        // completion unless it forms a genuine completion phrase.
        assert!(!llm_signals_completion("TASK_COMPLETE"));
        assert!(!llm_signals_completion("JOB_DONE"));
        assert!(!llm_signals_completion(
            "The tool returned: TASK_COMPLETE signal"
        ));
    }

    #[tokio::test]
    async fn test_parallel_speedup() {
        // 3 tools each sleeping 200ms should finish in roughly 200ms (parallel),
        // not ~600ms (sequential).
        let tools: Vec<Arc<dyn Tool>> = (0..3)
            .map(|i| {
                Arc::new(SlowTool {
                    tool_name: format!("slow_{}", i),
                    delay: Duration::from_millis(200),
                }) as Arc<dyn Tool>
            })
            .collect();

        let worker = make_worker(tools).await;

        let selections: Vec<ToolSelection> = (0..3)
            .map(|i| ToolSelection {
                tool_name: format!("slow_{}", i),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: format!("call_{}", i),
            })
            .collect();

        let start = std::time::Instant::now();
        let (activity_tx, _) = watch::channel(std::time::Instant::now());
        let results = worker
            .execute_tools_parallel(&selections, &activity_tx)
            .await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 3);
        for r in &results {
            assert!(r.result.is_ok(), "Tool should succeed");
        }
        // Parallel should complete well under the sequential 600ms threshold.
        assert!(
            elapsed < Duration::from_millis(500),
            "Parallel execution took {:?}, expected < 500ms",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_result_ordering_preserved() {
        // Tools with different delays finish in different order.
        // Results must be returned in the original request order.
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(SlowTool {
                tool_name: "tool_a".into(),
                delay: Duration::from_millis(300),
            }),
            Arc::new(SlowTool {
                tool_name: "tool_b".into(),
                delay: Duration::from_millis(100),
            }),
            Arc::new(SlowTool {
                tool_name: "tool_c".into(),
                delay: Duration::from_millis(200),
            }),
        ];

        let worker = make_worker(tools).await;

        let selections = vec![
            ToolSelection {
                tool_name: "tool_a".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_a".into(),
            },
            ToolSelection {
                tool_name: "tool_b".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_b".into(),
            },
            ToolSelection {
                tool_name: "tool_c".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_c".into(),
            },
        ];

        let (activity_tx, _) = watch::channel(std::time::Instant::now());
        let results = worker
            .execute_tools_parallel(&selections, &activity_tx)
            .await;

        // Results must be in same order as selections, not completion order.
        assert!(results[0].result.as_ref().unwrap().contains("done_tool_a"));
        assert!(results[1].result.as_ref().unwrap().contains("done_tool_b"));
        assert!(results[2].result.as_ref().unwrap().contains("done_tool_c"));
    }

    #[tokio::test]
    async fn test_missing_tool_produces_error_not_panic() {
        // If a tool doesn't exist, the result slot should contain an error.
        let worker = make_worker(vec![]).await;

        let selections = vec![ToolSelection {
            tool_name: "nonexistent_tool".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_x".into(),
        }];

        let (activity_tx, _) = watch::channel(std::time::Instant::now());
        let results = worker
            .execute_tools_parallel(&selections, &activity_tx)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            results[0].result.is_err(),
            "Missing tool should produce an error, not a panic"
        );
    }
}

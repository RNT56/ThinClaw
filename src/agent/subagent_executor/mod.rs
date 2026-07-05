//! Sub-agent executor — lightweight in-process agentic loops.
//!
//! Spawns parallel sub-agents as tokio tasks, each with their own LLM context,
//! system prompt, and filtered tool set. Results are injected back into the
//! parent agent's message stream via an mpsc channel.
//!
//! ## Bidirectional communication
//!
//! Each sub-agent has a pair of channels for communicating with the parent:
//! - `to_parent_tx`: Sub-agent sends results/questions to the parent
//! - `from_parent_rx`: Sub-agent receives messages/answers from the parent
//!
//! ```text
//!   Main Agent ─────►  SubagentExecutor::spawn()
//!                          │
//!                          ├── tokio::spawn(mini_agentic_loop)
//!                          │       ├── LLM call with task prompt
//!                          │       ├── Tool calls (filtered set)
//!                          │       ├── emit_user_message → SubagentProgress event
//!                          │       └── Return final text → SubagentCompleted event
//!                          │
//!                          └── result_tx → parent agent re-inject
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thinclaw_agent::prompt_assembly::{
    SUBAGENT_AVAILABLE_SKILL_INSTRUCTION, render_skill_sections,
};
pub use thinclaw_agent::subagent::{
    DEFAULT_MAX_CONCURRENT, DEFAULT_TIMEOUT_SECS, SUBAGENT_MAX_ITERATIONS, SubagentConfig,
    SubagentInfo, SubagentResult, SubagentResultMessage, SubagentSpawnRequest, SubagentStatus,
};
use thinclaw_agent::subagent::{
    SUBAGENT_RUN_ORPHANED_REASON, SubagentCompletionOutcome, SubagentConcurrency,
    SubagentJobMetadataInput, SubagentLearningRiskTier, SubagentRunRecord, SubagentSpawnAdmission,
    SubagentSystemPromptSections, extract_subagent_message, llm_metadata_from_json,
    normalize_subagent_progress_category, render_subagent_system_prompt, resolve_parent_thread_id,
    should_cancel_subagent, should_emit_subagent_heartbeat, should_force_subagent_text,
    should_reinject_subagent_result, subagent_activity_category, subagent_allows_skill,
    subagent_cancelled_status, subagent_completion_status_response, subagent_default_system_prompt,
    subagent_execution_grants, subagent_heartbeat_message, subagent_identity_defaults,
    subagent_iteration_limit_reason, subagent_job_metadata, subagent_learning_completion,
    subagent_learning_risk_tier, subagent_parent_message, subagent_result_from_completion,
    subagent_routine_actor, subagent_routine_completion, subagent_run_status_for_completion,
    subagent_spawned_response, subagent_status_from_result, subagent_tool_activity_message,
    subagent_tool_warning_message, subagent_warning_category, with_subagent_thread_metadata,
};
pub use thinclaw_types::{
    SubagentMemoryMode, SubagentProvidedContext, SubagentSkillMode, SubagentTaskPacket,
    SubagentToolMode,
};
use tokio::sync::{RwLock, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::learning::{
    ImprovementClass, LearningEvent as RuntimeLearningEvent, LearningOrchestrator, RiskTier,
};
use crate::agent::routine::{
    routine_state_has_runtime_advance_for_run, routine_state_with_runtime_advance,
};
use crate::agent::routine_engine::persist_routine_runtime_update;
use crate::channels::web::types::SseEvent;
use crate::channels::{ChannelManager, StatusUpdate};
use crate::config::SkillsConfig;
use crate::context::JobContext;
use crate::error::Error;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult};
use crate::safety::SafetyLayer;
use crate::skills::{LoadedSkill, SkillRegistry, prefilter_skills};
use crate::tools::{ToolExecutionLane, ToolProfile, ToolRegistry, execution};
use crate::workspace::Workspace;

const SUBAGENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// How long a cancelled sub-agent gets to exit cooperatively (and run its
/// finalization) before the task is hard-aborted.
const SUBAGENT_CANCEL_GRACE: Duration = Duration::from_secs(20);
/// How long finished sub-agent handles stay visible in list() before being
/// evicted from the active map.
const SUBAGENT_HANDLE_RETENTION: Duration = Duration::from_secs(3600);

/// Shared heartbeat task for a running sub-agent.
struct SubagentHeartbeat {
    cancel_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

impl SubagentHeartbeat {
    fn spawn(
        channels: Arc<ChannelManager>,
        channel_name: String,
        channel_metadata: serde_json::Value,
        agent_id: String,
        agent_name: String,
        activity_tx: watch::Sender<Instant>,
        mut activity_rx: watch::Receiver<Instant>,
        cancel_tx: watch::Sender<bool>,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Self {
        let join_handle = tokio::spawn(async move {
            let mut last_activity = *activity_rx.borrow();

            loop {
                let sleep_for = SUBAGENT_HEARTBEAT_INTERVAL.saturating_sub(last_activity.elapsed());

                tokio::select! {
                    _ = cancel_rx.changed() => break,
                    changed = activity_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        last_activity = *activity_rx.borrow();
                    }
                    _ = tokio::time::sleep(sleep_for) => {
                        if !should_emit_subagent_heartbeat(
                            last_activity.elapsed(),
                            SUBAGENT_HEARTBEAT_INTERVAL,
                        ) {
                            continue;
                        }

                        let _ = channels
                            .send_status(
                                &channel_name,
                                StatusUpdate::SubagentProgress {
                                    agent_id: agent_id.clone(),
                                    message: subagent_heartbeat_message(&agent_name),
                                    category: subagent_activity_category().to_string(),
                                },
                                &channel_metadata,
                            )
                            .await;

                        last_activity = Instant::now();
                        let _ = activity_tx.send(last_activity);
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

impl Drop for SubagentHeartbeat {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        self.join_handle.abort();
    }
}

fn touch_subagent_activity(activity_tx: &watch::Sender<Instant>) {
    let _ = activity_tx.send(Instant::now());
}

/// Handle to a running sub-agent.
pub struct SubagentHandle {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: chrono::DateTime<chrono::Utc>,
    /// When the subagent reached a terminal status. Retention eviction
    /// measures from here, not from `spawned_at`, so a long-running
    /// subagent's result stays visible for the full retention window after
    /// it finishes.
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    cancel_tx: watch::Sender<bool>,
    join_handle: Option<JoinHandle<SubagentResult>>,
    /// Send messages from the parent to this sub-agent.
    pub parent_to_sub_tx: mpsc::Sender<String>,
    /// Principal that spawned this sub-agent, used for per-principal
    /// concurrency accounting so one principal cannot starve others.
    pub parent_user_id: String,
}

/// The sub-agent executor manages sub-agent lifecycle.
#[derive(Clone)]
pub struct SubagentExecutor {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    channels: Arc<ChannelManager>,
    config: SubagentConfig,
    /// Currently active sub-agents.
    active: Arc<RwLock<HashMap<Uuid, SubagentHandle>>>,
    /// Sender for injecting sub-agent results back to the parent agent.
    result_tx: mpsc::Sender<SubagentResultMessage>,
    /// Optional database for finalizing routine runs.
    store: Option<Arc<dyn crate::db::Database>>,
    /// Optional SSE broadcast sender for routine lifecycle events.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Optional shared cost tracker for sub-agent LLM calls.
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    /// Optional shared cost guard enforcing daily-budget/hourly-rate limits.
    ///
    /// This is the SAME guard instance that gates the main dispatcher loop, so
    /// delegated sub-agent work draws from the same budget rather than being
    /// an unmetered side channel. Checked before every sub-agent LLM iteration.
    cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
    /// Optional workspace for loading identity/system prompt context.
    workspace: Option<Arc<Workspace>>,
    /// Optional skill registry for skill discovery in sub-agents.
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
    /// Optional skills config for deterministic skill prefiltering.
    skills_config: Option<SkillsConfig>,
}

impl SubagentExecutor {
    /// Create a new sub-agent executor.
    ///
    /// Returns `(executor, result_rx)` — the receiver should be polled by a
    /// background task that re-injects sub-agent results into the main agent.
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        channels: Arc<ChannelManager>,
        config: SubagentConfig,
    ) -> (Self, mpsc::Receiver<SubagentResultMessage>) {
        let (result_tx, result_rx) = mpsc::channel(32);
        let executor = Self {
            llm,
            safety,
            tools,
            channels,
            config,
            active: Arc::new(RwLock::new(HashMap::new())),
            result_tx,
            store: None,
            sse_tx: None,
            cost_tracker: None,
            cost_guard: None,
            workspace: None,
            skill_registry: None,
            skills_config: None,
        };
        (executor, result_rx)
    }

    /// Set the database for routine run finalization.
    pub fn with_store(mut self, store: Arc<dyn crate::db::Database>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the SSE broadcast sender for routine lifecycle events.
    pub fn with_sse_tx(mut self, tx: tokio::sync::broadcast::Sender<SseEvent>) -> Self {
        self.sse_tx = Some(tx);
        self
    }

    /// Set the cost tracker so sub-agent LLM calls are visible in the Cost Dashboard.
    pub fn with_cost_tracker(
        mut self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// Set the cost guard so sub-agent iterations are gated by the same
    /// daily-budget/hourly-rate limits as the main dispatcher loop.
    pub fn with_cost_guard(mut self, guard: Arc<crate::agent::cost_guard::CostGuard>) -> Self {
        self.cost_guard = Some(guard);
        self
    }

    /// Set the workspace so sub-agents can inherit identity/system prompt files.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Set the skill registry/config so sub-agents can discover skills.
    pub fn with_skill_registry(
        mut self,
        skill_registry: Arc<RwLock<SkillRegistry>>,
        skills_config: SkillsConfig,
    ) -> Self {
        self.skill_registry = Some(skill_registry);
        self.skills_config = Some(skills_config);
        self
    }

    /// Return the current autonomous tool names available to sub-agents.
    pub async fn autonomous_tool_names(&self) -> Vec<String> {
        let mut names = self
            .tools
            .tool_definitions_for_autonomous()
            .await
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    /// Return the currently loaded skill names available to sub-agents.
    pub async fn available_skill_names(&self) -> Vec<String> {
        let Some(skill_registry) = self.skill_registry.as_ref() else {
            return Vec::new();
        };
        let guard = skill_registry.read().await;
        let mut names = guard
            .skills()
            .iter()
            .map(|skill| skill.manifest.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    /// Spawn a sub-agent.
    ///
    /// When `request.wait` is true, waits for the sub-agent to finish and
    /// returns the final result inline. Otherwise returns immediately and
    /// re-injects the result through the `result_tx` channel on completion.
    pub async fn spawn(
        &self,
        mut request: SubagentSpawnRequest,
        channel_name: &str,
        channel_metadata: &serde_json::Value,
        parent_user_id: &str,
        parent_identity: Option<&ResolvedIdentity>,
        parent_thread_id: Option<&str>,
    ) -> Result<SubagentResult, Error> {
        request.normalize_strict(None, None, self.config.default_tool_profile);
        let id = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let heartbeat_cancel_tx = cancel_tx.clone();
        let (completion_tx, completion_rx) = oneshot::channel::<SubagentResult>();
        // Bidirectional: parent → sub-agent message channel
        let (parent_to_sub_tx, parent_to_sub_rx) = mpsc::channel::<String>(16);

        // Check concurrency limit AND insert tracking entry under a single
        // write lock to prevent TOCTOU races (Bug 37 fix).
        let spawned_at = chrono::Utc::now();
        {
            let mut active = self.active.write().await;
            // Evict aged terminal handles here — no background task calls
            // cleanup(), so without this the map grows for the process
            // lifetime, one entry per subagent ever spawned.
            let cutoff = chrono::Utc::now()
                - chrono::Duration::from_std(SUBAGENT_HANDLE_RETENTION).unwrap_or_default();
            active.retain(|_, h| {
                h.status == SubagentStatus::Running
                    || h.completed_at.unwrap_or(h.spawned_at) > cutoff
            });
            let running = active
                .values()
                .filter(|h| h.status == SubagentStatus::Running)
                .count();
            // Per-principal running count so a single principal's burst cannot
            // consume the whole global pool on a shared/multi-user gateway.
            let principal_running = active
                .values()
                .filter(|h| {
                    h.status == SubagentStatus::Running && h.parent_user_id == parent_user_id
                })
                .count();
            let concurrency = SubagentConcurrency::with_principal_scope(
                running,
                self.config.max_concurrent,
                principal_running,
                self.config.max_per_principal,
            );
            if let SubagentSpawnAdmission::Rejected { reason } = concurrency.admission() {
                return Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                    name: "spawn_subagent".to_string(),
                    reason,
                }));
            }

            // Insert tracking entry BEFORE spawning so fast-completing agents
            // don't become zombies (Bug 38 fix). JoinHandle added after spawn.
            active.insert(
                id,
                SubagentHandle {
                    id,
                    name: request.name.clone(),
                    task: request.task.clone(),
                    status: SubagentStatus::Running,
                    spawned_at,
                    completed_at: None,
                    cancel_tx: cancel_tx.clone(),
                    join_handle: None, // filled in after tokio::spawn
                    parent_to_sub_tx: parent_to_sub_tx.clone(),
                    parent_user_id: parent_user_id.to_string(),
                },
            );
        }

        // ── Durable ledger: record the run BEFORE spawning ──────────────
        // Without this, a running sub-agent lives only in the in-memory
        // `active` map above, so a process restart silently drops it and
        // leaks any routine run it was finalizing. Best-effort: a ledger
        // write failure does not block the sub-agent from running — the
        // in-memory tracking above is still authoritative for this process.
        let resolved_parent_thread_id =
            resolve_parent_thread_id(parent_thread_id, channel_metadata);
        let routine_run_id_for_ledger = channel_metadata
            .get("routine_run_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(ref store) = self.store {
            let run = SubagentRunRecord::new_running(
                id,
                request.name.clone(),
                request.task.clone(),
                Some(resolved_parent_thread_id.clone()),
                routine_run_id_for_ledger.clone(),
                spawned_at,
            );
            if let Err(e) = store.insert_subagent_run(&run).await {
                tracing::warn!(
                    agent_id = %id,
                    "Failed to persist subagent run ledger entry: {}", e
                );
            }
        }

        let timeout = Duration::from_secs(
            request
                .timeout_secs
                .unwrap_or(self.config.default_timeout_secs),
        );
        let max_iterations = self.config.max_tool_iterations;
        let wait_for_completion = request.wait;

        // Build system prompt
        let system_prompt = request
            .system_prompt
            .clone()
            .unwrap_or_else(|| subagent_default_system_prompt(&request.name));

        let name = request.name.clone();
        let task = request.task.clone();
        let task_packet = request.task_packet();
        let memory_mode = request.memory_mode.clone().unwrap_or_default();
        let tool_mode = request.tool_mode.clone().unwrap_or_default();
        let skill_mode = request.skill_mode.clone().unwrap_or_default();
        let tool_profile = request
            .tool_profile
            .unwrap_or(self.config.default_tool_profile);

        // Clone shared deps for the spawned task
        let llm = if let Some(model_spec) = request.model.as_ref() {
            crate::tools::builtin::llm_tools::wrap_model_spec_override(
                self.llm.clone(),
                model_spec.clone(),
            )
        } else {
            self.llm.clone()
        };
        let safety = self.safety.clone();
        let tools = self.tools.clone();
        let channels = self.channels.clone();
        let ch_name = channel_name.to_string();
        let grants = subagent_execution_grants(
            request.allowed_tools.as_deref(),
            request.allowed_skills.as_deref(),
            &memory_mode,
            &tool_mode,
            &skill_mode,
        );
        let allowed = grants.allowed_tools.clone();
        let allowed_skills = grants.allowed_skills.clone();
        let result_tx = self.result_tx.clone();
        let principal_id = request.principal_id.clone();
        let actor_id = request.actor_id.clone();
        let agent_workspace_id = request.agent_workspace_id;
        let parent_user_id = parent_user_id.to_string();
        let parent_identity = parent_identity.cloned();
        let workspace = self.workspace.clone();
        let skill_registry = self.skill_registry.clone();
        let skills_config = self.skills_config.clone();

        // For the result injection message (already resolved above for the ledger write)
        let parent_thread_id = resolved_parent_thread_id;
        let ch_meta =
            with_subagent_thread_metadata(channel_metadata, &parent_thread_id, channel_name);

        let agent_name = name.clone();
        let agent_task = task.clone();
        let event_task_packet = task_packet.clone();
        let event_allowed_tools = grants.event_allowed_tools.clone();
        let event_allowed_skills = grants.event_allowed_skills.clone();
        let event_memory_mode = grants.memory_mode_label.to_string();
        let event_tool_mode = grants.tool_mode_label.to_string();
        let event_skill_mode = grants.skill_mode_label.to_string();

        // Clone store + sse_tx + cost_tracker for routine finalization inside spawned task
        let store_for_task = self.store.clone();
        let sse_tx_for_task = self.sse_tx.clone();
        let cost_tracker_for_task = self.cost_tracker.clone();
        let cost_guard_for_task = self.cost_guard.clone();

        // Emit SubagentSpawned event
        let _ = channels
            .send_status(
                &ch_name,
                StatusUpdate::SubagentSpawned {
                    agent_id: id.to_string(),
                    name: agent_name.clone(),
                    task: agent_task.clone(),
                    task_packet: event_task_packet.clone(),
                    allowed_tools: event_allowed_tools.clone(),
                    allowed_skills: event_allowed_skills.clone(),
                    memory_mode: event_memory_mode.clone(),
                    tool_mode: event_tool_mode.clone(),
                    skill_mode: event_skill_mode.clone(),
                },
                &ch_meta,
            )
            .await;

        let active_ref = self.active.clone();
        let active_for_task = self.active.clone();
        let join_handle = tokio::spawn(async move {
            let start = Instant::now();
            let (activity_tx, activity_rx) = watch::channel(Instant::now());
            let store_for_subagent_loop = store_for_task.clone();
            let cancel_watch_outer = cancel_rx.clone();
            let heartbeat = SubagentHeartbeat::spawn(
                channels.clone(),
                ch_name.clone(),
                ch_meta.clone(),
                id.to_string(),
                agent_name.clone(),
                activity_tx.clone(),
                activity_rx,
                heartbeat_cancel_tx,
                cancel_rx.clone(),
            );

            let result = tokio::time::timeout(
                timeout,
                run_subagent_loop(
                    llm,
                    safety,
                    tools,
                    channels.clone(),
                    &system_prompt,
                    &agent_task,
                    &ch_name,
                    &ch_meta,
                    cancel_rx,
                    parent_to_sub_rx,
                    max_iterations,
                    timeout.as_secs(),
                    allowed.as_deref(),
                    allowed_skills.as_deref(),
                    &task_packet,
                    &memory_mode,
                    &tool_mode,
                    &skill_mode,
                    tool_profile,
                    principal_id.as_deref(),
                    actor_id.as_deref(),
                    agent_workspace_id,
                    store_for_subagent_loop,
                    workspace,
                    skill_registry,
                    skills_config,
                    &id.to_string(),
                    activity_tx,
                    cost_tracker_for_task.clone(),
                    cost_guard_for_task,
                ),
            )
            .await;

            drop(heartbeat);

            let duration_ms = start.elapsed().as_millis() as u64;

            let subagent_result = match result {
                Ok(Ok((response, iterations))) => subagent_result_from_completion(
                    id,
                    agent_name.clone(),
                    duration_ms,
                    SubagentCompletionOutcome::Success {
                        response,
                        iterations,
                    },
                ),
                Ok(Err(e)) => {
                    // A cancel signal makes the loop exit with an error; report
                    // it as a cancellation, not a failure.
                    let outcome = if *cancel_watch_outer.borrow() {
                        SubagentCompletionOutcome::Cancelled
                    } else {
                        SubagentCompletionOutcome::Error(e.to_string())
                    };
                    subagent_result_from_completion(id, agent_name.clone(), duration_ms, outcome)
                }
                Err(_timeout) => subagent_result_from_completion(
                    id,
                    agent_name.clone(),
                    duration_ms,
                    SubagentCompletionOutcome::TimedOut,
                ),
            };

            let _ = channels
                .send_status(
                    &ch_name,
                    StatusUpdate::SubagentCompleted {
                        agent_id: id.to_string(),
                        name: subagent_result.name.clone(),
                        success: subagent_result.success,
                        response: subagent_completion_status_response(&subagent_result),
                        duration_ms,
                        iterations: subagent_result.iterations,
                        task_packet: event_task_packet.clone(),
                        allowed_tools: event_allowed_tools.clone(),
                        allowed_skills: event_allowed_skills.clone(),
                        memory_mode: event_memory_mode.clone(),
                        tool_mode: event_tool_mode.clone(),
                        skill_mode: event_skill_mode.clone(),
                    },
                    &ch_meta,
                )
                .await;

            // Persist a learning event for sub-agent completions so the
            // orchestrator can learn from delegated task outcomes.
            if let Some(ref store) = store_for_task {
                let conversation_id = Uuid::parse_str(&parent_thread_id).ok();
                let actor = parent_identity
                    .as_ref()
                    .map(|identity| identity.actor_id.clone())
                    .or_else(|| actor_id.clone());

                let completion = subagent_learning_completion(&subagent_result);
                let risk_tier = match subagent_learning_risk_tier(&subagent_result) {
                    SubagentLearningRiskTier::Low => RiskTier::Low,
                    SubagentLearningRiskTier::Medium => RiskTier::Medium,
                };
                let event = RuntimeLearningEvent::new(
                    "subagent_executor::completion",
                    ImprovementClass::Skill,
                    risk_tier,
                    completion.summary,
                )
                .with_target("subagent")
                .with_confidence(completion.confidence)
                .with_metadata(completion.metadata);

                let persisted = event.into_persisted(
                    parent_user_id.clone(),
                    actor,
                    Some(ch_name.clone()),
                    Some(parent_thread_id.clone()),
                    conversation_id,
                    None,
                    None,
                );
                if let Err(err) = store.insert_learning_event(&persisted).await {
                    tracing::debug!(
                        error = %err,
                        subagent_id = %subagent_result.agent_id,
                        "Failed to persist subagent learning event"
                    );
                } else {
                    let orchestrator = LearningOrchestrator::new(Arc::clone(store), None, None);
                    if let Err(err) = orchestrator
                        .handle_event("subagent_completion", &persisted)
                        .await
                    {
                        tracing::debug!(
                            error = %err,
                            subagent_id = %subagent_result.agent_id,
                            "Learning orchestrator skipped subagent completion event"
                        );
                    }
                }
            }

            // ── Routine run finalization ─────────────────────────────
            // If this subagent was spawned by a routine, finalize the
            // routine_run record and emit a RoutineLifecycle SSE event.
            if let (Some(routine_name), Some(run_id_str)) = (
                ch_meta.get("routine_name").and_then(|v| v.as_str()),
                ch_meta.get("routine_run_id").and_then(|v| v.as_str()),
            ) {
                let completion = subagent_routine_completion(&subagent_result);
                let run_status = completion.run_status;
                let summary = Some(completion.summary.clone());

                if let Some(ref store) = store_for_task
                    && let Ok(run_id) = run_id_str.parse::<Uuid>()
                {
                    if let Err(e) = store
                        .complete_routine_run(run_id, run_status, summary.as_deref(), None)
                        .await
                    {
                        tracing::error!(
                            routine = %routine_name,
                            run_id = %run_id_str,
                            "Failed to finalize routine run from subagent: {}", e
                        );
                    } else {
                        tracing::info!(
                            routine = %routine_name,
                            run_id = %run_id_str,
                            success = %subagent_result.success,
                            "Finalized routine run from subagent"
                        );
                    }

                    let routine_actor = subagent_routine_actor(
                        parent_identity
                            .as_ref()
                            .map(|identity| identity.actor_id.as_str()),
                        actor_id.as_deref(),
                        &parent_user_id,
                    );
                    let routine = ch_meta
                        .get("routine_id")
                        .and_then(|value| value.as_str())
                        .and_then(|value| Uuid::parse_str(value).ok());
                    let mut resolved_routine = None;
                    if let Some(routine_id) = routine
                        && let Ok(Some(found)) = store.get_routine(routine_id).await
                        && found.user_id == parent_user_id
                        && found.owner_actor_id() == routine_actor
                    {
                        resolved_routine = Some(found);
                    }
                    if resolved_routine.is_none()
                        && let Ok(Some(found)) = store
                            .get_routine_by_name_for_actor(
                                &parent_user_id,
                                &routine_actor,
                                routine_name,
                            )
                            .await
                    {
                        resolved_routine = Some(found);
                    }
                    if let Some(routine) = resolved_routine {
                        let completed_at = chrono::Utc::now();
                        let runtime_already_advanced =
                            routine_state_has_runtime_advance_for_run(&routine.state, run_id);
                        let next_fire_at = if runtime_already_advanced {
                            routine.next_fire_at
                        } else {
                            crate::agent::routine::next_fire_for_routine(
                                &routine,
                                None,
                                completed_at,
                            )
                            .unwrap_or(routine.next_fire_at)
                        };
                        let run_count = if runtime_already_advanced {
                            routine.run_count
                        } else {
                            routine.run_count + 1
                        };
                        let consecutive_failures =
                            if run_status == crate::agent::routine::RunStatus::Failed {
                                routine.consecutive_failures + 1
                            } else {
                                0
                            };
                        let state = routine_state_with_runtime_advance(
                            &routine.state,
                            run_id,
                            completed_at,
                        );
                        if let Err(error) = persist_routine_runtime_update(
                            store,
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
                                run_id = %run_id_str,
                                "Failed to update routine runtime after subagent finalization: {}",
                                error
                            );
                        }
                    }
                }

                // Emit SSE lifecycle event
                if let Some(ref sse_tx) = sse_tx_for_task {
                    let _ = sse_tx.send(SseEvent::RoutineLifecycle {
                        routine_name: routine_name.to_string(),
                        event: completion.lifecycle_event.to_string(),
                        run_id: Some(run_id_str.to_string()),
                        result_summary: summary.clone(),
                    });
                }
            }

            let ledger_status = subagent_status_from_result(&subagent_result);

            {
                let mut active = active_for_task.write().await;
                if let Some(handle) = active.get_mut(&id) {
                    handle.status = ledger_status.clone();
                    handle.completed_at = Some(chrono::Utc::now());
                    handle.join_handle = None;
                }
            }

            // ── Durable ledger: record completion (success, failure, ──────
            // timeout, or cancellation). This finalization block runs for
            // every exit path, including cancellation (two-phase cancel
            // routes back through the normal completion path), so the
            // ledger row is always closed out alongside the in-memory
            // status update above.
            if let Some(ref store) = store_for_task {
                let (ledger_status_str, ledger_error) =
                    subagent_run_status_for_completion(&ledger_status);
                if let Err(e) = store
                    .complete_subagent_run(id, ledger_status_str, ledger_error.as_deref())
                    .await
                {
                    tracing::warn!(
                        agent_id = %id,
                        "Failed to update subagent run ledger entry: {}", e
                    );
                }
            }

            let _ = completion_tx.send(subagent_result.clone());

            // Inject result back to parent agent via the result channel
            if !wait_for_completion && should_reinject_subagent_result(&ch_meta) {
                let _ = result_tx
                    .send(SubagentResultMessage {
                        result: subagent_result.clone(),
                        channel_name: ch_name,
                        parent_user_id,
                        parent_identity,
                        channel_metadata: ch_meta,
                        parent_thread_id,
                    })
                    .await;
            }

            subagent_result
        });

        // Store the JoinHandle now that we have it (Bug 38 — entry was pre-inserted above).
        {
            let mut active = active_ref.write().await;
            if let Some(handle) = active.get_mut(&id)
                && handle.status == SubagentStatus::Running
            {
                handle.join_handle = Some(join_handle);
            }
        }

        if wait_for_completion {
            completion_rx.await.map_err(|_| {
                Error::Tool(crate::error::ToolError::ExecutionFailed {
                    name: "spawn_subagent".to_string(),
                    reason: "Sub-agent task ended unexpectedly".to_string(),
                })
            })
        } else {
            Ok(SubagentResult {
                agent_id: id,
                name,
                response: subagent_spawned_response(id),
                iterations: 0,
                duration_ms: 0,
                success: true,
                error: None,
            })
        }
    }

    /// Send a message from the parent agent to a running sub-agent.
    pub async fn send_to_subagent(&self, agent_id: Uuid, message: String) -> bool {
        let active = self.active.read().await;
        if let Some(handle) = active.get(&agent_id)
            && handle.status == SubagentStatus::Running
        {
            return handle.parent_to_sub_tx.send(message).await.is_ok();
        }
        false
    }

    /// Cancel a running sub-agent.
    ///
    /// Two-phase: the cooperative cancel watch fires first so the loop exits
    /// through its normal completion path — running finalization (completion
    /// event, learning event, routine-run completion, result reinjection).
    /// The task is hard-aborted only if it fails to exit within the grace
    /// period, since an abort skips all of that.
    pub async fn cancel(&self, agent_id: Uuid) -> bool {
        let mut active = self.active.write().await;
        if let Some(handle) = active.get_mut(&agent_id)
            && should_cancel_subagent(&handle.status)
        {
            let _ = handle.cancel_tx.send(true);
            handle.status = subagent_cancelled_status();
            handle.completed_at = Some(chrono::Utc::now());
            if let Some(jh) = handle.join_handle.take() {
                let abort_handle = jh.abort_handle();
                let ledger_store = self.store.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = jh => {}
                        _ = tokio::time::sleep(SUBAGENT_CANCEL_GRACE) => {
                            tracing::warn!(
                                agent_id = %agent_id,
                                grace_secs = SUBAGENT_CANCEL_GRACE.as_secs(),
                                "Sub-agent did not exit within cancel grace period — aborting"
                            );
                            abort_handle.abort();
                            // The abort skips the task's finalization block,
                            // so close the ledger row here — otherwise it
                            // stays 'running' until the next restart's
                            // reconciliation sweep.
                            if let Some(store) = ledger_store
                                && let Err(e) = store
                                    .complete_subagent_run(
                                        agent_id,
                                        "cancelled",
                                        Some("aborted after cancel grace period"),
                                    )
                                    .await
                            {
                                tracing::debug!(
                                    agent_id = %agent_id,
                                    "Failed to close subagent ledger row after abort: {}",
                                    e
                                );
                            }
                        }
                    }
                });
            }
            tracing::info!(agent_id = %agent_id, "Sub-agent cancelled");
            return true;
        }
        false
    }

    /// List all sub-agents (active and finished).
    pub async fn list(&self) -> Vec<SubagentInfo> {
        let active = self.active.read().await;
        active
            .values()
            .map(|h| SubagentInfo {
                id: h.id,
                name: h.name.clone(),
                task: h.task.clone(),
                status: h.status.clone(),
                spawned_at: h.spawned_at.to_rfc3339(),
            })
            .collect()
    }

    /// Count running sub-agents.
    pub async fn running_count(&self) -> usize {
        let active = self.active.read().await;
        active
            .values()
            .filter(|h| h.status == SubagentStatus::Running)
            .count()
    }

    /// Remove completed/failed entries older than the given duration.
    pub async fn cleanup(&self, max_age: Duration) {
        let cutoff = chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default();
        let mut active = self.active.write().await;
        active.retain(|_, h| {
            h.status == SubagentStatus::Running || h.completed_at.unwrap_or(h.spawned_at) > cutoff
        });
    }
}

/// Startup reconciliation for the durable sub-agent run ledger.
///
/// Sub-agents only exist as live tokio tasks — a process restart drops them
/// without ever reaching `SubagentExecutor`'s completion finalization block,
/// which is what normally closes out a `subagent_runs` row. This leaves any
/// row still `running` from a previous process, and (transitively) leaks
/// the routine run it may have been finalizing, since nothing will ever
/// call `complete_routine_run` for it either.
///
/// This walks every row [`crate::db::Database::list_incomplete_subagent_runs`]
/// returns and:
/// - marks the `subagent_runs` row as `failed` with
///   [`SUBAGENT_RUN_ORPHANED_REASON`], and
/// - when the row carries a `routine_run_id`, also completes that routine
///   run as failed via [`crate::db::Database::complete_routine_run`], so the
///   routine's run history and concurrency counters aren't left stuck on a
///   phantom `running` entry.
///
/// Wired into startup in `src/main.rs` (immediately after the executor's
/// store is attached, before new spawns are accepted).
pub async fn reconcile_orphaned_subagent_runs(store: Arc<dyn crate::db::Database>) {
    let orphaned = match store.list_incomplete_subagent_runs().await {
        Ok(runs) => runs,
        Err(e) => {
            tracing::warn!(
                "Failed to list incomplete subagent runs for reconciliation: {}",
                e
            );
            return;
        }
    };

    for run in orphaned {
        tracing::warn!(
            subagent_run_id = %run.id,
            name = %run.name,
            "Reconciling orphaned subagent run left running by a previous process"
        );

        if let Err(e) = store
            .complete_subagent_run(
                run.id,
                thinclaw_agent::subagent::SUBAGENT_RUN_STATUS_FAILED,
                Some(SUBAGENT_RUN_ORPHANED_REASON),
            )
            .await
        {
            tracing::warn!(
                subagent_run_id = %run.id,
                "Failed to mark orphaned subagent run as failed: {}", e
            );
            continue;
        }

        let Some(routine_run_id) = run.routine_run_id.as_deref() else {
            continue;
        };
        let Ok(routine_run_uuid) = routine_run_id.parse::<Uuid>() else {
            tracing::warn!(
                subagent_run_id = %run.id,
                routine_run_id = %routine_run_id,
                "Orphaned subagent run has an unparsable routine_run_id, skipping routine finalization"
            );
            continue;
        };

        if let Err(e) = store
            .complete_routine_run(
                routine_run_uuid,
                crate::agent::routine::RunStatus::Failed,
                Some(SUBAGENT_RUN_ORPHANED_REASON),
                None,
            )
            .await
        {
            tracing::warn!(
                subagent_run_id = %run.id,
                routine_run_id = %routine_run_id,
                "Failed to finalize routine run for orphaned subagent: {}", e
            );
        } else {
            tracing::info!(
                subagent_run_id = %run.id,
                routine_run_id = %routine_run_id,
                "Finalized routine run as failed for orphaned subagent"
            );
        }
    }
}

/// Run a mini agentic loop for a sub-agent.
///
/// This is a simplified version of `Agent::run_agentic_loop()` that doesn't
/// need sessions, threads, undo, compaction, or hooks. It just runs
/// LLM → tool → LLM → ... until a text response or iteration limit.
#[allow(clippy::too_many_arguments)]
async fn run_subagent_loop(
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    channels: Arc<ChannelManager>,
    system_prompt: &str,
    task: &str,
    channel_name: &str,
    channel_metadata: &serde_json::Value,
    cancel_rx: watch::Receiver<bool>,
    mut parent_rx: mpsc::Receiver<String>,
    max_iterations: usize,
    timeout_secs: u64,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    task_packet: &SubagentTaskPacket,
    memory_mode: &SubagentMemoryMode,
    tool_mode: &SubagentToolMode,
    skill_mode: &SubagentSkillMode,
    tool_profile: ToolProfile,
    principal_id: Option<&str>,
    actor_id: Option<&str>,
    agent_workspace_id: Option<Uuid>,
    store: Option<Arc<dyn crate::db::Database>>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
    skills_config: Option<SkillsConfig>,
    agent_id: &str,
    activity_tx: watch::Sender<Instant>,
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
) -> Result<(String, usize), Error> {
    let mut context_messages = vec![ChatMessage::user(task.to_string())];

    let identity_defaults = subagent_identity_defaults(principal_id, actor_id);
    let principal_id = identity_defaults.principal_id;
    let actor_id = identity_defaults.actor_id;
    let mut job_ctx =
        JobContext::with_identity(principal_id, actor_id, "subagent", "Sub-agent task");
    job_ctx.metadata = subagent_job_metadata(SubagentJobMetadataInput {
        channel_metadata,
        principal_id,
        actor_id,
        agent_workspace_id,
        allowed_tools,
        allowed_skills,
        tool_profile,
    });

    // If this subagent was spawned by a routine (see `execute_as_subagent` in
    // routine_engine.rs), keep a store handle + parsed run id around so the
    // iteration loop below can renew the routine run's DB lease. This is what
    // lets the zombie reaper distinguish an actively-executing subagent-run
    // routine from a genuinely orphaned one, instead of relying on a fixed
    // wall-clock TTL.
    let routine_lease_run_id = channel_metadata
        .get("routine_run_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok());
    let store_for_lease = store.clone();

    let provider_tool_extensions = if let Some(store) = store {
        let orchestrator =
            LearningOrchestrator::new(store, workspace.clone(), skill_registry.clone());
        let active_tools = orchestrator
            .provider_tool_extensions(&job_ctx.user_id)
            .await;
        match (memory_mode, allowed_tools) {
            (SubagentMemoryMode::GrantedToolsOnly, Some(allowed_tools)) => active_tools
                .into_iter()
                .filter(|tool_name| allowed_tools.iter().any(|allowed| allowed == tool_name))
                .collect(),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let combined_system_prompt = build_subagent_system_prompt(
        system_prompt,
        task_packet,
        channel_name,
        &job_ctx.metadata,
        principal_id,
        actor_id,
        agent_workspace_id,
        workspace,
        skill_registry,
        skills_config,
        allowed_tools,
        allowed_skills,
        memory_mode,
        tool_mode,
        skill_mode,
        tool_profile,
        &safety,
        agent_id,
    )
    .await;
    let model_name = llm.active_model_name();
    let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();
    let mut reasoning = Reasoning::new(llm)
        .with_system_prompt(combined_system_prompt)
        .with_model_name(model_name);
    // Wire cost tracker so sub-agent LLM calls appear in the Cost Dashboard
    if let Some(ref tracker) = cost_tracker {
        reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
    }

    let routine_lease_renewed_at = std::sync::Mutex::new(None);
    for iteration in 0..max_iterations {
        // Check cancellation
        if *cancel_rx.borrow() {
            return Err(subagent_cancelled_error());
        }

        // Enforce the shared cost guardrails before every LLM call, exactly
        // like the main dispatcher loop (dispatcher/loop.rs). Without this,
        // delegated sub-agent work was an unmetered side channel around the
        // operator's daily-budget/hourly-rate limits.
        if let Some(ref guard) = cost_guard
            && let Err(limit) = guard.check_allowed().await
        {
            return Err(crate::error::LlmError::InvalidResponse {
                provider: "subagent".to_string(),
                reason: limit.to_string(),
            }
            .into());
        }

        // Renew the routine run's DB lease (time-gated inside the helper) so
        // a long-running subagent-executed routine isn't falsely reaped by
        // the zombie cleanup while it's still actively making progress. Uses
        // the spawn request's REAL timeout: a fixed default here let leases
        // expire mid-iteration for subagents configured with longer budgets.
        if let (Some(run_id), Some(store)) = (routine_lease_run_id, store_for_lease.as_ref()) {
            crate::agent::routine_engine::renew_routine_run_lease_if_due(
                store,
                run_id,
                timeout_secs,
                &routine_lease_renewed_at,
            )
            .await;
        }

        // Check for messages from the parent (non-blocking)
        while let Ok(parent_msg) = parent_rx.try_recv() {
            context_messages.push(ChatMessage::user(subagent_parent_message(&parent_msg)));
        }

        // Force text on last usable iteration so the model produces a text
        // response before the fallback error at the end of the loop (Bug 39 fix).
        // Use max_iterations - 2 because the loop is 0-indexed and the fallback
        // fires AFTER the loop completes.
        let force_text = should_force_subagent_text(iteration, max_iterations);

        let ctx = ReasoningContext {
            messages: context_messages.clone(),
            context_documents: Vec::new(),
            available_tools: if force_text {
                vec![]
            } else {
                let defs = tools
                    .tool_definitions_for_capabilities(
                        allowed_tools,
                        allowed_skills,
                        Some(&provider_tool_extensions),
                    )
                    .await;
                let defs =
                    tool_policies.filter_tool_definitions_for_metadata(defs, &job_ctx.metadata);
                tools
                    .filter_tool_definitions_for_execution_profile(
                        defs,
                        ToolExecutionLane::Subagent,
                        tool_profile,
                        &job_ctx.metadata,
                    )
                    .await
            },
            job_description: None,
            current_state: None,
            metadata: llm_metadata_from_json(&job_ctx.metadata),
            force_text,
            thinking: Default::default(),
            max_output_tokens: None,
        };

        // Race the LLM call against cancellation so a cancel is observed
        // within one await point instead of only at the next iteration —
        // the loop then exits through the normal completion path and
        // finalization (events, learning, routine-run completion) runs.
        let output = {
            let mut cancel_watch = cancel_rx.clone();
            tokio::select! {
                biased;
                _ = wait_for_subagent_cancel(&mut cancel_watch) => {
                    return Err(subagent_cancelled_error());
                }
                output = reasoning.respond_with_tools(&ctx) => output?,
            }
        };

        match output.result {
            RespondResult::Text(text) => {
                return Ok((text, iteration + 1));
            }
            RespondResult::ToolCalls {
                tool_calls,
                content,
            } => {
                // Add the assistant message with tool calls to context
                context_messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Execute each tool call
                for tc in tool_calls {
                    // Handle emit_user_message specially — forward as SubagentProgress
                    if tc.name == "emit_user_message" {
                        let message = extract_subagent_message(&tc.arguments)
                            .unwrap_or_else(|| "[no message]".to_string());
                        let msg_type = tc
                            .arguments
                            .get("message_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("progress");

                        let _ = channels
                            .send_status(
                                channel_name,
                                StatusUpdate::SubagentProgress {
                                    agent_id: agent_id.to_string(),
                                    message,
                                    category: normalize_subagent_progress_category(msg_type)
                                        .to_string(),
                                },
                                channel_metadata,
                            )
                            .await;
                        touch_subagent_activity(&activity_tx);

                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            serde_json::json!({
                                "status": "message_sent",
                                "message": extract_subagent_message(&tc.arguments)
                                    .unwrap_or_else(|| "[no message]".to_string()),
                                "message_type": msg_type,
                            })
                            .to_string(),
                        ));
                        continue;
                    }

                    // Handle agent_think — just record it
                    if tc.name == "agent_think" {
                        let thought = tc
                            .arguments
                            .get("thought")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            serde_json::json!({
                                "status": "thought_recorded",
                                "thought": thought,
                            })
                            .to_string(),
                        ));
                        continue;
                    }

                    // Emit tool progress
                    let _ = channels
                        .send_status(
                            channel_name,
                            StatusUpdate::SubagentProgress {
                                agent_id: agent_id.to_string(),
                                message: subagent_tool_activity_message(&tc.name, &tc.arguments),
                                category: subagent_activity_category().to_string(),
                            },
                            channel_metadata,
                        )
                        .await;
                    touch_subagent_activity(&activity_tx);

                    let prepared = match execution::prepare_tool_call(
                        execution::ToolPrepareRequest {
                            tools: &tools,
                            safety: &safety,
                            job_ctx: &job_ctx,
                            tool_name: &tc.name,
                            params: &tc.arguments,
                            lane: ToolExecutionLane::Subagent,
                            default_profile: tool_profile,
                            profile_override: None,
                            approval_mode: execution::ToolApprovalMode::Autonomous,
                            hooks: None,
                        },
                    )
                    .await
                    {
                        Ok(execution::ToolPrepareOutcome::Ready(prepared)) => prepared,
                        Ok(execution::ToolPrepareOutcome::NeedsApproval(_)) => {
                            let warning = format!(
                                "Tool '{}' requires explicit approval and cannot run in this delegated context",
                                tc.name
                            );
                            let _ = channels
                                .send_status(
                                    channel_name,
                                    StatusUpdate::SubagentProgress {
                                        agent_id: agent_id.to_string(),
                                        message: subagent_tool_warning_message(&tc.name, &warning),
                                        category: subagent_warning_category().to_string(),
                                    },
                                    channel_metadata,
                                )
                                .await;
                            touch_subagent_activity(&activity_tx);
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                format!("Error: {}", warning),
                            ));
                            continue;
                        }
                        Err(err) => {
                            let warning = err.to_string();
                            let _ = channels
                                .send_status(
                                    channel_name,
                                    StatusUpdate::SubagentProgress {
                                        agent_id: agent_id.to_string(),
                                        message: subagent_tool_warning_message(&tc.name, &warning),
                                        category: subagent_warning_category().to_string(),
                                    },
                                    channel_metadata,
                                )
                                .await;
                            touch_subagent_activity(&activity_tx);
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                format!("Error: {}", warning),
                            ));
                            continue;
                        }
                    };

                    let mut cancel_watch = cancel_rx.clone();
                    let execution_result = tokio::select! {
                        biased;
                        _ = wait_for_subagent_cancel(&mut cancel_watch) => {
                            return Err(subagent_cancelled_error());
                        }
                        result = execution::execute_tool_call(&prepared, &safety, &job_ctx) => result,
                    };
                    let result_str = match execution_result {
                        Ok(output) => output.sanitized_content,
                        Err(err) => {
                            let warning = err.to_string();
                            let _ = channels
                                .send_status(
                                    channel_name,
                                    StatusUpdate::SubagentProgress {
                                        agent_id: agent_id.to_string(),
                                        message: subagent_tool_warning_message(&tc.name, &warning),
                                        category: subagent_warning_category().to_string(),
                                    },
                                    channel_metadata,
                                )
                                .await;
                            touch_subagent_activity(&activity_tx);
                            format!("Error: {}", warning)
                        }
                    };

                    context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, result_str));
                }
            }
        }
    }

    Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
        name: "subagent".to_string(),
        reason: subagent_iteration_limit_reason(max_iterations),
    }))
}

fn subagent_cancelled_error() -> Error {
    Error::Tool(crate::error::ToolError::ExecutionFailed {
        name: "subagent".to_string(),
        reason: "Cancelled".to_string(),
    })
}

/// Resolve when the cancel watch flips to `true`. If the sender is gone,
/// cancellation can no longer be signalled, so pend forever instead of
/// resolving spuriously and killing a healthy loop.
async fn wait_for_subagent_cancel(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn build_subagent_system_prompt(
    base_system_prompt: &str,
    task_packet: &SubagentTaskPacket,
    channel_name: &str,
    channel_metadata: &serde_json::Value,
    principal_id: &str,
    actor_id: &str,
    agent_workspace_id: Option<Uuid>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
    skills_config: Option<SkillsConfig>,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    memory_mode: &SubagentMemoryMode,
    tool_mode: &SubagentToolMode,
    skill_mode: &SubagentSkillMode,
    tool_profile: ToolProfile,
    safety: &SafetyLayer,
    agent_id: &str,
) -> String {
    let workspace_prompt = build_subagent_workspace_prompt(
        workspace,
        channel_name,
        channel_metadata,
        principal_id,
        actor_id,
        agent_workspace_id,
        safety,
        agent_id,
    )
    .await;
    let skill_context = build_subagent_skill_context(
        skill_registry,
        skills_config,
        &task_packet.objective,
        allowed_skills,
    )
    .await;

    render_subagent_system_prompt(SubagentSystemPromptSections {
        workspace_prompt: workspace_prompt.as_deref(),
        base_system_prompt,
        task_packet,
        skill_context: skill_context.as_deref(),
        allowed_tools,
        allowed_skills,
        memory_mode,
        tool_mode,
        skill_mode,
        tool_profile_label: tool_profile.as_str(),
    })
}

async fn build_subagent_workspace_prompt(
    workspace: Option<Arc<Workspace>>,
    channel_name: &str,
    channel_metadata: &serde_json::Value,
    principal_id: &str,
    actor_id: &str,
    agent_workspace_id: Option<Uuid>,
    safety: &SafetyLayer,
    agent_id: &str,
) -> Option<String> {
    let base_workspace = workspace?;
    let effective_workspace = if let Some(workspace_id) = agent_workspace_id {
        Arc::new(base_workspace.as_ref().clone().with_agent(workspace_id))
    } else {
        base_workspace
    };

    let conversation_kind = ConversationKind::Group;
    let thread_key = channel_metadata
        .get("thread_id")
        .and_then(|value| value.as_str())
        .unwrap_or(agent_id);
    let external_key = format!("subagent:{channel_name}:{thread_key}:{actor_id}");
    let identity = ResolvedIdentity {
        principal_id: principal_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: scope_id_from_key(&external_key),
        conversation_kind,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key: external_key,
    };

    match effective_workspace
        .system_prompt_for_identity(
            Some(&identity),
            channel_name,
            safety.redact_pii_in_prompts(),
        )
        .await
    {
        Ok(prompt) if !prompt.is_empty() => Some(prompt),
        Ok(_) => None,
        Err(error) => {
            tracing::debug!(error = %error, "Could not load sub-agent workspace prompt");
            None
        }
    }
}

async fn build_subagent_skill_context(
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
    skills_config: Option<SkillsConfig>,
    task: &str,
    allowed_skills: Option<&[String]>,
) -> Option<String> {
    let skill_registry = skill_registry?;
    let guard = skill_registry.read().await;
    let available_skills: Vec<LoadedSkill> = guard
        .skills()
        .iter()
        .filter(|skill| subagent_allows_skill(allowed_skills, skill.manifest.name.as_str()))
        .cloned()
        .collect();
    if available_skills.is_empty() {
        return None;
    }

    let config = skills_config.unwrap_or_default();
    let active_skills = prefilter_skills(
        task,
        &available_skills,
        config.max_active_skills,
        config.max_context_tokens,
    )
    .into_iter()
    .cloned()
    .collect::<Vec<_>>();

    let active_names = active_skills
        .iter()
        .map(|skill| skill.name())
        .collect::<std::collections::HashSet<_>>();
    let inactive_skills = available_skills
        .iter()
        .filter(|skill| !active_names.contains(skill.name()))
        .collect::<Vec<_>>();
    let active_summaries = active_skills
        .iter()
        .map(crate::agent::skill_context_store::skill_summary)
        .collect::<Vec<_>>();
    let inactive_summaries = inactive_skills
        .iter()
        .map(|skill| crate::agent::skill_context_store::skill_summary(skill))
        .collect::<Vec<_>>();

    render_skill_sections(
        &active_summaries,
        &inactive_summaries,
        SUBAGENT_AVAILABLE_SKILL_INSTRUCTION,
    )
}

#[cfg(test)]
mod tests;

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

pub use thinclaw_agent::subagent::{
    DEFAULT_MAX_CONCURRENT, DEFAULT_TIMEOUT_SECS, SUBAGENT_MAX_ITERATIONS, SubagentConfig,
    SubagentInfo, SubagentResult, SubagentResultMessage, SubagentSpawnRequest, SubagentStatus,
};
use thinclaw_agent::subagent::{
    SubagentSystemPromptSections, extract_subagent_message, filter_tools_for_memory_mode,
    llm_metadata_from_json, normalize_subagent_progress_category, render_subagent_system_prompt,
    should_reinject_subagent_result, subagent_learning_completion, subagent_memory_mode_label,
    subagent_routine_actor, subagent_routine_completion, subagent_skill_mode_label,
    subagent_status_from_result, subagent_tool_activity_message, subagent_tool_mode_label,
    subagent_tool_warning_message, with_subagent_thread_metadata,
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
                        if last_activity.elapsed() < SUBAGENT_HEARTBEAT_INTERVAL {
                            continue;
                        }

                        let _ = channels
                            .send_status(
                                &channel_name,
                                StatusUpdate::SubagentProgress {
                                    agent_id: agent_id.clone(),
                                    message: format!("sub-agent '{agent_name}' still working"),
                                    category: "activity".to_string(),
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
    cancel_tx: watch::Sender<bool>,
    join_handle: Option<JoinHandle<SubagentResult>>,
    /// Send messages from the parent to this sub-agent.
    pub parent_to_sub_tx: mpsc::Sender<String>,
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
        {
            let mut active = self.active.write().await;
            let running = active
                .values()
                .filter(|h| h.status == SubagentStatus::Running)
                .count();
            if running >= self.config.max_concurrent {
                return Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                    name: "spawn_subagent".to_string(),
                    reason: format!(
                        "Maximum concurrent sub-agents reached ({}/{})",
                        running, self.config.max_concurrent
                    ),
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
                    spawned_at: chrono::Utc::now(),
                    cancel_tx: cancel_tx.clone(),
                    join_handle: None, // filled in after tokio::spawn
                    parent_to_sub_tx: parent_to_sub_tx.clone(),
                },
            );
        }

        let timeout = Duration::from_secs(
            request
                .timeout_secs
                .unwrap_or(self.config.default_timeout_secs),
        );
        let max_iterations = self.config.max_tool_iterations;
        let wait_for_completion = request.wait;

        // Build system prompt
        let system_prompt = request.system_prompt.clone().unwrap_or_else(|| {
            format!(
                "You are a focused sub-agent named '{}'. \
                 You have been delegated a specific task by the main agent.\n\n\
                 Complete the task thoroughly and concisely. \
                 Return a clear, actionable summary when done.\n\n\
                 Use `emit_user_message` only for meaningful checkpoints, interim findings, \
                 blockers, or clarifying questions that help the user stay oriented. \
                 Do not narrate every routine tool call unless detailed progress is explicitly requested.",
                request.name
            )
        });

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
        let allowed = Some(filter_tools_for_memory_mode(
            request.allowed_tools.clone().unwrap_or_default(),
            &memory_mode,
        ));
        let allowed_skills = request.allowed_skills.clone();
        let result_tx = self.result_tx.clone();
        let principal_id = request.principal_id.clone();
        let actor_id = request.actor_id.clone();
        let agent_workspace_id = request.agent_workspace_id;
        let parent_user_id = parent_user_id.to_string();
        let parent_identity = parent_identity.cloned();
        let workspace = self.workspace.clone();
        let skill_registry = self.skill_registry.clone();
        let skills_config = self.skills_config.clone();

        // For the result injection message
        let parent_thread_id = parent_thread_id
            .map(str::to_string)
            .or_else(|| {
                channel_metadata
                    .get("thread_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "agent:main".to_string());
        let ch_meta =
            with_subagent_thread_metadata(channel_metadata, &parent_thread_id, channel_name);

        let agent_name = name.clone();
        let agent_task = task.clone();
        let event_task_packet = task_packet.clone();
        let event_allowed_tools = allowed.clone().unwrap_or_default();
        let event_allowed_skills = allowed_skills.clone().unwrap_or_default();
        let event_memory_mode = subagent_memory_mode_label(&memory_mode).to_string();
        let event_tool_mode = subagent_tool_mode_label(&tool_mode).to_string();
        let event_skill_mode = subagent_skill_mode_label(&skill_mode).to_string();

        // Clone store + sse_tx + cost_tracker for routine finalization inside spawned task
        let store_for_task = self.store.clone();
        let sse_tx_for_task = self.sse_tx.clone();
        let cost_tracker_for_task = self.cost_tracker.clone();

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
                ),
            )
            .await;

            drop(heartbeat);

            let elapsed = start.elapsed();

            let subagent_result = match result {
                Ok(Ok((response, iterations))) => {
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::SubagentCompleted {
                                agent_id: id.to_string(),
                                name: agent_name.clone(),
                                success: true,
                                response: response.clone(),
                                duration_ms: elapsed.as_millis() as u64,
                                iterations,
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

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response,
                        iterations,
                        duration_ms: elapsed.as_millis() as u64,
                        success: true,
                        error: None,
                    }
                }
                Ok(Err(e)) => {
                    let err_msg = e.to_string();
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::SubagentCompleted {
                                agent_id: id.to_string(),
                                name: agent_name.clone(),
                                success: false,
                                response: err_msg.clone(),
                                duration_ms: elapsed.as_millis() as u64,
                                iterations: 0,
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

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response: String::new(),
                        iterations: 0,
                        duration_ms: elapsed.as_millis() as u64,
                        success: false,
                        error: Some(err_msg),
                    }
                }
                Err(_timeout) => {
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::SubagentCompleted {
                                agent_id: id.to_string(),
                                name: agent_name.clone(),
                                success: false,
                                response: "Timed out".to_string(),
                                duration_ms: elapsed.as_millis() as u64,
                                iterations: 0,
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

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response: String::new(),
                        iterations: 0,
                        duration_ms: elapsed.as_millis() as u64,
                        success: false,
                        error: Some("Timed out".to_string()),
                    }
                }
            };

            // Persist a learning event for sub-agent completions so the
            // orchestrator can learn from delegated task outcomes.
            if let Some(ref store) = store_for_task {
                let conversation_id = Uuid::parse_str(&parent_thread_id).ok();
                let actor = parent_identity
                    .as_ref()
                    .map(|identity| identity.actor_id.clone())
                    .or_else(|| actor_id.clone());

                let completion = subagent_learning_completion(&subagent_result);
                let event = RuntimeLearningEvent::new(
                    "subagent_executor::completion",
                    ImprovementClass::Skill,
                    if subagent_result.success {
                        RiskTier::Low
                    } else {
                        RiskTier::Medium
                    },
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

            {
                let mut active = active_for_task.write().await;
                if let Some(handle) = active.get_mut(&id) {
                    handle.status = subagent_status_from_result(&subagent_result);
                    handle.join_handle = None;
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
                response: format!(
                    "Sub-agent spawned (id: {}). Results will arrive when complete.",
                    id
                ),
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
    pub async fn cancel(&self, agent_id: Uuid) -> bool {
        let mut active = self.active.write().await;
        if let Some(handle) = active.get_mut(&agent_id)
            && handle.status == SubagentStatus::Running
        {
            let _ = handle.cancel_tx.send(true);
            handle.status = SubagentStatus::Cancelled;
            // Abort the task if we have a handle
            if let Some(jh) = handle.join_handle.take() {
                jh.abort();
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
        active.retain(|_, h| h.status == SubagentStatus::Running || h.spawned_at > cutoff);
    }

    /// Mark a sub-agent as completed (called when the join handle resolves).
    pub async fn mark_completed(&self, agent_id: Uuid, success: bool, error: Option<String>) {
        let mut active = self.active.write().await;
        if let Some(h) = active.get_mut(&agent_id) {
            h.status = if success {
                SubagentStatus::Completed
            } else if error.as_deref() == Some("Timed out") {
                SubagentStatus::TimedOut
            } else {
                SubagentStatus::Failed(error.unwrap_or_default())
            };
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
) -> Result<(String, usize), Error> {
    let mut context_messages = vec![ChatMessage::user(task.to_string())];

    let principal_id = principal_id.unwrap_or("subagent");
    let actor_id = actor_id.unwrap_or(principal_id);
    let mut job_ctx =
        JobContext::with_identity(principal_id, actor_id, "subagent", "Sub-agent task");
    job_ctx.metadata = channel_metadata.clone();
    if !job_ctx.metadata.is_object() {
        job_ctx.metadata = serde_json::json!({});
    }
    if let Some(metadata) = job_ctx.metadata.as_object_mut() {
        metadata
            .entry("conversation_kind".to_string())
            .or_insert_with(|| serde_json::json!("direct"));
        metadata
            .entry("principal_id".to_string())
            .or_insert_with(|| serde_json::json!(principal_id));
        metadata
            .entry("actor_id".to_string())
            .or_insert_with(|| serde_json::json!(actor_id));
        if let Some(agent_workspace_id) = agent_workspace_id {
            metadata.insert(
                "agent_workspace_id".to_string(),
                serde_json::json!(agent_workspace_id.to_string()),
            );
        }
        if let Some(allowed_tools) = allowed_tools {
            metadata.insert(
                "allowed_tools".to_string(),
                serde_json::json!(allowed_tools),
            );
        }
        if let Some(allowed_skills) = allowed_skills {
            metadata.insert(
                "allowed_skills".to_string(),
                serde_json::json!(allowed_skills),
            );
        }
        metadata.insert(
            "tool_profile".to_string(),
            serde_json::json!(tool_profile.as_str()),
        );
    }

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
    let mut reasoning = Reasoning::new(llm, safety.clone())
        .with_system_prompt(combined_system_prompt)
        .with_model_name(model_name);
    // Wire cost tracker so sub-agent LLM calls appear in the Cost Dashboard
    if let Some(ref tracker) = cost_tracker {
        reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
    }

    for iteration in 0..max_iterations {
        // Check cancellation
        if *cancel_rx.borrow() {
            return Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                name: "subagent".to_string(),
                reason: "Cancelled".to_string(),
            }));
        }

        // Check for messages from the parent (non-blocking)
        while let Ok(parent_msg) = parent_rx.try_recv() {
            context_messages.push(ChatMessage::user(format!(
                "[Message from main agent]: {}",
                parent_msg
            )));
        }

        // Force text on last usable iteration so the model produces a text
        // response before the fallback error at the end of the loop (Bug 39 fix).
        // Use max_iterations - 2 because the loop is 0-indexed and the fallback
        // fires AFTER the loop completes.
        let force_text = iteration >= max_iterations.saturating_sub(2);

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

        let output = reasoning.respond_with_tools(&ctx).await?;

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
                                category: "activity".to_string(),
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
                                        category: "warning".to_string(),
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
                                        category: "warning".to_string(),
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

                    let result_str =
                        match execution::execute_tool_call(&prepared, &safety, &job_ctx).await {
                            Ok(output) => output.sanitized_content,
                            Err(err) => {
                                let warning = err.to_string();
                                let _ = channels
                                    .send_status(
                                        channel_name,
                                        StatusUpdate::SubagentProgress {
                                            agent_id: agent_id.to_string(),
                                            message: subagent_tool_warning_message(
                                                &tc.name, &warning,
                                            ),
                                            category: "warning".to_string(),
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
        reason: format!("Exceeded maximum iterations ({})", max_iterations),
    }))
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
    let allowed_names = allowed_skills.map(|skills| {
        skills
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>()
    });
    let available_skills: Vec<LoadedSkill> = guard
        .skills()
        .iter()
        .filter(|skill| {
            allowed_names
                .as_ref()
                .is_none_or(|allowed| allowed.contains(skill.manifest.name.as_str()))
        })
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

    let mut sections = Vec::new();
    if !active_skills.is_empty() {
        let mut active_lines = vec!["### Active Skills".to_string()];
        for skill in &active_skills {
            active_lines.push(format!(
                "- **{}** (v{}, {}): {}",
                skill.name(),
                skill.version(),
                skill.trust,
                skill.manifest.description,
            ));
        }
        active_lines.push(
            "\nUse `skill_read` with the skill name to load full instructions before using a skill."
                .to_string(),
        );
        sections.push(active_lines.join("\n"));
    }

    let active_names = active_skills
        .iter()
        .map(|skill| skill.name())
        .collect::<std::collections::HashSet<_>>();
    let inactive = available_skills
        .iter()
        .filter(|skill| !active_names.contains(skill.name()))
        .collect::<Vec<_>>();

    if !inactive.is_empty() {
        let mut available_lines = vec!["### Available Skills".to_string()];
        for skill in inactive {
            available_lines.push(format!(
                "- **{}**: {}",
                skill.name(),
                skill.manifest.description
            ));
        }
        available_lines.push(
            "\nIf a task would benefit from one of these skills, use `skill_read` to load its full instructions first.".to_string(),
        );
        sections.push(available_lines.join("\n"));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SafetyConfig;
    use crate::testing::StubLlm;

    #[test]
    fn test_subagent_config_defaults() {
        let config = SubagentConfig::default();
        assert_eq!(config.max_concurrent, 5);
        assert_eq!(config.default_timeout_secs, 300);
        assert!(!config.allow_nested);
        assert_eq!(config.max_tool_iterations, 30);
    }

    #[test]
    fn test_subagent_status_equality() {
        assert_eq!(SubagentStatus::Running, SubagentStatus::Running);
        assert_ne!(SubagentStatus::Running, SubagentStatus::Completed);
        assert_eq!(
            SubagentStatus::Failed("err".to_string()),
            SubagentStatus::Failed("err".to_string())
        );
    }

    #[test]
    fn test_subagent_result_serialization() {
        let result = SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "researcher".to_string(),
            response: "Found 3 papers".to_string(),
            iterations: 5,
            duration_ms: 3200,
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("researcher"));
        assert!(json.contains("Found 3 papers"));

        let deserialized: SubagentResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "researcher");
        assert_eq!(deserialized.iterations, 5);
    }

    #[test]
    fn test_spawn_request_with_defaults() {
        let req = SubagentSpawnRequest {
            name: "test".to_string(),
            task: "do something".to_string(),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            allowed_tools: None,
            allowed_skills: None,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: None,
            wait: true,
        };
        assert_eq!(req.name, "test");
        assert!(req.wait);
        assert!(req.allowed_tools.is_none());
    }

    #[test]
    fn test_spawn_request_serialization() {
        let request = SubagentSpawnRequest {
            name: "researcher".to_string(),
            task: "Find papers about AI".to_string(),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            allowed_tools: Some(vec!["http".to_string(), "read_file".to_string()]),
            allowed_skills: None,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: Some(120),
            wait: true,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("researcher"));
        assert!(json.contains("Find papers about AI"));

        let deserialized: SubagentSpawnRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "researcher");
        assert_eq!(deserialized.allowed_tools.unwrap().len(), 2);
    }

    #[test]
    fn test_spawn_request_defaults() {
        let json = r#"{"name":"test","task":"do work"}"#;
        let request: SubagentSpawnRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.name, "test");
        assert!(request.system_prompt.is_none());
        assert!(request.model.is_none());
        assert!(request.task_packet.is_none());
        assert!(request.allowed_tools.is_none());
        assert!(request.timeout_secs.is_none());
        assert!(!request.wait);
    }

    #[test]
    fn normalize_strict_inherits_parent_tool_and_skill_ceilings() {
        let mut request = SubagentSpawnRequest {
            name: "researcher".to_string(),
            task: "Inspect the repo".to_string(),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            allowed_tools: None,
            allowed_skills: None,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: None,
            wait: false,
        };

        request.normalize_strict(
            Some(&["time".to_string(), "json".to_string()]),
            Some(&["github".to_string(), "openai-docs".to_string()]),
            ToolProfile::Restricted,
        );

        assert_eq!(
            request.allowed_tools,
            Some(vec!["json".to_string(), "time".to_string()])
        );
        assert_eq!(
            request.allowed_skills,
            Some(vec!["github".to_string(), "openai-docs".to_string()])
        );
        assert_eq!(request.tool_profile, Some(ToolProfile::Restricted));
    }

    #[test]
    fn normalize_strict_intersects_requested_tools_with_parent_ceiling() {
        let mut request = SubagentSpawnRequest {
            name: "researcher".to_string(),
            task: "Inspect the repo".to_string(),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            allowed_tools: Some(vec!["json".to_string(), "shell".to_string()]),
            allowed_skills: Some(vec!["github".to_string(), "skill-creator".to_string()]),
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: None,
            wait: false,
        };

        request.normalize_strict(
            Some(&["time".to_string(), "json".to_string()]),
            Some(&["github".to_string(), "openai-docs".to_string()]),
            ToolProfile::Restricted,
        );

        assert_eq!(request.allowed_tools, Some(vec!["json".to_string()]));
        assert_eq!(request.allowed_skills, Some(vec!["github".to_string()]));
    }

    #[test]
    fn extract_subagent_message_prefers_message_and_falls_back_to_content() {
        let from_message = serde_json::json!({
            "message": "Checking the docs",
            "content": "older field"
        });
        let from_content = serde_json::json!({
            "content": "Legacy progress payload"
        });

        assert_eq!(
            extract_subagent_message(&from_message).as_deref(),
            Some("Checking the docs")
        );
        assert_eq!(
            extract_subagent_message(&from_content).as_deref(),
            Some("Legacy progress payload")
        );
    }

    #[test]
    fn with_subagent_thread_metadata_inserts_thread_id_for_non_object_metadata() {
        let metadata = serde_json::json!("legacy");
        let merged = with_subagent_thread_metadata(&metadata, "thread-123", "web");

        assert_eq!(merged["thread_id"], serde_json::json!("thread-123"));
    }

    #[test]
    fn with_subagent_thread_metadata_overrides_existing_thread_id() {
        let metadata = serde_json::json!({
            "thread_id": "stale-thread",
            "channel": "web"
        });
        let merged = with_subagent_thread_metadata(&metadata, "thread-fresh", "web");

        assert_eq!(merged["thread_id"], serde_json::json!("thread-fresh"));
        assert_eq!(merged["channel"], serde_json::json!("web"));
    }

    #[test]
    fn normalize_subagent_progress_category_maps_known_message_types() {
        assert_eq!(
            normalize_subagent_progress_category("progress"),
            "milestone"
        );
        assert_eq!(
            normalize_subagent_progress_category("interim_result"),
            "finding"
        );
        assert_eq!(normalize_subagent_progress_category("question"), "question");
        assert_eq!(normalize_subagent_progress_category("warning"), "warning");
        assert_eq!(normalize_subagent_progress_category("tool"), "activity");
        assert_eq!(normalize_subagent_progress_category("other"), "update");
    }

    #[test]
    fn subagent_tool_activity_message_uses_argument_hints() {
        let path_message = subagent_tool_activity_message(
            "read_file",
            &serde_json::json!({ "path": "/tmp/demo.txt" }),
        );
        let query_message = subagent_tool_activity_message(
            "web_search",
            &serde_json::json!({ "query": "Rust async channels" }),
        );

        assert_eq!(path_message, "Running read file on /tmp/demo.txt");
        assert_eq!(query_message, "Running web search for Rust async channels");
    }

    #[tokio::test]
    async fn subagent_heartbeat_emits_progress_and_stops_on_cancel() {
        use crate::channels::Channel;
        use futures::stream;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CaptureChannel {
            progress_count: Arc<AtomicUsize>,
            tx: tokio::sync::mpsc::UnboundedSender<StatusUpdate>,
        }

        #[async_trait::async_trait]
        impl Channel for CaptureChannel {
            fn name(&self) -> &str {
                "capture"
            }

            async fn start(
                &self,
            ) -> Result<crate::channels::MessageStream, crate::error::ChannelError> {
                Ok(Box::pin(stream::empty()))
            }

            async fn respond(
                &self,
                _msg: &crate::channels::IncomingMessage,
                _response: crate::channels::OutgoingResponse,
            ) -> Result<(), crate::error::ChannelError> {
                Ok(())
            }

            async fn send_status(
                &self,
                status: StatusUpdate,
                _metadata: &serde_json::Value,
            ) -> Result<(), crate::error::ChannelError> {
                if matches!(status, StatusUpdate::SubagentProgress { .. }) {
                    self.progress_count.fetch_add(1, Ordering::SeqCst);
                }
                let _ = self.tx.send(status);
                Ok(())
            }

            async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
                Ok(())
            }
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let progress_count = Arc::new(AtomicUsize::new(0));
        let channel = CaptureChannel {
            progress_count: Arc::clone(&progress_count),
            tx,
        };

        let channels = Arc::new(ChannelManager::new());
        channels.add(Box::new(channel)).await;

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (activity_tx, activity_rx) = watch::channel(
            Instant::now()
                .checked_sub(SUBAGENT_HEARTBEAT_INTERVAL)
                .unwrap_or_else(Instant::now),
        );

        let heartbeat = SubagentHeartbeat::spawn(
            Arc::clone(&channels),
            "capture".to_string(),
            serde_json::json!({"thread_id": "thread-1"}),
            "agent-1".to_string(),
            "researcher".to_string(),
            activity_tx.clone(),
            activity_rx,
            cancel_tx.clone(),
            cancel_rx,
        );

        let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("heartbeat should emit")
            .expect("status channel should remain open");
        assert!(matches!(first, StatusUpdate::SubagentProgress { .. }));
        assert_eq!(progress_count.load(Ordering::SeqCst), 1);

        touch_subagent_activity(&activity_tx);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            rx.try_recv().is_err(),
            "activity reset should suppress immediate re-heartbeat"
        );

        let _ = cancel_tx.send(true);
        drop(heartbeat);

        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            rx.try_recv().is_err(),
            "heartbeat task should stop after cancellation"
        );
    }

    #[tokio::test]
    async fn completed_subagent_is_marked_completed_and_not_running() {
        let llm = Arc::new(StubLlm::new("done"));
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
        }));
        let tools = Arc::new(ToolRegistry::new());
        let channels = Arc::new(ChannelManager::new());
        let (executor, mut result_rx) =
            SubagentExecutor::new(llm, safety, tools, channels, SubagentConfig::default());

        let spawned = executor
            .spawn(
                SubagentSpawnRequest {
                    name: "test".to_string(),
                    task: "say done".to_string(),
                    system_prompt: None,
                    model: None,
                    task_packet: None,
                    memory_mode: None,
                    tool_mode: None,
                    skill_mode: None,
                    tool_profile: None,
                    allowed_tools: None,
                    allowed_skills: None,
                    principal_id: None,
                    actor_id: None,
                    agent_workspace_id: None,
                    timeout_secs: Some(5),
                    wait: false,
                },
                "web",
                &serde_json::json!({ "thread_id": "agent:main" }),
                "default",
                None,
                Some("agent:main"),
            )
            .await
            .expect("subagent should spawn");

        let completed = tokio::time::timeout(Duration::from_secs(2), result_rx.recv())
            .await
            .expect("result should arrive")
            .expect("channel should stay open");
        assert_eq!(completed.result.agent_id, spawned.agent_id);
        assert!(completed.result.success);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if executor.running_count().await == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("running count should drop after completion");

        let info = executor
            .list()
            .await
            .into_iter()
            .find(|entry| entry.id == spawned.agent_id)
            .expect("spawned agent should stay listed");
        assert_eq!(info.status, SubagentStatus::Completed);
    }
}

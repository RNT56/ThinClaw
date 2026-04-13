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

use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::learning::{
    ImprovementClass, LearningEvent as RuntimeLearningEvent, LearningOrchestrator, RiskTier,
};
use crate::agent::routine::RunStatus;
use crate::channels::web::types::SseEvent;
use crate::channels::{ChannelManager, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::identity::ResolvedIdentity;
use crate::llm::{
    ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolDefinition,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Maximum tool iterations for a sub-agent (less than the main agent).
const SUBAGENT_MAX_ITERATIONS: usize = 30;

/// Default sub-agent timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Maximum number of concurrent sub-agents.
const DEFAULT_MAX_CONCURRENT: usize = 5;

const SUBAGENT_PROGRESS_PREVIEW_MAX: usize = 80;

/// Configuration for the sub-agent system.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Maximum number of concurrent sub-agents.
    pub max_concurrent: usize,
    /// Default timeout for sub-agents in seconds.
    pub default_timeout_secs: u64,
    /// Whether sub-agents can spawn other sub-agents.
    pub allow_nested: bool,
    /// Maximum tool iterations per sub-agent.
    pub max_tool_iterations: usize,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            default_timeout_secs: DEFAULT_TIMEOUT_SECS,
            allow_nested: false,
            max_tool_iterations: SUBAGENT_MAX_ITERATIONS,
        }
    }
}

fn truncate_progress_preview(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }

    let truncated: String = value.chars().take(max_len.saturating_sub(3)).collect();
    format!("{truncated}...")
}

fn extract_subagent_message(arguments: &serde_json::Value) -> Option<String> {
    ["message", "content"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

fn with_subagent_thread_metadata(
    metadata: &serde_json::Value,
    parent_thread_id: &str,
) -> serde_json::Value {
    let mut merged = if metadata.is_object() {
        metadata.clone()
    } else {
        serde_json::json!({})
    };

    if let Some(object) = merged.as_object_mut() {
        object.insert(
            "thread_id".to_string(),
            serde_json::Value::String(parent_thread_id.to_string()),
        );
    }

    merged
}

fn normalize_subagent_progress_category(message_type: &str) -> &'static str {
    match message_type {
        "progress" => "milestone",
        "interim_result" => "finding",
        "question" => "question",
        "warning" => "warning",
        "tool" => "activity",
        _ => "update",
    }
}

fn first_argument_preview(arguments: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| arguments.get(*key))
        .and_then(|value| match value {
            serde_json::Value::String(s) => Some(truncate_progress_preview(
                s.trim(),
                SUBAGENT_PROGRESS_PREVIEW_MAX,
            )),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

fn subagent_tool_activity_message(tool_name: &str, arguments: &serde_json::Value) -> String {
    let tool_label = tool_name.replace('_', " ");

    if let Some(path) = first_argument_preview(arguments, &["path", "target", "file"]) {
        return format!("Running {tool_label} on {path}");
    }

    if let Some(query) = first_argument_preview(arguments, &["query", "q", "pattern", "task"]) {
        return format!("Running {tool_label} for {query}");
    }

    if let Some(url) = first_argument_preview(arguments, &["url"]) {
        return format!("Running {tool_label} on {url}");
    }

    if let Some(command) = first_argument_preview(arguments, &["command", "cmd"]) {
        return format!("Running {tool_label}: {command}");
    }

    format!("Running {tool_label}")
}

fn subagent_tool_warning_message(tool_name: &str, detail: &str) -> String {
    format!(
        "{tool_name} needs attention: {}",
        truncate_progress_preview(detail.trim(), SUBAGENT_PROGRESS_PREVIEW_MAX)
    )
}

/// A completed sub-agent result ready for injection into the main agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResultMessage {
    /// The sub-agent result.
    pub result: SubagentResult,
    /// Channel the parent agent was on when it spawned this sub-agent.
    pub channel_name: String,
    /// User ID to re-inject the result under.
    pub parent_user_id: String,
    /// Resolved identity so the re-injected message lands in the same session scope.
    pub parent_identity: Option<ResolvedIdentity>,
    /// Metadata for routing (contains thread_id etc).
    pub channel_metadata: serde_json::Value,
    /// Thread ID of the parent conversation.
    pub parent_thread_id: String,
}

/// Result from a completed sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    /// The sub-agent's unique ID.
    pub agent_id: Uuid,
    /// Display name of the sub-agent.
    pub name: String,
    /// The sub-agent's final response text.
    pub response: String,
    /// How many tool iterations were used.
    pub iterations: usize,
    /// Duration the sub-agent ran.
    pub duration_ms: u64,
    /// Whether the sub-agent completed successfully.
    pub success: bool,
    /// Error message if the sub-agent failed.
    pub error: Option<String>,
}

/// Status of a running sub-agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed(String),
    TimedOut,
    Cancelled,
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

/// Request to spawn a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnRequest {
    /// Display name for the sub-agent.
    pub name: String,
    /// Task description — becomes the user message in the sub-agent's context.
    pub task: String,
    /// Optional custom system prompt. If None, a task-focused default is used.
    pub system_prompt: Option<String>,
    /// Optional model override for the sub-agent.
    pub model: Option<String>,
    /// Optional list of allowed tool names. If None, all tools are available.
    pub allowed_tools: Option<Vec<String>>,
    /// Optional list of allowed skill names. If None, all skills remain visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    /// Optional principal owner for workspace-scoped tool access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// Optional actor owner for actor-scoped memory overlays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Optional routed agent workspace UUID for memory/tool isolation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_workspace_id: Option<Uuid>,
    /// Timeout in seconds. Falls back to config default.
    pub timeout_secs: Option<u64>,
    /// If true, wait for the sub-agent to complete and return its result inline.
    /// If false, return immediately and re-inject the result on completion.
    #[serde(default)]
    pub wait: bool,
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

    /// Spawn a sub-agent.
    ///
    /// When `request.wait` is true, waits for the sub-agent to finish and
    /// returns the final result inline. Otherwise returns immediately and
    /// re-injects the result through the `result_tx` channel on completion.
    pub async fn spawn(
        &self,
        request: SubagentSpawnRequest,
        channel_name: &str,
        channel_metadata: &serde_json::Value,
        parent_user_id: &str,
        parent_identity: Option<&ResolvedIdentity>,
        parent_thread_id: Option<&str>,
    ) -> Result<SubagentResult, Error> {
        let id = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = watch::channel(false);
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
                    cancel_tx,
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
        let allowed = request.allowed_tools.clone();
        let allowed_skills = request.allowed_skills.clone();
        let result_tx = self.result_tx.clone();
        let principal_id = request.principal_id.clone();
        let actor_id = request.actor_id.clone();
        let agent_workspace_id = request.agent_workspace_id;
        let parent_user_id = parent_user_id.to_string();
        let parent_identity = parent_identity.cloned();

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
        let ch_meta = with_subagent_thread_metadata(channel_metadata, &parent_thread_id);

        let agent_name = name.clone();
        let agent_task = task.clone();

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
                },
                &ch_meta,
            )
            .await;

        let active_ref = self.active.clone();
        let active_for_task = self.active.clone();
        let join_handle = tokio::spawn(async move {
            let start = Instant::now();

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
                    principal_id.as_deref(),
                    actor_id.as_deref(),
                    agent_workspace_id,
                    &id.to_string(),
                    cost_tracker_for_task.clone(),
                ),
            )
            .await;

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

                let event = RuntimeLearningEvent::new(
                    "subagent_executor::completion",
                    ImprovementClass::Skill,
                    if subagent_result.success {
                        RiskTier::Low
                    } else {
                        RiskTier::Medium
                    },
                    if subagent_result.success {
                        "Sub-agent completed successfully"
                    } else {
                        "Sub-agent failed to complete task"
                    },
                )
                .with_target("subagent")
                .with_confidence(if subagent_result.success { 0.82 } else { 0.38 })
                .with_metadata(serde_json::json!({
                    "subagent_id": subagent_result.agent_id,
                    "subagent_name": subagent_result.name,
                    "success": subagent_result.success,
                    "iterations": subagent_result.iterations,
                    "duration_ms": subagent_result.duration_ms,
                    "error": subagent_result.error,
                    "response_preview": truncate_progress_preview(&subagent_result.response, 240),
                    "target_type": "subagent",
                    "target": subagent_result.name,
                    "correction_count": if subagent_result.success { 0 } else { 1 },
                    "repeated_failures": if subagent_result.success { 0 } else { 1 },
                }));

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
                let run_status = if subagent_result.success {
                    RunStatus::Ok
                } else {
                    RunStatus::Failed
                };
                let summary = if subagent_result.success {
                    Some(subagent_result.response.clone())
                } else {
                    Some(
                        subagent_result
                            .error
                            .clone()
                            .unwrap_or_else(|| "Unknown error".to_string()),
                    )
                };

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
                }

                // Emit SSE lifecycle event
                if let Some(ref sse_tx) = sse_tx_for_task {
                    let event_type = if subagent_result.success {
                        "completed"
                    } else {
                        "failed"
                    };
                    let _ = sse_tx.send(SseEvent::RoutineLifecycle {
                        routine_name: routine_name.to_string(),
                        event: event_type.to_string(),
                        run_id: Some(run_id_str.to_string()),
                        result_summary: summary.clone(),
                    });
                }
            }

            {
                let mut active = active_for_task.write().await;
                if let Some(handle) = active.get_mut(&id) {
                    handle.status = if subagent_result.success {
                        SubagentStatus::Completed
                    } else if subagent_result.error.as_deref() == Some("Timed out") {
                        SubagentStatus::TimedOut
                    } else {
                        SubagentStatus::Failed(
                            subagent_result
                                .error
                                .clone()
                                .unwrap_or_else(|| "Unknown error".to_string()),
                        )
                    };
                    handle.join_handle = None;
                }
            }

            let _ = completion_tx.send(subagent_result.clone());

            // Inject result back to parent agent via the result channel
            let reinject_result = ch_meta
                .get("reinject_result")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if !wait_for_completion && reinject_result {
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

/// Info about a sub-agent (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: String,
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
    principal_id: Option<&str>,
    actor_id: Option<&str>,
    agent_workspace_id: Option<Uuid>,
    agent_id: &str,
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
    }

    // Build tool definitions (filtered if needed)
    let tool_defs: Vec<ToolDefinition> = tools
        .tool_definitions_for_capabilities(allowed_tools, allowed_skills)
        .await;

    let model_name = llm.active_model_name();
    let mut reasoning = Reasoning::new(llm, safety.clone())
        .with_system_prompt(system_prompt.to_string())
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
            available_tools: if force_text {
                vec![]
            } else {
                tool_defs.clone()
            },
            job_description: None,
            current_state: None,
            metadata: std::collections::HashMap::new(),
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

                    if !crate::tools::ToolRegistry::tool_name_allowed_by_metadata(
                        &job_ctx.metadata,
                        &tc.name,
                    ) {
                        let warning =
                            format!("Tool '{}' is not allowed for this delegated task", tc.name);
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
                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            format!("Error: {}", warning),
                        ));
                        continue;
                    }

                    // Execute normal tool
                    let tool = match tools.get(&tc.name).await {
                        Some(t) => t,
                        None => {
                            let warning = format!("Tool '{}' is unavailable", tc.name);
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
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                format!("Error: {}", warning),
                            ));
                            continue;
                        }
                    };

                    let tool_timeout = tool.execution_timeout();
                    let result = tokio::time::timeout(
                        tool_timeout,
                        tool.execute(tc.arguments.clone(), &job_ctx),
                    )
                    .await;

                    let result_str = match result {
                        Ok(Ok(output)) => {
                            // Convert serde_json::Value to string for the tool result
                            let raw = match &output.result {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            let sanitized = safety.sanitize_tool_output(&tc.name, &raw);
                            sanitized.content
                        }
                        Ok(Err(e)) => {
                            let error = format!("{}", e);
                            let _ = channels
                                .send_status(
                                    channel_name,
                                    StatusUpdate::SubagentProgress {
                                        agent_id: agent_id.to_string(),
                                        message: subagent_tool_warning_message(&tc.name, &error),
                                        category: "warning".to_string(),
                                    },
                                    channel_metadata,
                                )
                                .await;
                            format!("Error: {}", error)
                        }
                        Err(_) => {
                            let timeout_message = format!("Tool '{}' timed out", tc.name);
                            let _ = channels
                                .send_status(
                                    channel_name,
                                    StatusUpdate::SubagentProgress {
                                        agent_id: agent_id.to_string(),
                                        message: subagent_tool_warning_message(
                                            &tc.name,
                                            &timeout_message,
                                        ),
                                        category: "warning".to_string(),
                                    },
                                    channel_metadata,
                                )
                                .await;
                            format!("Error: {}", timeout_message)
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
        assert!(request.allowed_tools.is_none());
        assert!(request.timeout_secs.is_none());
        assert!(!request.wait);
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
        let merged = with_subagent_thread_metadata(&metadata, "thread-123");

        assert_eq!(merged["thread_id"], serde_json::json!("thread-123"));
    }

    #[test]
    fn with_subagent_thread_metadata_overrides_existing_thread_id() {
        let metadata = serde_json::json!({
            "thread_id": "stale-thread",
            "channel": "web"
        });
        let merged = with_subagent_thread_metadata(&metadata, "thread-fresh");

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
    async fn completed_subagent_is_marked_completed_and_not_running() {
        let llm = Arc::new(StubLlm::new("done"));
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
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

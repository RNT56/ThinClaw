//! Root-independent sub-agent DTOs and policy helpers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thinclaw_identity::ResolvedIdentity;
use thinclaw_types::{
    SubagentMemoryMode, SubagentSkillMode, SubagentTaskPacket, SubagentToolMode, ToolProfile,
};
use uuid::Uuid;

pub const SUBAGENT_MAX_ITERATIONS: usize = 30;
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_MAX_CONCURRENT: usize = 5;
pub const SUBAGENT_PROGRESS_PREVIEW_MAX: usize = 80;

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
    /// Default execution profile for delegated sub-agents.
    pub default_tool_profile: ToolProfile,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            default_timeout_secs: DEFAULT_TIMEOUT_SECS,
            allow_nested: false,
            max_tool_iterations: SUBAGENT_MAX_ITERATIONS,
            default_tool_profile: ToolProfile::ExplicitOnly,
        }
    }
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

/// Info about a sub-agent (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: String,
}

/// Status string stored in the `subagent_runs.status` column.
///
/// This is a coarse, DB-friendly status distinct from [`SubagentStatus`]
/// (which carries a `Failed(String)` payload) — the ledger keeps the reason
/// in the separate `error` column instead.
pub const SUBAGENT_RUN_STATUS_RUNNING: &str = "running";
pub const SUBAGENT_RUN_STATUS_COMPLETED: &str = "completed";
pub const SUBAGENT_RUN_STATUS_FAILED: &str = "failed";
pub const SUBAGENT_RUN_STATUS_TIMED_OUT: &str = "timed_out";
pub const SUBAGENT_RUN_STATUS_CANCELLED: &str = "cancelled";

/// Reason recorded on a `subagent_runs` row that was still `running` when
/// the process restarted, and is reconciled as failed at startup.
pub const SUBAGENT_RUN_ORPHANED_REASON: &str = "orphaned by restart";

/// A durable row in the `subagent_runs` ledger.
///
/// Written when a sub-agent is spawned and updated when it finishes, so a
/// process restart doesn't silently drop in-flight delegated work. See
/// `SubagentExecutor::spawn` (write) and its completion block (update) in
/// `src/agent/subagent_executor.rs`, plus
/// `reconcile_orphaned_subagent_runs` for startup recovery.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentRunRecord {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: String,
    pub parent_thread_id: Option<String>,
    pub routine_run_id: Option<String>,
    pub spawned_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
}

impl SubagentRunRecord {
    /// Build the initial `running` row written at spawn time.
    pub fn new_running(
        id: Uuid,
        name: impl Into<String>,
        task: impl Into<String>,
        parent_thread_id: Option<String>,
        routine_run_id: Option<String>,
        spawned_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            task: task.into(),
            status: SUBAGENT_RUN_STATUS_RUNNING.to_string(),
            parent_thread_id,
            routine_run_id,
            spawned_at,
            completed_at: None,
            error: None,
        }
    }
}

/// Map a [`SubagentStatus`] to the coarse status string stored in the
/// `subagent_runs.status` column, plus the error text (if any) to persist
/// in the `error` column.
pub fn subagent_run_status_for_completion(
    status: &SubagentStatus,
) -> (&'static str, Option<String>) {
    match status {
        SubagentStatus::Running => (SUBAGENT_RUN_STATUS_RUNNING, None),
        SubagentStatus::Completed => (SUBAGENT_RUN_STATUS_COMPLETED, None),
        SubagentStatus::Failed(reason) => (SUBAGENT_RUN_STATUS_FAILED, Some(reason.clone())),
        SubagentStatus::TimedOut => (
            SUBAGENT_RUN_STATUS_TIMED_OUT,
            Some("Sub-agent timed out".to_string()),
        ),
        SubagentStatus::Cancelled => (SUBAGENT_RUN_STATUS_CANCELLED, None),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentLearningCompletion {
    pub summary: &'static str,
    pub confidence: f32,
    pub correction_count: u64,
    pub repeated_failures: u64,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRoutineCompletion {
    pub run_status: crate::routine::RunStatus,
    pub summary: String,
    pub lifecycle_event: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentLearningRiskTier {
    Low,
    Medium,
}

#[derive(Debug, Clone, Copy)]
pub struct SubagentSystemPromptSections<'a> {
    pub workspace_prompt: Option<&'a str>,
    pub base_system_prompt: &'a str,
    pub task_packet: &'a SubagentTaskPacket,
    pub skill_context: Option<&'a str>,
    pub allowed_tools: Option<&'a [String]>,
    pub allowed_skills: Option<&'a [String]>,
    pub memory_mode: &'a SubagentMemoryMode,
    pub tool_mode: &'a SubagentToolMode,
    pub skill_mode: &'a SubagentSkillMode,
    pub tool_profile_label: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentConcurrency {
    pub running: usize,
    pub max_concurrent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentSpawnAdmission {
    Admitted,
    Rejected { reason: String },
}

impl SubagentConcurrency {
    pub fn new(running: usize, max_concurrent: usize) -> Self {
        Self {
            running,
            max_concurrent,
        }
    }

    pub fn allows_spawn(self) -> bool {
        matches!(self.admission(), SubagentSpawnAdmission::Admitted)
    }

    pub fn rejection_reason(self) -> String {
        format!(
            "Maximum concurrent sub-agents reached ({}/{})",
            self.running, self.max_concurrent
        )
    }

    pub fn admission(self) -> SubagentSpawnAdmission {
        if self.running < self.max_concurrent {
            SubagentSpawnAdmission::Admitted
        } else {
            SubagentSpawnAdmission::Rejected {
                reason: self.rejection_reason(),
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubagentJobMetadataInput<'a> {
    pub channel_metadata: &'a serde_json::Value,
    pub principal_id: &'a str,
    pub actor_id: &'a str,
    pub agent_workspace_id: Option<Uuid>,
    pub allowed_tools: Option<&'a [String]>,
    pub allowed_skills: Option<&'a [String]>,
    pub tool_profile: ToolProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentExecutionGrants {
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    pub event_allowed_tools: Vec<String>,
    pub event_allowed_skills: Vec<String>,
    pub memory_mode_label: &'static str,
    pub tool_mode_label: &'static str,
    pub skill_mode_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentIdentityDefaults<'a> {
    pub principal_id: &'a str,
    pub actor_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentCompletionOutcome {
    Success { response: String, iterations: usize },
    Error(String),
    TimedOut,
    Cancelled,
}

/// Request to spawn a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnRequest {
    /// Display name for the sub-agent.
    pub name: String,
    /// Task description -- becomes the user message in the sub-agent's context.
    pub task: String,
    /// Optional custom system prompt. If None, a task-focused default is used.
    pub system_prompt: Option<String>,
    /// Optional model override for the sub-agent.
    pub model: Option<String>,
    /// Structured task packet used as the canonical bounded assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_packet: Option<SubagentTaskPacket>,
    /// How the sub-agent may source memory/context beyond the provided task packet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mode: Option<SubagentMemoryMode>,
    /// Tool gating policy for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_mode: Option<SubagentToolMode>,
    /// Skill gating policy for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_mode: Option<SubagentSkillMode>,
    /// Optional execution profile override for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_profile: Option<ToolProfile>,
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

impl SubagentSpawnRequest {
    pub fn normalize_strict(
        &mut self,
        inherited_tools: Option<&[String]>,
        inherited_skills: Option<&[String]>,
        default_tool_profile: ToolProfile,
    ) {
        let objective = self
            .task_packet
            .as_ref()
            .map(|packet| packet.objective.trim().to_string())
            .filter(|objective| !objective.is_empty())
            .unwrap_or_else(|| self.task.trim().to_string());

        let packet = self
            .task_packet
            .get_or_insert_with(SubagentTaskPacket::default);
        packet.objective = objective.clone();
        packet.todos.retain(|item| !item.trim().is_empty());
        packet
            .acceptance_criteria
            .retain(|item| !item.trim().is_empty());
        packet.constraints.retain(|item| !item.trim().is_empty());
        packet
            .provided_context
            .retain(|item| !item.title.trim().is_empty() || !item.content.trim().is_empty());
        if packet
            .parent_summary
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            packet.parent_summary = None;
        }

        self.task = objective;
        self.memory_mode = Some(self.memory_mode.clone().unwrap_or_default());
        self.tool_mode = Some(self.tool_mode.clone().unwrap_or_default());
        self.skill_mode = Some(self.skill_mode.clone().unwrap_or_default());
        self.tool_profile = Some(self.tool_profile.unwrap_or(default_tool_profile));

        let requested_tools = self.allowed_tools.take();
        let normalized_tools = normalize_capability_allowlist(inherited_tools, requested_tools);
        self.allowed_tools = if inherited_tools.is_some()
            || self.tool_profile == Some(ToolProfile::ExplicitOnly)
            || !normalized_tools.is_empty()
        {
            Some(normalized_tools)
        } else {
            None
        };
        self.allowed_skills = Some(normalize_capability_allowlist(
            inherited_skills,
            self.allowed_skills.take(),
        ));
    }

    pub fn task_packet(&self) -> SubagentTaskPacket {
        let mut packet = self.task_packet.clone().unwrap_or_default();
        if packet.objective.trim().is_empty() {
            packet.objective = self.task.clone();
        }
        packet
    }
}

pub fn truncate_progress_preview(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }

    let truncated: String = value.chars().take(max_len.saturating_sub(3)).collect();
    format!("{truncated}...")
}

pub fn subagent_default_system_prompt(name: &str) -> String {
    format!(
        "You are a focused sub-agent named '{}'. \
         You have been delegated a specific task by the main agent.\n\n\
         Complete the task thoroughly and concisely. \
         Return a clear, actionable summary when done.\n\n\
         Use `emit_user_message` only for meaningful checkpoints, interim findings, \
         blockers, or clarifying questions that help the user stay oriented. \
         Do not narrate every routine tool call unless detailed progress is explicitly requested.",
        name
    )
}

pub fn resolve_parent_thread_id(
    explicit_parent_thread_id: Option<&str>,
    channel_metadata: &serde_json::Value,
) -> String {
    explicit_parent_thread_id
        .map(str::to_string)
        .or_else(|| {
            channel_metadata
                .get("thread_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "agent:main".to_string())
}

pub fn subagent_spawned_response(agent_id: Uuid) -> String {
    format!(
        "Sub-agent spawned (id: {}). Results will arrive when complete.",
        agent_id
    )
}

pub fn subagent_heartbeat_message(agent_name: &str) -> String {
    format!("sub-agent '{agent_name}' still working")
}

pub fn should_emit_subagent_heartbeat(
    elapsed_since_activity: Duration,
    interval: Duration,
) -> bool {
    elapsed_since_activity >= interval
}

pub fn subagent_activity_category() -> &'static str {
    "activity"
}

pub fn subagent_warning_category() -> &'static str {
    "warning"
}

pub fn subagent_parent_message(parent_message: &str) -> String {
    format!("[Message from main agent]: {parent_message}")
}

pub fn should_force_subagent_text(iteration: usize, max_iterations: usize) -> bool {
    iteration >= max_iterations.saturating_sub(2)
}

pub fn subagent_iteration_limit_reason(max_iterations: usize) -> String {
    format!("Exceeded maximum iterations ({})", max_iterations)
}

pub fn subagent_job_metadata(input: SubagentJobMetadataInput<'_>) -> serde_json::Value {
    let mut metadata = if input.channel_metadata.is_object() {
        input.channel_metadata.clone()
    } else {
        serde_json::json!({})
    };

    if let Some(object) = metadata.as_object_mut() {
        object
            .entry("conversation_kind".to_string())
            .or_insert_with(|| serde_json::json!("direct"));
        object
            .entry("principal_id".to_string())
            .or_insert_with(|| serde_json::json!(input.principal_id));
        object
            .entry("actor_id".to_string())
            .or_insert_with(|| serde_json::json!(input.actor_id));
        if let Some(agent_workspace_id) = input.agent_workspace_id {
            object.insert(
                "agent_workspace_id".to_string(),
                serde_json::json!(agent_workspace_id.to_string()),
            );
        }
        if let Some(allowed_tools) = input.allowed_tools {
            object.insert(
                "allowed_tools".to_string(),
                serde_json::json!(allowed_tools),
            );
        }
        if let Some(allowed_skills) = input.allowed_skills {
            object.insert(
                "allowed_skills".to_string(),
                serde_json::json!(allowed_skills),
            );
        }
        object.insert(
            "tool_profile".to_string(),
            serde_json::json!(input.tool_profile.as_str()),
        );
    }

    metadata
}

pub fn subagent_identity_defaults<'a>(
    principal_id: Option<&'a str>,
    actor_id: Option<&'a str>,
) -> SubagentIdentityDefaults<'a> {
    let principal_id = principal_id.unwrap_or("subagent");
    SubagentIdentityDefaults {
        principal_id,
        actor_id: actor_id.unwrap_or(principal_id),
    }
}

pub fn subagent_execution_grants(
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    memory_mode: &SubagentMemoryMode,
    tool_mode: &SubagentToolMode,
    skill_mode: &SubagentSkillMode,
) -> SubagentExecutionGrants {
    let allowed_tools = Some(filter_tools_for_memory_mode(
        allowed_tools.map(<[String]>::to_vec).unwrap_or_default(),
        memory_mode,
    ));
    let allowed_skills = allowed_skills.map(<[String]>::to_vec);
    let event_allowed_tools = allowed_tools.clone().unwrap_or_default();
    let event_allowed_skills = allowed_skills.clone().unwrap_or_default();

    SubagentExecutionGrants {
        allowed_tools,
        allowed_skills,
        event_allowed_tools,
        event_allowed_skills,
        memory_mode_label: subagent_memory_mode_label(memory_mode),
        tool_mode_label: subagent_tool_mode_label(tool_mode),
        skill_mode_label: subagent_skill_mode_label(skill_mode),
    }
}

pub fn extract_subagent_message(arguments: &serde_json::Value) -> Option<String> {
    ["message", "content"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

pub fn with_subagent_thread_metadata(
    metadata: &serde_json::Value,
    parent_thread_id: &str,
    channel_name: &str,
) -> serde_json::Value {
    let mut merged = if metadata.is_object() {
        metadata.clone()
    } else {
        serde_json::json!({})
    };

    if let Some(object) = merged.as_object_mut() {
        object.insert(
            "channel".to_string(),
            serde_json::Value::String(channel_name.to_string()),
        );
        object.insert(
            "thread_id".to_string(),
            serde_json::Value::String(parent_thread_id.to_string()),
        );
    }

    merged
}

pub fn llm_metadata_from_json(value: &serde_json::Value) -> HashMap<String, String> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                    serde_json::Value::Bool(boolean) => Some((key.clone(), boolean.to_string())),
                    serde_json::Value::Number(number) => Some((key.clone(), number.to_string())),
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        serde_json::to_string(value)
                            .ok()
                            .map(|json| (key.clone(), json))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn normalize_subagent_progress_category(message_type: &str) -> &'static str {
    match message_type {
        "progress" => "milestone",
        "interim_result" => "finding",
        "question" => "question",
        "warning" => "warning",
        "tool" => "activity",
        _ => "update",
    }
}

pub fn subagent_status_after_mark_completed(success: bool, error: Option<&str>) -> SubagentStatus {
    if success {
        SubagentStatus::Completed
    } else if error == Some("Timed out") {
        SubagentStatus::TimedOut
    } else if error == Some("Cancelled") {
        SubagentStatus::Cancelled
    } else {
        SubagentStatus::Failed(error.unwrap_or_default().to_string())
    }
}

pub fn subagent_status_from_result(result: &SubagentResult) -> SubagentStatus {
    if result.success {
        SubagentStatus::Completed
    } else if result.error.as_deref() == Some("Timed out") {
        SubagentStatus::TimedOut
    } else if result.error.as_deref() == Some("Cancelled") {
        SubagentStatus::Cancelled
    } else {
        SubagentStatus::Failed(
            result
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string()),
        )
    }
}

pub fn should_cancel_subagent(status: &SubagentStatus) -> bool {
    *status == SubagentStatus::Running
}

pub fn subagent_cancelled_status() -> SubagentStatus {
    SubagentStatus::Cancelled
}

pub fn subagent_result_from_completion(
    agent_id: Uuid,
    name: impl Into<String>,
    duration_ms: u64,
    outcome: SubagentCompletionOutcome,
) -> SubagentResult {
    match outcome {
        SubagentCompletionOutcome::Success {
            response,
            iterations,
        } => SubagentResult {
            agent_id,
            name: name.into(),
            response,
            iterations,
            duration_ms,
            success: true,
            error: None,
        },
        SubagentCompletionOutcome::Error(error) => SubagentResult {
            agent_id,
            name: name.into(),
            response: String::new(),
            iterations: 0,
            duration_ms,
            success: false,
            error: Some(error),
        },
        SubagentCompletionOutcome::TimedOut => SubagentResult {
            agent_id,
            name: name.into(),
            response: String::new(),
            iterations: 0,
            duration_ms,
            success: false,
            error: Some("Timed out".to_string()),
        },
        // The literal "Cancelled" is what subagent_status_from_result keys on.
        SubagentCompletionOutcome::Cancelled => SubagentResult {
            agent_id,
            name: name.into(),
            response: String::new(),
            iterations: 0,
            duration_ms,
            success: false,
            error: Some("Cancelled".to_string()),
        },
    }
}

pub fn subagent_completion_status_response(result: &SubagentResult) -> String {
    if result.success {
        result.response.clone()
    } else {
        result
            .error
            .clone()
            .unwrap_or_else(|| "Unknown error".to_string())
    }
}

/// Lower bound for derived subagent learning confidence.
///
/// Even a maximally-hard success or a hard failure should never collapse to
/// zero: the learning system still needs a nonzero signal to weight against.
const SUBAGENT_CONFIDENCE_FLOOR: f32 = 0.05;

/// Upper bound for derived subagent learning confidence.
///
/// Reserves headroom above "certain" so genuinely verified/human-confirmed
/// outcomes elsewhere in the learning pipeline can still be rated higher.
const SUBAGENT_CONFIDENCE_CEILING: f32 = 0.95;

/// Neutral confidence used for outcomes that are not quality signals at all
/// (timeouts, cancellations): the subagent may have been on track, so this
/// should neither reward nor penalize like a real success/failure would.
const SUBAGENT_CONFIDENCE_NEUTRAL: f32 = 0.5;

/// Iteration count above which additional iterations no longer move
/// confidence, expressed as a fraction of `SUBAGENT_MAX_ITERATIONS`. Chosen
/// so a subagent that used the full iteration budget is treated as
/// "maximally hard" rather than driving confidence further down.
const SUBAGENT_CONFIDENCE_ITERATION_SCALE: f64 = SUBAGENT_MAX_ITERATIONS as f64;

/// Duration above which additional wall-clock time no longer moves
/// confidence. Long-running-but-successful subagents are still discounted a
/// little (they likely struggled), but the discount saturates rather than
/// growing without bound.
const SUBAGENT_CONFIDENCE_DURATION_SCALE_MS: f64 = (DEFAULT_TIMEOUT_SECS * 1000) as f64;

/// Whether a failed subagent's error reflects an execution/environment
/// interruption rather than a judgment about task quality.
fn is_non_quality_failure(error: Option<&str>) -> bool {
    matches!(error, Some("Timed out") | Some("Cancelled"))
}

/// Derive a `[0.05, 0.95]` confidence score for subagent learning signals
/// purely from observables on `SubagentResult`.
///
/// The formula is monotonic in each input taken independently:
/// - More iterations never increases confidence (harder task -> less sure).
/// - Longer duration never increases confidence (struggled longer -> less sure).
/// - Timeouts/cancellations are treated as neutral (~0.5): they say nothing
///   about whether the subagent's approach was good, so they should not be
///   punished as hard as a genuine failure.
/// - Ordinary failures are rated below neutral, ordinary successes above it.
fn subagent_learning_confidence(result: &SubagentResult) -> f32 {
    // How much of the "iterations" and "duration" budget was consumed,
    // clamped to [0, 1]. Harder/slower runs push the discount toward 1.0.
    let iteration_load =
        (result.iterations as f64 / SUBAGENT_CONFIDENCE_ITERATION_SCALE).clamp(0.0, 1.0);
    let duration_load =
        (result.duration_ms as f64 / SUBAGENT_CONFIDENCE_DURATION_SCALE_MS).clamp(0.0, 1.0);
    // Blend the two load signals; iterations dominate since they more
    // directly reflect how much the subagent had to fight the task.
    let effort_load = (0.7 * iteration_load + 0.3 * duration_load).clamp(0.0, 1.0);

    let base: f64 = if result.success {
        // Successes start high and lose a modest amount of confidence as
        // effort load increases (a hard-won success is still a success, but
        // slightly less certain than an easy one).
        0.90 - 0.20 * effort_load
    } else if is_non_quality_failure(result.error.as_deref()) {
        // Timeouts/cancellations are not a quality signal: stay neutral,
        // nudged only slightly by how much effort was already spent.
        SUBAGENT_CONFIDENCE_NEUTRAL as f64 - 0.05 * effort_load
    } else {
        // Ordinary failures start low and lose further confidence the more
        // effort was burned before failing (more evidence the task was
        // genuinely hard/wrong, not a fluke).
        0.35 - 0.15 * effort_load
    };

    (base as f32).clamp(SUBAGENT_CONFIDENCE_FLOOR, SUBAGENT_CONFIDENCE_CEILING)
}

pub fn subagent_learning_completion(result: &SubagentResult) -> SubagentLearningCompletion {
    SubagentLearningCompletion {
        summary: if result.success {
            "Sub-agent completed successfully"
        } else {
            "Sub-agent failed to complete task"
        },
        confidence: subagent_learning_confidence(result),
        correction_count: if result.success { 0 } else { 1 },
        repeated_failures: if result.success { 0 } else { 1 },
        metadata: serde_json::json!({
            "subagent_id": result.agent_id,
            "subagent_name": result.name,
            "success": result.success,
            "iterations": result.iterations,
            "duration_ms": result.duration_ms,
            "error": result.error,
            "response_preview": truncate_progress_preview(&result.response, 240),
            "target_type": "subagent",
            "target": result.name,
            "correction_count": if result.success { 0 } else { 1 },
            "repeated_failures": if result.success { 0 } else { 1 },
        }),
    }
}

pub fn subagent_learning_risk_tier(result: &SubagentResult) -> SubagentLearningRiskTier {
    if result.success {
        SubagentLearningRiskTier::Low
    } else {
        SubagentLearningRiskTier::Medium
    }
}

pub fn subagent_routine_completion(result: &SubagentResult) -> SubagentRoutineCompletion {
    SubagentRoutineCompletion {
        run_status: if result.success {
            crate::routine::RunStatus::Ok
        } else {
            crate::routine::RunStatus::Failed
        },
        summary: if result.success {
            result.response.clone()
        } else {
            result
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string())
        },
        lifecycle_event: if result.success {
            "completed"
        } else {
            "failed"
        },
    }
}

pub fn subagent_routine_actor(
    parent_identity_actor: Option<&str>,
    request_actor_id: Option<&str>,
    parent_user_id: &str,
) -> String {
    parent_identity_actor
        .or(request_actor_id)
        .unwrap_or(parent_user_id)
        .to_string()
}

pub fn should_reinject_subagent_result(metadata: &serde_json::Value) -> bool {
    metadata
        .get("reinject_result")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

pub fn subagent_allows_skill(allowed_skills: Option<&[String]>, skill_name: &str) -> bool {
    allowed_skills.is_none_or(|allowed| allowed.iter().any(|allowed| allowed == skill_name))
}

pub fn render_subagent_system_prompt(sections: SubagentSystemPromptSections<'_>) -> String {
    let mut rendered = Vec::new();

    if let Some(workspace_prompt) = sections
        .workspace_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push(workspace_prompt.to_string());
    }

    rendered.push(format!(
        "## Sub-agent Mission\n\n{}",
        sections.base_system_prompt
    ));
    rendered.push(format!(
        "## Task Packet\n\n{}",
        render_task_packet(sections.task_packet)
    ));
    rendered.push(format!(
        "## Operating Contract\n\n\
         - Use the supplied task packet as the primary source of truth.\n\
         - Do not assume access to the parent agent's broader memory, transcript history, or personal context.\n\
         - Do not browse or search for additional context unless the parent explicitly granted the necessary tools.\n\
         - If the packet is insufficient, ask the parent for what is missing instead of widening scope.\n\
         - Complete the bounded assignment against the acceptance criteria and todos.\n\n\
         Memory mode: `{}`\n\
         Tool mode: `{}`\n\
         Tool profile: `{}`\n\
         Skill mode: `{}`\n\
         Explicit tool grants: {}\n\
         Explicit skill grants: {}",
        subagent_memory_mode_label(sections.memory_mode),
        subagent_tool_mode_label(sections.tool_mode),
        sections.tool_profile_label,
        subagent_skill_mode_label(sections.skill_mode),
        format_allowlist(sections.allowed_tools),
        format_allowlist(sections.allowed_skills),
    ));

    if let Some(skill_context) = sections
        .skill_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push(format!("## Skills\n{skill_context}"));
    }

    rendered.join("\n\n")
}

pub fn subagent_tool_activity_message(tool_name: &str, arguments: &serde_json::Value) -> String {
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

pub fn subagent_tool_warning_message(tool_name: &str, detail: &str) -> String {
    format!(
        "{tool_name} needs attention: {}",
        truncate_progress_preview(detail.trim(), SUBAGENT_PROGRESS_PREVIEW_MAX)
    )
}

pub fn normalize_capability_allowlist(
    inherited: Option<&[String]>,
    requested: Option<Vec<String>>,
) -> Vec<String> {
    let mut merged = match (inherited, requested) {
        (Some(inherited), Some(requested)) => {
            let inherited: std::collections::HashSet<&str> =
                inherited.iter().map(String::as_str).collect();
            requested
                .into_iter()
                .filter(|name| inherited.contains(name.as_str()))
                .collect::<Vec<_>>()
        }
        (Some(inherited), None) => inherited.to_vec(),
        (None, Some(requested)) => requested,
        (None, None) => Vec::new(),
    };
    merged.sort();
    merged.dedup();
    merged
}

pub fn subagent_memory_tool_names() -> &'static [&'static str] {
    &[
        "session_search",
        "memory_search",
        "memory_read",
        "external_memory_recall",
        "external_memory_status",
    ]
}

pub fn filter_tools_for_memory_mode(
    tools: Vec<String>,
    memory_mode: &SubagentMemoryMode,
) -> Vec<String> {
    if *memory_mode == SubagentMemoryMode::GrantedToolsOnly {
        return tools;
    }

    let blocked: std::collections::HashSet<&str> =
        subagent_memory_tool_names().iter().copied().collect();
    tools
        .into_iter()
        .filter(|tool| !blocked.contains(tool.as_str()))
        .collect()
}

pub fn subagent_memory_mode_label(mode: &SubagentMemoryMode) -> &'static str {
    match mode {
        SubagentMemoryMode::ProvidedContextOnly => "provided_context_only",
        SubagentMemoryMode::GrantedToolsOnly => "granted_tools_only",
    }
}

pub fn subagent_tool_mode_label(mode: &SubagentToolMode) -> &'static str {
    match mode {
        SubagentToolMode::ExplicitOnly => "explicit_only",
    }
}

pub fn subagent_skill_mode_label(mode: &SubagentSkillMode) -> &'static str {
    match mode {
        SubagentSkillMode::ExplicitOnly => "explicit_only",
    }
}

pub fn render_task_packet(packet: &SubagentTaskPacket) -> String {
    let mut sections = vec![format!("Objective: {}", packet.objective.trim())];

    if !packet.todos.is_empty() {
        sections.push(format!(
            "Todos:\n{}",
            packet
                .todos
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.acceptance_criteria.is_empty() {
        sections.push(format!(
            "Acceptance Criteria:\n{}",
            packet
                .acceptance_criteria
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.constraints.is_empty() {
        sections.push(format!(
            "Constraints:\n{}",
            packet
                .constraints
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.provided_context.is_empty() {
        sections.push(format!(
            "Provided Context:\n{}",
            packet
                .provided_context
                .iter()
                .map(|item| format!("### {}\n{}", item.title.trim(), item.content.trim()))
                .collect::<Vec<_>>()
                .join("\n\n")
        ));
    }

    if let Some(summary) = packet
        .parent_summary
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("Parent Summary:\n{}", summary.trim()));
    }

    sections.join("\n\n")
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

fn format_allowlist(items: Option<&[String]>) -> String {
    items
        .map(|items| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items.join(", ")
            }
        })
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strict_inherits_parent_tool_and_skill_ceilings() {
        let mut request = SubagentSpawnRequest {
            name: "worker".to_string(),
            task: "  Do work  ".to_string(),
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
            Some(&["shell".to_string(), "read_file".to_string()]),
            Some(&["rust".to_string()]),
            ToolProfile::ExplicitOnly,
        );

        assert_eq!(request.task, "Do work");
        assert_eq!(
            request.allowed_tools,
            Some(vec!["read_file".to_string(), "shell".to_string()])
        );
        assert_eq!(request.allowed_skills, Some(vec!["rust".to_string()]));
    }

    #[test]
    fn subagent_tool_activity_message_uses_argument_hints() {
        assert_eq!(
            subagent_tool_activity_message("read_file", &serde_json::json!({"path": "/tmp/a"})),
            "Running read file on /tmp/a"
        );
    }

    #[test]
    fn subagent_spawn_policy_uses_legacy_messages_and_thread_fallbacks() {
        let concurrency = SubagentConcurrency::new(5, 5);
        assert!(!concurrency.allows_spawn());
        assert_eq!(
            concurrency.rejection_reason(),
            "Maximum concurrent sub-agents reached (5/5)"
        );

        let metadata = serde_json::json!({ "thread_id": "from-metadata" });
        assert_eq!(
            resolve_parent_thread_id(Some("explicit"), &metadata),
            "explicit"
        );
        assert_eq!(resolve_parent_thread_id(None, &metadata), "from-metadata");
        assert_eq!(
            resolve_parent_thread_id(None, &serde_json::Value::Null),
            "agent:main"
        );

        let id = Uuid::nil();
        assert_eq!(
            subagent_spawned_response(id),
            "Sub-agent spawned (id: 00000000-0000-0000-0000-000000000000). Results will arrive when complete."
        );
    }

    #[test]
    fn subagent_loop_policy_formats_prompt_parent_message_and_limits() {
        let prompt = subagent_default_system_prompt("researcher");
        assert!(prompt.contains("researcher"));
        assert!(prompt.contains("emit_user_message"));

        assert_eq!(
            subagent_heartbeat_message("researcher"),
            "sub-agent 'researcher' still working"
        );
        assert_eq!(
            subagent_parent_message("Need more detail"),
            "[Message from main agent]: Need more detail"
        );
        assert!(!should_force_subagent_text(27, 30));
        assert!(should_force_subagent_text(28, 30));
        assert!(should_force_subagent_text(0, 0));
        assert_eq!(
            subagent_iteration_limit_reason(30),
            "Exceeded maximum iterations (30)"
        );
    }

    #[test]
    fn subagent_job_metadata_preserves_existing_scope_and_adds_capabilities() {
        let workspace_id = Uuid::nil();
        let allowed_tools = vec!["read_file".to_string()];
        let allowed_skills = vec!["github".to_string()];
        let metadata = subagent_job_metadata(SubagentJobMetadataInput {
            channel_metadata: &serde_json::json!({
                "conversation_kind": "group",
                "principal_id": "existing-principal",
                "thread_id": "thread-1"
            }),
            principal_id: "fallback-principal",
            actor_id: "actor-1",
            agent_workspace_id: Some(workspace_id),
            allowed_tools: Some(&allowed_tools),
            allowed_skills: Some(&allowed_skills),
            tool_profile: ToolProfile::ExplicitOnly,
        });

        assert_eq!(metadata["conversation_kind"], "group");
        assert_eq!(metadata["principal_id"], "existing-principal");
        assert_eq!(metadata["actor_id"], "actor-1");
        assert_eq!(
            metadata["agent_workspace_id"],
            "00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(metadata["allowed_tools"], serde_json::json!(["read_file"]));
        assert_eq!(metadata["allowed_skills"], serde_json::json!(["github"]));
        assert_eq!(metadata["tool_profile"], "explicit_only");
    }

    #[test]
    fn subagent_status_and_completion_policy_follow_result() {
        let success = SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "worker".to_string(),
            response: "done".to_string(),
            iterations: 3,
            duration_ms: 42,
            success: true,
            error: None,
        };
        assert_eq!(
            subagent_status_from_result(&success),
            SubagentStatus::Completed
        );
        assert_eq!(
            subagent_routine_completion(&success).lifecycle_event,
            "completed"
        );
        // Quick, low-iteration success: near the top of the success range.
        let confidence = subagent_learning_completion(&success).confidence;
        assert!(
            confidence > 0.85 && confidence <= 0.90,
            "expected near-ceiling success confidence, got {confidence}"
        );

        let timeout = SubagentResult {
            success: false,
            error: Some("Timed out".to_string()),
            ..success.clone()
        };
        assert_eq!(
            subagent_status_from_result(&timeout),
            SubagentStatus::TimedOut
        );
        assert_eq!(subagent_routine_completion(&timeout).summary, "Timed out");
    }

    #[test]
    fn subagent_lifecycle_policy_maps_admission_cancellation_and_completion_results() {
        assert_eq!(
            SubagentConcurrency::new(1, 2).admission(),
            SubagentSpawnAdmission::Admitted
        );
        assert_eq!(
            SubagentConcurrency::new(2, 2).admission(),
            SubagentSpawnAdmission::Rejected {
                reason: "Maximum concurrent sub-agents reached (2/2)".to_string()
            }
        );

        assert!(should_cancel_subagent(&SubagentStatus::Running));
        assert!(!should_cancel_subagent(&SubagentStatus::Completed));
        assert_eq!(subagent_cancelled_status(), SubagentStatus::Cancelled);
        assert_eq!(
            subagent_status_after_mark_completed(false, Some("Cancelled")),
            SubagentStatus::Cancelled
        );

        let cancelled = subagent_result_from_completion(
            Uuid::nil(),
            "worker",
            12,
            SubagentCompletionOutcome::Error("Cancelled".to_string()),
        );
        assert_eq!(
            subagent_status_from_result(&cancelled),
            SubagentStatus::Cancelled
        );
        assert_eq!(subagent_completion_status_response(&cancelled), "Cancelled");

        let completed = subagent_result_from_completion(
            Uuid::nil(),
            "worker",
            34,
            SubagentCompletionOutcome::Success {
                response: "done".to_string(),
                iterations: 4,
            },
        );
        assert!(completed.success);
        assert_eq!(completed.iterations, 4);
        assert_eq!(subagent_completion_status_response(&completed), "done");
    }

    #[test]
    fn subagent_grant_and_identity_policy_defaults_and_filters() {
        let allowed_tools = vec![
            "memory_search".to_string(),
            "read_file".to_string(),
            "session_search".to_string(),
        ];
        let allowed_skills = vec!["github".to_string()];
        let grants = subagent_execution_grants(
            Some(&allowed_tools),
            Some(&allowed_skills),
            &SubagentMemoryMode::ProvidedContextOnly,
            &SubagentToolMode::ExplicitOnly,
            &SubagentSkillMode::ExplicitOnly,
        );

        assert_eq!(grants.allowed_tools, Some(vec!["read_file".to_string()]));
        assert_eq!(grants.event_allowed_tools, vec!["read_file".to_string()]);
        assert_eq!(grants.allowed_skills, Some(vec!["github".to_string()]));
        assert_eq!(grants.memory_mode_label, "provided_context_only");
        assert!(subagent_allows_skill(
            grants.allowed_skills.as_deref(),
            "github"
        ));
        assert!(!subagent_allows_skill(
            grants.allowed_skills.as_deref(),
            "openai-docs"
        ));
        assert!(subagent_allows_skill(None, "openai-docs"));

        let defaults = subagent_identity_defaults(None, None);
        assert_eq!(defaults.principal_id, "subagent");
        assert_eq!(defaults.actor_id, "subagent");
        let inherited_actor = subagent_identity_defaults(Some("principal"), None);
        assert_eq!(inherited_actor.actor_id, "principal");
    }

    #[test]
    fn subagent_activity_and_learning_policy_are_stable() {
        assert!(!should_emit_subagent_heartbeat(
            Duration::from_secs(29),
            Duration::from_secs(30)
        ));
        assert!(should_emit_subagent_heartbeat(
            Duration::from_secs(30),
            Duration::from_secs(30)
        ));
        assert_eq!(subagent_activity_category(), "activity");
        assert_eq!(subagent_warning_category(), "warning");

        let success = SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "worker".to_string(),
            response: "done".to_string(),
            iterations: 1,
            duration_ms: 10,
            success: true,
            error: None,
        };
        let failure = SubagentResult {
            success: false,
            error: Some("failed".to_string()),
            ..success.clone()
        };
        assert_eq!(
            subagent_learning_risk_tier(&success),
            SubagentLearningRiskTier::Low
        );
        assert_eq!(
            subagent_learning_risk_tier(&failure),
            SubagentLearningRiskTier::Medium
        );
    }

    fn confidence_result(
        success: bool,
        iterations: usize,
        duration_ms: u64,
        error: Option<&str>,
    ) -> SubagentResult {
        SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "worker".to_string(),
            response: "done".to_string(),
            iterations,
            duration_ms,
            success,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn subagent_learning_confidence_stays_within_bounds() {
        let cases = [
            confidence_result(true, 1, 10, None),
            confidence_result(true, SUBAGENT_MAX_ITERATIONS, 10, None),
            confidence_result(false, 5, 100, Some("boom")),
            confidence_result(false, 1, 10, Some("Timed out")),
            confidence_result(false, 1, 10, Some("Cancelled")),
        ];
        for result in &cases {
            let confidence = subagent_learning_completion(result).confidence;
            assert!(
                (SUBAGENT_CONFIDENCE_FLOOR..=SUBAGENT_CONFIDENCE_CEILING).contains(&confidence),
                "confidence {confidence} out of bounds for {result:?}"
            );
        }
    }

    #[test]
    fn subagent_learning_confidence_quick_success_is_high() {
        let quick_success = confidence_result(true, 1, 50, None);
        let confidence = subagent_learning_completion(&quick_success).confidence;
        assert!(
            confidence > 0.8,
            "expected high confidence for a quick success, got {confidence}"
        );
    }

    #[test]
    fn subagent_learning_confidence_many_iteration_success_is_lower_than_quick_success() {
        let quick_success = confidence_result(true, 1, 50, None);
        let hard_success = confidence_result(true, SUBAGENT_MAX_ITERATIONS, 50, None);

        let quick_confidence = subagent_learning_completion(&quick_success).confidence;
        let hard_confidence = subagent_learning_completion(&hard_success).confidence;

        assert!(
            hard_confidence < quick_confidence,
            "many-iteration success ({hard_confidence}) should be less confident than a quick \
             success ({quick_confidence})"
        );
        // Still a success: should stay above the neutral timeout/cancel band.
        assert!(hard_confidence > SUBAGENT_CONFIDENCE_NEUTRAL);
    }

    #[test]
    fn subagent_learning_confidence_generic_failure_is_low() {
        let quick_failure = confidence_result(false, 1, 50, Some("tool exploded"));
        let hard_failure = confidence_result(
            false,
            SUBAGENT_MAX_ITERATIONS,
            DEFAULT_TIMEOUT_SECS * 1000,
            Some("tool exploded"),
        );

        let quick_confidence = subagent_learning_completion(&quick_failure).confidence;
        let hard_confidence = subagent_learning_completion(&hard_failure).confidence;

        assert!(
            quick_confidence < SUBAGENT_CONFIDENCE_NEUTRAL,
            "generic failure should be rated below neutral, got {quick_confidence}"
        );
        assert!(
            hard_confidence <= quick_confidence,
            "a failure reached after more effort should not be rated more confident"
        );
    }

    #[test]
    fn subagent_learning_confidence_timeout_is_neutral_not_punitive() {
        let timeout = confidence_result(false, 5, 60_000, Some("Timed out"));
        let generic_failure = confidence_result(false, 5, 60_000, Some("tool exploded"));

        let timeout_confidence = subagent_learning_completion(&timeout).confidence;
        let failure_confidence = subagent_learning_completion(&generic_failure).confidence;

        assert!(
            (timeout_confidence - SUBAGENT_CONFIDENCE_NEUTRAL).abs() < 0.15,
            "timeout confidence {timeout_confidence} should stay near neutral \
             ({SUBAGENT_CONFIDENCE_NEUTRAL})"
        );
        assert!(
            timeout_confidence > failure_confidence,
            "timeout ({timeout_confidence}) should not be punished as hard as a generic \
             failure ({failure_confidence})"
        );
    }

    #[test]
    fn subagent_learning_confidence_cancelled_is_neutral_not_punitive() {
        let cancelled = confidence_result(false, 2, 5_000, Some("Cancelled"));
        let generic_failure = confidence_result(false, 2, 5_000, Some("tool exploded"));

        let cancelled_confidence = subagent_learning_completion(&cancelled).confidence;
        let failure_confidence = subagent_learning_completion(&generic_failure).confidence;

        assert!(
            (cancelled_confidence - SUBAGENT_CONFIDENCE_NEUTRAL).abs() < 0.15,
            "cancelled confidence {cancelled_confidence} should stay near neutral \
             ({SUBAGENT_CONFIDENCE_NEUTRAL})"
        );
        assert!(
            cancelled_confidence > failure_confidence,
            "cancellation ({cancelled_confidence}) should not be punished as hard as a generic \
             failure ({failure_confidence})"
        );
    }

    #[test]
    fn render_system_prompt_includes_contract_and_grants() {
        let packet = SubagentTaskPacket {
            objective: "Inspect the adapter".to_string(),
            todos: vec!["Read code".to_string()],
            acceptance_criteria: vec![],
            constraints: vec![],
            provided_context: vec![],
            parent_summary: None,
        };
        let tools = vec!["read_file".to_string()];
        let skills: Vec<String> = Vec::new();
        let prompt = render_subagent_system_prompt(SubagentSystemPromptSections {
            workspace_prompt: Some("Workspace prompt"),
            base_system_prompt: "You are focused.",
            task_packet: &packet,
            skill_context: Some("### Active Skills"),
            allowed_tools: Some(&tools),
            allowed_skills: Some(&skills),
            memory_mode: &SubagentMemoryMode::ProvidedContextOnly,
            tool_mode: &SubagentToolMode::ExplicitOnly,
            skill_mode: &SubagentSkillMode::ExplicitOnly,
            tool_profile_label: "explicit_only",
        });

        assert!(prompt.contains("Workspace prompt"));
        assert!(prompt.contains("Objective: Inspect the adapter"));
        assert!(prompt.contains("Explicit tool grants: read_file"));
        assert!(prompt.contains("Explicit skill grants: none"));
    }

    #[test]
    fn llm_metadata_from_json_stringifies_scalar_and_structured_values() {
        let metadata = llm_metadata_from_json(&serde_json::json!({
            "string": "value",
            "boolean": true,
            "number": 42,
            "object": { "nested": "yes" },
            "array": [1, 2],
            "null": null
        }));

        assert_eq!(metadata.get("string").map(String::as_str), Some("value"));
        assert_eq!(metadata.get("boolean").map(String::as_str), Some("true"));
        assert_eq!(metadata.get("number").map(String::as_str), Some("42"));
        assert_eq!(
            metadata.get("object").map(String::as_str),
            Some("{\"nested\":\"yes\"}")
        );
        assert_eq!(metadata.get("array").map(String::as_str), Some("[1,2]"));
        assert!(!metadata.contains_key("null"));
    }
}

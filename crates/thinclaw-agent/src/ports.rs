//! Agent-owned runtime ports.
//!
//! These traits describe the persistence surface the extracted agent runtime
//! needs without making `thinclaw-agent` depend on a concrete database crate.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_channels_core::{IncomingMessage, OutgoingResponse, StatusUpdate};
use thinclaw_llm_core::{ChatMessage, ToolCall};
use thinclaw_tools_core::{ToolDescriptor, ToolExecutionLane, ToolOutput, ToolProfile};
use thinclaw_types::error::{ChannelError, DatabaseError, RoutineError, ToolError, WorkspaceError};
use thinclaw_types::{ActionRecord, JobContext};
use uuid::Uuid;

use crate::prompt_assembly::PromptAssemblyResult;
use crate::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};

/// Common runtime identity and routing scope for agent-owned ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentScope {
    pub principal_id: String,
    pub actor_id: String,
    pub channel: Option<String>,
    pub thread_id: Option<Uuid>,
    pub external_thread_id: Option<String>,
}

impl AgentScope {
    pub fn new(principal_id: impl Into<String>, actor_id: impl Into<String>) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            channel: None,
            thread_id: None,
            external_thread_id: None,
        }
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub fn with_thread(mut self, thread_id: Uuid) -> Self {
        self.thread_id = Some(thread_id);
        self
    }

    pub fn with_external_thread(mut self, external_thread_id: impl Into<String>) -> Self {
        self.external_thread_id = Some(external_thread_id.into());
        self
    }
}

/// A compact setting row that does not depend on a concrete settings crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Minimal conversation summary needed for recall and thread selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: Uuid,
    pub user_id: String,
    pub channel: String,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub message_count: i64,
    pub updated_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

/// Portable conversation message row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: String,
    pub content: String,
    pub actor_id: Option<String>,
    pub actor_display_name: Option<String>,
    pub raw_sender_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// State of a runtime thread without depending on the root session module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableThreadState {
    #[default]
    Idle,
    Processing,
    AwaitingApproval,
    Completed,
    Interrupted,
}

/// Pending auth token or OAuth request stored on a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortablePendingAuthMode {
    ManualToken,
    ExternalOAuth,
}

/// Portable auth request stored in thread runtime metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortablePendingAuth {
    pub extension_name: String,
    pub auth_mode: PortablePendingAuthMode,
}

/// Portable tool approval request stored in thread runtime metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortablePendingApproval {
    pub request_id: Uuid,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub description: String,
    pub tool_call_id: String,
    pub context_messages: Vec<ChatMessage>,
    #[serde(default)]
    pub deferred_tool_calls: Vec<ToolCall>,
}

/// Agent-selected LLM override scoped to a thread, conversation, or identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOverride {
    pub model_spec: String,
    pub reason: Option<String>,
}

/// Durable sub-agent state that can be serialized without the root executor types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableSubagentState {
    pub agent_id: Uuid,
    pub name: String,
    pub request: serde_json::Value,
    pub channel_name: String,
    #[serde(default)]
    pub channel_metadata: serde_json::Value,
    pub parent_user_id: String,
    pub parent_thread_id: String,
    #[serde(default)]
    pub reinject_result: bool,
}

/// Durable runtime envelope stored in thread metadata by adapters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadRuntimeSnapshot {
    #[serde(default)]
    pub state: PortableThreadState,
    #[serde(default)]
    pub pending_approval: Option<PortablePendingApproval>,
    #[serde(default)]
    pub pending_auth: Option<PortablePendingAuth>,
    #[serde(default)]
    pub owner_agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<ModelOverride>,
    #[serde(default)]
    pub auto_approved_tools: Vec<String>,
    #[serde(default)]
    pub active_subagents: Vec<PortableSubagentState>,
    #[serde(default)]
    pub last_context_pressure: Option<serde_json::Value>,
    #[serde(default)]
    pub post_compaction_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_workspace_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_provider_system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_segment_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
}

/// Target used for status, proactive messages, and broadcasts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelTarget {
    pub channel: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// A message accepted by the agent runtime for async processing.
#[derive(Debug, Clone)]
pub struct ChannelSubmission {
    pub message: IncomingMessage,
    pub scope: AgentScope,
    pub source: ChannelSubmissionSource,
}

/// Source of an accepted runtime submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSubmissionSource {
    User,
    Routine,
    System,
    Subagent,
    Retry,
}

/// Acknowledgement returned once the runtime accepts a submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelSubmissionAck {
    pub run_id: Uuid,
    pub thread_id: Option<Uuid>,
    pub accepted_at: DateTime<Utc>,
    pub status: SubmissionStatus,
}

/// Coarse state of a submitted run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Accepted,
    Queued,
    Running,
    Completed,
    Rejected,
}

/// Tool invocation request for host-mediated execution.
#[derive(Debug, Clone)]
pub struct ToolExecutionRequest {
    pub tool_name: String,
    pub params: serde_json::Value,
    pub job_ctx: JobContext,
    pub lane: ToolExecutionLane,
    pub profile: ToolProfile,
    pub approval_mode: ToolApprovalMode,
}

/// Approval posture for a tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalMode {
    Interactive {
        auto_approve_tools: bool,
        session_auto_approved: bool,
    },
    Autonomous,
    Bypass,
}

/// Serializable mirror of tool-core approval requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableApprovalRequirement {
    Never,
    UnlessAutoApproved,
    Always,
}

/// Result of preparing a tool call without necessarily executing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolPreparation {
    Ready {
        descriptor: ToolDescriptor,
        params: serde_json::Value,
        lane: ToolExecutionLane,
        profile: ToolProfile,
    },
    NeedsApproval {
        request_id: Uuid,
        descriptor: ToolDescriptor,
        params: serde_json::Value,
        lane: ToolExecutionLane,
        profile: ToolProfile,
        approval: PortableApprovalRequirement,
        description: String,
    },
}

/// Sanitized result shape returned by the shared tool execution pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub output: ToolOutput,
    pub sanitized_content: String,
    pub sanitized_value: serde_json::Value,
    pub was_modified: bool,
    pub warnings: Vec<String>,
    pub elapsed: Duration,
    pub sanitized_bytes: usize,
    pub sanitized_hash: String,
}

/// Lifecycle hook point identifiers owned by the agent crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentHookPoint {
    BeforeInbound,
    BeforeToolCall,
    BeforeOutbound,
    OnSessionStart,
    OnSessionEnd,
    TransformResponse,
    BeforeAgentStart,
    BeforeMessageWrite,
    BeforeLlmInput,
    AfterLlmOutput,
    BeforeTranscribeAudio,
}

/// Portable hook event envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentHookEvent {
    pub point: AgentHookPoint,
    pub payload: serde_json::Value,
}

/// Metadata passed alongside a hook invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentHookContext {
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub scope: Option<AgentScope>,
}

/// Result of dispatching hooks for an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AgentHookOutcome {
    Continue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modified: Option<String>,
    },
    Reject {
        reason: String,
    },
}

/// Hook execution failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookFailureMode {
    FailOpen,
    FailClosed,
}

/// Error returned by hook dispatch adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookPortError {
    ExecutionFailed { reason: String },
    Timeout { timeout_ms: u64 },
    Rejected { reason: String },
}

impl std::fmt::Display for HookPortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutionFailed { reason } => write!(f, "hook execution failed: {reason}"),
            Self::Timeout { timeout_ms } => write!(f, "hook timed out after {timeout_ms}ms"),
            Self::Rejected { reason } => write!(f, "hook rejected: {reason}"),
        }
    }
}

impl std::error::Error for HookPortError {}

/// Loaded skill summary for prompt assembly and sub-agent scoping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub trust: String,
    pub path: Option<String>,
}

/// Request for active and available skill context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillContextRequest {
    pub user_input: String,
    pub allowed_skills: Option<Vec<String>>,
    pub include_available_index: bool,
    pub include_active_matches: bool,
}

/// Rendered skill prompt context and the backing selected skills.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillContext {
    pub available_skills: Vec<SkillSummary>,
    pub active_skills: Vec<SkillSummary>,
    pub available_index_block: Option<String>,
    pub active_skill_block: Option<String>,
}

/// Request for workspace/provider prompt assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePromptRequest {
    pub scope: AgentScope,
    pub user_input: String,
    pub channel: String,
    pub routed_workspace_id: Option<Uuid>,
    pub agent_system_prompt: Option<String>,
    pub session_freeze_enabled: bool,
    pub existing_runtime: Option<ThreadRuntimeSnapshot>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Workspace prompt inputs after loading and sanitation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePromptMaterials {
    pub workspace_prompt: Option<String>,
    pub provider_system_prompt: Option<String>,
    pub provider_recall_block: Option<String>,
    pub provider_context_refs: Vec<String>,
    pub linked_recall_block: Option<String>,
    pub channel_formatting_hints: Option<String>,
    pub runtime_capability_hint: Option<String>,
    pub post_compaction_context: Option<String>,
}

/// Fully assembled prompt payload for the dispatcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePromptAssembly {
    pub materials: WorkspacePromptMaterials,
    pub skill_context: SkillContext,
    pub assembly: PromptAssemblyResult,
}

/// Generic learning event shape used by agent-runtime adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningEventRecord {
    pub id: Option<Uuid>,
    pub user_id: String,
    pub actor_id: Option<String>,
    pub channel: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Query for learning event retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningEventQuery {
    pub user_id: String,
    pub actor_id: Option<String>,
    pub channel: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: Option<String>,
    pub limit: i64,
}

/// Outcome contract shape used by the runtime without depending on history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeContractRecord {
    pub id: Uuid,
    pub user_id: String,
    pub actor_id: Option<String>,
    pub channel: Option<String>,
    pub thread_id: Option<String>,
    pub source_kind: String,
    pub source_id: String,
    pub status: String,
    pub due_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Query for outcome contract retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeContractQuery {
    pub user_id: String,
    pub actor_id: Option<String>,
    pub channel: Option<String>,
    pub thread_id: Option<String>,
    pub status: Option<String>,
    pub limit: i64,
}

/// Observation associated with an outcome contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeObservationRecord {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub evaluator: String,
    pub result: serde_json::Value,
    pub fingerprint: Option<String>,
}

/// Request to run routine behavior without moving the engine implementation yet.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum RoutineExecutionRequest {
    IncomingEvent(IncomingMessage),
    DueCronTick,
    Trigger(RoutineTrigger),
    RoutineRun {
        routine: Routine,
        trigger_key: String,
    },
}

/// Coarse result of routine execution or scheduling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineExecutionOutcome {
    pub fired_count: usize,
    pub run_ids: Vec<Uuid>,
    pub diagnostics: serde_json::Value,
}

/// Model override scope key used by the runtime and `llm_select`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ModelOverrideScope {
    Thread(Uuid),
    ConversationScope(Uuid),
    Identity {
        principal_id: String,
        actor_id: String,
    },
    Custom(String),
}

impl std::fmt::Display for ModelOverrideScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Thread(id) => write!(f, "thread:{id}"),
            Self::ConversationScope(id) => write!(f, "scope:{id}"),
            Self::Identity {
                principal_id,
                actor_id,
            } => write!(f, "identity:{principal_id}:{actor_id}"),
            Self::Custom(key) => f.write_str(key),
        }
    }
}

/// Runtime entrypoint for accepted channel submissions.
#[async_trait]
pub trait ChannelSubmissionPort: Send + Sync {
    async fn submit(
        &self,
        submission: ChannelSubmission,
    ) -> Result<ChannelSubmissionAck, ChannelError>;
}

/// Outbound channel status and response surface used by the runtime.
#[async_trait]
pub trait ChannelStatusPort: Send + Sync {
    async fn respond(
        &self,
        original: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError>;

    async fn send_status(
        &self,
        target: &ChannelTarget,
        status: StatusUpdate,
    ) -> Result<(), ChannelError>;

    async fn broadcast(
        &self,
        target: &ChannelTarget,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError>;
}

/// Tool registry and execution surface required by the agent runtime.
#[async_trait]
pub trait ToolExecutionPort: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, ToolError>;
    async fn get_tool(&self, name: &str) -> Result<Option<ToolDescriptor>, ToolError>;
    async fn prepare_tool(
        &self,
        request: ToolExecutionRequest,
    ) -> Result<ToolPreparation, ToolError>;
    async fn execute_tool(
        &self,
        request: ToolExecutionRequest,
    ) -> Result<ToolExecutionResult, ToolError>;
}

/// Hook registry dispatch surface.
#[async_trait]
pub trait HookDispatchPort: Send + Sync {
    async fn dispatch_hook(
        &self,
        event: AgentHookEvent,
        context: AgentHookContext,
    ) -> Result<AgentHookOutcome, HookPortError>;
}

/// Thread and conversation persistence surface used by extraction work.
#[async_trait]
pub trait ThreadStorePort: Send + Sync {
    async fn ensure_thread(
        &self,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
        external_thread_id: Option<&str>,
    ) -> Result<(), DatabaseError>;

    async fn load_thread_runtime(
        &self,
        thread_id: Uuid,
    ) -> Result<Option<ThreadRuntimeSnapshot>, DatabaseError>;

    async fn save_thread_runtime(
        &self,
        thread_id: Uuid,
        runtime: &ThreadRuntimeSnapshot,
    ) -> Result<(), DatabaseError>;

    async fn append_thread_message(
        &self,
        thread_id: Uuid,
        role: &str,
        content: &str,
        attribution: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_thread_messages(
        &self,
        thread_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<ThreadMessage>, DatabaseError>;

    async fn list_threads_for_recall(
        &self,
        scope: &AgentScope,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ThreadSummary>, DatabaseError>;
}

/// User settings persistence surface required by prompt/runtime code.
#[async_trait]
pub trait SettingsPort: Send + Sync {
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingEntry>, DatabaseError>;
    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError>;
    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError>;
    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError>;
}

/// Skill index and active-skill context surface.
#[async_trait]
pub trait SkillContextPort: Send + Sync {
    async fn skill_context(
        &self,
        request: SkillContextRequest,
    ) -> Result<SkillContext, WorkspaceError>;

    async fn reload_skills(&self) -> Result<(), WorkspaceError>;
}

/// Learning and outcome persistence surface needed after agent turns.
#[async_trait]
pub trait LearningOutcomesPort: Send + Sync {
    async fn record_action(&self, job_id: Uuid, action: &ActionRecord)
    -> Result<(), DatabaseError>;

    async fn record_learning_event(
        &self,
        event: &LearningEventRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_learning_events(
        &self,
        query: &LearningEventQuery,
    ) -> Result<Vec<LearningEventRecord>, DatabaseError>;

    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContractRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContractRecord>, DatabaseError>;

    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContractRecord,
    ) -> Result<(), DatabaseError>;

    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservationRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservationRecord>, DatabaseError>;
}

/// Routine execution facade for later engine extraction.
#[async_trait]
pub trait RoutineExecutionPort: Send + Sync {
    async fn execute_routine_request(
        &self,
        request: RoutineExecutionRequest,
    ) -> Result<RoutineExecutionOutcome, RoutineError>;
}

/// Workspace prompt material loading and final assembly surface.
#[async_trait]
pub trait WorkspacePromptAssemblyPort: Send + Sync {
    async fn load_prompt_materials(
        &self,
        request: &WorkspacePromptRequest,
    ) -> Result<WorkspacePromptMaterials, WorkspaceError>;

    async fn assemble_workspace_prompt(
        &self,
        request: WorkspacePromptRequest,
        materials: WorkspacePromptMaterials,
        skills: SkillContext,
    ) -> Result<WorkspacePromptAssembly, WorkspaceError>;
}

/// Shared model override state used by runtime LLM routing.
#[async_trait]
pub trait ModelOverridePort: Send + Sync {
    async fn get_model_override(
        &self,
        scope: &ModelOverrideScope,
    ) -> Result<Option<ModelOverride>, DatabaseError>;

    async fn set_model_override(
        &self,
        scope: &ModelOverrideScope,
        value: ModelOverride,
    ) -> Result<(), DatabaseError>;

    async fn clear_model_override(&self, scope: &ModelOverrideScope) -> Result<(), DatabaseError>;
}

/// Persistence operations required by routine scheduling and execution.
///
/// Backends should implement this port in storage crates. Keeping the trait in
/// `thinclaw-agent` lets future extracted runtime code depend on the agent
/// crate instead of reaching back into root or `thinclaw-db`.
#[async_trait]
pub trait RoutineStorePort: Send + Sync {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError>;
    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError>;
    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError>;

    async fn get_routine_by_name_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let routine = self.get_routine_by_name(user_id, name).await?;
        Ok(routine.filter(|routine| routine.owner_actor_id() == actor_id))
    }

    async fn list_routines_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
    ) -> Result<Vec<Routine>, DatabaseError> {
        let routines = self.list_routines(user_id).await?;
        Ok(routines
            .into_iter()
            .filter(|routine| routine.owner_actor_id() == actor_id)
            .collect())
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn get_routine_event_cache_version(&self) -> Result<i64, DatabaseError>;
    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;
    async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError>;
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
    async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError>;
    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError>;
    async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError>;

    async fn create_routine_event(
        &self,
        event: &RoutineEvent,
    ) -> Result<RoutineEvent, DatabaseError>;
    async fn claim_routine_event(
        &self,
        id: Uuid,
        worker_id: &str,
        stale_before: DateTime<Utc>,
    ) -> Result<Option<RoutineEvent>, DatabaseError>;
    async fn release_routine_event(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_pending_routine_events(
        &self,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError>;
    async fn complete_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        matched_routines: u32,
        fired_routines: u32,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn fail_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_events_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError>;
    async fn upsert_routine_event_evaluation(
        &self,
        evaluation: &RoutineEventEvaluation,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_event_evaluations_for_event(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError>;
    async fn list_routine_event_evaluations(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError>;

    async fn routine_run_exists_for_trigger_key(
        &self,
        routine_id: Uuid,
        trigger_key: &str,
    ) -> Result<bool, DatabaseError>;
    async fn enqueue_routine_trigger(&self, trigger: &RoutineTrigger) -> Result<(), DatabaseError>;
    async fn claim_routine_triggers(
        &self,
        worker_id: &str,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError>;
    async fn release_routine_trigger(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn complete_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        decision: RoutineTriggerDecision,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn fail_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_triggers(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError>;
}

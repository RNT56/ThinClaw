//! Staging ports for job context, worker scheduling, and channel streaming.
//!
//! This module intentionally avoids root crate types. It gives the next
//! scheduler, worker, and dispatcher extraction steps serializable DTOs and
//! object-safe traits without wiring adapters into the root application yet.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thinclaw_channels_core::StreamMode;
use thinclaw_types::error::{ChannelError, JobError};
use thinclaw_types::{JobContext, JobState, StateTransition};
use uuid::Uuid;

use crate::ports::{AgentScope, ChannelTarget};

/// Root-independent mirror of the durable direct-job state machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableJobState {
    #[default]
    Pending,
    InProgress,
    Completed,
    Submitted,
    Accepted,
    Failed,
    Stuck,
    Cancelled,
    Abandoned,
}

impl PortableJobState {
    /// Durable terminal states used by the existing job persistence layer.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Failed | Self::Cancelled | Self::Abandoned
        )
    }

    /// Worker cleanup terminal states. `Completed` and `Stuck` stop workers
    /// even though they are still visible to follow-up job flows.
    pub fn is_worker_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Stuck | Self::Cancelled | Self::Abandoned
        )
    }

    pub fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

impl From<JobState> for PortableJobState {
    fn from(value: JobState) -> Self {
        match value {
            JobState::Pending => Self::Pending,
            JobState::InProgress => Self::InProgress,
            JobState::Completed => Self::Completed,
            JobState::Submitted => Self::Submitted,
            JobState::Accepted => Self::Accepted,
            JobState::Failed => Self::Failed,
            JobState::Stuck => Self::Stuck,
            JobState::Cancelled => Self::Cancelled,
            JobState::Abandoned => Self::Abandoned,
        }
    }
}

impl From<PortableJobState> for JobState {
    fn from(value: PortableJobState) -> Self {
        match value {
            PortableJobState::Pending => Self::Pending,
            PortableJobState::InProgress => Self::InProgress,
            PortableJobState::Completed => Self::Completed,
            PortableJobState::Submitted => Self::Submitted,
            PortableJobState::Accepted => Self::Accepted,
            PortableJobState::Failed => Self::Failed,
            PortableJobState::Stuck => Self::Stuck,
            PortableJobState::Cancelled => Self::Cancelled,
            PortableJobState::Abandoned => Self::Abandoned,
        }
    }
}

impl std::fmt::Display for PortableJobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Submitted => "submitted",
            Self::Accepted => "accepted",
            Self::Failed => "failed",
            Self::Stuck => "stuck",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        };
        f.write_str(value)
    }
}

/// Serializable state transition without depending on the root context module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableStateTransition {
    pub from: PortableJobState,
    pub to: PortableJobState,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

impl From<StateTransition> for PortableStateTransition {
    fn from(value: StateTransition) -> Self {
        Self {
            from: value.from.into(),
            to: value.to.into(),
            timestamp: value.timestamp,
            reason: value.reason,
        }
    }
}

impl From<PortableStateTransition> for StateTransition {
    fn from(value: PortableStateTransition) -> Self {
        Self {
            from: value.from.into(),
            to: value.to.into(),
            timestamp: value.timestamp,
            reason: value.reason,
        }
    }
}

/// Serializable job context snapshot. This deliberately omits runtime-only
/// process data such as injected environment handles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableJobContext {
    pub job_id: Uuid,
    pub state: PortableJobState,
    pub user_id: String,
    pub principal_id: String,
    pub actor_id: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub budget: Option<Decimal>,
    pub budget_token: Option<String>,
    pub bid_amount: Option<Decimal>,
    pub estimated_cost: Option<Decimal>,
    pub estimated_duration_secs: Option<u64>,
    pub actual_cost: Decimal,
    pub total_tokens_used: u64,
    pub max_tokens: u64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub repair_attempts: u32,
    pub transitions: Vec<PortableStateTransition>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl PortableJobContext {
    pub fn new(
        scope: AgentScope,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            job_id: Uuid::new_v4(),
            state: PortableJobState::Pending,
            user_id: scope.principal_id.clone(),
            principal_id: scope.principal_id,
            actor_id: Some(scope.actor_id),
            conversation_id: scope.thread_id,
            title: title.into(),
            description: description.into(),
            category: None,
            budget: None,
            budget_token: None,
            bid_amount: None,
            estimated_cost: None,
            estimated_duration_secs: None,
            actual_cost: Decimal::ZERO,
            total_tokens_used: 0,
            max_tokens: 0,
            created_at: now,
            started_at: None,
            completed_at: None,
            repair_attempts: 0,
            transitions: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn owner_actor_id(&self) -> &str {
        self.actor_id.as_deref().unwrap_or(&self.user_id)
    }
}

impl From<JobContext> for PortableJobContext {
    fn from(value: JobContext) -> Self {
        Self {
            job_id: value.job_id,
            state: value.state.into(),
            user_id: value.user_id,
            principal_id: value.principal_id,
            actor_id: value.actor_id,
            conversation_id: value.conversation_id,
            title: value.title,
            description: value.description,
            category: value.category,
            budget: value.budget,
            budget_token: value.budget_token,
            bid_amount: value.bid_amount,
            estimated_cost: value.estimated_cost,
            estimated_duration_secs: value.estimated_duration.map(|duration| duration.as_secs()),
            actual_cost: value.actual_cost,
            total_tokens_used: value.total_tokens_used,
            max_tokens: value.max_tokens,
            created_at: value.created_at,
            started_at: value.started_at,
            completed_at: value.completed_at,
            repair_attempts: value.repair_attempts,
            transitions: value.transitions.into_iter().map(Into::into).collect(),
            metadata: value.metadata,
        }
    }
}

impl From<PortableJobContext> for JobContext {
    fn from(value: PortableJobContext) -> Self {
        Self {
            job_id: value.job_id,
            state: value.state.into(),
            user_id: value.user_id,
            principal_id: value.principal_id,
            actor_id: value.actor_id,
            conversation_id: value.conversation_id,
            title: value.title,
            description: value.description,
            category: value.category,
            budget: value.budget,
            budget_token: value.budget_token,
            bid_amount: value.bid_amount,
            estimated_cost: value.estimated_cost,
            estimated_duration: value.estimated_duration_secs.map(Duration::from_secs),
            actual_cost: value.actual_cost,
            total_tokens_used: value.total_tokens_used,
            max_tokens: value.max_tokens,
            created_at: value.created_at,
            started_at: value.started_at,
            completed_at: value.completed_at,
            repair_attempts: value.repair_attempts,
            transitions: value.transitions.into_iter().map(Into::into).collect(),
            metadata: value.metadata,
            extra_env: Arc::new(HashMap::new()),
        }
    }
}

/// Count summary for direct job contexts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableJobSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub submitted: usize,
    pub accepted: usize,
    pub failed: usize,
    pub stuck: usize,
    pub cancelled: usize,
    pub abandoned: usize,
}

impl PortableJobSummary {
    pub fn record(&mut self, state: PortableJobState) {
        self.total += 1;
        match state {
            PortableJobState::Pending => self.pending += 1,
            PortableJobState::InProgress => self.in_progress += 1,
            PortableJobState::Completed => self.completed += 1,
            PortableJobState::Submitted => self.submitted += 1,
            PortableJobState::Accepted => self.accepted += 1,
            PortableJobState::Failed => self.failed += 1,
            PortableJobState::Stuck => self.stuck += 1,
            PortableJobState::Cancelled => self.cancelled += 1,
            PortableJobState::Abandoned => self.abandoned += 1,
        }
    }
}

/// Create a new direct job context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateJobContextRequest {
    pub scope: AgentScope,
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub budget: Option<Decimal>,
    pub budget_token: Option<String>,
    pub bid_amount: Option<Decimal>,
    pub max_tokens: u64,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub reserved_system_slot: bool,
}

/// Query direct job snapshots without exposing a concrete store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobContextQuery {
    pub principal_id: Option<String>,
    pub actor_id: Option<String>,
    pub job_ids: Vec<Uuid>,
    pub states: Vec<PortableJobState>,
    pub active_only: bool,
    pub limit: Option<u64>,
}

/// Transition a direct job to a new state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobTransitionRequest {
    pub job_id: Uuid,
    pub target: PortableJobState,
    pub reason: Option<String>,
    pub transitioned_at: DateTime<Utc>,
}

/// Persist only the cheap status fields when a full snapshot is not needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStatusUpdate {
    pub job_id: Uuid,
    pub status: PortableJobState,
    pub failure_reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Serializable error for context and scheduler port adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobPortError {
    #[error("job {job_id} not found")]
    NotFound { job_id: Uuid },
    #[error("job {job_id} cannot transition from {from} to {to}")]
    InvalidTransition {
        job_id: Uuid,
        from: PortableJobState,
        to: PortableJobState,
    },
    #[error("maximum parallel jobs ({limit}) exceeded")]
    CapacityExceeded { running: usize, limit: usize },
    #[error("job {job_id} was cancelled")]
    Cancelled { job_id: Uuid },
    #[error("job {job_id} failed: {reason}")]
    Failed { job_id: Uuid, reason: String },
    #[error("persistence error: {reason}")]
    Persistence { reason: String },
    #[error("scheduler unavailable: {reason}")]
    SchedulerUnavailable { reason: String },
    #[error("{reason}")]
    Other { reason: String },
}

impl From<JobError> for JobPortError {
    fn from(value: JobError) -> Self {
        match value {
            JobError::NotFound { id } => Self::NotFound { job_id: id },
            JobError::InvalidTransition { id, state, target } => Self::Other {
                reason: format!("job {id} cannot transition from {state} to {target}"),
            },
            JobError::Failed { id, reason } => Self::Failed { job_id: id, reason },
            JobError::Stuck { id, duration } => Self::Other {
                reason: format!("job {id} stuck for {}ms", duration.as_millis()),
            },
            JobError::MaxJobsExceeded { max } => Self::CapacityExceeded {
                running: max,
                limit: max,
            },
            JobError::ContextError { id, reason } => Self::Failed { job_id: id, reason },
        }
    }
}

/// Volatile context manager surface used by workers and schedulers.
#[async_trait]
pub trait JobContextStatePort: Send + Sync {
    async fn create_job_context(
        &self,
        request: CreateJobContextRequest,
    ) -> Result<PortableJobContext, JobPortError>;

    async fn load_job_context(
        &self,
        job_id: Uuid,
    ) -> Result<Option<PortableJobContext>, JobPortError>;

    async fn save_job_context(&self, snapshot: &PortableJobContext) -> Result<(), JobPortError>;

    async fn transition_job_context(
        &self,
        request: JobTransitionRequest,
    ) -> Result<PortableJobContext, JobPortError>;

    async fn remove_job_context(
        &self,
        job_id: Uuid,
    ) -> Result<Option<PortableJobContext>, JobPortError>;

    async fn list_job_contexts(
        &self,
        query: &JobContextQuery,
    ) -> Result<Vec<PortableJobContext>, JobPortError>;

    async fn summarize_job_contexts(
        &self,
        query: &JobContextQuery,
    ) -> Result<PortableJobSummary, JobPortError>;
}

/// Durable direct-job snapshot store surface.
#[async_trait]
pub trait JobStateStorePort: Send + Sync {
    async fn save_job_snapshot(&self, snapshot: &PortableJobContext) -> Result<(), JobPortError>;

    async fn load_job_snapshot(
        &self,
        job_id: Uuid,
    ) -> Result<Option<PortableJobContext>, JobPortError>;

    async fn list_job_snapshots(
        &self,
        query: &JobContextQuery,
    ) -> Result<Vec<PortableJobContext>, JobPortError>;

    async fn update_job_status(&self, update: JobStatusUpdate) -> Result<(), JobPortError>;

    async fn abandon_active_jobs(&self, reason: &str) -> Result<u64, JobPortError>;

    async fn mark_job_stuck(&self, job_id: Uuid) -> Result<(), JobPortError>;

    async fn list_stuck_job_ids(&self) -> Result<Vec<Uuid>, JobPortError>;
}

/// Serializable mirror of per-channel streaming behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableStreamMode {
    #[default]
    None,
    EditFirst,
    StatusLine,
    EventChunks,
}

impl From<StreamMode> for PortableStreamMode {
    fn from(value: StreamMode) -> Self {
        match value {
            StreamMode::None => Self::None,
            StreamMode::EditFirst => Self::EditFirst,
            StreamMode::StatusLine => Self::StatusLine,
            StreamMode::EventChunks => Self::EventChunks,
        }
    }
}

impl From<PortableStreamMode> for StreamMode {
    fn from(value: PortableStreamMode) -> Self {
        match value {
            PortableStreamMode::None => Self::None,
            PortableStreamMode::EditFirst => Self::EditFirst,
            PortableStreamMode::StatusLine => Self::StatusLine,
            PortableStreamMode::EventChunks => Self::EventChunks,
        }
    }
}

/// Portable draft state. This replaces the channel-core `Instant` with fields
/// that can be persisted or sent over an event stream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableDraftReplyState {
    pub message_id: Option<String>,
    pub channel_id: String,
    pub accumulated: String,
    pub posted: bool,
    pub overflow: bool,
    pub last_edit_at: Option<DateTime<Utc>>,
}

impl PortableDraftReplyState {
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            ..Self::default()
        }
    }
}

/// Request to update an in-progress channel draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDraftUpdate {
    pub target: ChannelTarget,
    pub draft: PortableDraftReplyState,
    #[serde(default)]
    pub final_update: bool,
}

/// Result of sending or editing a channel draft.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDraftAck {
    pub message_id: Option<String>,
    pub posted: bool,
    pub overflow: bool,
}

/// Request to delete a previously posted streaming draft message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDraftDelete {
    pub target: ChannelTarget,
    pub message_id: String,
}

/// Serializable event frame for status streams such as SSE or channel adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatusStreamFrame {
    pub target: ChannelTarget,
    pub emitted_at: DateTime<Utc>,
    pub run_id: Option<Uuid>,
    pub sequence: Option<u64>,
    pub payload: ChannelStatusStreamPayload,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Portable status payloads for streaming transports. Direct channel responses
/// still belong to `ChannelStatusPort` in `ports.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelStatusStreamPayload {
    Thinking {
        text: String,
    },
    Status {
        text: String,
    },
    StreamChunk {
        chunk: String,
    },
    Draft {
        draft: PortableDraftReplyState,
    },
    ToolStarted {
        name: String,
        parameters: Option<serde_json::Value>,
    },
    ToolCompleted {
        name: String,
        success: bool,
        result_preview: Option<String>,
    },
    ToolResult {
        name: String,
        preview: String,
        #[serde(default)]
        artifacts: serde_json::Value,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cost_usd: Option<f64>,
        model: Option<String>,
    },
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
    },
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
        auth_url: Option<String>,
        setup_url: Option<String>,
        auth_mode: String,
        auth_status: String,
        shared_auth_provider: Option<String>,
        missing_scopes: Vec<String>,
        thread_id: Option<String>,
    },
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
        auth_mode: Option<String>,
        auth_status: Option<String>,
        shared_auth_provider: Option<String>,
        missing_scopes: Vec<String>,
        thread_id: Option<String>,
    },
    Error {
        message: String,
        code: Option<String>,
    },
    AgentMessage {
        content: String,
        message_type: String,
    },
    LifecycleStart {
        run_id: String,
    },
    LifecycleEnd {
        run_id: String,
        phase: String,
    },
    Custom {
        kind: String,
        data: serde_json::Value,
    },
}

/// Missing channel draft surface for dispatcher extraction.
#[async_trait]
pub trait ChannelDraftPort: Send + Sync {
    async fn stream_mode(&self, target: &ChannelTarget)
    -> Result<PortableStreamMode, ChannelError>;

    async fn send_draft(&self, update: ChannelDraftUpdate)
    -> Result<ChannelDraftAck, ChannelError>;

    async fn delete_draft(&self, request: ChannelDraftDelete) -> Result<(), ChannelError>;
}

/// Event-stream fanout surface. This does not replace `ChannelStatusPort`;
/// it is for transports that need serialized status frames.
#[async_trait]
pub trait ChannelStatusStreamPort: Send + Sync {
    async fn publish_status_frame(
        &self,
        frame: ChannelStatusStreamFrame,
    ) -> Result<(), ChannelError>;
}

/// Capacity snapshot for scheduling decisions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableSchedulerCapacity {
    pub running: usize,
    pub limit: usize,
}

impl PortableSchedulerCapacity {
    pub fn new(running: usize, limit: usize) -> Self {
        Self { running, limit }
    }

    pub fn allows_schedule(self) -> bool {
        self.running < self.limit
    }
}

/// Serializable worker control message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerControlCommand {
    Start,
    Stop,
    Ping,
}

/// Optional routine context attached to a scheduled worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRoutineContext {
    pub routine_id: Uuid,
    pub routine_name: String,
    pub routine_run_id: String,
    #[serde(default)]
    pub reserved_system_slot: bool,
}

/// Policy inputs for deciding whether a worker may be started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerScheduleRequest {
    pub job_id: Uuid,
    pub current_state: PortableJobState,
    pub capacity: PortableSchedulerCapacity,
    pub already_running: bool,
    pub requested_at: DateTime<Utc>,
    pub routine: Option<WorkerRoutineContext>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Scheduler decision for a worker start request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum WorkerScheduleDecision {
    Start {
        job_id: Uuid,
        command: WorkerControlCommand,
        transition: JobTransitionRequest,
        capacity: PortableSchedulerCapacity,
        routine: Option<WorkerRoutineContext>,
    },
    AlreadyRunning {
        job_id: Uuid,
    },
    RejectCapacity {
        job_id: Uuid,
        capacity: PortableSchedulerCapacity,
    },
    RejectInvalidState {
        job_id: Uuid,
        current_state: PortableJobState,
        reason: String,
    },
}

impl WorkerScheduleDecision {
    pub fn from_request(request: WorkerScheduleRequest) -> Self {
        if request.already_running {
            return Self::AlreadyRunning {
                job_id: request.job_id,
            };
        }

        if !request.capacity.allows_schedule() {
            return Self::RejectCapacity {
                job_id: request.job_id,
                capacity: request.capacity,
            };
        }

        if request.current_state.is_worker_terminal() {
            return Self::RejectInvalidState {
                job_id: request.job_id,
                current_state: request.current_state,
                reason: "worker terminal state cannot be scheduled".to_string(),
            };
        }

        let reason = if request
            .routine
            .as_ref()
            .is_some_and(|routine| routine.reserved_system_slot)
        {
            "Scheduled for execution (reserved slot)"
        } else {
            "Scheduled for execution"
        };

        Self::Start {
            job_id: request.job_id,
            command: WorkerControlCommand::Start,
            transition: JobTransitionRequest {
                job_id: request.job_id,
                target: PortableJobState::InProgress,
                reason: Some(reason.to_string()),
                transitioned_at: request.requested_at,
            },
            capacity: request.capacity,
            routine: request.routine,
        }
    }
}

/// Snapshot of a volatile worker slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRunSnapshot {
    pub job_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub stopping: bool,
}

/// Request to stop a worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStopRequest {
    pub job_id: Uuid,
    pub reason: String,
    pub requested_at: DateTime<Utc>,
    pub abort_after_ms: Option<u64>,
}

/// Result of a worker stop request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WorkerStopDecision {
    Stop {
        job_id: Uuid,
        command: WorkerControlCommand,
        transition: JobTransitionRequest,
        abort_after_ms: Option<u64>,
    },
    NotRunning {
        job_id: Uuid,
    },
}

impl WorkerStopDecision {
    pub fn stop(request: WorkerStopRequest) -> Self {
        Self::Stop {
            job_id: request.job_id,
            command: WorkerControlCommand::Stop,
            transition: JobTransitionRequest {
                job_id: request.job_id,
                target: PortableJobState::Cancelled,
                reason: Some(request.reason),
                transitioned_at: request.requested_at,
            },
            abort_after_ms: request.abort_after_ms,
        }
    }
}

/// Scheduler runner surface. This owns volatile worker slots, not durable job
/// snapshots.
#[async_trait]
pub trait WorkerSchedulingPort: Send + Sync {
    async fn schedule_worker(
        &self,
        request: WorkerScheduleRequest,
    ) -> Result<WorkerScheduleDecision, JobPortError>;

    async fn stop_worker(
        &self,
        request: WorkerStopRequest,
    ) -> Result<WorkerStopDecision, JobPortError>;

    async fn running_workers(&self) -> Result<Vec<WorkerRunSnapshot>, JobPortError>;

    async fn running_count(&self) -> Result<usize, JobPortError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_job_state_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&PortableJobState::InProgress).unwrap();
        assert_eq!(encoded, "\"in_progress\"");

        let decoded: PortableJobState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, PortableJobState::InProgress);
    }

    #[test]
    fn job_context_round_trips_without_runtime_env() {
        let scope = AgentScope::new("principal", "actor");
        let mut snapshot = PortableJobContext::new(scope, "Title", "Description");
        snapshot.metadata = serde_json::json!({ "source": "test" });
        snapshot.estimated_duration_secs = Some(30);

        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: PortableJobContext = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.title, "Title");
        assert_eq!(decoded.owner_actor_id(), "actor");
        assert_eq!(decoded.estimated_duration_secs, Some(30));
        assert_eq!(decoded.metadata["source"], "test");
    }

    #[test]
    fn draft_defaults_are_serializable_and_not_posted() {
        let draft = PortableDraftReplyState::new("discord");
        let value = serde_json::to_value(&draft).unwrap();

        assert_eq!(value["channel_id"], "discord");
        assert_eq!(value["posted"], false);
        assert_eq!(value["overflow"], false);
    }

    #[test]
    fn schedule_decision_rejects_full_capacity() {
        let request = WorkerScheduleRequest {
            job_id: Uuid::new_v4(),
            current_state: PortableJobState::Pending,
            capacity: PortableSchedulerCapacity::new(2, 2),
            already_running: false,
            requested_at: Utc::now(),
            routine: None,
            metadata: serde_json::Value::Null,
        };

        assert!(matches!(
            WorkerScheduleDecision::from_request(request),
            WorkerScheduleDecision::RejectCapacity { .. }
        ));
    }

    #[test]
    fn schedule_decision_starts_pending_job() {
        let request = WorkerScheduleRequest {
            job_id: Uuid::new_v4(),
            current_state: PortableJobState::Pending,
            capacity: PortableSchedulerCapacity::new(1, 2),
            already_running: false,
            requested_at: Utc::now(),
            routine: None,
            metadata: serde_json::Value::Null,
        };

        let decision = WorkerScheduleDecision::from_request(request);
        let encoded = serde_json::to_string(&decision).unwrap();

        assert!(encoded.contains("\"decision\":\"start\""));
        assert!(matches!(
            decision,
            WorkerScheduleDecision::Start {
                command: WorkerControlCommand::Start,
                ..
            }
        ));
    }

    #[test]
    fn staged_traits_are_object_safe() {
        fn assert_object_safe<T: ?Sized + Send + Sync>() {}

        assert_object_safe::<dyn JobContextStatePort>();
        assert_object_safe::<dyn JobStateStorePort>();
        assert_object_safe::<dyn ChannelDraftPort>();
        assert_object_safe::<dyn ChannelStatusStreamPort>();
        assert_object_safe::<dyn WorkerSchedulingPort>();
    }
}

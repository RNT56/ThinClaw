//! Persistence traits shared across ThinClaw crates.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

use thinclaw_experiments::{
    ExperimentArtifactRef, ExperimentCampaign, ExperimentLease, ExperimentModelUsageRecord,
    ExperimentProject, ExperimentRunnerProfile, ExperimentTarget, ExperimentTargetLink,
    ExperimentTrial,
};
use thinclaw_history::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationSummary,
    JobEventRecord, LearningArtifactVersion, LearningCandidate, LearningCodeProposal,
    LearningEvaluation, LearningEvent, LearningFeedbackRecord, LearningRollbackRecord,
    LlmCallRecord, OutcomeContract, OutcomeContractQuery, OutcomeEvaluatorHealth,
    OutcomeObservation, OutcomePendingUser, OutcomeSummaryStats, SessionSearchHit, SettingRow,
};
use thinclaw_identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, ActorStatus, NewActorEndpointRecord,
    NewActorRecord,
};
use thinclaw_repo_projects::{
    MergeGateDecision, RepoProject, RepoProjectEvent, RepoProjectRepo, RepoProjectRun,
    RepoProjectTask, RepoWebhookDelivery, RepoWorkerRun,
};
pub use thinclaw_types::AgentWorkspaceRecord;
pub use thinclaw_types::error::{DatabaseError, WorkspaceError};
use thinclaw_types::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};
use thinclaw_types::subagent::SubagentRunRecord;
use thinclaw_types::{
    ActionRecord, BrokenTool, JobContext, JobState, SandboxJobRecord, SandboxJobSummary,
};

#[cfg(feature = "libsql")]
pub mod libsql;
#[cfg(feature = "libsql")]
pub mod libsql_migrations;
#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "postgres")]
mod postgres_store;
#[cfg(feature = "postgres")]
mod postgres_workspace;

pub use thinclaw_workspace::WorkspaceStore;

// ==================== Sub-traits ====================
//
// Each sub-trait groups related persistence methods. The `Database` supertrait
// combines them all, so existing `Arc<dyn Database>` consumers keep working.
// Leaf consumers can depend on a specific sub-trait instead.

#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError>;
    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError>;
    #[allow(clippy::too_many_arguments)]
    async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError>;
    /// Record the exact user instruction sent to the model after trusted
    /// `BeforeLlmInput` hooks have transformed it. The raw row content remains
    /// unchanged for user-facing audit/history; implementations must update
    /// only the identified user row in the identified conversation.
    async fn set_effective_user_instruction(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        effective_instruction: &str,
    ) -> Result<(), DatabaseError>;
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError>;
    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_conversation_identity(
        &self,
        id: Uuid,
        principal_id: Option<&str>,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError>;
    /// Find the latest durable thread addressed by this ingress identity.
    ///
    /// Direct conversations are actor-private and, when an external thread is
    /// supplied, additionally channel/thread scoped. Group conversations are
    /// selected only by the exact principal + stable conversation scope. This
    /// is used to restore native non-UUID channel threads after a restart.
    #[allow(clippy::too_many_arguments)]
    async fn find_latest_conversation_for_ingress(
        &self,
        principal_id: &str,
        actor_id: &str,
        conversation_scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> Result<Option<Uuid>, DatabaseError>;
    async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError>;
    async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError>;
    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;
    /// Load an exact bounded slice from the append-only message log.
    ///
    /// `start_row` is a zero-based offset in chronological order and `limit`
    /// is the maximum number of rows returned. Conversation rows are never
    /// physically removed by thread lifecycle operations, which makes this
    /// window stable across undo, clear, compaction, and process restarts.
    async fn list_conversation_messages_window(
        &self,
        conversation_id: Uuid,
        start_row: i64,
        limit: i64,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;
    async fn count_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<i64, DatabaseError>;
    async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError>;
    async fn list_conversation_messages_for_learning(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        role: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError>;
    async fn insert_learning_event(&self, event: &LearningEvent) -> Result<Uuid, DatabaseError>;
    async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError>;
    async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError>;
    async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError>;
    async fn update_learning_candidate_proposal(
        &self,
        candidate_id: Uuid,
        proposal: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError>;
    async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError>;
    async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError>;
    async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError>;
    async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError>;
    async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError>;
    async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError>;
    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<Uuid, DatabaseError>;
    async fn get_outcome_contract(
        &self,
        user_id: &str,
        contract_id: Uuid,
    ) -> Result<Option<OutcomeContract>, DatabaseError>;
    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContract>, DatabaseError>;
    async fn claim_due_outcome_contracts(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError>;
    async fn claim_due_outcome_contracts_for_user(
        &self,
        user_id: &str,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError>;
    async fn claim_due_outcome_contracts_with_lease(
        &self,
        worker_id: &str,
        limit: i64,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Vec<OutcomeContract>, DatabaseError>;
    async fn claim_due_outcome_contracts_for_user_with_lease(
        &self,
        user_id: &str,
        worker_id: &str,
        limit: i64,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Vec<OutcomeContract>, DatabaseError>;
    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<(), DatabaseError>;
    async fn update_claimed_outcome_contract(
        &self,
        contract: &OutcomeContract,
        worker_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn outcome_summary_stats(
        &self,
        user_id: &str,
    ) -> Result<OutcomeSummaryStats, DatabaseError>;
    async fn list_users_with_pending_outcome_work(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomePendingUser>, DatabaseError>;
    async fn outcome_evaluator_health(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutcomeEvaluatorHealth, DatabaseError>;
    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservation,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservation>, DatabaseError>;
    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError>;
    /// Authorize a conversation against the complete ingress identity.
    /// Direct conversations are actor-owned; group conversations are shared
    /// only within the exact stable conversation scope.
    async fn conversation_belongs_to_identity(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
        conversation_scope_id: Uuid,
        conversation_kind: ConversationKind,
    ) -> Result<bool, DatabaseError>;

    /// Delete a conversation and all its messages (cascading).
    ///
    /// Returns `true` if the conversation existed and was deleted.
    async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError>;

    /// Delete all messages from a conversation without deleting the conversation itself.
    ///
    /// Returns the number of messages deleted.
    async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError>;
}

/// String-based identity trait — the legacy/ergonomic API layer.
///
/// # Dual-trait architecture
///
/// ThinClaw uses two identity traits that work together:
///
/// - **`IdentityStore`** (this trait): String-based `actor_id` parameters.
///   Consumed by CLI commands and external callers that work with string IDs.
///   Implemented via the `IdentityRegistryStore` bridge below.
///
/// - **`IdentityRegistryStore`**: UUID-based `actor_id` parameters.
///   The canonical trait that database backends implement directly.
///
/// The blanket impl at the bottom of this file bridges `IdentityRegistryStore`
/// → `IdentityStore` by parsing `&str` → `Uuid` and delegating. The `Database`
/// supertrait requires **both** traits, so any `dyn Database` is guaranteed to
/// have the full UUID-based implementation.
#[async_trait]
pub trait IdentityStore: Send + Sync {
    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError>;
    async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError>;
    async fn upsert_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError>;
    async fn rename_actor(&self, actor_id: &str, display_name: &str) -> Result<(), DatabaseError>;
    async fn set_actor_preferred_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
    ) -> Result<(), DatabaseError>;
    async fn link_actor_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
        metadata: &serde_json::Value,
        approval_status: &str,
    ) -> Result<(), DatabaseError>;
    async fn unlink_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn list_actor_endpoints(
        &self,
        actor_id: &str,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError>;
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;
    async fn list_jobs_for_user(&self, user_id: &str) -> Result<Vec<JobContext>, DatabaseError>;
    async fn list_jobs_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
    ) -> Result<Vec<JobContext>, DatabaseError> {
        let jobs = self.list_jobs_for_user(user_id).await?;
        Ok(jobs
            .into_iter()
            .filter(|job| job.owner_actor_id() == actor_id)
            .collect())
    }
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn abandon_active_direct_jobs(&self, reason: &str) -> Result<u64, DatabaseError>;
    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError>;
    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError>;
    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError>;
    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError>;
    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SandboxStore: Send + Sync {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError>;
    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError>;
    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    /// Atomically win a sandbox job's terminal transition and append its
    /// durable result event. Returns `false` when another terminal transition
    /// already won (or the job is not an active sandbox job).
    async fn finalize_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: bool,
        message: Option<&str>,
        completed_at: DateTime<Utc>,
        event_data: &serde_json::Value,
    ) -> Result<bool, DatabaseError>;
    async fn cleanup_stale_sandbox_jobs(&self, runtime_scope: &str) -> Result<u64, DatabaseError>;
    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError>;
    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn list_sandbox_jobs_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let jobs = self.list_sandbox_jobs_for_user(user_id).await?;
        Ok(jobs
            .into_iter()
            .filter(|job| job.spec.actor_id == actor_id)
            .collect())
    }
    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError>;
    async fn sandbox_job_summary_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let jobs = self.list_sandbox_jobs_for_actor(user_id, actor_id).await?;
        let mut summary = SandboxJobSummary::default();
        for job in jobs {
            summary.total += 1;
            match job.status.as_str() {
                "creating" => summary.creating += 1,
                "running" => summary.running += 1,
                "completed" => summary.completed += 1,
                "failed" => summary.failed += 1,
                "cancelled" => summary.cancelled += 1,
                "interrupted" => summary.interrupted += 1,
                "stuck" => summary.stuck += 1,
                _ => {}
            }
        }
        Ok(summary)
    }
    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn sandbox_job_belongs_to_actor(
        &self,
        job_id: Uuid,
        user_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        let Some(job) = self.get_sandbox_job(job_id).await? else {
            return Ok(false);
        };
        Ok(job.spec.principal_id == user_id && job.spec.actor_id == actor_id)
    }
    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError>;
    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError>;
    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError>;
}

/// Fallback TTL (seconds) applied to legacy `routine_runs` rows with a NULL
/// `lease_expires_at` when reaping zombie runs. Callers that don't have a
/// more specific value (e.g. `AGENT_JOB_TIMEOUT_SECS`) should use this.
pub const DEFAULT_LEGACY_ROUTINE_RUN_TTL_SECS: i64 = 3600;

/// Result of atomically admitting a routine run against both the routine-local
/// and process-global durable capacity limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineRunAdmission {
    Admitted,
    /// An existing run already owns the same `(routine_id, trigger_key)`.
    /// Returning its id makes retried trigger delivery idempotent.
    Duplicate(Uuid),
    RoutineCapacity,
    GlobalCapacity,
}

/// Result of the terminal compare-and-set for a routine run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineRunCompletion {
    /// The run was already terminal (or no longer exists), so no runtime
    /// counters were changed.
    AlreadyTerminal,
    /// This call won the terminal transition and atomically updated the
    /// routine's consecutive-failure counter.
    Completed {
        routine_id: Uuid,
        consecutive_failures: u32,
    },
}

/// Durable effects produced while reaping expired routine-run leases.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutineRunReapResult {
    pub reaped: u64,
    /// Final failure streak for each transitioned run. Repeated routine ids
    /// are intentional when several expired concurrent runs are closed; policy
    /// application is compare-and-set against each exact streak.
    pub failure_streaks: Vec<(Uuid, u32)>,
}

#[async_trait]
pub trait RoutineStore: Send + Sync {
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
        Ok(routine.filter(|r| r.owner_actor_id() == actor_id))
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
    /// Advance a run-less system-event routine without writing a stale full
    /// routine snapshot over concurrent runtime updates.
    async fn advance_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    /// Move only the next-fire cursor (for catch-up collapse/skip paths).
    async fn set_routine_next_fire_at(
        &self,
        id: Uuid,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;
    /// Atomically enforce durable routine/global capacity, deduplicate a
    /// trigger key, create the run with its initial lease, and advance the
    /// parent routine's schedule/counter exactly once.
    async fn try_admit_routine_run(
        &self,
        run: &RoutineRun,
        routine_limit: i64,
        global_limit: i64,
        initial_lease_expires_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<RoutineRunAdmission, DatabaseError>;
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<RoutineRunCompletion, DatabaseError>;
    /// Apply failure backoff/auto-disable only if the failure streak still
    /// equals the value returned by `complete_routine_run`. This prevents a
    /// stale failed finalizer from overriding a newer successful completion.
    async fn apply_routine_failure_policy(
        &self,
        routine_id: Uuid,
        expected_consecutive_failures: u32,
        not_before: DateTime<Utc>,
        disable: bool,
    ) -> Result<bool, DatabaseError>;
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;

    /// Count ALL routine runs currently in `running` status across all routines.
    ///
    /// Used by the routine engine for global concurrency gating. This is the
    /// single source of truth — replacing the previous fragile `AtomicUsize`
    /// counter that drifted out of sync with DB state.
    async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError>;
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;

    /// Renew (or set) the lease on a RUNNING routine run.
    ///
    /// Workers and subagents call this periodically (on each iteration or
    /// keepalive tick) while actively executing a routine run, and the
    /// engine sets an initial lease at spawn time for lightweight/immediate
    /// runs. `lease_secs` is the duration from now until the new expiry.
    ///
    /// This is what lets [`RoutineStore::cleanup_stale_routine_runs`] reap
    /// only genuinely orphaned runs instead of any run older than a fixed
    /// wall-clock cutoff — full-job routine runs can legitimately stay
    /// RUNNING for up to `AGENT_JOB_TIMEOUT_SECS` (default 3600s).
    async fn renew_routine_run_lease(
        &self,
        run_id: Uuid,
        lease_secs: i64,
    ) -> Result<(), DatabaseError>;

    /// Mark RUNNING routine runs with an expired lease as failed.
    ///
    /// Used by the zombie reaper to clean up orphaned runs whose worker
    /// crashed, hung, or was killed mid-execution. A run is reaped only
    /// when its `lease_expires_at` has passed. Legacy rows with a NULL
    /// lease (e.g. from before this column existed, or runs that never
    /// renewed) fall back to `legacy_ttl_secs` measured from `started_at`
    /// instead of a fixed 10-minute cutoff.
    ///
    /// At startup, this is called to clean up runs from a previous process.
    async fn cleanup_stale_routine_runs(
        &self,
        legacy_ttl_secs: i64,
    ) -> Result<RoutineRunReapResult, DatabaseError>;

    /// Delete all run records for a specific routine.
    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError>;

    /// Delete ALL routine run records across all routines.
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
        next_attempt_at: DateTime<Utc>,
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
    async fn dead_letter_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn replay_routine_event(
        &self,
        id: Uuid,
        user_id: &str,
        actor_id: &str,
        diagnostics: &serde_json::Value,
    ) -> Result<Option<RoutineEvent>, DatabaseError>;
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
    /// Returns true when this routine has already fired for an event whose
    /// content hash matches `content_hash` since `since`. Backs the
    /// `RoutineGuardrails.dedup_window` content dedup so semantically duplicate
    /// distinct events within the window fire only once.
    async fn routine_event_recent_content_match(
        &self,
        routine_id: Uuid,
        content_hash: &str,
        since: DateTime<Utc>,
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
        next_attempt_at: DateTime<Utc>,
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

/// Durable ledger for in-process sub-agent runs (`SubagentExecutor`).
///
/// Running sub-agents previously lived only in an in-memory map, so a
/// process restart silently dropped in-flight delegated work — including
/// any routine run a sub-agent was finalizing. This store gives the
/// executor a durable record: written on spawn, updated on completion, and
/// reconciled at startup for rows orphaned by a crash.
#[async_trait]
pub trait SubagentRunStore: Send + Sync {
    /// Record a sub-agent run starting.
    async fn insert_subagent_run(&self, run: &SubagentRunRecord) -> Result<(), DatabaseError>;

    /// Mark a sub-agent run as finished (success, failure, timeout, or
    /// cancellation). `status` should be one of the `SUBAGENT_RUN_STATUS_*`
    /// constants in `thinclaw_types::subagent`.
    /// First-write-wins: only a row still in `running` is updated, so a
    /// racing second completion (e.g. grace-abort fallback vs the task's own
    /// finalization) cannot overwrite the first recorded terminal outcome.
    async fn complete_subagent_run(
        &self,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DatabaseError>;

    /// List all sub-agent runs still marked `running`.
    ///
    /// Used at startup to reconcile rows left behind by a crash — see
    /// `reconcile_orphaned_subagent_runs` in `src/agent/subagent_executor.rs`.
    async fn list_incomplete_subagent_runs(&self) -> Result<Vec<SubagentRunRecord>, DatabaseError>;
}

#[async_trait]
pub trait IdentityRegistryStore: Send + Sync {
    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError>;
    async fn get_actor(&self, actor_id: Uuid) -> Result<Option<ActorRecord>, DatabaseError>;
    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError>;
    async fn update_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError>;
    async fn delete_actor(&self, actor_id: Uuid) -> Result<bool, DatabaseError>;
    async fn rename_actor(&self, actor_id: Uuid, display_name: &str) -> Result<(), DatabaseError>;
    async fn set_actor_status(
        &self,
        actor_id: Uuid,
        status: ActorStatus,
    ) -> Result<(), DatabaseError>;
    async fn set_actor_preferred_delivery_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError>;
    async fn set_actor_last_active_direct_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError>;
    async fn upsert_actor_endpoint(
        &self,
        record: &NewActorEndpointRecord,
    ) -> Result<ActorEndpointRecord, DatabaseError>;
    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError>;
    async fn list_actor_endpoints(
        &self,
        actor_id: Uuid,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError>;
    async fn delete_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn resolve_actor_for_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorRecord>, DatabaseError>;
}

#[async_trait]
impl<T> IdentityStore for T
where
    T: IdentityRegistryStore + Send + Sync,
{
    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError> {
        IdentityRegistryStore::list_actors(self, principal_id).await
    }

    async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError> {
        let actor_id = Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Serialization(format!("invalid actor_id: {e}")))?;
        IdentityRegistryStore::get_actor(self, actor_id).await
    }

    async fn upsert_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError> {
        if IdentityRegistryStore::get_actor(self, actor.actor_id)
            .await?
            .is_some()
        {
            return IdentityRegistryStore::update_actor(self, actor).await;
        }

        let new_actor = NewActorRecord {
            principal_id: actor.principal_id.clone(),
            display_name: actor.display_name.clone(),
            status: actor.status,
            preferred_delivery_endpoint: actor.preferred_delivery_endpoint.clone(),
            last_active_direct_endpoint: actor.last_active_direct_endpoint.clone(),
        };
        let _ = IdentityRegistryStore::create_actor(self, &new_actor).await?;
        Ok(())
    }

    async fn rename_actor(&self, actor_id: &str, display_name: &str) -> Result<(), DatabaseError> {
        let actor_id = Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Serialization(format!("invalid actor_id: {e}")))?;
        IdentityRegistryStore::rename_actor(self, actor_id, display_name).await
    }

    async fn set_actor_preferred_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
    ) -> Result<(), DatabaseError> {
        let actor_id = Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Serialization(format!("invalid actor_id: {e}")))?;
        let endpoint = ActorEndpointRef::new(channel, external_user_id);
        IdentityRegistryStore::set_actor_preferred_delivery_endpoint(
            self,
            actor_id,
            Some(&endpoint),
        )
        .await
    }

    async fn link_actor_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
        metadata: &serde_json::Value,
        approval_status: &str,
    ) -> Result<(), DatabaseError> {
        let actor_id = Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Serialization(format!("invalid actor_id: {e}")))?;
        let approval_status = approval_status
            .parse()
            .map_err(|e: String| DatabaseError::Serialization(e))?;
        let record = NewActorEndpointRecord {
            endpoint: ActorEndpointRef::new(channel, external_user_id),
            actor_id,
            metadata: metadata.clone(),
            approval_status,
        };
        IdentityRegistryStore::upsert_actor_endpoint(self, &record).await?;
        Ok(())
    }

    async fn unlink_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        IdentityRegistryStore::delete_actor_endpoint(self, channel, external_user_id).await
    }

    async fn list_actor_endpoints(
        &self,
        actor_id: &str,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError> {
        let actor_id = Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Serialization(format!("invalid actor_id: {e}")))?;
        IdentityRegistryStore::list_actor_endpoints(self, actor_id).await
    }
}

#[async_trait]
pub trait ToolFailureStore: Send + Sync {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError>;
    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn quarantine_tool_failure(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn record_tool_repair_result(
        &self,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait ExperimentStore: Send + Sync {
    async fn create_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError>;
    async fn get_experiment_project(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentProject>, DatabaseError>;
    async fn get_experiment_project_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentProject>, DatabaseError>;
    async fn list_experiment_projects(&self) -> Result<Vec<ExperimentProject>, DatabaseError>;
    async fn list_experiment_projects_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentProject>, DatabaseError>;
    async fn update_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError>;
    async fn delete_experiment_project(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn delete_experiment_project_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<bool, DatabaseError>;

    async fn create_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError>;
    async fn get_experiment_runner_profile(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError>;
    async fn get_experiment_runner_profile_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError>;
    async fn list_experiment_runner_profiles(
        &self,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError>;
    async fn list_experiment_runner_profiles_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError>;
    async fn update_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError>;
    async fn delete_experiment_runner_profile(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn delete_experiment_runner_profile_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<bool, DatabaseError>;

    async fn create_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError>;
    async fn get_experiment_campaign(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError>;
    async fn get_experiment_campaign_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError>;
    async fn list_experiment_campaigns(&self) -> Result<Vec<ExperimentCampaign>, DatabaseError>;
    async fn list_experiment_campaigns_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentCampaign>, DatabaseError>;
    async fn update_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError>;

    async fn create_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError>;
    async fn get_experiment_trial(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTrial>, DatabaseError>;
    async fn get_experiment_trial_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentTrial>, DatabaseError>;
    async fn list_experiment_trials(
        &self,
        campaign_id: Uuid,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError>;
    async fn list_experiment_trials_for_owner(
        &self,
        campaign_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError>;
    async fn update_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError>;

    async fn replace_experiment_artifacts(
        &self,
        trial_id: Uuid,
        artifacts: &[ExperimentArtifactRef],
    ) -> Result<(), DatabaseError>;
    async fn list_experiment_artifacts(
        &self,
        trial_id: Uuid,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError>;
    async fn list_experiment_artifacts_for_owner(
        &self,
        trial_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError>;

    async fn create_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError>;
    async fn get_experiment_target(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTarget>, DatabaseError>;
    async fn list_experiment_targets(&self) -> Result<Vec<ExperimentTarget>, DatabaseError>;
    async fn update_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError>;
    async fn delete_experiment_target(&self, id: Uuid) -> Result<bool, DatabaseError>;

    async fn upsert_experiment_target_link(
        &self,
        link: &ExperimentTargetLink,
    ) -> Result<(), DatabaseError>;
    async fn list_experiment_target_links(
        &self,
    ) -> Result<Vec<ExperimentTargetLink>, DatabaseError>;
    async fn delete_experiment_target_links_for_target(
        &self,
        target_id: Uuid,
    ) -> Result<(), DatabaseError>;

    async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError>;
    async fn list_experiment_model_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError>;
    async fn list_experiment_model_usage_for_campaign(
        &self,
        campaign_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError>;
    async fn list_experiment_model_usage_for_trial(
        &self,
        trial_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError>;

    async fn create_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError>;
    async fn get_experiment_lease(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError>;
    async fn get_experiment_lease_for_trial(
        &self,
        trial_id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError>;
    async fn update_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait RepoProjectStore: Send + Sync {
    async fn create_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError>;
    async fn get_repo_project(&self, id: Uuid) -> Result<Option<RepoProject>, DatabaseError>;
    async fn list_repo_projects(&self) -> Result<Vec<RepoProject>, DatabaseError>;
    async fn update_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError>;
    async fn delete_repo_project(&self, id: Uuid) -> Result<bool, DatabaseError>;

    async fn upsert_repo_project_repo(&self, repo: &RepoProjectRepo) -> Result<(), DatabaseError>;
    async fn list_repo_project_repos(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRepo>, DatabaseError>;

    async fn upsert_repo_project_task(&self, task: &RepoProjectTask) -> Result<(), DatabaseError>;
    async fn get_repo_project_task(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectTask>, DatabaseError>;
    async fn list_repo_project_tasks(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectTask>, DatabaseError>;

    async fn upsert_repo_worker_run(&self, run: &RepoWorkerRun) -> Result<(), DatabaseError>;
    async fn list_repo_worker_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoWorkerRun>, DatabaseError>;

    async fn append_repo_project_event(
        &self,
        event: &RepoProjectEvent,
    ) -> Result<(), DatabaseError>;
    async fn list_repo_project_events(
        &self,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RepoProjectEvent>, DatabaseError>;

    async fn upsert_repo_merge_gate_decision(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        decision: &MergeGateDecision,
    ) -> Result<(), DatabaseError>;
    async fn list_repo_merge_gate_decisions(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<(Uuid, MergeGateDecision)>, DatabaseError>;

    /// Persist a received GitHub webhook delivery. Returns `true` when the
    /// delivery was newly recorded and `false` when it was already present
    /// (a duplicate/redelivery), giving restart-surviving idempotency.
    async fn record_repo_webhook_delivery(
        &self,
        delivery: &RepoWebhookDelivery,
    ) -> Result<bool, DatabaseError>;
    async fn get_repo_webhook_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<Option<RepoWebhookDelivery>, DatabaseError>;
    async fn list_repo_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<RepoWebhookDelivery>, DatabaseError>;

    async fn upsert_repo_project_run(&self, run: &RepoProjectRun) -> Result<(), DatabaseError>;
    async fn get_repo_project_run(&self, id: Uuid)
    -> Result<Option<RepoProjectRun>, DatabaseError>;
    async fn list_repo_project_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRun>, DatabaseError>;
}

#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError>;
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError>;
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError>;
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

// ==================== Agent Registry ====================

#[async_trait]
pub trait AgentRegistryStore: Send + Sync {
    /// Save (insert) a new agent workspace.
    async fn save_agent_workspace(&self, ws: &AgentWorkspaceRecord) -> Result<(), DatabaseError>;

    /// Get an agent workspace by its human-readable `agent_id`.
    async fn get_agent_workspace(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentWorkspaceRecord>, DatabaseError>;

    /// List all agent workspaces.
    async fn list_agent_workspaces(&self) -> Result<Vec<AgentWorkspaceRecord>, DatabaseError>;

    /// Delete an agent workspace by `agent_id`. Returns true if it existed.
    async fn delete_agent_workspace(&self, agent_id: &str) -> Result<bool, DatabaseError>;

    /// Update an existing agent workspace (matched by `agent_id`).
    async fn update_agent_workspace(&self, ws: &AgentWorkspaceRecord) -> Result<(), DatabaseError>;
}

/// Backend-agnostic database supertrait.
///
/// Combines all sub-traits into one. Existing `Arc<dyn Database>` consumers
/// continue to work; leaf consumers can depend on a specific sub-trait instead.
#[async_trait]
pub trait Database:
    ConversationStore
    + IdentityStore
    + JobStore
    + SandboxStore
    + RoutineStore
    + SubagentRunStore
    + IdentityRegistryStore
    + ToolFailureStore
    + ExperimentStore
    + RepoProjectStore
    + SettingsStore
    + WorkspaceStore
    + AgentRegistryStore
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    /// Lightweight readiness ping that exercises the database connection.
    ///
    /// The default performs a cheap bounded read (a metadata lookup for a
    /// non-existent id) so it round-trips to a real backend without depending on
    /// any seeded data: a healthy backend returns `Ok(None)` → `Ok(())`, while a
    /// connection failure surfaces as `Err`. Readiness probes treat `Err` (or a
    /// timeout applied by the caller) as not-ready. In-memory/mock backends with
    /// no connection to fail are free to keep the default.
    async fn health_check(&self) -> Result<(), DatabaseError> {
        self.get_conversation_metadata(Uuid::nil())
            .await
            .map(|_| ())
    }

    /// Create a portable snapshot (backup) of the database.
    ///
    /// For file-based backends: flushes the WAL, then copies the database
    /// file to the given path. The destination file is a self-contained
    /// SQLite database that can be restored by overwriting the original.
    ///
    /// Returns the number of bytes written, or an error if the backend
    /// does not support snapshotting (e.g., in-memory databases).
    async fn snapshot(&self, dest: &std::path::Path) -> Result<u64, DatabaseError>;

    /// Return the path to the database file, if file-backed.
    fn db_path(&self) -> Option<&std::path::Path>;
}

//! PostgreSQL backend for the Database trait.
//!
//! Delegates to the existing `Store` (history) and `Repository` (workspace)
//! implementations, avoiding SQL duplication.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::config::DatabaseConfig;
use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::{
    AgentRegistryStore, AgentWorkspaceRecord, ConversationStore, Database, ExperimentStore,
    IdentityRegistryStore, JobStore, RoutineStore, SandboxStore, SettingsStore, ToolFailureStore,
    WorkspaceStore,
};
use crate::error::{DatabaseError, WorkspaceError};
use crate::experiments::{
    ExperimentArtifactRef, ExperimentCampaign, ExperimentLease, ExperimentModelUsageRecord,
    ExperimentProject, ExperimentRunnerProfile, ExperimentTarget, ExperimentTargetLink,
    ExperimentTrial,
};
use crate::history::{
    ConversationMessage, ConversationSummary, JobEventRecord, LearningArtifactVersion,
    LearningCandidate, LearningCodeProposal, LearningEvaluation, LearningEvent,
    LearningFeedbackRecord, LearningRollbackRecord, LlmCallRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeEvaluatorHealth, OutcomeObservation, OutcomePendingUser,
    OutcomeSummaryStats, SandboxJobRecord, SandboxJobSummary, SessionSearchHit, SettingRow, Store,
};
use crate::identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, ActorStatus, EndpointApprovalStatus,
    NewActorEndpointRecord, NewActorRecord,
};
use crate::workspace::{
    MemoryChunk, MemoryDocument, Repository, SearchConfig, SearchResult, WorkspaceEntry,
};

/// PostgreSQL database backend.
///
/// Wraps the existing `Store` (for history/conversations/jobs/routines/settings)
/// and `Repository` (for workspace documents/chunks/search) to implement the
/// unified `Database` trait.
pub struct PgBackend {
    store: Store,
    repo: Repository,
}

impl PgBackend {
    /// Create a new PostgreSQL backend from configuration.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let store = Store::new(config).await?;
        let repo = Repository::new(store.pool());
        Ok(Self { store, repo })
    }

    /// Get a clone of the connection pool.
    ///
    /// Useful for sharing with components that still need raw pool access.
    pub fn pool(&self) -> Pool {
        self.store.pool()
    }
}

// ==================== Database (supertrait) ====================

#[async_trait]
impl Database for PgBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        self.store.run_migrations().await
    }

    async fn snapshot(&self, _dest: &std::path::Path) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "Snapshotting is not supported for PostgreSQL backends. Use pg_dump instead."
                .to_string(),
        ))
    }

    fn db_path(&self) -> Option<&std::path::Path> {
        None // PostgreSQL is not file-backed
    }
}

// ==================== ConversationStore ====================

#[async_trait]
impl ConversationStore for PgBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation(channel, user_id, thread_id)
            .await
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.touch_conversation(id).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message_with_attribution(
                conversation_id,
                role,
                content,
                actor_id,
                actor_display_name,
                raw_sender_id,
                metadata,
            )
            .await
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .ensure_conversation(id, channel, user_id, thread_id)
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_with_preview(user_id, channel, limit)
            .await
    }

    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError> {
        self.store.infer_primary_user_id_for_channel(channel).await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_assistant_conversation(user_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation_with_metadata(channel, user_id, metadata)
            .await
    }

    async fn update_conversation_identity(
        &self,
        id: Uuid,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: crate::history::ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_identity(
                id,
                actor_id,
                conversation_scope_id,
                conversation_kind,
                stable_external_conversation_key,
            )
            .await
    }

    async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &crate::history::ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError> {
        self.store
            .set_conversation_handoff_metadata(id, handoff)
            .await
    }

    async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_actor_conversations_for_recall(
                principal_id,
                actor_id,
                include_group_history,
                limit,
            )
            .await
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        self.store
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_conversation_metadata(id).await
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.store.list_conversation_messages(conversation_id).await
    }

    async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        self.store
            .search_conversation_messages(user_id, query, actor_id, channel, thread_id, limit)
            .await
    }

    async fn list_conversation_messages_for_learning(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        role: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        self.store
            .list_conversation_messages_for_learning(
                user_id, actor_id, channel, thread_id, role, limit,
            )
            .await
    }

    async fn insert_learning_event(&self, event: &LearningEvent) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_event(event).await
    }

    async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError> {
        self.store
            .list_learning_events(user_id, actor_id, channel, thread_id, limit)
            .await
    }

    async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_evaluation(evaluation).await
    }

    async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError> {
        self.store.list_learning_evaluations(user_id, limit).await
    }

    async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_candidate(candidate).await
    }

    async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError> {
        self.store
            .list_learning_candidates(user_id, candidate_type, risk_tier, limit)
            .await
    }

    async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_artifact_version(version).await
    }

    async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError> {
        self.store
            .list_learning_artifact_versions(user_id, artifact_type, artifact_name, limit)
            .await
    }

    async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_feedback(feedback).await
    }

    async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError> {
        self.store
            .list_learning_feedback(user_id, target_type, target_id, limit)
            .await
    }

    async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_rollback(rollback).await
    }

    async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError> {
        self.store
            .list_learning_rollbacks(user_id, artifact_type, artifact_name, limit)
            .await
    }

    async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_code_proposal(proposal).await
    }

    async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError> {
        self.store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
    }

    async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError> {
        self.store
            .list_learning_code_proposals(user_id, status, limit)
            .await
    }

    async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_learning_code_proposal(proposal_id, status, branch_name, pr_url, metadata)
            .await
    }

    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_outcome_contract(contract).await
    }

    async fn get_outcome_contract(
        &self,
        user_id: &str,
        contract_id: Uuid,
    ) -> Result<Option<OutcomeContract>, DatabaseError> {
        self.store.get_outcome_contract(user_id, contract_id).await
    }

    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.store.list_outcome_contracts(query).await
    }

    async fn claim_due_outcome_contracts(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.store.claim_due_outcome_contracts(limit, now).await
    }

    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<(), DatabaseError> {
        self.store.update_outcome_contract(contract).await
    }

    async fn outcome_summary_stats(
        &self,
        user_id: &str,
    ) -> Result<OutcomeSummaryStats, DatabaseError> {
        self.store.outcome_summary_stats(user_id).await
    }

    async fn list_users_with_pending_outcome_work(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomePendingUser>, DatabaseError> {
        self.store.list_users_with_pending_outcome_work(now).await
    }

    async fn outcome_evaluator_health(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutcomeEvaluatorHealth, DatabaseError> {
        self.store.outcome_evaluator_health(user_id, now).await
    }

    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservation,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_outcome_observation(observation).await
    }

    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservation>, DatabaseError> {
        self.store.list_outcome_observations(contract_id).await
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_user(conversation_id, user_id)
            .await
    }

    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_actor(conversation_id, principal_id, actor_id)
            .await
    }

    async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_conversation(id).await
    }

    async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        self.store
            .delete_conversation_messages(conversation_id)
            .await
    }
}

// ==================== JobStore ====================

#[async_trait]
impl JobStore for PgBackend {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.store.save_job(ctx).await
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.store.get_job(id).await
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_job_status(id, status, failure_reason)
            .await
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.mark_job_stuck(id).await
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.store.get_stuck_jobs().await
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        self.store.save_action(job_id, action).await
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.store.get_job_actions(job_id).await
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.store.record_llm_call(record).await
    }

    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .save_estimation_snapshot(
                job_id,
                category,
                tool_names,
                estimated_cost,
                estimated_time_secs,
                estimated_value,
            )
            .await
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_estimation_actuals(id, actual_cost, actual_time_secs, actual_value)
            .await
    }
}

// ==================== SandboxStore ====================

#[async_trait]
impl SandboxStore for PgBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.store.save_sandbox_job(job).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.store.get_sandbox_job(id).await
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs().await
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_sandbox_job_status(id, status, success, message, started_at, completed_at)
            .await
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.store.cleanup_stale_sandbox_jobs().await
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary().await
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .sandbox_job_belongs_to_user(job_id, user_id)
            .await
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.store.update_sandbox_job_mode(id, mode).await
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.store.get_sandbox_job_mode(id).await
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.save_job_event(job_id, event_type, data).await
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        self.store.list_job_events(job_id, limit).await
    }
}

// ==================== RoutineStore ====================

#[async_trait]
impl RoutineStore for PgBackend {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.create_routine(routine).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine_by_name(user_id, name).await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_routines(user_id).await
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_event_routines().await
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_due_cron_routines().await
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.update_routine(routine).await
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_routine_runtime(
                id,
                last_run_at,
                next_fire_at,
                run_count,
                consecutive_failures,
                state,
            )
            .await
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_routine(id).await
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.store.create_routine_run(run).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        self.store
            .complete_routine_run(id, status, result_summary, tokens_used)
            .await
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.store.list_routine_runs(routine_id, limit).await
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.store.count_running_routine_runs(routine_id).await
    }

    async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError> {
        self.store.count_all_running_routine_runs().await
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store.link_routine_run_to_job(run_id, job_id).await
    }

    async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError> {
        self.store.cleanup_stale_routine_runs().await
    }

    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError> {
        self.store.delete_routine_runs(routine_id).await
    }

    async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError> {
        self.store.delete_all_routine_runs().await
    }
}

// ==================== ExperimentStore ====================

#[async_trait]
impl ExperimentStore for PgBackend {
    async fn create_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_project(project).await
    }

    async fn get_experiment_project(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentProject>, DatabaseError> {
        self.store.get_experiment_project(id).await
    }

    async fn list_experiment_projects(&self) -> Result<Vec<ExperimentProject>, DatabaseError> {
        self.store.list_experiment_projects().await
    }

    async fn update_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_project(project).await
    }

    async fn delete_experiment_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_project(id).await
    }

    async fn create_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_runner_profile(profile).await
    }

    async fn get_experiment_runner_profile(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError> {
        self.store.get_experiment_runner_profile(id).await
    }

    async fn list_experiment_runner_profiles(
        &self,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError> {
        self.store.list_experiment_runner_profiles().await
    }

    async fn update_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_runner_profile(profile).await
    }

    async fn delete_experiment_runner_profile(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_runner_profile(id).await
    }

    async fn create_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_campaign(campaign).await
    }

    async fn get_experiment_campaign(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        self.store.get_experiment_campaign(id).await
    }

    async fn list_experiment_campaigns(&self) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        self.store.list_experiment_campaigns().await
    }

    async fn update_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_campaign(campaign).await
    }

    async fn create_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        self.store.create_experiment_trial(trial).await
    }

    async fn get_experiment_trial(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        self.store.get_experiment_trial(id).await
    }

    async fn list_experiment_trials(
        &self,
        campaign_id: Uuid,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        self.store.list_experiment_trials(campaign_id).await
    }

    async fn update_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        self.store.update_experiment_trial(trial).await
    }

    async fn replace_experiment_artifacts(
        &self,
        trial_id: Uuid,
        artifacts: &[ExperimentArtifactRef],
    ) -> Result<(), DatabaseError> {
        self.store
            .replace_experiment_artifacts(trial_id, artifacts)
            .await
    }

    async fn list_experiment_artifacts(
        &self,
        trial_id: Uuid,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        self.store.list_experiment_artifacts(trial_id).await
    }

    async fn create_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_target(target).await
    }

    async fn get_experiment_target(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTarget>, DatabaseError> {
        self.store.get_experiment_target(id).await
    }

    async fn list_experiment_targets(&self) -> Result<Vec<ExperimentTarget>, DatabaseError> {
        self.store.list_experiment_targets().await
    }

    async fn update_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_target(target).await
    }

    async fn delete_experiment_target(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_target(id).await
    }

    async fn upsert_experiment_target_link(
        &self,
        link: &ExperimentTargetLink,
    ) -> Result<(), DatabaseError> {
        self.store.upsert_experiment_target_link(link).await
    }

    async fn list_experiment_target_links(
        &self,
    ) -> Result<Vec<ExperimentTargetLink>, DatabaseError> {
        self.store.list_experiment_target_links().await
    }

    async fn delete_experiment_target_links_for_target(
        &self,
        target_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store
            .delete_experiment_target_links_for_target(target_id)
            .await
    }

    async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_model_usage(usage).await
    }

    async fn list_experiment_model_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store.list_experiment_model_usage(limit).await
    }

    async fn list_experiment_model_usage_for_campaign(
        &self,
        campaign_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store
            .list_experiment_model_usage_for_campaign(campaign_id, limit)
            .await
    }

    async fn list_experiment_model_usage_for_trial(
        &self,
        trial_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store
            .list_experiment_model_usage_for_trial(trial_id, limit)
            .await
    }

    async fn create_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        self.store.create_experiment_lease(lease).await
    }

    async fn get_experiment_lease(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        self.store.get_experiment_lease(id).await
    }

    async fn get_experiment_lease_for_trial(
        &self,
        trial_id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        self.store.get_experiment_lease_for_trial(trial_id).await
    }

    async fn update_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        self.store.update_experiment_lease(lease).await
    }
}

// ==================== ToolFailureStore ====================

#[async_trait]
impl ToolFailureStore for PgBackend {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.store
            .record_tool_failure(tool_name, error_message)
            .await
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.store.get_broken_tools(threshold).await
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.mark_tool_repaired(tool_name).await
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.increment_repair_attempts(tool_name).await
    }
}

// ==================== SettingsStore ====================

#[async_trait]
impl SettingsStore for PgBackend {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_setting(user_id, key).await
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.store.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.set_setting(user_id, key, value).await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        self.store.delete_setting(user_id, key).await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        self.store.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.has_settings(user_id).await
    }
}

// ==================== WorkspaceStore ====================

#[async_trait]
impl WorkspaceStore for PgBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        self.repo.get_document_by_id(id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_or_create_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        self.repo.update_document(id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        self.repo
            .delete_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.repo.list_directory(user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        self.repo.list_all_paths(user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.repo.list_documents(user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        self.repo.delete_chunks(document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        self.repo
            .insert_chunk(document_id, chunk_index, content, embedding)
            .await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        self.repo.update_chunk_embedding(chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        self.repo
            .get_chunks_without_embeddings(user_id, agent_id, limit)
            .await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.repo
            .hybrid_search(user_id, agent_id, query, embedding, config)
            .await
    }
}

// ==================== IdentityRegistryStore ====================

const PG_ACTOR_COLUMNS: &str = "\
    actor_id, principal_id, display_name, status, \
    preferred_delivery_channel, preferred_delivery_external_user_id, \
    last_active_direct_channel, last_active_direct_external_user_id, \
    created_at, updated_at";

const PG_ACTOR_ENDPOINT_COLUMNS: &str = "\
    channel, external_user_id, actor_id, endpoint_metadata, approval_status, \
    created_at, updated_at";

fn pg_endpoint_ref(
    channel: Option<String>,
    external_user_id: Option<String>,
) -> Option<ActorEndpointRef> {
    match (channel, external_user_id) {
        (Some(channel), Some(external_user_id)) => {
            Some(ActorEndpointRef::new(channel, external_user_id))
        }
        _ => None,
    }
}

fn pg_row_to_actor(row: &tokio_postgres::Row) -> Result<ActorRecord, DatabaseError> {
    let status = row
        .get::<_, String>(3)
        .parse::<ActorStatus>()
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(ActorRecord {
        actor_id: row.get::<_, Uuid>(0),
        principal_id: row.get::<_, String>(1),
        display_name: row.get::<_, String>(2),
        status,
        preferred_delivery_endpoint: pg_endpoint_ref(
            row.get::<_, Option<String>>(4),
            row.get::<_, Option<String>>(5),
        ),
        last_active_direct_endpoint: pg_endpoint_ref(
            row.get::<_, Option<String>>(6),
            row.get::<_, Option<String>>(7),
        ),
        created_at: row.get::<_, DateTime<Utc>>(8),
        updated_at: row.get::<_, DateTime<Utc>>(9),
    })
}

fn pg_row_to_actor_endpoint(
    row: &tokio_postgres::Row,
) -> Result<ActorEndpointRecord, DatabaseError> {
    let approval_status = row
        .get::<_, String>(4)
        .parse::<EndpointApprovalStatus>()
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(ActorEndpointRecord {
        endpoint: ActorEndpointRef::new(row.get::<_, String>(0), row.get::<_, String>(1)),
        actor_id: row.get::<_, Uuid>(2),
        metadata: row.get::<_, serde_json::Value>(3),
        approval_status,
        created_at: row.get::<_, DateTime<Utc>>(5),
        updated_at: row.get::<_, DateTime<Utc>>(6),
    })
}

#[async_trait]
impl IdentityRegistryStore for PgBackend {
    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_one(
                &format!(
                    "INSERT INTO actors (principal_id, display_name, status, preferred_delivery_channel, preferred_delivery_external_user_id, last_active_direct_channel, last_active_direct_external_user_id) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {PG_ACTOR_COLUMNS}"
                ),
                &[
                    &actor.principal_id,
                    &actor.display_name,
                    &actor.status.as_str(),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to create actor: {e}")))?;

        pg_row_to_actor(&row)
    }

    async fn get_actor(&self, actor_id: Uuid) -> Result<Option<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                &format!("SELECT {PG_ACTOR_COLUMNS} FROM actors WHERE actor_id = $1"),
                &[&actor_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get actor: {e}")))?;

        row.map(|row| pg_row_to_actor(&row)).transpose()
    }

    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let rows = client
            .query(
                &format!(
                    "SELECT {PG_ACTOR_COLUMNS} FROM actors WHERE principal_id = $1 ORDER BY created_at ASC"
                ),
                &[&principal_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to list actors: {e}")))?;

        rows.iter()
            .map(pg_row_to_actor)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn update_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    principal_id = $2,
                    display_name = $3,
                    status = $4,
                    preferred_delivery_channel = $5,
                    preferred_delivery_external_user_id = $6,
                    last_active_direct_channel = $7,
                    last_active_direct_external_user_id = $8,
                    updated_at = $9
                WHERE actor_id = $1
                "#,
                &[
                    &actor.actor_id,
                    &actor.principal_id,
                    &actor.display_name,
                    &actor.status.as_str(),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor.updated_at,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to update actor: {e}")))?;

        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor.actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_actor(&self, actor_id: Uuid) -> Result<bool, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute("DELETE FROM actors WHERE actor_id = $1", &[&actor_id])
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to delete actor: {e}")))?;
        Ok(affected > 0)
    }

    async fn rename_actor(&self, actor_id: Uuid, display_name: &str) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "UPDATE actors SET display_name = $2, updated_at = NOW() WHERE actor_id = $1",
                &[&actor_id, &display_name],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to rename actor: {e}")))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_status(
        &self,
        actor_id: Uuid,
        status: ActorStatus,
    ) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "UPDATE actors SET status = $2, updated_at = NOW() WHERE actor_id = $1",
                &[&actor_id, &status.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to update actor status: {e}")))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_preferred_delivery_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    preferred_delivery_channel = $2,
                    preferred_delivery_external_user_id = $3,
                    updated_at = NOW()
                WHERE actor_id = $1
                "#,
                &[
                    &actor_id,
                    &endpoint.as_ref().map(|e| e.channel.as_str()),
                    &endpoint.as_ref().map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to set preferred endpoint: {e}")))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_last_active_direct_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    last_active_direct_channel = $2,
                    last_active_direct_external_user_id = $3,
                    updated_at = NOW()
                WHERE actor_id = $1
                "#,
                &[
                    &actor_id,
                    &endpoint.as_ref().map(|e| e.channel.as_str()),
                    &endpoint.as_ref().map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to set last active endpoint: {e}"))
            })?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn upsert_actor_endpoint(
        &self,
        record: &NewActorEndpointRecord,
    ) -> Result<ActorEndpointRecord, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_one(
                &format!(
                    "INSERT INTO actor_endpoints (channel, external_user_id, actor_id, endpoint_metadata, approval_status) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (channel, external_user_id) DO UPDATE SET actor_id = EXCLUDED.actor_id, endpoint_metadata = EXCLUDED.endpoint_metadata, approval_status = EXCLUDED.approval_status, updated_at = NOW() RETURNING {PG_ACTOR_ENDPOINT_COLUMNS}"
                ),
                &[
                    &record.endpoint.channel,
                    &record.endpoint.external_user_id,
                    &record.actor_id,
                    &record.metadata,
                    &record.approval_status.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to upsert actor endpoint: {e}")))?;

        pg_row_to_actor_endpoint(&row)
    }

    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                &format!(
                    "SELECT {PG_ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE channel = $1 AND external_user_id = $2"
                ),
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get actor endpoint: {e}")))?;

        row.map(|row| pg_row_to_actor_endpoint(&row)).transpose()
    }

    async fn list_actor_endpoints(
        &self,
        actor_id: Uuid,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let rows = client
            .query(
                &format!(
                    "SELECT {PG_ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE actor_id = $1 ORDER BY channel, external_user_id"
                ),
                &[&actor_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to list actor endpoints: {e}")))?;

        rows.iter()
            .map(pg_row_to_actor_endpoint)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn delete_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "DELETE FROM actor_endpoints WHERE channel = $1 AND external_user_id = $2",
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to delete actor endpoint: {e}")))?;
        Ok(affected > 0)
    }

    async fn resolve_actor_for_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                "SELECT a.actor_id, a.principal_id, a.display_name, a.status, \
                 a.preferred_delivery_channel, a.preferred_delivery_external_user_id, \
                 a.last_active_direct_channel, a.last_active_direct_external_user_id, \
                 a.created_at, a.updated_at \
                 FROM actor_endpoints e JOIN actors a ON a.actor_id = e.actor_id \
                 WHERE e.channel = $1 AND e.external_user_id = $2",
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to resolve actor: {e}")))?;

        row.map(|row| pg_row_to_actor(&row)).transpose()
    }
}

// ==================== AgentRegistryStore ====================

#[async_trait]
impl AgentRegistryStore for PgBackend {
    async fn save_agent_workspace(&self, ws: &AgentWorkspaceRecord) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        // Ensure table exists (only on first call per process lifetime)
        static TABLE_CREATED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !TABLE_CREATED.load(std::sync::atomic::Ordering::Relaxed) {
            client
                .execute(
                    "CREATE TABLE IF NOT EXISTS agent_workspaces (
                        id UUID PRIMARY KEY,
                        agent_id TEXT NOT NULL UNIQUE,
                        display_name TEXT NOT NULL,
                        system_prompt TEXT,
                        model TEXT,
                        bound_channels JSONB NOT NULL DEFAULT '[]',
                        trigger_keywords JSONB NOT NULL DEFAULT '[]',
                        allowed_tools JSONB,
                        allowed_skills JSONB,
                        is_default BOOLEAN NOT NULL DEFAULT FALSE,
                        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                    )",
                    &[],
                )
                .await
                .map_err(|e| {
                    DatabaseError::Query(format!("Failed to ensure agent_workspaces table: {e}"))
                })?;
            TABLE_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        let bound_channels =
            serde_json::to_value(&ws.bound_channels).unwrap_or(serde_json::Value::Array(vec![]));
        let trigger_keywords =
            serde_json::to_value(&ws.trigger_keywords).unwrap_or(serde_json::Value::Array(vec![]));
        let allowed_tools = ws
            .allowed_tools
            .as_ref()
            .map(|tools| serde_json::to_value(tools).unwrap_or(serde_json::Value::Null));
        let allowed_skills = ws
            .allowed_skills
            .as_ref()
            .map(|skills| serde_json::to_value(skills).unwrap_or(serde_json::Value::Null));

        client
            .execute(
                "INSERT INTO agent_workspaces \
                 (id, agent_id, display_name, system_prompt, model, \
                  bound_channels, trigger_keywords, allowed_tools, allowed_skills, \
                  is_default, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &ws.id,
                    &ws.agent_id,
                    &ws.display_name,
                    &ws.system_prompt,
                    &ws.model,
                    &bound_channels,
                    &trigger_keywords,
                    &allowed_tools,
                    &allowed_skills,
                    &ws.is_default,
                    &ws.created_at,
                    &ws.updated_at,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to save agent workspace: {e}")))?;

        Ok(())
    }

    async fn get_agent_workspace(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentWorkspaceRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                "SELECT id, agent_id, display_name, system_prompt, model, \
                 bound_channels, trigger_keywords, allowed_tools, allowed_skills, \
                 is_default, created_at, updated_at \
                 FROM agent_workspaces WHERE agent_id = $1",
                &[&agent_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get agent workspace: {e}")))?;

        Ok(row.map(|r| pg_row_to_agent_workspace(&r)))
    }

    async fn list_agent_workspaces(&self) -> Result<Vec<AgentWorkspaceRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let rows = client
            .query(
                "SELECT id, agent_id, display_name, system_prompt, model, \
                 bound_channels, trigger_keywords, allowed_tools, allowed_skills, \
                 is_default, created_at, updated_at \
                 FROM agent_workspaces ORDER BY created_at ASC",
                &[],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to list agent workspaces: {e}")))?;

        Ok(rows.iter().map(pg_row_to_agent_workspace).collect())
    }

    async fn delete_agent_workspace(&self, agent_id: &str) -> Result<bool, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "DELETE FROM agent_workspaces WHERE agent_id = $1",
                &[&agent_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to delete agent workspace: {e}")))?;

        Ok(affected > 0)
    }

    async fn update_agent_workspace(&self, ws: &AgentWorkspaceRecord) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let bound_channels =
            serde_json::to_value(&ws.bound_channels).unwrap_or(serde_json::Value::Array(vec![]));
        let trigger_keywords =
            serde_json::to_value(&ws.trigger_keywords).unwrap_or(serde_json::Value::Array(vec![]));
        let allowed_tools = ws
            .allowed_tools
            .as_ref()
            .map(|tools| serde_json::to_value(tools).unwrap_or(serde_json::Value::Null));
        let allowed_skills = ws
            .allowed_skills
            .as_ref()
            .map(|skills| serde_json::to_value(skills).unwrap_or(serde_json::Value::Null));

        let affected = client
            .execute(
                "UPDATE agent_workspaces SET \
                 display_name = $1, system_prompt = $2, model = $3, \
                 bound_channels = $4, trigger_keywords = $5, allowed_tools = $6, \
                 allowed_skills = $7, is_default = $8, updated_at = NOW() \
                 WHERE agent_id = $9",
                &[
                    &ws.display_name,
                    &ws.system_prompt,
                    &ws.model,
                    &bound_channels,
                    &trigger_keywords,
                    &allowed_tools,
                    &allowed_skills,
                    &ws.is_default,
                    &ws.agent_id,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to update agent workspace: {e}")))?;

        if affected == 0 {
            return Err(DatabaseError::Query(format!(
                "Agent workspace '{}' not found",
                ws.agent_id
            )));
        }

        Ok(())
    }
}

#[cfg(feature = "postgres")]
fn pg_row_to_agent_workspace(row: &tokio_postgres::Row) -> AgentWorkspaceRecord {
    let bound_channels: serde_json::Value = row.get("bound_channels");
    let trigger_keywords: serde_json::Value = row.get("trigger_keywords");
    let allowed_tools: Option<serde_json::Value> = row.get("allowed_tools");
    let allowed_skills: Option<serde_json::Value> = row.get("allowed_skills");

    AgentWorkspaceRecord {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        display_name: row.get("display_name"),
        system_prompt: row.get("system_prompt"),
        model: row.get("model"),
        bound_channels: serde_json::from_value(bound_channels).unwrap_or_default(),
        trigger_keywords: serde_json::from_value(trigger_keywords).unwrap_or_default(),
        allowed_tools: allowed_tools.and_then(|value| serde_json::from_value(value).ok()),
        allowed_skills: allowed_skills.and_then(|value| serde_json::from_value(value).ok()),
        is_default: row.get("is_default"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

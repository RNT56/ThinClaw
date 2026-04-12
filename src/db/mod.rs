//! Database abstraction layer.
//!
//! Provides a backend-agnostic `Database` trait that unifies all persistence
//! operations. Two implementations exist behind feature flags:
//!
//! - `postgres` (default): Uses `deadpool-postgres` + `tokio-postgres`
//! - `libsql`: Uses libSQL (Turso's SQLite fork) for embedded/edge deployment
//!
//! The existing `Store`, `Repository`, `SecretsStore`, and `WasmToolStore`
//! types become thin wrappers that delegate to `Arc<dyn Database>`.

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
pub mod libsql_migrations;

use std::collections::HashMap;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::error::DatabaseError;
use crate::error::WorkspaceError;
use crate::history::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationSummary,
    JobEventRecord, LearningArtifactVersion, LearningCandidate, LearningCodeProposal,
    LearningEvaluation, LearningEvent, LearningFeedbackRecord, LearningRollbackRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SessionSearchHit, SettingRow,
};
use crate::identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, ActorStatus, NewActorEndpointRecord,
    NewActorRecord,
};
use crate::workspace::{MemoryChunk, MemoryDocument, WorkspaceEntry};
use crate::workspace::{SearchConfig, SearchResult};

/// Create a database backend from configuration, run migrations, and return it.
///
/// This is the shared helper for CLI commands and other call sites that need
/// a simple `Arc<dyn Database>` without retaining backend-specific handles
/// (e.g., `pg_pool` or `libsql_conn` for the secrets store). The main agent
/// startup in `main.rs` uses its own initialization block because it also
/// captures those backend-specific handles.
pub async fn connect_from_config(
    config: &crate::config::DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            Ok(Arc::new(backend))
        }
        #[cfg(feature = "postgres")]
        _ => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            Ok(Arc::new(pg))
        }
        #[cfg(not(feature = "postgres"))]
        _ => Err(DatabaseError::Pool(
            "No database backend available. Enable 'postgres' or 'libsql' feature.".to_string(),
        )),
    }
}

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
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError>;
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
    async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
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

#[async_trait]
pub trait IdentityStore: Send + Sync {
    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError> {
        let _ = principal_id;
        Err(DatabaseError::Pool(
            "actor identity registry is not available in this build".to_string(),
        ))
    }
    async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError> {
        let _ = actor_id;
        Ok(None)
    }
    async fn upsert_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError> {
        let _ = actor;
        Err(DatabaseError::Pool(
            "actor identity registry is not available in this build".to_string(),
        ))
    }
    async fn rename_actor(&self, actor_id: &str, display_name: &str) -> Result<(), DatabaseError> {
        let _ = (actor_id, display_name);
        Err(DatabaseError::Pool(
            "actor identity registry is not available in this build".to_string(),
        ))
    }
    async fn set_actor_preferred_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
    ) -> Result<(), DatabaseError> {
        let _ = (actor_id, channel, external_user_id);
        Err(DatabaseError::Pool(
            "actor identity registry is not available in this build".to_string(),
        ))
    }
    async fn link_actor_endpoint(
        &self,
        actor_id: &str,
        channel: &str,
        external_user_id: &str,
        metadata: &serde_json::Value,
        approval_status: &str,
    ) -> Result<(), DatabaseError> {
        let _ = (
            actor_id,
            channel,
            external_user_id,
            metadata,
            approval_status,
        );
        Err(DatabaseError::Pool(
            "actor identity registry is not available in this build".to_string(),
        ))
    }
    async fn unlink_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let _ = (channel, external_user_id);
        Ok(false)
    }
    async fn list_actor_endpoints(
        &self,
        actor_id: &str,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError> {
        let _ = actor_id;
        Ok(Vec::new())
    }
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;
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
    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError>;
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
            .filter(|job| job.actor_id == actor_id)
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
                "interrupted" => summary.interrupted += 1,
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
        Ok(job.user_id == user_id && job.actor_id == actor_id)
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
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;

    /// Mark all RUNNING routine runs as failed.
    ///
    /// Called at startup to clean up orphaned runs from a previous process
    /// that crashed or was killed before the worker could update the status.
    /// Without this, routines with `max_concurrent = 1` would be permanently
    /// blocked.
    async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError>;

    /// Delete all run records for a specific routine.
    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError>;

    /// Delete ALL routine run records across all routines.
    async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError>;
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
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;
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

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;

    /// Atomically replace all chunks for a document.
    ///
    /// Deletes all existing chunks and inserts the new set in a single
    /// database transaction. This prevents the split-brain state where
    /// old chunks have been deleted but new ones have not yet been inserted
    /// (which would leave the document invisible in search).
    ///
    /// # Default implementation
    ///
    /// Falls back to sequential `delete_chunks` + `insert_chunk` calls for
    /// backends that do not override this method (e.g. PostgreSQL, where
    /// connection-pool transactions are less straightforward to express in a
    /// trait default). Backends with embedded connections (libSQL) override
    /// this with a proper `BEGIN` / `COMMIT` block.
    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        self.delete_chunks(document_id).await?;
        for (index, content, embedding) in chunks {
            self.insert_chunk(document_id, *index, content, embedding.as_deref())
                .await?;
        }
        Ok(())
    }
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;
}

// ==================== Agent Registry ====================

/// Persistent record for an agent workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkspaceRecord {
    /// Primary key (UUID).
    pub id: Uuid,
    /// Unique human-readable identifier (slug). Validated: `[a-z0-9_-]{2,32}`.
    pub agent_id: String,
    /// Display name for the agent.
    pub display_name: String,
    /// System prompt override for this agent.
    pub system_prompt: Option<String>,
    /// Model override (e.g. "openai/gpt-4o").
    pub model: Option<String>,
    /// Channels this agent is bound to (empty = all channels).
    pub bound_channels: Vec<String>,
    /// Keywords/mentions that trigger routing to this agent.
    pub trigger_keywords: Vec<String>,
    /// Optional per-agent tool allowlist.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Optional per-agent skill allowlist.
    #[serde(default)]
    pub allowed_skills: Option<Vec<String>>,
    /// Whether this is the default agent (receives unrouted messages).
    pub is_default: bool,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
    /// When the record was last updated.
    pub updated_at: DateTime<Utc>,
}

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
    + IdentityRegistryStore
    + ToolFailureStore
    + SettingsStore
    + WorkspaceStore
    + AgentRegistryStore
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

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

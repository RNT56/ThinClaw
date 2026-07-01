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

use crate::postgres_store::Store;
use crate::postgres_workspace::Repository;
use crate::{
    AgentRegistryStore, AgentWorkspaceRecord, ConversationStore, Database, ExperimentStore,
    IdentityRegistryStore, JobStore, RepoProjectStore, RoutineStore, SandboxStore, SettingsStore,
    ToolFailureStore, WorkspaceStore,
};
use thinclaw_types::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};
use thinclaw_experiments::{
    ExperimentArtifactRef, ExperimentCampaign, ExperimentLease, ExperimentModelUsageRecord,
    ExperimentProject, ExperimentRunnerProfile, ExperimentTarget, ExperimentTargetLink,
    ExperimentTrial,
};
use thinclaw_history::{
    ConversationMessage, ConversationSummary, JobEventRecord, LearningArtifactVersion,
    LearningCandidate, LearningCodeProposal, LearningEvaluation, LearningEvent,
    LearningFeedbackRecord, LearningRollbackRecord, LlmCallRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeEvaluatorHealth, OutcomeObservation, OutcomePendingUser,
    OutcomeSummaryStats, SessionSearchHit, SettingRow,
};
use thinclaw_identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, ActorStatus, EndpointApprovalStatus,
    NewActorEndpointRecord, NewActorRecord,
};
use thinclaw_repo_projects::{
    MergeGateDecision, RepoProject, RepoProjectEvent, RepoProjectRepo, RepoProjectRun,
    RepoProjectTask, RepoWebhookDelivery, RepoWorkerRun,
};
use thinclaw_types::BrokenTool;
use thinclaw_types::error::{DatabaseError, WorkspaceError};
use thinclaw_types::{ActionRecord, JobContext, JobState, SandboxJobRecord, SandboxJobSummary};
use thinclaw_workspace::{MemoryChunk, MemoryDocument, SearchConfig, SearchResult, WorkspaceEntry};
/// Minimal configuration required to construct a PostgreSQL backend.
///
/// The root crate implements this for its compatibility `DatabaseConfig`
/// facade so `thinclaw-db` does not depend back on the root package.
pub trait PgBackendConfig {
    fn postgres_url(&self) -> &str;
    fn postgres_pool_size(&self) -> usize;
}

impl PgBackendConfig for thinclaw_config::database::DatabaseConfig {
    fn postgres_url(&self) -> &str {
        self.url()
    }

    fn postgres_pool_size(&self) -> usize {
        self.pool_size
    }
}

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
    pub async fn new<C>(config: &C) -> Result<Self, DatabaseError>
    where
        C: PgBackendConfig + ?Sized,
    {
        let store = Store::new(config).await?;
        let repo = Repository::new(store.pool());
        Ok(Self { store, repo })
    }

    /// Create a backend from an existing connection pool.
    ///
    /// Useful for callers (e.g. the setup wizard) that already hold a
    /// `deadpool_postgres::Pool` and want the unified `Database` surface
    /// without reconnecting.
    pub fn from_pool(pool: Pool) -> Self {
        let store = Store::from_pool(pool.clone());
        let repo = Repository::new(pool);
        Self { store, repo }
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

#[cfg(feature = "postgres")]
fn pg_row_to_agent_workspace(row: &tokio_postgres::Row) -> AgentWorkspaceRecord {
    let bound_channels: serde_json::Value = row.get("bound_channels");
    let trigger_keywords: serde_json::Value = row.get("trigger_keywords");
    let allowed_tools: Option<serde_json::Value> = row.get("allowed_tools");
    let allowed_skills: Option<serde_json::Value> = row.get("allowed_skills");
    let tool_profile: Option<String> = row.get("tool_profile");

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
        tool_profile: tool_profile.and_then(|value| match value.as_str() {
            "standard" => Some(thinclaw_types::ToolProfile::Standard),
            "restricted" => Some(thinclaw_types::ToolProfile::Restricted),
            "explicit_only" => Some(thinclaw_types::ToolProfile::ExplicitOnly),
            _ => None,
        }),
        is_default: row.get("is_default"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

mod agent_registry_store;
mod conversation_store;
mod experiment_store;
mod identity_registry_store;
mod job_store;
mod repo_project_store;
mod routine_store;
mod sandbox_store;
mod settings_store;
mod tool_failure_store;
mod workspace_store;

//! Shared sub-agent task assignment types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentMemoryMode {
    #[default]
    ProvidedContextOnly,
    GrantedToolsOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentToolMode {
    #[default]
    ExplicitOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentSkillMode {
    #[default]
    ExplicitOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubagentProvidedContext {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubagentTaskPacket {
    pub objective: String,
    #[serde(default)]
    pub todos: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub provided_context: Vec<SubagentProvidedContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_summary: Option<String>,
}

/// Status string stored in the `subagent_runs.status` column.
///
/// This is a coarse, DB-friendly status distinct from the agent-side
/// `SubagentStatus` (which carries a `Failed(String)` payload) — the ledger
/// keeps the reason in the separate `error` column instead.
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
    pub id: uuid::Uuid,
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
        id: uuid::Uuid,
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

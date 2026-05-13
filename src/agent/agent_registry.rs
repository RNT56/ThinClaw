//! Agent registry compatibility facade.
//!
//! Validation and router synchronization live in `thinclaw-agent`. Root keeps
//! concrete database persistence and workspace seeding adapters.

use std::sync::Arc;

use async_trait::async_trait;
pub use thinclaw_agent::agent_registry::AgentRegistryError;
use thinclaw_agent::agent_registry::{AgentRegistryStorePort, AgentWorkspaceSeeder};
use thinclaw_types::{AgentWorkspaceRecord, ToolProfile};

use crate::agent::agent_router::{AgentRouter, AgentWorkspace};
use crate::db::Database;
use crate::workspace::Workspace;

/// Unified agent registry: Router + DB persistence + validation + workspace seeding.
pub struct AgentRegistry {
    inner: thinclaw_agent::agent_registry::AgentRegistry,
    db: Option<Arc<dyn Database>>,
}

impl AgentRegistry {
    /// Create a new registry. Pass None for db to operate in-memory only.
    pub fn new(router: Arc<AgentRouter>, db: Option<Arc<dyn Database>>) -> Self {
        let store = db.as_ref().map(|db| {
            Arc::new(RootAgentRegistryStore { db: Arc::clone(db) })
                as Arc<dyn AgentRegistryStorePort>
        });
        let seeder = db.as_ref().map(|db| {
            Arc::new(RootAgentWorkspaceSeeder { db: Arc::clone(db) })
                as Arc<dyn AgentWorkspaceSeeder>
        });
        let inner =
            thinclaw_agent::agent_registry::AgentRegistry::new(router, store).with_seeder(seeder);
        Self { inner, db }
    }

    /// Get a reference to the underlying router.
    pub fn router(&self) -> &Arc<AgentRouter> {
        self.inner.router()
    }

    /// Load all persisted agent workspaces from DB into the router.
    pub async fn load_from_db(&self) -> Result<usize, AgentRegistryError> {
        self.inner.load_from_db().await
    }

    /// Create a new agent workspace.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_agent(
        &self,
        agent_id: &str,
        display_name: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        bound_channels: Vec<String>,
        trigger_keywords: Vec<String>,
        is_default: bool,
        allowed_tools: Option<Vec<String>>,
        allowed_skills: Option<Vec<String>>,
        tool_profile: Option<ToolProfile>,
    ) -> Result<AgentWorkspaceRecord, AgentRegistryError> {
        self.inner
            .create_agent(
                agent_id,
                display_name,
                system_prompt,
                model,
                bound_channels,
                trigger_keywords,
                is_default,
                allowed_tools,
                allowed_skills,
                tool_profile,
            )
            .await
    }

    /// Remove an agent workspace.
    pub async fn remove_agent(
        &self,
        agent_id: &str,
        force: bool,
    ) -> Result<bool, AgentRegistryError> {
        self.inner.remove_agent(agent_id, force).await
    }

    /// Update an existing agent workspace.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_agent(
        &self,
        agent_id: &str,
        display_name: Option<&str>,
        system_prompt: Option<Option<&str>>,
        model: Option<Option<&str>>,
        bound_channels: Option<Vec<String>>,
        trigger_keywords: Option<Vec<String>>,
        is_default: Option<bool>,
        allowed_tools: Option<Option<Vec<String>>>,
        allowed_skills: Option<Option<Vec<String>>>,
        tool_profile: Option<Option<ToolProfile>>,
    ) -> Result<AgentWorkspaceRecord, AgentRegistryError> {
        self.inner
            .update_agent(
                agent_id,
                display_name,
                system_prompt,
                model,
                bound_channels,
                trigger_keywords,
                is_default,
                allowed_tools,
                allowed_skills,
                tool_profile,
            )
            .await
    }

    /// List all registered agents.
    pub async fn list_agents(&self) -> Vec<AgentWorkspace> {
        self.inner.list_agents().await
    }

    /// Get agent count.
    pub async fn agent_count(&self) -> usize {
        self.inner.agent_count().await
    }

    /// Get a specific agent workspace record from DB.
    pub async fn get_agent_record(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentWorkspaceRecord>, AgentRegistryError> {
        self.inner.get_agent_record(agent_id).await
    }

    /// Build a Workspace instance scoped to a specific agent's UUID.
    pub fn build_workspace_for_agent(
        &self,
        record: &AgentWorkspaceRecord,
        user_id: &str,
    ) -> Option<Arc<Workspace>> {
        let db = self.db.as_ref()?;
        let ws = Workspace::new_with_db(user_id, Arc::clone(db)).with_agent(record.id);
        Some(Arc::new(ws))
    }
}

struct RootAgentRegistryStore {
    db: Arc<dyn Database>,
}

#[async_trait]
impl AgentRegistryStorePort for RootAgentRegistryStore {
    async fn save_agent_workspace(
        &self,
        ws: &AgentWorkspaceRecord,
    ) -> Result<(), AgentRegistryError> {
        self.db
            .save_agent_workspace(ws)
            .await
            .map_err(|error| AgentRegistryError::Store(error.to_string()))
    }

    async fn get_agent_workspace(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentWorkspaceRecord>, AgentRegistryError> {
        self.db
            .get_agent_workspace(agent_id)
            .await
            .map_err(|error| AgentRegistryError::Store(error.to_string()))
    }

    async fn list_agent_workspaces(&self) -> Result<Vec<AgentWorkspaceRecord>, AgentRegistryError> {
        self.db
            .list_agent_workspaces()
            .await
            .map_err(|error| AgentRegistryError::Store(error.to_string()))
    }

    async fn delete_agent_workspace(&self, agent_id: &str) -> Result<bool, AgentRegistryError> {
        self.db
            .delete_agent_workspace(agent_id)
            .await
            .map_err(|error| AgentRegistryError::Store(error.to_string()))
    }

    async fn update_agent_workspace(
        &self,
        ws: &AgentWorkspaceRecord,
    ) -> Result<(), AgentRegistryError> {
        self.db
            .update_agent_workspace(ws)
            .await
            .map_err(|error| AgentRegistryError::Store(error.to_string()))
    }
}

struct RootAgentWorkspaceSeeder {
    db: Arc<dyn Database>,
}

#[async_trait]
impl AgentWorkspaceSeeder for RootAgentWorkspaceSeeder {
    async fn seed_workspace(&self, record: &AgentWorkspaceRecord) -> Result<(), String> {
        let ws = Workspace::new_with_db("default", Arc::clone(&self.db)).with_agent(record.id);

        let identity_content = format!(
            "# {}\n\n{}\n\n_Created: {}_\n",
            record.display_name,
            record
                .system_prompt
                .as_deref()
                .unwrap_or("A specialized agent workspace."),
            record.created_at.format("%Y-%m-%d %H:%M UTC"),
        );

        ws.write("IDENTITY.md", &identity_content)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

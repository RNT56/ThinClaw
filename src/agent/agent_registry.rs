//! Unified agent registry: in-memory router + DB persistence + validation.
//!
//! Wraps [`AgentRouter`] for routing decisions and [`Database`] for persistence.
//! Provides CRUD operations with validation, workspace seeding, and
//! automatic router synchronization.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::agent::agent_router::{AgentRouter, AgentWorkspace};
use crate::db::{AgentWorkspaceRecord, Database};
use crate::error::DatabaseError;
use crate::workspace::Workspace;

/// Maximum number of agent workspaces allowed (prevents resource exhaustion).
const MAX_AGENTS: usize = 20;

/// Reserved agent IDs that cannot be used.
const RESERVED_IDS: &[&str] = &[
    "default", "system", "main", "admin", "root", "agent", "bot", "self",
];

/// Validation error for agent configuration.
#[derive(Debug, thiserror::Error)]
pub enum AgentRegistryError {
    #[error(
        "Invalid agent_id '{0}': must be 2-32 chars, lowercase alphanumeric, hyphens, or underscores"
    )]
    InvalidAgentId(String),
    #[error("Reserved agent_id '{0}': cannot use reserved names")]
    ReservedAgentId(String),
    #[error("Agent '{0}' already exists")]
    DuplicateAgent(String),
    #[error("Agent '{0}' not found")]
    NotFound(String),
    #[error("Maximum number of agents ({MAX_AGENTS}) reached")]
    MaxAgentsReached,
    #[error("Cannot delete default agent '{0}' without force (other agents exist)")]
    CannotDeleteDefault(String),
    #[error("Display name must be 1-64 characters")]
    InvalidDisplayName,
    #[error("System prompt exceeds maximum length (10,000 characters)")]
    SystemPromptTooLong,
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),
}

/// Unified agent registry: Router + DB persistence + validation + workspace seeding.
pub struct AgentRegistry {
    router: Arc<AgentRouter>,
    db: Option<Arc<dyn Database>>,
}

impl AgentRegistry {
    /// Create a new registry. Pass None for db to operate in-memory only.
    pub fn new(router: Arc<AgentRouter>, db: Option<Arc<dyn Database>>) -> Self {
        Self { router, db }
    }

    /// Get a reference to the underlying router.
    pub fn router(&self) -> &Arc<AgentRouter> {
        &self.router
    }

    /// Load all persisted agent workspaces from DB into the router.
    /// Called at startup to restore state.
    pub async fn load_from_db(&self) -> Result<usize, AgentRegistryError> {
        let db = match &self.db {
            Some(db) => db,
            None => return Ok(0),
        };

        let records = db.list_agent_workspaces().await?;
        let count = records.len();

        for record in records {
            let ws = record_to_workspace(&record);
            self.router.register_agent(ws).await;
            tracing::debug!(
                agent_id = %record.agent_id,
                display_name = %record.display_name,
                is_default = record.is_default,
                "Loaded agent workspace from DB"
            );
        }

        Ok(count)
    }

    /// Create a new agent workspace. Validates, persists to DB, registers in router,
    /// and seeds the agent's workspace with IDENTITY.md.
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
    ) -> Result<AgentWorkspaceRecord, AgentRegistryError> {
        // Validate
        self.validate_agent_id(agent_id)?;
        self.validate_display_name(display_name)?;
        if matches!(system_prompt, Some(p) if p.len() > 10_000) {
            return Err(AgentRegistryError::SystemPromptTooLong);
        }

        // Check max agents
        let current_count = self.router.agent_count().await;
        if current_count >= MAX_AGENTS {
            return Err(AgentRegistryError::MaxAgentsReached);
        }

        // Check duplicate
        if self.router.get_agent(agent_id).await.is_some() {
            return Err(AgentRegistryError::DuplicateAgent(agent_id.to_string()));
        }

        let now = Utc::now();
        let record = AgentWorkspaceRecord {
            id: Uuid::new_v4(),
            agent_id: agent_id.to_string(),
            display_name: display_name.trim().to_string(),
            system_prompt: system_prompt.map(String::from),
            model: model.map(String::from),
            bound_channels,
            trigger_keywords,
            allowed_tools,
            allowed_skills,
            is_default,
            created_at: now,
            updated_at: now,
        };

        // Persist to DB
        if let Some(ref db) = self.db {
            db.save_agent_workspace(&record).await?;
        }

        // Register in router
        let ws = record_to_workspace(&record);
        self.router.register_agent(ws).await;

        // Seed workspace
        self.seed_workspace(&record).await;

        tracing::info!(
            agent_id = %record.agent_id,
            display_name = %record.display_name,
            is_default = record.is_default,
            "Created agent workspace"
        );

        Ok(record)
    }

    /// Remove an agent workspace.
    pub async fn remove_agent(
        &self,
        agent_id: &str,
        force: bool,
    ) -> Result<bool, AgentRegistryError> {
        // Check if agent exists
        let agent = self.router.get_agent(agent_id).await;
        if agent.is_none() {
            return Err(AgentRegistryError::NotFound(agent_id.to_string()));
        }

        let agent = agent.unwrap();

        // Protect default agent unless force
        if agent.is_default && !force {
            return Err(AgentRegistryError::CannotDeleteDefault(
                agent_id.to_string(),
            ));
        }

        // Remove from DB
        if let Some(ref db) = self.db {
            db.delete_agent_workspace(agent_id).await?;
        }

        // Remove from router
        self.router.unregister_agent(agent_id).await;

        tracing::info!(agent_id = %agent_id, "Removed agent workspace");
        Ok(true)
    }

    /// Update an existing agent workspace.
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
    ) -> Result<AgentWorkspaceRecord, AgentRegistryError> {
        // Get current record from DB (or construct from router)
        let mut record = if let Some(ref db) = self.db {
            db.get_agent_workspace(agent_id)
                .await?
                .ok_or_else(|| AgentRegistryError::NotFound(agent_id.to_string()))?
        } else {
            let ws = self
                .router
                .get_agent(agent_id)
                .await
                .ok_or_else(|| AgentRegistryError::NotFound(agent_id.to_string()))?;
            workspace_to_record(&ws)
        };

        // Apply updates
        if let Some(name) = display_name {
            self.validate_display_name(name)?;
            record.display_name = name.trim().to_string();
        }
        if let Some(prompt) = system_prompt {
            if matches!(prompt, Some(p) if p.len() > 10_000) {
                return Err(AgentRegistryError::SystemPromptTooLong);
            }
            record.system_prompt = prompt.map(String::from);
        }
        if let Some(m) = model {
            record.model = m.map(String::from);
        }
        if let Some(channels) = bound_channels {
            record.bound_channels = channels;
        }
        if let Some(keywords) = trigger_keywords {
            record.trigger_keywords = keywords;
        }
        if let Some(allowed_tools) = allowed_tools {
            record.allowed_tools = allowed_tools;
        }
        if let Some(allowed_skills) = allowed_skills {
            record.allowed_skills = allowed_skills;
        }
        if let Some(default) = is_default {
            record.is_default = default;
        }

        record.updated_at = Utc::now();

        // Persist
        if let Some(ref db) = self.db {
            db.update_agent_workspace(&record).await?;
        }

        // Re-register in router (unregister + register to update)
        self.router.unregister_agent(agent_id).await;
        self.router
            .register_agent(record_to_workspace(&record))
            .await;

        tracing::info!(agent_id = %agent_id, "Updated agent workspace");
        Ok(record)
    }

    /// List all registered agents.
    pub async fn list_agents(&self) -> Vec<AgentWorkspace> {
        self.router.list_agents().await
    }

    /// Get agent count.
    pub async fn agent_count(&self) -> usize {
        self.router.agent_count().await
    }

    /// Get a specific agent workspace record from DB.
    pub async fn get_agent_record(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentWorkspaceRecord>, AgentRegistryError> {
        if let Some(ref db) = self.db {
            Ok(db.get_agent_workspace(agent_id).await?)
        } else {
            Ok(self
                .router
                .get_agent(agent_id)
                .await
                .map(|ws| workspace_to_record(&ws)))
        }
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

    // ── Private helpers ──────────────────────────────────────────────

    fn validate_agent_id(&self, agent_id: &str) -> Result<(), AgentRegistryError> {
        let len = agent_id.len();
        let valid = (2..=32).contains(&len)
            && agent_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !valid {
            return Err(AgentRegistryError::InvalidAgentId(agent_id.to_string()));
        }
        if RESERVED_IDS.contains(&agent_id) {
            return Err(AgentRegistryError::ReservedAgentId(agent_id.to_string()));
        }
        Ok(())
    }

    fn validate_display_name(&self, name: &str) -> Result<(), AgentRegistryError> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(AgentRegistryError::InvalidDisplayName);
        }
        Ok(())
    }

    /// Seed a new agent's workspace with IDENTITY.md.
    async fn seed_workspace(&self, record: &AgentWorkspaceRecord) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        let ws = Workspace::new_with_db("default", Arc::clone(db)).with_agent(record.id);

        let identity_content = format!(
            "# {}\n\n{}\n\n_Created: {}_\n",
            record.display_name,
            record
                .system_prompt
                .as_deref()
                .unwrap_or("A specialized agent workspace."),
            record.created_at.format("%Y-%m-%d %H:%M UTC"),
        );

        if let Err(e) = ws.write("IDENTITY.md", &identity_content).await {
            tracing::warn!(
                agent_id = %record.agent_id,
                error = %e,
                "Failed to seed IDENTITY.md for new agent workspace"
            );
        }
    }
}

/// Convert an `AgentWorkspaceRecord` (DB) to an `AgentWorkspace` (router).
fn record_to_workspace(record: &AgentWorkspaceRecord) -> AgentWorkspace {
    AgentWorkspace {
        workspace_id: Some(record.id),
        agent_id: record.agent_id.clone(),
        display_name: record.display_name.clone(),
        system_prompt: record.system_prompt.clone(),
        bound_channels: record.bound_channels.clone(),
        trigger_keywords: record.trigger_keywords.clone(),
        allowed_tools: record.allowed_tools.clone(),
        allowed_skills: record.allowed_skills.clone(),
        is_default: record.is_default,
        model: record.model.clone(),
    }
}

/// Convert an `AgentWorkspace` (router) to an `AgentWorkspaceRecord` (DB).
/// Uses a new UUID and current timestamp since router data doesn't carry these.
fn workspace_to_record(ws: &AgentWorkspace) -> AgentWorkspaceRecord {
    let now = Utc::now();
    AgentWorkspaceRecord {
        id: ws.workspace_id.unwrap_or_else(Uuid::new_v4),
        agent_id: ws.agent_id.clone(),
        display_name: ws.display_name.clone(),
        system_prompt: ws.system_prompt.clone(),
        model: ws.model.clone(),
        bound_channels: ws.bound_channels.clone(),
        trigger_keywords: ws.trigger_keywords.clone(),
        allowed_tools: ws.allowed_tools.clone(),
        allowed_skills: ws.allowed_skills.clone(),
        is_default: ws.is_default,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_router() -> Arc<AgentRouter> {
        Arc::new(AgentRouter::new())
    }

    fn make_registry() -> AgentRegistry {
        AgentRegistry::new(make_router(), None)
    }

    #[tokio::test]
    async fn test_create_agent() {
        let reg = make_registry();
        let record = reg
            .create_agent(
                "test-bot",
                "Test Bot",
                Some("You are helpful."),
                None,
                vec![],
                vec![],
                false,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(record.agent_id, "test-bot");
        assert_eq!(record.display_name, "Test Bot");
        assert_eq!(reg.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_invalid_agent_id() {
        let reg = make_registry();
        assert!(
            reg.create_agent(
                "UPPER",
                "Name",
                None,
                None,
                vec![],
                vec![],
                false,
                None,
                None
            )
            .await
            .is_err()
        );
        assert!(
            reg.create_agent("a", "Name", None, None, vec![], vec![], false, None, None)
                .await
                .is_err()
        );
        assert!(
            reg.create_agent(
                "spaces here",
                "Name",
                None,
                None,
                vec![],
                vec![],
                false,
                None,
                None,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn test_reserved_agent_id() {
        let reg = make_registry();
        assert!(
            reg.create_agent(
                "default",
                "Default",
                None,
                None,
                vec![],
                vec![],
                false,
                None,
                None,
            )
            .await
            .is_err()
        );
        assert!(
            reg.create_agent(
                "system",
                "System",
                None,
                None,
                vec![],
                vec![],
                false,
                None,
                None,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn test_duplicate_rejected() {
        let reg = make_registry();
        reg.create_agent(
            "bot-a",
            "Bot A",
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            reg.create_agent(
                "bot-a",
                "Bot A Again",
                None,
                None,
                vec![],
                vec![],
                false,
                None,
                None,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let reg = make_registry();
        reg.create_agent(
            "to-remove",
            "Remove Me",
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reg.remove_agent("to-remove", false).await.unwrap());
        assert_eq!(reg.agent_count().await, 0);
    }

    #[tokio::test]
    async fn test_default_agent_protected() {
        let reg = make_registry();
        reg.create_agent(
            "main-bot",
            "Main",
            None,
            None,
            vec![],
            vec![],
            true,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reg.remove_agent("main-bot", false).await.is_err());
        assert!(reg.remove_agent("main-bot", true).await.unwrap());
    }

    #[tokio::test]
    async fn test_update_agent() {
        let reg = make_registry();
        reg.create_agent(
            "updatable",
            "Original",
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let updated = reg
            .update_agent(
                "updatable",
                Some("Updated Name"),
                Some(Some("New prompt")),
                Some(Some("openai/gpt-4o")),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(updated.display_name, "Updated Name");

        // Verify router was updated
        let ws = reg.router.get_agent("updatable").await.unwrap();
        assert_eq!(ws.display_name, "Updated Name");
    }

    #[tokio::test]
    async fn test_list_agents() {
        let reg = make_registry();
        reg.create_agent(
            "bot-1",
            "Bot 1",
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .unwrap();
        reg.create_agent(
            "bot-2",
            "Bot 2",
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reg.list_agents().await.len(), 2);
    }

    #[tokio::test]
    async fn test_capability_allowlists_propagate_to_router() {
        let reg = make_registry();
        let record = reg
            .create_agent(
                "bounded",
                "Bounded",
                None,
                None,
                vec![],
                vec![],
                false,
                Some(vec!["read_file".to_string(), "memory_read".to_string()]),
                Some(vec!["github".to_string()]),
            )
            .await
            .unwrap();

        assert_eq!(
            record.allowed_tools,
            Some(vec!["read_file".to_string(), "memory_read".to_string()])
        );
        assert_eq!(record.allowed_skills, Some(vec!["github".to_string()]));

        let updated = reg
            .update_agent(
                "bounded",
                None,
                None,
                None,
                None,
                None,
                None,
                Some(Some(vec!["shell".to_string()])),
                Some(Some(vec!["openai-docs".to_string()])),
            )
            .await
            .unwrap();

        assert_eq!(updated.allowed_tools, Some(vec!["shell".to_string()]));
        assert_eq!(
            updated.allowed_skills,
            Some(vec!["openai-docs".to_string()])
        );

        let ws = reg.router.get_agent("bounded").await.unwrap();
        assert_eq!(ws.allowed_tools, Some(vec!["shell".to_string()]));
        assert_eq!(ws.allowed_skills, Some(vec!["openai-docs".to_string()]));
    }
}

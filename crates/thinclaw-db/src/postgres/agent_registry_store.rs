//! postgres: agent_registry_store.

use super::*;

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
                        tool_profile TEXT,
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
            client
                .execute(
                    "ALTER TABLE agent_workspaces ADD COLUMN IF NOT EXISTS tool_profile TEXT",
                    &[],
                )
                .await
                .map_err(|e| {
                    DatabaseError::Query(format!(
                        "Failed to ensure agent_workspaces.tool_profile column: {e}"
                    ))
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
        let tool_profile = ws.tool_profile.map(|profile| profile.as_str().to_string());

        client
            .execute(
                "INSERT INTO agent_workspaces \
                 (id, agent_id, display_name, system_prompt, model, \
                  bound_channels, trigger_keywords, allowed_tools, allowed_skills, tool_profile, \
                  is_default, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
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
                    &tool_profile,
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
                 bound_channels, trigger_keywords, allowed_tools, allowed_skills, tool_profile, \
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
                 bound_channels, trigger_keywords, allowed_tools, allowed_skills, tool_profile, \
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
        let tool_profile = ws.tool_profile.map(|profile| profile.as_str().to_string());

        let affected = client
            .execute(
                "UPDATE agent_workspaces SET \
                 display_name = $1, system_prompt = $2, model = $3, \
                 bound_channels = $4, trigger_keywords = $5, allowed_tools = $6, \
                 allowed_skills = $7, tool_profile = $8, is_default = $9, updated_at = NOW() \
                 WHERE agent_id = $10",
                &[
                    &ws.display_name,
                    &ws.system_prompt,
                    &ws.model,
                    &bound_channels,
                    &trigger_keywords,
                    &allowed_tools,
                    &allowed_skills,
                    &tool_profile,
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

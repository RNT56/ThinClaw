//! libSQL implementation of `AgentRegistryStore`.

use async_trait::async_trait;

use crate::db::{AgentRegistryStore, AgentWorkspaceRecord};
use crate::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_text, get_ts};

/// Column list for agent_workspaces table (matches positional access).
const AGENT_WS_COLUMNS: &str = "\
    id, agent_id, display_name, system_prompt, model, \
    bound_channels, trigger_keywords, is_default, \
    created_at, updated_at";

fn row_to_agent_workspace(row: &libsql::Row) -> AgentWorkspaceRecord {
    let bound_channels_json = get_text(row, 5);
    let trigger_keywords_json = get_text(row, 6);

    AgentWorkspaceRecord {
        id: get_text(row, 0).parse().unwrap_or_default(),
        agent_id: get_text(row, 1),
        display_name: get_text(row, 2),
        system_prompt: get_opt_text(row, 3),
        model: get_opt_text(row, 4),
        bound_channels: serde_json::from_str(&bound_channels_json).unwrap_or_default(),
        trigger_keywords: serde_json::from_str(&trigger_keywords_json).unwrap_or_default(),
        is_default: get_i64(row, 7) != 0,
        created_at: get_ts(row, 8),
        updated_at: get_ts(row, 9),
    }
}

#[async_trait]
impl AgentRegistryStore for LibSqlBackend {
    async fn save_agent_workspace(&self, ws: &AgentWorkspaceRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let bound_channels =
            serde_json::to_string(&ws.bound_channels).unwrap_or_else(|_| "[]".into());
        let trigger_keywords =
            serde_json::to_string(&ws.trigger_keywords).unwrap_or_else(|_| "[]".into());

        conn.execute(
            &format!(
                "INSERT INTO agent_workspaces ({AGENT_WS_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            libsql::params![
                ws.id.to_string(),
                ws.agent_id.clone(),
                ws.display_name.clone(),
                ws.system_prompt
                    .as_deref()
                    .map(|s| libsql::Value::Text(s.to_string()))
                    .unwrap_or(libsql::Value::Null),
                ws.model
                    .as_deref()
                    .map(|s| libsql::Value::Text(s.to_string()))
                    .unwrap_or(libsql::Value::Null),
                bound_channels,
                trigger_keywords,
                ws.is_default as i64,
                fmt_ts(&ws.created_at),
                fmt_ts(&ws.updated_at),
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
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {AGENT_WS_COLUMNS} FROM agent_workspaces WHERE agent_id = ?1"
                ),
                libsql::params![agent_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get agent workspace: {e}")))?;

        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_agent_workspace(&row))),
            Ok(None) => Ok(None),
            Err(e) => Err(DatabaseError::Query(format!(
                "Failed to read agent workspace row: {e}"
            ))),
        }
    }

    async fn list_agent_workspaces(&self) -> Result<Vec<AgentWorkspaceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {AGENT_WS_COLUMNS} FROM agent_workspaces ORDER BY created_at ASC"
                ),
                (),
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to list agent workspaces: {e}"))
            })?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            results.push(row_to_agent_workspace(&row));
        }
        Ok(results)
    }

    async fn delete_agent_workspace(&self, agent_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                "DELETE FROM agent_workspaces WHERE agent_id = ?1",
                libsql::params![agent_id],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to delete agent workspace: {e}"))
            })?;

        Ok(affected > 0)
    }

    async fn update_agent_workspace(
        &self,
        ws: &AgentWorkspaceRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let bound_channels =
            serde_json::to_string(&ws.bound_channels).unwrap_or_else(|_| "[]".into());
        let trigger_keywords =
            serde_json::to_string(&ws.trigger_keywords).unwrap_or_else(|_| "[]".into());

        let affected = conn
            .execute(
                "UPDATE agent_workspaces SET \
                 display_name = ?1, system_prompt = ?2, model = ?3, \
                 bound_channels = ?4, trigger_keywords = ?5, is_default = ?6, \
                 updated_at = ?7 \
                 WHERE agent_id = ?8",
                libsql::params![
                    ws.display_name.clone(),
                    ws.system_prompt
                        .as_deref()
                        .map(|s| libsql::Value::Text(s.to_string()))
                        .unwrap_or(libsql::Value::Null),
                    ws.model
                        .as_deref()
                        .map(|s| libsql::Value::Text(s.to_string()))
                        .unwrap_or(libsql::Value::Null),
                    bound_channels,
                    trigger_keywords,
                    ws.is_default as i64,
                    fmt_ts(&ws.updated_at),
                    ws.agent_id.clone(),
                ],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to update agent workspace: {e}"))
            })?;

        if affected == 0 {
            return Err(DatabaseError::Query(format!(
                "Agent workspace '{}' not found",
                ws.agent_id
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use chrono::Utc;
    use uuid::Uuid;

    /// Create a file-based test backend (in-memory DBs have per-connection isolation in libSQL).
    async fn test_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("test_agent_registry.db");
        let backend = LibSqlBackend::new_local(&path).await.unwrap();
        backend.run_migrations().await.unwrap();
        (backend, dir)
    }

    fn test_record(agent_id: &str) -> AgentWorkspaceRecord {
        AgentWorkspaceRecord {
            id: Uuid::new_v4(),
            agent_id: agent_id.to_string(),
            display_name: format!("Test Agent {}", agent_id),
            system_prompt: Some("You are a test agent.".into()),
            model: None,
            bound_channels: vec![],
            trigger_keywords: vec![],
            is_default: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_save_and_get() {
        let (backend, _dir) = test_backend().await;
        let rec = test_record("test-agent");
        backend.save_agent_workspace(&rec).await.unwrap();

        let loaded = backend
            .get_agent_workspace("test-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_id, "test-agent");
        assert_eq!(loaded.display_name, "Test Agent test-agent");
        assert_eq!(
            loaded.system_prompt.as_deref(),
            Some("You are a test agent.")
        );
    }

    #[tokio::test]
    async fn test_list() {
        let (backend, _dir) = test_backend().await;
        backend
            .save_agent_workspace(&test_record("agent-a"))
            .await
            .unwrap();
        backend
            .save_agent_workspace(&test_record("agent-b"))
            .await
            .unwrap();

        let list = backend.list_agent_workspaces().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_delete() {
        let (backend, _dir) = test_backend().await;
        backend
            .save_agent_workspace(&test_record("to-delete"))
            .await
            .unwrap();

        assert!(backend.delete_agent_workspace("to-delete").await.unwrap());
        assert!(!backend
            .delete_agent_workspace("nonexistent")
            .await
            .unwrap());
        assert!(backend
            .get_agent_workspace("to-delete")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_update() {
        let (backend, _dir) = test_backend().await;
        let mut rec = test_record("updatable");
        backend.save_agent_workspace(&rec).await.unwrap();

        rec.display_name = "Updated Name".into();
        rec.model = Some("openai/gpt-4o".into());
        rec.bound_channels = vec!["telegram".into()];
        backend.update_agent_workspace(&rec).await.unwrap();

        let loaded = backend
            .get_agent_workspace("updatable")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.display_name, "Updated Name");
        assert_eq!(loaded.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(loaded.bound_channels, vec!["telegram".to_string()]);
    }

    #[tokio::test]
    async fn test_duplicate_agent_id_rejected() {
        let (backend, _dir) = test_backend().await;
        backend
            .save_agent_workspace(&test_record("unique"))
            .await
            .unwrap();
        let result = backend.save_agent_workspace(&test_record("unique")).await;
        assert!(result.is_err());
    }
}

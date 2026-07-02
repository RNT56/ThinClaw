//! Sub-agent run ledger — SubagentRunStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, SUBAGENT_RUN_COLUMNS, fmt_ts, opt_text, row_to_subagent_run_libsql};
use crate::SubagentRunStore;
use thinclaw_agent::subagent::SubagentRunRecord;
use thinclaw_types::error::DatabaseError;

#[async_trait]
impl SubagentRunStore for LibSqlBackend {
    async fn insert_subagent_run(&self, run: &SubagentRunRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO subagent_runs (
                    id, name, task, status, parent_thread_id, routine_run_id,
                    spawned_at, completed_at, error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                run.id.to_string(),
                run.name.clone(),
                run.task.clone(),
                run.status.clone(),
                opt_text(run.parent_thread_id.as_deref()),
                opt_text(run.routine_run_id.as_deref()),
                fmt_ts(&run.spawned_at),
                run.completed_at.as_ref().map(fmt_ts),
                opt_text(run.error.as_deref()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn complete_subagent_run(
        &self,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&chrono::Utc::now());
        conn.execute(
            r#"
                UPDATE subagent_runs SET
                    status = ?2, completed_at = ?3, error = ?4
                WHERE id = ?1
            "#,
            params![id.to_string(), status, now, opt_text(error)],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_incomplete_subagent_runs(&self) -> Result<Vec<SubagentRunRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM subagent_runs WHERE status = 'running' ORDER BY spawned_at ASC",
                    SUBAGENT_RUN_COLUMNS
                ),
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut runs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            runs.push(row_to_subagent_run_libsql(&row)?);
        }
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use thinclaw_agent::subagent::{
        SUBAGENT_RUN_STATUS_COMPLETED, SUBAGENT_RUN_STATUS_FAILED, SUBAGENT_RUN_STATUS_RUNNING,
    };

    /// Build a migrated, file-backed test backend.
    ///
    /// Deliberately NOT `LibSqlBackend::new_memory()`: each `connect()` call
    /// against a `:memory:` libSQL database yields an independent, unshared
    /// in-memory database, so a connection opened after `run_migrations()`
    /// (which uses its own internal connection) sees no tables at all. A
    /// tempfile-backed database persists across connections like production
    /// use does. Mirrors the pattern in `routines.rs`'s `lease_tests`.
    async fn test_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("subagent_runs_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create local backend");
        backend.run_migrations().await.expect("run migrations");
        (backend, dir)
    }

    #[tokio::test]
    async fn insert_and_list_incomplete_subagent_run_round_trips() {
        let (backend, _dir) = test_backend().await;
        let id = Uuid::new_v4();
        let run = SubagentRunRecord::new_running(
            id,
            "researcher",
            "Find papers about AI",
            Some("thread-123".to_string()),
            Some("routine-run-456".to_string()),
            chrono::Utc::now(),
        );

        backend
            .insert_subagent_run(&run)
            .await
            .expect("insert should succeed");

        let incomplete = backend
            .list_incomplete_subagent_runs()
            .await
            .expect("list should succeed");
        assert_eq!(incomplete.len(), 1);
        let stored = &incomplete[0];
        assert_eq!(stored.id, id);
        assert_eq!(stored.name, "researcher");
        assert_eq!(stored.task, "Find papers about AI");
        assert_eq!(stored.status, SUBAGENT_RUN_STATUS_RUNNING);
        assert_eq!(stored.parent_thread_id.as_deref(), Some("thread-123"));
        assert_eq!(stored.routine_run_id.as_deref(), Some("routine-run-456"));
        assert!(stored.completed_at.is_none());
        assert!(stored.error.is_none());
    }

    #[tokio::test]
    async fn complete_subagent_run_marks_success_and_drops_from_incomplete_list() {
        let (backend, _dir) = test_backend().await;
        let id = Uuid::new_v4();
        let run = SubagentRunRecord::new_running(
            id,
            "researcher",
            "task",
            None,
            None,
            chrono::Utc::now(),
        );
        backend.insert_subagent_run(&run).await.unwrap();

        backend
            .complete_subagent_run(id, SUBAGENT_RUN_STATUS_COMPLETED, None)
            .await
            .expect("complete should succeed");

        let incomplete = backend.list_incomplete_subagent_runs().await.unwrap();
        assert!(incomplete.is_empty());
    }

    #[tokio::test]
    async fn complete_subagent_run_persists_failure_reason() {
        let (backend, _dir) = test_backend().await;
        let id = Uuid::new_v4();
        let run = SubagentRunRecord::new_running(
            id,
            "researcher",
            "task",
            None,
            None,
            chrono::Utc::now(),
        );
        backend.insert_subagent_run(&run).await.unwrap();

        backend
            .complete_subagent_run(id, SUBAGENT_RUN_STATUS_FAILED, Some("boom"))
            .await
            .expect("complete should succeed");

        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT status, error, completed_at FROM subagent_runs WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("row should exist");
        let status: String = row.get(0).unwrap();
        let error: String = row.get(1).unwrap();
        let completed_at: Option<String> = row.get(2).ok();
        assert_eq!(status, SUBAGENT_RUN_STATUS_FAILED);
        assert_eq!(error, "boom");
        assert!(completed_at.is_some());
    }

    #[tokio::test]
    async fn list_incomplete_subagent_runs_excludes_completed_rows() {
        let (backend, _dir) = test_backend().await;
        let running_id = Uuid::new_v4();
        let done_id = Uuid::new_v4();
        backend
            .insert_subagent_run(&SubagentRunRecord::new_running(
                running_id,
                "still-running",
                "task",
                None,
                None,
                chrono::Utc::now(),
            ))
            .await
            .unwrap();
        backend
            .insert_subagent_run(&SubagentRunRecord::new_running(
                done_id,
                "already-done",
                "task",
                None,
                None,
                chrono::Utc::now(),
            ))
            .await
            .unwrap();
        backend
            .complete_subagent_run(done_id, SUBAGENT_RUN_STATUS_COMPLETED, None)
            .await
            .unwrap();

        let incomplete = backend.list_incomplete_subagent_runs().await.unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, running_id);
    }
}

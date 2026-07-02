//! Sub-agent run ledger — durable record of `SubagentExecutor` runs.
//!
//! See [`crate::SubagentRunStore`] for the rationale: running sub-agents
//! previously lived only in an in-memory map, so a process restart
//! silently dropped in-flight delegated work.

#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
use thinclaw_agent::subagent::SubagentRunRecord;

#[cfg(feature = "postgres")]
pub(super) fn row_to_subagent_run(
    row: &tokio_postgres::Row,
) -> Result<SubagentRunRecord, DatabaseError> {
    Ok(SubagentRunRecord {
        id: row.get("id"),
        name: row.get("name"),
        task: row.get("task"),
        status: row.get("status"),
        parent_thread_id: row.get("parent_thread_id"),
        routine_run_id: row.get("routine_run_id"),
        spawned_at: row.get("spawned_at"),
        completed_at: row.get("completed_at"),
        error: row.get("error"),
    })
}

#[cfg(feature = "postgres")]
impl Store {
    /// Record a sub-agent run starting.
    pub async fn insert_subagent_run(&self, run: &SubagentRunRecord) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO subagent_runs (
                id, name, task, status, parent_thread_id, routine_run_id,
                spawned_at, completed_at, error
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            &[
                &run.id,
                &run.name,
                &run.task,
                &run.status,
                &run.parent_thread_id,
                &run.routine_run_id,
                &run.spawned_at,
                &run.completed_at,
                &run.error,
            ],
        )
        .await?;
        Ok(())
    }

    /// Mark a sub-agent run as finished.
    pub async fn complete_subagent_run(
        &self,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        conn.execute(
            r#"
            UPDATE subagent_runs SET
                status = $2, completed_at = $3, error = $4
            WHERE id = $1
            "#,
            &[&id, &status, &now, &error],
        )
        .await?;
        Ok(())
    }

    /// List all sub-agent runs still marked `running`.
    pub async fn list_incomplete_subagent_runs(
        &self,
    ) -> Result<Vec<SubagentRunRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM subagent_runs WHERE status = 'running' ORDER BY spawned_at ASC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_subagent_run).collect()
    }
}

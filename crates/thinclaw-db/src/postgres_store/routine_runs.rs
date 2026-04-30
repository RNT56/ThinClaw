#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    /// Record a routine run starting.
    pub async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status = run.status.to_string();
        conn.execute(
            r#"
            INSERT INTO routine_runs (
                id, routine_id, trigger_type, trigger_detail, trigger_key,
                started_at, status, job_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            &[
                &run.id,
                &run.routine_id,
                &run.trigger_type,
                &run.trigger_detail,
                &run.trigger_key,
                &run.started_at,
                &status,
                &run.job_id,
            ],
        )
        .await?;
        Ok(())
    }

    /// Complete a routine run.
    pub async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();
        let now = Utc::now();
        conn.execute(
            r#"
            UPDATE routine_runs SET
                completed_at = $2, status = $3,
                result_summary = $4, tokens_used = $5
            WHERE id = $1
            "#,
            &[&id, &now, &status_str, &result_summary, &tokens_used],
        )
        .await?;
        Ok(())
    }

    /// List recent runs for a routine.
    pub async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_runs
                WHERE routine_id = $1
                ORDER BY started_at DESC
                LIMIT $2
                "#,
                &[&routine_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_run).collect()
    }

    /// Count currently running runs for a routine.
    pub async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE routine_id = $1 AND status = 'running'",
                &[&routine_id],
            )
            .await?;
        Ok(row.get("cnt"))
    }

    /// Count ALL currently running routine runs across all routines.
    pub async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE status = 'running'",
                &[],
            )
            .await?;
        Ok(row.get("cnt"))
    }

    /// Link a routine run to a dispatched job.
    pub async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE routine_runs SET job_id = $1 WHERE id = $2",
            &[&job_id, &run_id],
        )
        .await?;
        Ok(())
    }

    /// Mark RUNNING routine runs older than 10 minutes as failed (zombie reaping).
    ///
    /// Only reaps runs that have been in `running` status for more than
    /// 10 minutes. This prevents the reaper from killing actively-executing
    /// worker jobs that were dispatched recently.
    pub async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let cutoff = now
            - chrono::Duration::try_minutes(10).expect("10 minutes is a valid chrono::Duration");
        let count = conn
            .execute(
                r#"
                UPDATE routine_runs SET
                    status = 'failed',
                    completed_at = $1,
                    result_summary = 'Orphaned: routine exceeded 10-minute TTL'
                WHERE status = 'running'
                  AND started_at < $2
                "#,
                &[&now, &cutoff],
            )
            .await?;
        Ok(count)
    }

    pub async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                "DELETE FROM routine_runs WHERE routine_id = $1",
                &[&routine_id],
            )
            .await?;
        Ok(count)
    }

    pub async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn.execute("DELETE FROM routine_runs", &[]).await?;
        Ok(count)
    }
}

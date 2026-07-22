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

    pub async fn try_admit_routine_run(
        &self,
        run: &RoutineRun,
        routine_limit: i64,
        global_limit: i64,
        initial_lease_expires_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<RoutineRunAdmission, DatabaseError> {
        const ROUTINE_ADMISSION_LOCK_KEY: i64 = 0x5448_494E_434C_4157;

        let mut conn = self.conn().await?;
        let tx = conn.transaction().await?;
        tx.batch_execute("SET LOCAL lock_timeout = '5s'; SET LOCAL statement_timeout = '5s'")
            .await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock($1)",
            &[&ROUTINE_ADMISSION_LOCK_KEY],
        )
        .await?;

        if let Some(trigger_key) = run.trigger_key.as_deref()
            && let Some(row) = tx
                .query_opt(
                    "SELECT id FROM routine_runs \
                     WHERE routine_id = $1 AND trigger_key = $2 \
                     ORDER BY started_at ASC LIMIT 1",
                    &[&run.routine_id, &trigger_key],
                )
                .await?
        {
            let existing: Uuid = row.get(0);
            tx.commit().await?;
            return Ok(RoutineRunAdmission::Duplicate(existing));
        }

        let counts = tx
            .query_one(
                r#"
                SELECT
                    COUNT(*) FILTER (WHERE routine_id = $1) AS routine_running,
                    COUNT(*) AS global_running
                FROM routine_runs
                WHERE status = 'running'
                "#,
                &[&run.routine_id],
            )
            .await?;
        let routine_running: i64 = counts.get("routine_running");
        let global_running: i64 = counts.get("global_running");
        if routine_running >= routine_limit {
            tx.commit().await?;
            return Ok(RoutineRunAdmission::RoutineCapacity);
        }
        if global_running >= global_limit {
            tx.commit().await?;
            return Ok(RoutineRunAdmission::GlobalCapacity);
        }

        let status = run.status.to_string();
        tx.execute(
            r#"
            INSERT INTO routine_runs (
                id, routine_id, trigger_type, trigger_detail, trigger_key,
                started_at, status, job_id, lease_expires_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
                &initial_lease_expires_at,
            ],
        )
        .await?;
        tx.execute(
            r#"
            UPDATE routines SET
                last_run_at = $2,
                next_fire_at = $3,
                run_count = run_count + 1,
                updated_at = now()
            WHERE id = $1
            "#,
            &[&run.routine_id, &run.started_at, &next_fire_at],
        )
        .await?;
        tx.commit().await?;
        Ok(RoutineRunAdmission::Admitted)
    }

    /// Complete a routine run.
    pub async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<RoutineRunCompletion, DatabaseError> {
        if status == RunStatus::Running {
            return Err(DatabaseError::Constraint(
                "routine completion status must be terminal".to_string(),
            ));
        }
        let mut conn = self.conn().await?;
        let tx = conn.transaction().await?;
        tx.batch_execute("SET LOCAL lock_timeout = '5s'; SET LOCAL statement_timeout = '5s'")
            .await?;
        let status_str = status.to_string();
        let now = Utc::now();
        let Some(row) = tx
            .query_opt(
                r#"
            UPDATE routine_runs SET
                completed_at = $2, status = $3,
                result_summary = $4, tokens_used = $5,
                lease_expires_at = NULL
            WHERE id = $1 AND status = 'running'
            RETURNING routine_id
            "#,
                &[&id, &now, &status_str, &result_summary, &tokens_used],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(RoutineRunCompletion::AlreadyTerminal);
        };
        let routine_id: Uuid = row.get(0);
        let failure_row = tx
            .query_one(
                r#"
                UPDATE routines SET
                    consecutive_failures = CASE
                        WHEN $2 = 'failed' THEN consecutive_failures + 1
                        ELSE 0
                    END,
                    updated_at = now()
                WHERE id = $1
                RETURNING consecutive_failures
                "#,
                &[&routine_id, &status_str],
            )
            .await?;
        let failures: i32 = failure_row.get(0);
        let consecutive_failures = u32::try_from(failures).map_err(|error| {
            DatabaseError::Serialization(format!("invalid failure counter: {error}"))
        })?;
        tx.commit().await?;
        Ok(RoutineRunCompletion::Completed {
            routine_id,
            consecutive_failures,
        })
    }

    pub async fn apply_routine_failure_policy(
        &self,
        routine_id: Uuid,
        expected_consecutive_failures: u32,
        not_before: DateTime<Utc>,
        disable: bool,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let failures = i32::try_from(expected_consecutive_failures).map_err(|error| {
            DatabaseError::Serialization(format!("invalid failure counter: {error}"))
        })?;
        let changed = conn
            .execute(
                r#"
                UPDATE routines SET
                    next_fire_at = CASE
                        WHEN next_fire_at IS NOT NULL AND next_fire_at < $3 THEN $3
                        ELSE next_fire_at
                    END,
                    enabled = CASE WHEN $4 THEN FALSE ELSE enabled END,
                    updated_at = now()
                WHERE id = $1 AND consecutive_failures = $2
                "#,
                &[&routine_id, &failures, &not_before, &disable],
            )
            .await?;
        if changed > 0 && disable {
            self.bump_routine_event_cache_version(&conn).await?;
        }
        Ok(changed > 0)
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

    /// Renew (or set) the lease on a RUNNING routine run.
    ///
    /// See [`crate::RoutineStore::renew_routine_run_lease`] for the full
    /// rationale — this replaces the old fixed 10-minute zombie TTL with a
    /// renewable lease so long-running full-job routine runs aren't falsely
    /// reaped while their worker is still actively executing.
    pub async fn renew_routine_run_lease(
        &self,
        run_id: Uuid,
        lease_secs: i64,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let lease_expires_at =
            Utc::now() + chrono::Duration::try_seconds(lease_secs.max(0)).unwrap_or_default();
        conn.execute(
            r#"
            UPDATE routine_runs SET
                lease_expires_at = $1
            WHERE id = $2
              AND status = 'running'
            "#,
            &[&lease_expires_at, &run_id],
        )
        .await?;
        Ok(())
    }

    /// Mark RUNNING routine runs with an expired lease as failed (zombie reaping).
    ///
    /// Reaps runs whose `lease_expires_at` has passed. Legacy rows with a
    /// NULL lease fall back to `legacy_ttl_secs` measured from `started_at`
    /// instead of the old hardcoded 10-minute cutoff.
    pub async fn cleanup_stale_routine_runs(
        &self,
        legacy_ttl_secs: i64,
    ) -> Result<RoutineRunReapResult, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let legacy_cutoff =
            now - chrono::Duration::try_seconds(legacy_ttl_secs.max(0)).unwrap_or_default();
        let rows = conn
            .query(
                r#"
                SELECT id FROM routine_runs
                WHERE status = 'running'
                  AND (
                      (lease_expires_at IS NOT NULL AND lease_expires_at < $1)
                      OR (lease_expires_at IS NULL AND started_at < $2)
                  )
                "#,
                &[&now, &legacy_cutoff],
            )
            .await?;
        let run_ids = rows
            .into_iter()
            .map(|row| row.get::<_, Uuid>(0))
            .collect::<Vec<_>>();
        drop(conn);

        let mut result = RoutineRunReapResult::default();
        for run_id in run_ids {
            if let RoutineRunCompletion::Completed {
                routine_id,
                consecutive_failures,
            } = self
                .complete_routine_run(
                    run_id,
                    RunStatus::Failed,
                    Some("Orphaned: routine run lease expired"),
                    None,
                )
                .await?
            {
                result.reaped += 1;
                result
                    .failure_streaks
                    .push((routine_id, consecutive_failures));
            }
        }
        Ok(result)
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

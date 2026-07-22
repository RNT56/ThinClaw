//! Routine-related RoutineStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::{TransactionBehavior, params};
use uuid::Uuid;

use super::{
    LibSqlBackend, ROUTINE_COLUMNS, ROUTINE_EVENT_COLUMNS, ROUTINE_EVENT_EVALUATION_COLUMNS,
    ROUTINE_RUN_COLUMNS, ROUTINE_TRIGGER_COLUMNS, fmt_opt_ts, fmt_ts, get_i64, get_text, opt_text,
    opt_text_owned, row_to_routine_event_evaluation_libsql, row_to_routine_event_libsql,
    row_to_routine_libsql, row_to_routine_run_libsql, row_to_routine_trigger_libsql,
};
use crate::{RoutineRunAdmission, RoutineRunCompletion, RoutineRunReapResult, RoutineStore};
use thinclaw_types::error::DatabaseError;
use thinclaw_types::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};

const EVENT_CACHE_VERSION_USER: &str = "system";
const EVENT_CACHE_VERSION_KEY: &str = "routine.event_cache_version";

async fn bump_routine_event_cache_version(conn: &libsql::Connection) -> Result<(), DatabaseError> {
    conn.execute(
        r#"
            INSERT INTO settings (user_id, key, value, updated_at)
            VALUES (?1, ?2, '1', ?3)
            ON CONFLICT(user_id, key) DO UPDATE SET
                value = CAST(COALESCE(NULLIF(settings.value, ''), '0') AS INTEGER) + 1,
                updated_at = excluded.updated_at
        "#,
        params![
            EVENT_CACHE_VERSION_USER,
            EVENT_CACHE_VERSION_KEY,
            fmt_ts(&Utc::now())
        ],
    )
    .await
    .map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok(())
}

#[async_trait]
impl RoutineStore for LibSqlBackend {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i64;
        let max_concurrent = routine.guardrails.max_concurrent as i64;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i64);
        let policy_config = serde_json::to_string(&routine.policy)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
                INSERT INTO routines (
                    id, name, description, user_id, actor_id, enabled,
                    trigger_type, trigger_config, action_type, action_config,
                    cooldown_secs, max_concurrent, dedup_window_secs,
                    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
                    policy_config, state, next_fire_at, config_version, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18,
                    ?19, ?20, ?21, ?22, ?23, ?24
                )
            "#,
            params![
                routine.id.to_string(),
                routine.name.as_str(),
                routine.description.as_str(),
                routine.user_id.as_str(),
                routine.actor_id.as_str(),
                routine.enabled as i64,
                trigger_type,
                trigger_config.to_string(),
                action_type,
                action_config.to_string(),
                cooldown_secs,
                max_concurrent,
                dedup_window_secs,
                opt_text(routine.notify.channel.as_deref()),
                routine.notify.user.as_str(),
                routine.notify.on_success as i64,
                routine.notify.on_failure as i64,
                routine.notify.on_attention as i64,
                policy_config,
                routine.state.to_string(),
                fmt_opt_ts(&routine.next_fire_at),
                routine.config_version.max(1),
                fmt_ts(&routine.created_at),
                fmt_ts(&routine.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        bump_routine_event_cache_version(&conn).await?;
        Ok(())
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {} FROM routines WHERE id = ?1", ROUTINE_COLUMNS),
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_routine_libsql(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE user_id = ?1 AND name = ?2",
                    ROUTINE_COLUMNS
                ),
                params![user_id, name],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_routine_libsql(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE user_id = ?1 ORDER BY name",
                    ROUTINE_COLUMNS
                ),
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type = 'event'",
                    ROUTINE_COLUMNS
                ),
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn get_routine_event_cache_version(&self) -> Result<i64, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT value FROM settings WHERE user_id = ?1 AND key = ?2 LIMIT 1",
                params![EVENT_CACHE_VERSION_USER, EVENT_CACHE_VERSION_KEY],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_text(&row, 0).parse::<i64>().unwrap_or(0)),
            None => Ok(0),
        }
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type IN ('cron', 'system_event') AND next_fire_at IS NOT NULL AND next_fire_at <= ?1",
                    ROUTINE_COLUMNS
                ),
                params![now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i64;
        let max_concurrent = routine.guardrails.max_concurrent as i64;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i64);
        let policy_config = serde_json::to_string(&routine.policy)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let now = fmt_ts(&Utc::now());

        conn.execute(
            r#"
                UPDATE routines SET
                    name = ?2, description = ?3, actor_id = ?4, enabled = ?5,
                    trigger_type = ?6, trigger_config = ?7,
                    action_type = ?8, action_config = ?9,
                    cooldown_secs = ?10, max_concurrent = ?11, dedup_window_secs = ?12,
                    notify_channel = ?13, notify_user = ?14,
                    notify_on_success = ?15, notify_on_failure = ?16, notify_on_attention = ?17,
                    policy_config = ?18, state = ?19, next_fire_at = ?20,
                    config_version = config_version + 1,
                    updated_at = ?21
                WHERE id = ?1
            "#,
            params![
                routine.id.to_string(),
                routine.name.as_str(),
                routine.description.as_str(),
                routine.actor_id.as_str(),
                routine.enabled as i64,
                trigger_type,
                trigger_config.to_string(),
                action_type,
                action_config.to_string(),
                cooldown_secs,
                max_concurrent,
                dedup_window_secs,
                opt_text(routine.notify.channel.as_deref()),
                routine.notify.user.as_str(),
                routine.notify.on_success as i64,
                routine.notify.on_failure as i64,
                routine.notify.on_attention as i64,
                policy_config,
                routine.state.to_string(),
                fmt_opt_ts(&routine.next_fire_at),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        bump_routine_event_cache_version(&conn).await?;
        Ok(())
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                UPDATE routines SET
                    last_run_at = ?2, next_fire_at = ?3,
                    run_count = ?4, consecutive_failures = ?5,
                    state = ?6, updated_at = ?7
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_ts(&last_run_at),
                fmt_opt_ts(&next_fire_at),
                run_count as i64,
                consecutive_failures as i64,
                state.to_string(),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn advance_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routines SET
                    last_run_at = ?2,
                    next_fire_at = ?3,
                    run_count = run_count + 1,
                    consecutive_failures = 0,
                    updated_at = ?4
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_ts(&last_run_at),
                fmt_opt_ts(&next_fire_at),
                fmt_ts(&Utc::now()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn set_routine_next_fire_at(
        &self,
        id: Uuid,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE routines SET next_fire_at = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id.to_string(),
                fmt_opt_ts(&next_fire_at),
                fmt_ts(&Utc::now()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM routines WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if count > 0 {
            bump_routine_event_cache_version(&conn).await?;
        }
        Ok(count > 0)
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO routine_runs (
                    id, routine_id, trigger_type, trigger_detail, trigger_key,
                    started_at, status, job_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                run.id.to_string(),
                run.routine_id.to_string(),
                run.trigger_type.as_str(),
                opt_text(run.trigger_detail.as_deref()),
                opt_text(run.trigger_key.as_deref()),
                fmt_ts(&run.started_at),
                run.status.to_string(),
                opt_text_owned(run.job_id.map(|id| id.to_string())),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn try_admit_routine_run(
        &self,
        run: &RoutineRun,
        routine_limit: i64,
        global_limit: i64,
        initial_lease_expires_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> Result<RoutineRunAdmission, DatabaseError> {
        let _transaction_guard = self.transaction_lock.lock().await;
        let conn = self.connect().await?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            if let Some(trigger_key) = run.trigger_key.as_deref() {
                let mut rows = tx
                    .query(
                        "SELECT id FROM routine_runs \
                         WHERE routine_id = ?1 AND trigger_key = ?2 \
                         ORDER BY started_at ASC LIMIT 1",
                        params![run.routine_id.to_string(), trigger_key],
                    )
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                if let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?
                {
                    let existing = Uuid::parse_str(&get_text(&row, 0)).map_err(|error| {
                        DatabaseError::Serialization(format!(
                            "invalid duplicate routine run id: {error}"
                        ))
                    })?;
                    return Ok(RoutineRunAdmission::Duplicate(existing));
                }
            }

            let mut rows = tx
                .query(
                    "SELECT \
                         COALESCE(SUM(CASE WHEN routine_id = ?1 THEN 1 ELSE 0 END), 0), \
                         COUNT(*) \
                     FROM routine_runs WHERE status = 'running'",
                    params![run.routine_id.to_string()],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
                .ok_or_else(|| {
                    DatabaseError::Query("routine capacity query returned no row".into())
                })?;
            if get_i64(&row, 0) >= routine_limit {
                return Ok(RoutineRunAdmission::RoutineCapacity);
            }
            if get_i64(&row, 1) >= global_limit {
                return Ok(RoutineRunAdmission::GlobalCapacity);
            }

            tx.execute(
                r#"
                    INSERT INTO routine_runs (
                        id, routine_id, trigger_type, trigger_detail, trigger_key,
                        started_at, status, job_id, lease_expires_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                params![
                    run.id.to_string(),
                    run.routine_id.to_string(),
                    run.trigger_type.as_str(),
                    opt_text(run.trigger_detail.as_deref()),
                    opt_text(run.trigger_key.as_deref()),
                    fmt_ts(&run.started_at),
                    run.status.to_string(),
                    opt_text_owned(run.job_id.map(|id| id.to_string())),
                    fmt_ts(&initial_lease_expires_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            tx.execute(
                r#"
                    UPDATE routines SET
                        last_run_at = ?2,
                        next_fire_at = ?3,
                        run_count = run_count + 1,
                        updated_at = ?4
                    WHERE id = ?1
                "#,
                params![
                    run.routine_id.to_string(),
                    fmt_ts(&run.started_at),
                    fmt_opt_ts(&next_fire_at),
                    fmt_ts(&Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            Ok(RoutineRunAdmission::Admitted)
        }
        .await;

        match result {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn complete_routine_run(
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
        let _transaction_guard = self.transaction_lock.lock().await;
        let conn = self.connect().await?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let result = async {
            let changed = tx
                .execute(
                    r#"
                        UPDATE routine_runs SET
                            completed_at = ?5, status = ?2,
                            result_summary = ?3, tokens_used = ?4,
                            lease_expires_at = NULL
                        WHERE id = ?1 AND status = 'running'
                    "#,
                    params![
                        id.to_string(),
                        status.to_string(),
                        opt_text(result_summary),
                        tokens_used.map(|t| t as i64),
                        fmt_ts(&Utc::now()),
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            if changed == 0 {
                return Ok(RoutineRunCompletion::AlreadyTerminal);
            }

            let mut rows = tx
                .query(
                    "SELECT routine_id FROM routine_runs WHERE id = ?1 LIMIT 1",
                    params![id.to_string()],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
                .ok_or_else(|| DatabaseError::Query("completed routine run disappeared".into()))?;
            let routine_id = Uuid::parse_str(&get_text(&row, 0)).map_err(|error| {
                DatabaseError::Serialization(format!("invalid routine id on run: {error}"))
            })?;

            tx.execute(
                r#"
                    UPDATE routines SET
                        consecutive_failures = CASE
                            WHEN ?2 = 'failed' THEN consecutive_failures + 1
                            ELSE 0
                        END,
                        updated_at = ?3
                    WHERE id = ?1
                "#,
                params![
                    routine_id.to_string(),
                    status.to_string(),
                    fmt_ts(&Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let mut rows = tx
                .query(
                    "SELECT consecutive_failures FROM routines WHERE id = ?1 LIMIT 1",
                    params![routine_id.to_string()],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
                .ok_or_else(|| DatabaseError::Query("parent routine disappeared".into()))?;
            let consecutive_failures = u32::try_from(get_i64(&row, 0)).map_err(|error| {
                DatabaseError::Serialization(format!("invalid failure counter: {error}"))
            })?;
            Ok(RoutineRunCompletion::Completed {
                routine_id,
                consecutive_failures,
            })
        }
        .await;

        match result {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn apply_routine_failure_policy(
        &self,
        routine_id: Uuid,
        expected_consecutive_failures: u32,
        not_before: DateTime<Utc>,
        disable: bool,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let changed = conn
            .execute(
                r#"
                    UPDATE routines SET
                        next_fire_at = CASE
                            WHEN next_fire_at IS NOT NULL AND next_fire_at < ?3 THEN ?3
                            ELSE next_fire_at
                        END,
                        enabled = CASE WHEN ?4 THEN 0 ELSE enabled END,
                        updated_at = ?5
                    WHERE id = ?1 AND consecutive_failures = ?2
                "#,
                params![
                    routine_id.to_string(),
                    i64::from(expected_consecutive_failures),
                    fmt_ts(&not_before),
                    disable as i64,
                    fmt_ts(&Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if changed > 0 && disable {
            bump_routine_event_cache_version(&conn).await?;
        }
        Ok(changed > 0)
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_runs WHERE routine_id = ?1 ORDER BY started_at DESC LIMIT ?2",
                    ROUTINE_RUN_COLUMNS
                ),
                params![routine_id.to_string(), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut runs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            runs.push(row_to_routine_run_libsql(&row)?);
        }
        Ok(runs)
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE routine_id = ?1 AND status = 'running'",
                params![routine_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_i64(&row, 0)),
            None => Ok(0),
        }
    }

    async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE status = 'running'",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_i64(&row, 0)),
            None => Ok(0),
        }
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE routine_runs SET job_id = ?1 WHERE id = ?2",
            params![job_id.to_string(), run_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn renew_routine_run_lease(
        &self,
        run_id: Uuid,
        lease_secs: i64,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = Utc::now();
        let lease_expires_at = fmt_ts(
            &(now
                + chrono::Duration::try_seconds(lease_secs.max(0))
                    .unwrap_or_else(|| chrono::Duration::seconds(0))),
        );
        conn.execute(
            r#"
                UPDATE routine_runs SET
                    lease_expires_at = ?1
                WHERE id = ?2
                  AND status = 'running'
            "#,
            params![lease_expires_at, run_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn cleanup_stale_routine_runs(
        &self,
        legacy_ttl_secs: i64,
    ) -> Result<RoutineRunReapResult, DatabaseError> {
        let conn = self.connect().await?;
        let now = Utc::now();
        let now_str = fmt_ts(&now);
        let legacy_cutoff = fmt_ts(
            &(now
                - chrono::Duration::try_seconds(legacy_ttl_secs.max(0))
                    .unwrap_or_else(|| chrono::Duration::seconds(0))),
        );
        let mut rows = conn
            .query(
                r#"
                    SELECT id FROM routine_runs
                    WHERE status = 'running'
                      AND (
                          (lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                          OR (lease_expires_at IS NULL AND started_at < ?2)
                      )
                "#,
                params![now_str, legacy_cutoff],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut run_ids = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            run_ids.push(Uuid::parse_str(&get_text(&row, 0)).map_err(|error| {
                DatabaseError::Serialization(format!("invalid stale routine run id: {error}"))
            })?);
        }
        drop(rows);

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

    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM routine_runs WHERE routine_id = ?1",
                params![routine_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count)
    }

    async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute("DELETE FROM routine_runs", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count)
    }

    async fn create_routine_event(
        &self,
        event: &RoutineEvent,
    ) -> Result<RoutineEvent, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO routine_event_inbox (
                    id, principal_id, actor_id, channel, event_type, raw_sender_id,
                    conversation_scope_id, stable_external_conversation_key, idempotency_key,
                    content, content_hash, metadata, status, diagnostics,
                    claimed_by, claimed_at, lease_expires_at, processed_at, error_message,
                    matched_routines, fired_routines, attempt_count, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19,
                    ?20, ?21, ?22, ?23
                )
                ON CONFLICT(idempotency_key) DO NOTHING
            "#,
            params![
                event.id.to_string(),
                event.principal_id.as_str(),
                event.actor_id.as_str(),
                event.channel.as_str(),
                event.event_type.as_str(),
                event.raw_sender_id.as_str(),
                event.conversation_scope_id.as_str(),
                event.stable_external_conversation_key.as_str(),
                event.idempotency_key.as_str(),
                event.content.as_str(),
                event.content_hash.as_str(),
                event.metadata.to_string(),
                event.status.to_string(),
                event.diagnostics.to_string(),
                opt_text(event.claimed_by.as_deref()),
                fmt_opt_ts(&event.claimed_at),
                fmt_opt_ts(&event.lease_expires_at),
                fmt_opt_ts(&event.processed_at),
                opt_text(event.error_message.as_deref()),
                event.matched_routines as i64,
                event.fired_routines as i64,
                event.attempt_count as i64,
                fmt_ts(&event.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_inbox WHERE idempotency_key = ?1 LIMIT 1",
                    ROUTINE_EVENT_COLUMNS
                ),
                params![event.idempotency_key.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| DatabaseError::Query("failed to load persisted routine event".into()))?;
        row_to_routine_event_libsql(&row)
    }

    async fn claim_routine_event(
        &self,
        id: Uuid,
        worker_id: &str,
        stale_before: DateTime<Utc>,
    ) -> Result<Option<RoutineEvent>, DatabaseError> {
        let _transaction_guard = self.transaction_lock.lock().await;
        let conn = self.connect().await?;
        let now = Utc::now();
        let lease_duration = now.signed_duration_since(stale_before);
        let claimed_at = fmt_ts(&now);
        let lease_expires_at = fmt_ts(&(now + lease_duration));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let mut rows = tx
                .query(
                    &format!(
                        "SELECT {} FROM routine_event_inbox
                         WHERE id = ?1
                           AND (
                             (status = 'pending' AND (next_attempt_at IS NULL OR next_attempt_at <= ?2))
                             OR (
                               status = 'processing'
                               AND (
                                   (lease_expires_at IS NOT NULL AND lease_expires_at < ?2)
                                   OR (claimed_at IS NOT NULL AND claimed_at < ?3)
                               )
                             )
                           )
                         LIMIT 1",
                        ROUTINE_EVENT_COLUMNS
                    ),
                    params![id.to_string(), fmt_ts(&now), fmt_ts(&stale_before)],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            else {
                return Ok(None);
            };

            tx.execute(
                r#"
                    UPDATE routine_event_inbox
                    SET status = 'processing',
                        claimed_by = ?2,
                        claimed_at = ?3,
                        lease_expires_at = ?4,
                        attempt_count = attempt_count + 1,
                        error_message = NULL
                    WHERE id = ?1
                "#,
                params![id.to_string(), worker_id, claimed_at, lease_expires_at],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let mut refreshed = row_to_routine_event_libsql(&row)?;
            refreshed.status = thinclaw_types::routine::RoutineEventStatus::Processing;
            refreshed.claimed_by = Some(worker_id.to_string());
            refreshed.claimed_at = Some(now);
            refreshed.lease_expires_at = Some(now + lease_duration);
            refreshed.attempt_count += 1;
            refreshed.error_message = None;
            Ok(Some(refreshed))
        }
        .await;

        match result {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn release_routine_event(
        &self,
        id: Uuid,
        next_attempt_at: DateTime<Utc>,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_event_inbox
                SET status = 'pending',
                    diagnostics = ?2,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = ?3,
                    processed_at = NULL
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                diagnostics.to_string(),
                fmt_ts(&next_attempt_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_pending_routine_events(
        &self,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_inbox
                     WHERE (status = 'pending' AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                        OR (
                            status = 'processing'
                            AND (
                                (lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                                OR (claimed_at IS NOT NULL AND claimed_at < ?2)
                            )
                        )
                     ORDER BY attempt_count ASC, created_at ASC
                     LIMIT ?3",
                    ROUTINE_EVENT_COLUMNS
                ),
                params![now, fmt_ts(&stale_before), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(row_to_routine_event_libsql(&row)?);
        }
        Ok(events)
    }

    async fn complete_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        matched_routines: u32,
        fired_routines: u32,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_event_inbox
                SET status = 'processed',
                    processed_at = ?2,
                    matched_routines = ?3,
                    fired_routines = ?4,
                    diagnostics = ?5,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = NULL,
                    error_message = NULL
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_ts(&processed_at),
                matched_routines as i64,
                fired_routines as i64,
                diagnostics.to_string(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn fail_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_event_inbox
                SET status = 'failed',
                    processed_at = ?2,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = NULL,
                    error_message = ?3
                WHERE id = ?1
            "#,
            params![id.to_string(), fmt_ts(&processed_at), error_message],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn dead_letter_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_event_inbox
                SET status = 'dead_lettered',
                    processed_at = ?2,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = NULL,
                    error_message = ?3,
                    diagnostics = ?4
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_ts(&processed_at),
                error_message,
                diagnostics.to_string()
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn replay_routine_event(
        &self,
        id: Uuid,
        user_id: &str,
        actor_id: &str,
        diagnostics: &serde_json::Value,
    ) -> Result<Option<RoutineEvent>, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                r#"
                    UPDATE routine_event_inbox
                    SET status = 'pending',
                        diagnostics = ?4,
                        claimed_by = NULL,
                        claimed_at = NULL,
                        lease_expires_at = NULL,
                        next_attempt_at = NULL,
                        processed_at = NULL,
                        error_message = NULL,
                        matched_routines = 0,
                        fired_routines = 0,
                        attempt_count = 0
                    WHERE id = ?1
                      AND principal_id = ?2
                      AND actor_id = ?3
                      AND status IN ('failed', 'dead_lettered')
                "#,
                params![id.to_string(), user_id, actor_id, diagnostics.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if count == 0 {
            return Ok(None);
        }

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_inbox WHERE id = ?1 LIMIT 1",
                    ROUTINE_EVENT_COLUMNS
                ),
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        else {
            return Ok(None);
        };
        row_to_routine_event_libsql(&row).map(Some)
    }

    async fn list_routine_events_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_inbox
                     WHERE principal_id = ?1 AND actor_id = ?2
                     ORDER BY created_at DESC
                     LIMIT ?3",
                    ROUTINE_EVENT_COLUMNS
                ),
                params![user_id, actor_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(row_to_routine_event_libsql(&row)?);
        }
        Ok(events)
    }

    async fn upsert_routine_event_evaluation(
        &self,
        evaluation: &RoutineEventEvaluation,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO routine_event_evaluations (
                    id, event_id, routine_id, decision, reason, details, sequence_num,
                    channel, content_preview, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                )
                ON CONFLICT(event_id, routine_id) DO UPDATE SET
                    decision = excluded.decision,
                    reason = excluded.reason,
                    details = excluded.details,
                    sequence_num = excluded.sequence_num,
                    channel = excluded.channel,
                    content_preview = excluded.content_preview
            "#,
            params![
                evaluation.id.to_string(),
                evaluation.event_id.to_string(),
                evaluation.routine_id.to_string(),
                evaluation.decision.to_string(),
                opt_text(evaluation.reason.as_deref()),
                evaluation.details.to_string(),
                evaluation.sequence_num as i64,
                evaluation.channel.as_str(),
                evaluation.content_preview.as_str(),
                fmt_ts(&evaluation.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_routine_event_evaluations_for_event(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_evaluations
                     WHERE event_id = ?1
                     ORDER BY sequence_num ASC",
                    ROUTINE_EVENT_EVALUATION_COLUMNS
                ),
                params![event_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut evaluations = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            evaluations.push(row_to_routine_event_evaluation_libsql(&row)?);
        }
        Ok(evaluations)
    }

    async fn list_routine_event_evaluations(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_event_evaluations
                     WHERE routine_id = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2",
                    ROUTINE_EVENT_EVALUATION_COLUMNS
                ),
                params![routine_id.to_string(), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut evaluations = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            evaluations.push(row_to_routine_event_evaluation_libsql(&row)?);
        }
        Ok(evaluations)
    }

    async fn routine_run_exists_for_trigger_key(
        &self,
        routine_id: Uuid,
        trigger_key: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM routine_runs WHERE routine_id = ?1 AND trigger_key = ?2 AND status != 'failed'",
                params![routine_id.to_string(), trigger_key],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_i64(&row, 0) > 0),
            None => Ok(false),
        }
    }

    async fn routine_event_recent_content_match(
        &self,
        routine_id: Uuid,
        content_hash: &str,
        since: DateTime<Utc>,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) \
                 FROM routine_event_evaluations e \
                 JOIN routine_event_inbox i ON i.id = e.event_id \
                 WHERE e.routine_id = ?1 \
                   AND e.decision = 'fired' \
                   AND i.content_hash = ?2 \
                   AND e.created_at >= ?3",
                params![routine_id.to_string(), content_hash, fmt_ts(&since)],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_i64(&row, 0) > 0),
            None => Ok(false),
        }
    }

    async fn enqueue_routine_trigger(&self, trigger: &RoutineTrigger) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let decision = trigger.decision.map(|value| value.to_string());
        conn.execute(
            r#"
                INSERT INTO routine_trigger_queue (
                    id, routine_id, trigger_kind, trigger_label, due_at, status, decision,
                    active_key, idempotency_key, claimed_by, claimed_at, lease_expires_at,
                    processed_at, error_message, diagnostics, coalesced_count, backlog_collapsed,
                    routine_config_version, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17,
                    ?18, ?19
                )
                ON CONFLICT(active_key) DO UPDATE SET
                    due_at = CASE
                        WHEN excluded.due_at > routine_trigger_queue.due_at THEN excluded.due_at
                        ELSE routine_trigger_queue.due_at
                    END,
                    diagnostics = json_patch(routine_trigger_queue.diagnostics, excluded.diagnostics),
                    coalesced_count = routine_trigger_queue.coalesced_count + 1,
                    backlog_collapsed = CASE
                        WHEN excluded.backlog_collapsed != 0 OR routine_trigger_queue.backlog_collapsed != 0 THEN 1
                        ELSE 0
                    END,
                    routine_config_version = excluded.routine_config_version
            "#,
            params![
                trigger.id.to_string(),
                trigger.routine_id.to_string(),
                trigger.trigger_kind.to_string(),
                opt_text(trigger.trigger_label.as_deref()),
                fmt_ts(&trigger.due_at),
                trigger.status.to_string(),
                opt_text(decision.as_deref()),
                opt_text(trigger.active_key.as_deref()),
                trigger.idempotency_key.as_str(),
                opt_text(trigger.claimed_by.as_deref()),
                fmt_opt_ts(&trigger.claimed_at),
                fmt_opt_ts(&trigger.lease_expires_at),
                fmt_opt_ts(&trigger.processed_at),
                opt_text(trigger.error_message.as_deref()),
                trigger.diagnostics.to_string(),
                trigger.coalesced_count as i64,
                trigger.backlog_collapsed as i64,
                trigger.routine_config_version,
                fmt_ts(&trigger.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn claim_routine_triggers(
        &self,
        worker_id: &str,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        let _transaction_guard = self.transaction_lock.lock().await;
        let conn = self.connect().await?;
        let now = Utc::now();
        let lease_duration = now.signed_duration_since(stale_before);
        let claimed_at = fmt_ts(&now);
        let lease_expires_at = fmt_ts(&(now + lease_duration));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let mut rows = tx
                .query(
                    &format!(
                        "SELECT {} FROM routine_trigger_queue
                         WHERE (status = 'pending' AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                            OR (
                                status = 'processing'
                                AND (
                                    (lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                                    OR (claimed_at IS NOT NULL AND claimed_at < ?2)
                                )
                            )
                         ORDER BY
                            CASE WHEN status = 'pending' AND next_attempt_at IS NULL THEN 0 ELSE 1 END ASC,
                            due_at ASC,
                            created_at ASC
                         LIMIT ?3",
                        ROUTINE_TRIGGER_COLUMNS
                    ),
                    params![fmt_ts(&now), fmt_ts(&stale_before), limit],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let mut claimed = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let trigger = row_to_routine_trigger_libsql(&row)?;
                tx.execute(
                    r#"
                        UPDATE routine_trigger_queue
                        SET status = 'processing',
                            claimed_by = ?2,
                            claimed_at = ?3,
                            lease_expires_at = ?4,
                            error_message = NULL
                        WHERE id = ?1
                    "#,
                    params![
                        trigger.id.to_string(),
                        worker_id,
                        claimed_at.clone(),
                        lease_expires_at.clone()
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

                let mut refreshed = trigger.clone();
                refreshed.status = thinclaw_types::routine::RoutineTriggerStatus::Processing;
                refreshed.claimed_by = Some(worker_id.to_string());
                refreshed.claimed_at = Some(now);
                refreshed.lease_expires_at = Some(now + lease_duration);
                refreshed.error_message = None;
                claimed.push(refreshed);
            }

            Ok(claimed)
        }
        .await;

        match result {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn release_routine_trigger(
        &self,
        id: Uuid,
        next_attempt_at: DateTime<Utc>,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_trigger_queue
                SET status = 'pending',
                    diagnostics = json_patch(diagnostics, ?2),
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = ?3,
                    processed_at = NULL,
                    error_message = NULL
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                diagnostics.to_string(),
                fmt_ts(&next_attempt_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn complete_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        decision: RoutineTriggerDecision,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_trigger_queue
                SET status = 'processed',
                    decision = ?3,
                    processed_at = ?2,
                    diagnostics = ?4,
                    active_key = NULL,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = NULL,
                    error_message = NULL
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_ts(&processed_at),
                decision.to_string(),
                diagnostics.to_string(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn fail_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_trigger_queue
                SET status = 'failed',
                    processed_at = ?2,
                    active_key = NULL,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    next_attempt_at = NULL,
                    error_message = ?3
                WHERE id = ?1
            "#,
            params![id.to_string(), fmt_ts(&processed_at), error_message],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_routine_triggers(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_trigger_queue
                     WHERE routine_id = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2",
                    ROUTINE_TRIGGER_COLUMNS
                ),
                params![routine_id.to_string(), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut triggers = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            triggers.push(row_to_routine_trigger_libsql(&row)?);
        }
        Ok(triggers)
    }
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    use crate::Database;
    use crate::libsql::LibSqlBackend;
    use crate::libsql::get_opt_text;

    /// Build a migrated, file-backed test backend.
    ///
    /// Deliberately NOT `LibSqlBackend::new_memory()`: each `connect()` call
    /// against a `:memory:` libSQL database yields an independent, unshared
    /// in-memory database, so a connection opened after `run_migrations()`
    /// (which uses its own internal connection) sees no tables at all. A
    /// tempfile-backed database persists across connections like production
    /// use does.
    async fn new_test_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("routine_lease_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create local backend");
        backend.run_migrations().await.expect("run migrations");
        (backend, dir)
    }

    /// Insert a bare `routine_runs` row (plus a minimal parent `routines`
    /// row to satisfy the FK constraint, which this backend enforces on
    /// file-backed connections) with an optional `lease_expires_at`.
    async fn insert_run(
        conn: &libsql::Connection,
        id: Uuid,
        started_at: DateTime<Utc>,
        lease_expires_at: Option<DateTime<Utc>>,
    ) {
        let routine_id = Uuid::new_v4();
        conn.execute(
            r#"
                INSERT INTO routines (
                    id, name, user_id, trigger_type, trigger_config,
                    action_type, action_config
                ) VALUES (?1, ?2, 'default', 'manual', '{}', 'lightweight', '{}')
            "#,
            params![routine_id.to_string(), format!("lease-test-{routine_id}")],
        )
        .await
        .expect("insert parent routines row");

        conn.execute(
            r#"
                INSERT INTO routine_runs (
                    id, routine_id, trigger_type, started_at, status, lease_expires_at
                ) VALUES (?1, ?2, 'cron', ?3, 'running', ?4)
            "#,
            params![
                id.to_string(),
                routine_id.to_string(),
                fmt_ts(&started_at),
                fmt_opt_ts(&lease_expires_at),
            ],
        )
        .await
        .expect("insert routine_runs row");
    }

    async fn run_status(conn: &libsql::Connection, id: Uuid) -> String {
        let mut rows = conn
            .query(
                "SELECT status FROM routine_runs WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .expect("query status");
        let row = rows.next().await.expect("row result").expect("row present");
        get_text(&row, 0)
    }

    #[tokio::test]
    async fn cleanup_reaps_legacy_null_lease_past_fallback_ttl() {
        let (backend, _dir) = new_test_backend().await;
        let conn = backend.connect().await.unwrap();

        let old_run = Uuid::new_v4();
        // Legacy row: no lease at all, started well over an hour ago.
        insert_run(
            &conn,
            old_run,
            Utc::now() - chrono::Duration::try_hours(2).unwrap(),
            None,
        )
        .await;

        let reaped = backend.cleanup_stale_routine_runs(3600).await.unwrap();
        assert_eq!(reaped.reaped, 1);
        assert_eq!(reaped.failure_streaks.len(), 1);
        assert_eq!(run_status(&conn, old_run).await, "failed");
    }

    #[tokio::test]
    async fn cleanup_spares_legacy_null_lease_within_fallback_ttl() {
        let (backend, _dir) = new_test_backend().await;
        let conn = backend.connect().await.unwrap();

        let recent_run = Uuid::new_v4();
        // Legacy row with no lease, but started recently — within the
        // fallback TTL, so it must NOT be reaped even though it has no
        // lease. This is the exact false-positive the fixed 10-minute TTL
        // used to produce for long-running full-job routine runs.
        insert_run(
            &conn,
            recent_run,
            Utc::now() - chrono::Duration::try_minutes(30).unwrap(),
            None,
        )
        .await;

        let reaped = backend.cleanup_stale_routine_runs(3600).await.unwrap();
        assert_eq!(reaped.reaped, 0);
        assert_eq!(run_status(&conn, recent_run).await, "running");
    }

    #[tokio::test]
    async fn cleanup_reaps_only_expired_lease_not_fresh_lease() {
        let (backend, _dir) = new_test_backend().await;
        let conn = backend.connect().await.unwrap();

        let expired_run = Uuid::new_v4();
        let fresh_run = Uuid::new_v4();

        // Started 2 hours ago (well past the old fixed 10-minute TTL), but
        // its lease expired 1 minute ago — should be reaped.
        insert_run(
            &conn,
            expired_run,
            Utc::now() - chrono::Duration::try_hours(2).unwrap(),
            Some(Utc::now() - chrono::Duration::try_minutes(1).unwrap()),
        )
        .await;

        // Also started 2 hours ago, but has a lease that's still valid for
        // another hour (renewed recently by an actively-executing worker).
        // Must survive the reaper no matter how old `started_at` is — this
        // is the core fix for the false-positive "Orphaned" bug.
        insert_run(
            &conn,
            fresh_run,
            Utc::now() - chrono::Duration::try_hours(2).unwrap(),
            Some(Utc::now() + chrono::Duration::try_hours(1).unwrap()),
        )
        .await;

        let reaped = backend.cleanup_stale_routine_runs(3600).await.unwrap();
        assert_eq!(reaped.reaped, 1);
        assert_eq!(run_status(&conn, expired_run).await, "failed");
        assert_eq!(run_status(&conn, fresh_run).await, "running");
    }

    #[tokio::test]
    async fn renew_routine_run_lease_extends_expiry_and_survives_reap() {
        let (backend, _dir) = new_test_backend().await;
        let conn = backend.connect().await.unwrap();

        let run_id = Uuid::new_v4();
        // Started long ago with an already-expired lease.
        insert_run(
            &conn,
            run_id,
            Utc::now() - chrono::Duration::try_hours(2).unwrap(),
            Some(Utc::now() - chrono::Duration::try_minutes(5).unwrap()),
        )
        .await;

        // Renew for another hour — simulating a worker/subagent keepalive tick.
        backend.renew_routine_run_lease(run_id, 3600).await.unwrap();

        let reaped = backend.cleanup_stale_routine_runs(3600).await.unwrap();
        assert_eq!(
            reaped.reaped, 0,
            "renewed lease must protect the run from reaping"
        );
        assert_eq!(run_status(&conn, run_id).await, "running");
    }

    #[tokio::test]
    async fn renew_routine_run_lease_is_noop_for_non_running_run() {
        let (backend, _dir) = new_test_backend().await;
        let conn = backend.connect().await.unwrap();

        let routine_id = Uuid::new_v4();
        conn.execute(
            r#"
                INSERT INTO routines (
                    id, name, user_id, trigger_type, trigger_config,
                    action_type, action_config
                ) VALUES (?1, ?2, 'default', 'manual', '{}', 'lightweight', '{}')
            "#,
            params![routine_id.to_string(), format!("lease-test-{routine_id}")],
        )
        .await
        .unwrap();

        let run_id = Uuid::new_v4();
        conn.execute(
            r#"
                INSERT INTO routine_runs (
                    id, routine_id, trigger_type, started_at, status, completed_at
                ) VALUES (?1, ?2, 'cron', ?3, 'ok', ?3)
            "#,
            params![
                run_id.to_string(),
                routine_id.to_string(),
                fmt_ts(&Utc::now())
            ],
        )
        .await
        .unwrap();

        // Renewing a completed run should not resurrect it into a lease
        // that could confuse future queries — the WHERE status = 'running'
        // guard should make this a no-op.
        backend.renew_routine_run_lease(run_id, 3600).await.unwrap();

        let mut rows = conn
            .query(
                "SELECT lease_expires_at FROM routine_runs WHERE id = ?1",
                params![run_id.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert!(get_opt_text(&row, 0).is_none());
    }
}

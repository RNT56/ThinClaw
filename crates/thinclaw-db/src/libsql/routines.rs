//! Routine-related RoutineStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, ROUTINE_COLUMNS, ROUTINE_EVENT_COLUMNS, ROUTINE_EVENT_EVALUATION_COLUMNS,
    ROUTINE_RUN_COLUMNS, ROUTINE_TRIGGER_COLUMNS, fmt_opt_ts, fmt_ts, get_i64, get_text, opt_text,
    opt_text_owned, row_to_routine_event_evaluation_libsql, row_to_routine_event_libsql,
    row_to_routine_libsql, row_to_routine_run_libsql, row_to_routine_trigger_libsql,
};
use crate::RoutineStore;
use thinclaw_agent::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};
use thinclaw_types::error::DatabaseError;

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

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                UPDATE routine_runs SET
                    completed_at = ?5, status = ?2,
                    result_summary = ?3, tokens_used = ?4
                WHERE id = ?1
            "#,
            params![
                id.to_string(),
                status.to_string(),
                opt_text(result_summary),
                tokens_used.map(|t| t as i64),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
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

    async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let now = Utc::now();
        let now_str = fmt_ts(&now);
        let cutoff = fmt_ts(
            &(now
                - chrono::Duration::try_minutes(10)
                    .expect("10 minutes is a valid chrono::Duration")),
        );
        let count = conn
            .execute(
                r#"
                    UPDATE routine_runs SET
                        status = 'failed',
                        completed_at = ?1,
                        result_summary = 'Orphaned: routine exceeded 10-minute TTL'
                    WHERE status = 'running'
                      AND started_at < ?2
                "#,
                params![now_str, cutoff],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count)
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
        let conn = self.connect().await?;
        let now = Utc::now();
        let lease_duration = now.signed_duration_since(stale_before);
        let claimed_at = fmt_ts(&now);
        let lease_expires_at = fmt_ts(&(now + lease_duration));
        conn.execute("BEGIN IMMEDIATE TRANSACTION", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {} FROM routine_event_inbox
                         WHERE id = ?1
                           AND (
                             status = 'pending'
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

            conn.execute(
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
            refreshed.status = thinclaw_agent::routine::RoutineEventStatus::Processing;
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
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(error)
            }
        }
    }

    async fn release_routine_event(
        &self,
        id: Uuid,
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
                    processed_at = NULL
                WHERE id = ?1
            "#,
            params![id.to_string(), diagnostics.to_string()],
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
                     WHERE status = 'pending'
                        OR (
                            status = 'processing'
                            AND (
                                (lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                                OR (claimed_at IS NOT NULL AND claimed_at < ?2)
                            )
                        )
                     ORDER BY created_at ASC
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
                    error_message = ?3
                WHERE id = ?1
            "#,
            params![id.to_string(), fmt_ts(&processed_at), error_message],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
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
                    diagnostics = excluded.diagnostics,
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
        let conn = self.connect().await?;
        let now = Utc::now();
        let lease_duration = now.signed_duration_since(stale_before);
        let claimed_at = fmt_ts(&now);
        let lease_expires_at = fmt_ts(&(now + lease_duration));
        conn.execute("BEGIN IMMEDIATE TRANSACTION", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {} FROM routine_trigger_queue
                         WHERE status = 'pending'
                            OR (
                                status = 'processing'
                                AND (
                                    (lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                                    OR (claimed_at IS NOT NULL AND claimed_at < ?2)
                                )
                            )
                         ORDER BY due_at ASC, created_at ASC
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
                conn.execute(
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
                refreshed.status = thinclaw_agent::routine::RoutineTriggerStatus::Processing;
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
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(error)
            }
        }
    }

    async fn release_routine_trigger(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE routine_trigger_queue
                SET status = 'pending',
                    diagnostics = ?2,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    lease_expires_at = NULL,
                    processed_at = NULL,
                    error_message = NULL
                WHERE id = ?1
            "#,
            params![id.to_string(), diagnostics.to_string()],
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

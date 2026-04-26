#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;

// ==================== Routines ====================

#[cfg(feature = "postgres")]
impl Store {
    async fn bump_routine_event_cache_version(
        &self,
        conn: &tokio_postgres::Client,
    ) -> Result<(), DatabaseError> {
        conn.execute(
            r#"
            INSERT INTO settings (user_id, key, value, updated_at)
            VALUES ('system', 'routine.event_cache_version', '1'::jsonb, NOW())
            ON CONFLICT (user_id, key) DO UPDATE
            SET value = to_jsonb(
                    CASE
                        WHEN (settings.value #>> '{}') ~ '^-?[0-9]+$'
                            THEN (settings.value #>> '{}')::BIGINT
                        ELSE 0
                    END + 1
                ),
                updated_at = EXCLUDED.updated_at
            "#,
            &[],
        )
        .await?;
        Ok(())
    }

    /// Create a new routine.
    pub async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);
        let policy_config = serde_json::to_value(&routine.policy)
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
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10,
                $11, $12, $13,
                $14, $15, $16, $17, $18,
                $19, $20, $21, $22, $23, $24
            )
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.user_id,
                &routine.actor_id,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &policy_config,
                &routine.state,
                &routine.next_fire_at,
                &routine.config_version,
                &routine.created_at,
                &routine.updated_at,
            ],
        )
        .await?;
        self.bump_routine_event_cache_version(&conn).await?;

        Ok(())
    }

    /// Get a routine by ID.
    pub async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM routines WHERE id = $1", &[&id])
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// Get a routine by user_id and name.
    pub async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT * FROM routines WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// List routines for a user.
    pub async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE user_id = $1 ORDER BY name",
                &[&user_id],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// List all enabled routines with event triggers (for event matching).
    pub async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE enabled AND trigger_type = 'event'",
                &[],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    pub async fn get_routine_event_cache_version(&self) -> Result<i64, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT value FROM settings WHERE user_id = 'system' AND key = 'routine.event_cache_version'",
                &[],
            )
            .await?;
        Ok(row
            .and_then(|row| row.try_get::<_, serde_json::Value>("value").ok())
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|raw| raw.parse::<i64>().ok()))
            })
            .unwrap_or(0))
    }

    /// List all enabled cron/system_event routines whose next_fire_at <= now.
    pub async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let rows = conn
            .query(
                r#"
                SELECT * FROM routines
                WHERE enabled
                  AND trigger_type IN ('cron', 'system_event')
                  AND next_fire_at IS NOT NULL
                  AND next_fire_at <= $1
                "#,
                &[&now],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// Update a routine (full replacement of mutable fields).
    pub async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);
        let policy_config = serde_json::to_value(&routine.policy)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            UPDATE routines SET
                name = $2, description = $3, actor_id = $4, enabled = $5,
                trigger_type = $6, trigger_config = $7,
                action_type = $8, action_config = $9,
                cooldown_secs = $10, max_concurrent = $11, dedup_window_secs = $12,
                notify_channel = $13, notify_user = $14,
                notify_on_success = $15, notify_on_failure = $16, notify_on_attention = $17,
                policy_config = $18, state = $19, next_fire_at = $20,
                config_version = config_version + 1,
                updated_at = now()
            WHERE id = $1
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.actor_id,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &policy_config,
                &routine.state,
                &routine.next_fire_at,
            ],
        )
        .await?;
        self.bump_routine_event_cache_version(&conn).await?;
        Ok(())
    }

    /// Update runtime state after a routine fires.
    pub async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routines SET
                last_run_at = $2, next_fire_at = $3,
                run_count = $4, consecutive_failures = $5,
                state = $6, updated_at = now()
            WHERE id = $1
            "#,
            &[
                &id,
                &last_run_at,
                &next_fire_at,
                &(run_count as i64),
                &(consecutive_failures as i32),
                state,
            ],
        )
        .await?;
        Ok(())
    }

    /// Delete a routine.
    pub async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM routines WHERE id = $1", &[&id])
            .await?;
        if count > 0 {
            self.bump_routine_event_cache_version(&conn).await?;
        }
        Ok(count > 0)
    }
}

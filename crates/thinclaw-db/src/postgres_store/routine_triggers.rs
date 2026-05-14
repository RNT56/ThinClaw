#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    pub async fn routine_run_exists_for_trigger_key(
        &self,
        routine_id: Uuid,
        trigger_key: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) AS cnt FROM routine_runs WHERE routine_id = $1 AND trigger_key = $2 AND status != 'failed'",
                &[&routine_id, &trigger_key],
            )
            .await?;
        Ok(row.get::<_, i64>("cnt") > 0)
    }

    pub async fn enqueue_routine_trigger(
        &self,
        trigger: &RoutineTrigger,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let decision = trigger.decision.map(|value| value.to_string());
        conn.execute(
            r#"
            INSERT INTO routine_trigger_queue (
                id, routine_id, trigger_kind, trigger_label, due_at, status, decision,
                active_key, idempotency_key, claimed_by, claimed_at, lease_expires_at,
                processed_at, error_message, diagnostics, coalesced_count, backlog_collapsed,
                routine_config_version, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17,
                $18, $19
            )
            ON CONFLICT (active_key) WHERE active_key IS NOT NULL DO UPDATE
            SET due_at = GREATEST(routine_trigger_queue.due_at, EXCLUDED.due_at),
                diagnostics = EXCLUDED.diagnostics,
                coalesced_count = routine_trigger_queue.coalesced_count + 1,
                backlog_collapsed = routine_trigger_queue.backlog_collapsed OR EXCLUDED.backlog_collapsed,
                routine_config_version = EXCLUDED.routine_config_version
            "#,
            &[
                &trigger.id,
                &trigger.routine_id,
                &trigger.trigger_kind.to_string(),
                &trigger.trigger_label,
                &trigger.due_at,
                &trigger.status.to_string(),
                &decision,
                &trigger.active_key,
                &trigger.idempotency_key,
                &trigger.claimed_by,
                &trigger.claimed_at,
                &trigger.lease_expires_at,
                &trigger.processed_at,
                &trigger.error_message,
                &trigger.diagnostics,
                &(trigger.coalesced_count as i32),
                &trigger.backlog_collapsed,
                &trigger.routine_config_version,
                &trigger.created_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn claim_routine_triggers(
        &self,
        worker_id: &str,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let lease_expires_at = now + now.signed_duration_since(stale_before);
        let rows = conn
            .query(
                r#"
                WITH candidates AS (
                    SELECT id
                    FROM routine_trigger_queue
                    WHERE status = 'pending'
                       OR (
                            status = 'processing'
                            AND (
                                (lease_expires_at IS NOT NULL AND lease_expires_at < NOW())
                                OR (claimed_at IS NOT NULL AND claimed_at < $2)
                            )
                       )
                    ORDER BY due_at ASC, created_at ASC
                    LIMIT $3
                ),
                claimed AS (
                    UPDATE routine_trigger_queue
                    SET status = 'processing',
                        claimed_by = $1,
                        claimed_at = NOW(),
                        lease_expires_at = $4,
                        error_message = NULL
                    WHERE id IN (SELECT id FROM candidates)
                    RETURNING *
                )
                SELECT * FROM claimed
                ORDER BY due_at ASC, created_at ASC
                "#,
                &[&worker_id, &stale_before, &limit, &lease_expires_at],
            )
            .await?;
        rows.iter().map(row_to_routine_trigger).collect()
    }

    pub async fn release_routine_trigger(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routine_trigger_queue
            SET status = 'pending',
                diagnostics = $2,
                claimed_by = NULL,
                claimed_at = NULL,
                lease_expires_at = NULL,
                processed_at = NULL,
                error_message = NULL
            WHERE id = $1
            "#,
            &[&id, diagnostics],
        )
        .await?;
        Ok(())
    }

    pub async fn complete_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        decision: RoutineTriggerDecision,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let decision = decision.to_string();
        conn.execute(
            r#"
            UPDATE routine_trigger_queue
            SET status = 'processed',
                decision = $3,
                processed_at = $2,
                diagnostics = $4,
                active_key = NULL,
                claimed_by = NULL,
                claimed_at = NULL,
                lease_expires_at = NULL,
                error_message = NULL
            WHERE id = $1
            "#,
            &[&id, &processed_at, &decision, diagnostics],
        )
        .await?;
        Ok(())
    }

    pub async fn fail_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routine_trigger_queue
            SET status = 'failed',
                processed_at = $2,
                active_key = NULL,
                claimed_by = NULL,
                claimed_at = NULL,
                lease_expires_at = NULL,
                error_message = $3
            WHERE id = $1
            "#,
            &[&id, &processed_at, &error_message],
        )
        .await?;
        Ok(())
    }

    pub async fn list_routine_triggers(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_trigger_queue
                WHERE routine_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&routine_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_trigger).collect()
    }
}

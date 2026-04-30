#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    pub async fn create_routine_event(
        &self,
        event: &RoutineEvent,
    ) -> Result<RoutineEvent, DatabaseError> {
        let conn = self.conn().await?;
        let status = event.status.to_string();
        let conversation_scope_id = event
            .conversation_scope_id
            .parse::<Uuid>()
            .map_err(|error| DatabaseError::Serialization(error.to_string()))?;
        conn.execute(
            r#"
            INSERT INTO routine_event_inbox (
                id, principal_id, actor_id, channel, event_type, raw_sender_id,
                conversation_scope_id, stable_external_conversation_key, idempotency_key,
                content, content_hash, metadata, status, diagnostics,
                claimed_by, claimed_at, lease_expires_at, processed_at, error_message,
                matched_routines, fired_routines, attempt_count, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9,
                $10, $11, $12, $13, $14,
                $15, $16, $17, $18, $19,
                $20, $21, $22, $23
            )
            ON CONFLICT (idempotency_key) DO NOTHING
            "#,
            &[
                &event.id,
                &event.principal_id,
                &event.actor_id,
                &event.channel,
                &event.event_type,
                &event.raw_sender_id,
                &conversation_scope_id,
                &event.stable_external_conversation_key,
                &event.idempotency_key,
                &event.content,
                &event.content_hash,
                &event.metadata,
                &status,
                &event.diagnostics,
                &event.claimed_by,
                &event.claimed_at,
                &event.lease_expires_at,
                &event.processed_at,
                &event.error_message,
                &(event.matched_routines as i32),
                &(event.fired_routines as i32),
                &(event.attempt_count as i32),
                &event.created_at,
            ],
        )
        .await?;
        let row = conn
            .query_one(
                "SELECT * FROM routine_event_inbox WHERE idempotency_key = $1",
                &[&event.idempotency_key],
            )
            .await?;
        row_to_routine_event(&row)
    }

    pub async fn claim_routine_event(
        &self,
        id: Uuid,
        worker_id: &str,
        stale_before: DateTime<Utc>,
    ) -> Result<Option<RoutineEvent>, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let lease_expires_at = now + now.signed_duration_since(stale_before);
        let row = conn
            .query_opt(
                r#"
                WITH claimed AS (
                    UPDATE routine_event_inbox
                    SET status = 'processing',
                        claimed_by = $2,
                        claimed_at = NOW(),
                        lease_expires_at = $4,
                        attempt_count = attempt_count + 1,
                        error_message = NULL
                    WHERE id = $1
                      AND (
                        status = 'pending'
                        OR (
                            status = 'processing'
                            AND (
                                (lease_expires_at IS NOT NULL AND lease_expires_at < NOW())
                                OR (claimed_at IS NOT NULL AND claimed_at < $3)
                            )
                        )
                      )
                    RETURNING *
                )
                SELECT * FROM claimed
                "#,
                &[&id, &worker_id, &stale_before, &lease_expires_at],
            )
            .await?;
        row.map(|row| row_to_routine_event(&row)).transpose()
    }

    pub async fn release_routine_event(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routine_event_inbox
            SET status = 'pending',
                diagnostics = $2,
                claimed_by = NULL,
                claimed_at = NULL,
                lease_expires_at = NULL,
                processed_at = NULL
            WHERE id = $1
            "#,
            &[&id, diagnostics],
        )
        .await?;
        Ok(())
    }

    pub async fn list_pending_routine_events(
        &self,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_event_inbox
                WHERE status = 'pending'
                   OR (
                        status = 'processing'
                        AND (
                            (lease_expires_at IS NOT NULL AND lease_expires_at < NOW())
                            OR (claimed_at IS NOT NULL AND claimed_at < $1)
                        )
                   )
                ORDER BY created_at ASC
                LIMIT $2
                "#,
                &[&stale_before, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_event).collect()
    }

    pub async fn complete_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        matched_routines: u32,
        fired_routines: u32,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routine_event_inbox
            SET status = 'processed',
                processed_at = $2,
                matched_routines = $3,
                fired_routines = $4,
                diagnostics = $5,
                claimed_by = NULL,
                claimed_at = NULL,
                lease_expires_at = NULL,
                error_message = NULL
            WHERE id = $1
            "#,
            &[
                &id,
                &processed_at,
                &(matched_routines as i32),
                &(fired_routines as i32),
                diagnostics,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn fail_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routine_event_inbox
            SET status = 'failed',
                processed_at = $2,
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

    pub async fn list_routine_events_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_event_inbox
                WHERE principal_id = $1 AND actor_id = $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                &[&user_id, &actor_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_event).collect()
    }

    pub async fn upsert_routine_event_evaluation(
        &self,
        evaluation: &RoutineEventEvaluation,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let decision = evaluation.decision.to_string();
        conn.execute(
            r#"
            INSERT INTO routine_event_evaluations (
                id, event_id, routine_id, decision, reason, details, sequence_num,
                channel, content_preview, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
            )
            ON CONFLICT (event_id, routine_id) DO UPDATE
            SET decision = EXCLUDED.decision,
                reason = EXCLUDED.reason,
                details = EXCLUDED.details,
                sequence_num = EXCLUDED.sequence_num,
                channel = EXCLUDED.channel,
                content_preview = EXCLUDED.content_preview
            "#,
            &[
                &evaluation.id,
                &evaluation.event_id,
                &evaluation.routine_id,
                &decision,
                &evaluation.reason,
                &evaluation.details,
                &(evaluation.sequence_num as i32),
                &evaluation.channel,
                &evaluation.content_preview,
                &evaluation.created_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_routine_event_evaluations_for_event(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_event_evaluations
                WHERE event_id = $1
                ORDER BY sequence_num ASC
                "#,
                &[&event_id],
            )
            .await?;
        rows.iter().map(row_to_routine_event_evaluation).collect()
    }

    pub async fn list_routine_event_evaluations(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_event_evaluations
                WHERE routine_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&routine_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_event_evaluation).collect()
    }
}

#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    /// Persist an outcome contract, reusing the existing row when the dedupe key matches.
    pub async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if contract.id.is_nil() {
            Uuid::new_v4()
        } else {
            contract.id
        };
        let row = conn
            .query_one(
                r#"
                INSERT INTO outcome_contracts (
                    id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                    contract_type, status, summary, due_at, expires_at, final_verdict,
                    final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                    evaluated_at, created_at, updated_at
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7,
                    $8, $9, $10, $11, $12, $13,
                    $14, $15::jsonb, $16::jsonb, $17, $18,
                    $19, $20, $21
                )
                ON CONFLICT (dedupe_key) DO UPDATE
                    SET updated_at = outcome_contracts.updated_at
                RETURNING id
                "#,
                &[
                    &id,
                    &contract.user_id,
                    &contract.actor_id,
                    &contract.channel,
                    &contract.thread_id,
                    &contract.source_kind,
                    &contract.source_id,
                    &contract.contract_type,
                    &contract.status,
                    &contract.summary,
                    &contract.due_at,
                    &contract.expires_at,
                    &contract.final_verdict,
                    &contract.final_score,
                    &contract.evaluation_details,
                    &contract.metadata,
                    &contract.dedupe_key,
                    &contract.claimed_at,
                    &contract.evaluated_at,
                    &contract.created_at,
                    &contract.updated_at,
                ],
            )
            .await?;
        Ok(row.get("id"))
    }

    /// Retrieve a single outcome contract belonging to a user.
    pub async fn get_outcome_contract(
        &self,
        user_id: &str,
        contract_id: Uuid,
    ) -> Result<Option<OutcomeContract>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT
                    id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                    contract_type, status, summary, due_at, expires_at, final_verdict,
                    final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                    evaluated_at, created_at, updated_at
                FROM outcome_contracts
                WHERE id = $1 AND user_id = $2
                "#,
                &[&contract_id, &user_id],
            )
            .await?;
        Ok(row.as_ref().map(outcome_contract_from_row))
    }

    /// List outcome contracts for a user with optional filters.
    pub async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        if query.limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                    contract_type, status, summary, due_at, expires_at, final_verdict,
                    final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                    evaluated_at, created_at, updated_at
                FROM outcome_contracts
                WHERE user_id = $1
                  AND ($2::text IS NULL OR COALESCE(NULLIF(actor_id, ''), user_id) = $2)
                  AND ($3::text IS NULL OR status = $3)
                  AND ($4::text IS NULL OR contract_type = $4)
                  AND ($5::text IS NULL OR source_kind = $5)
                  AND ($6::text IS NULL OR source_id = $6)
                  AND ($7::text IS NULL OR thread_id = $7)
                ORDER BY created_at DESC, id DESC
                LIMIT $8
                "#,
                &[
                    &query.user_id,
                    &query.actor_id,
                    &query.status,
                    &query.contract_type,
                    &query.source_kind,
                    &query.source_id,
                    &query.thread_id,
                    &query.limit,
                ],
            )
            .await?;
        Ok(rows.iter().map(outcome_contract_from_row).collect())
    }

    /// Claim due contracts for evaluator processing.
    pub async fn claim_due_outcome_contracts(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.claim_due_outcome_contracts_for_user(None, limit, now)
            .await
    }

    /// Claim due contracts for a single user.
    pub async fn claim_due_outcome_contracts_for_user(
        &self,
        user_id: Option<&str>,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        match user_id {
            Some(user_id) => {
                conn.execute(
                    r#"
                    UPDATE outcome_contracts
                    SET status = 'expired',
                        updated_at = $1
                    WHERE user_id = $2
                      AND status IN ('open', 'evaluating')
                      AND evaluated_at IS NULL
                      AND expires_at <= $1
                    "#,
                    &[&now, &user_id],
                )
                .await?;
            }
            None => {
                conn.execute(
                    r#"
                    UPDATE outcome_contracts
                    SET status = 'expired',
                        updated_at = $1
                    WHERE status IN ('open', 'evaluating')
                      AND evaluated_at IS NULL
                      AND expires_at <= $1
                    "#,
                    &[&now],
                )
                .await?;
            }
        }

        let rows = match user_id {
            Some(user_id) => {
                conn.query(
                    r#"
                    WITH due AS (
                        SELECT id
                        FROM outcome_contracts
                        WHERE user_id = $3
                          AND status = 'open'
                          AND due_at <= $2
                          AND expires_at > $2
                        ORDER BY due_at ASC, created_at ASC
                        LIMIT $1
                        FOR UPDATE SKIP LOCKED
                    )
                    UPDATE outcome_contracts oc
                    SET status = 'evaluating',
                        claimed_at = $2,
                        updated_at = $2
                    FROM due
                    WHERE oc.id = due.id
                    RETURNING
                        oc.id, oc.user_id, oc.actor_id, oc.channel, oc.thread_id, oc.source_kind,
                        oc.source_id, oc.contract_type, oc.status, oc.summary, oc.due_at,
                        oc.expires_at, oc.final_verdict, oc.final_score, oc.evaluation_details,
                        oc.metadata, oc.dedupe_key, oc.claimed_at, oc.evaluated_at,
                        oc.created_at, oc.updated_at
                    "#,
                    &[&limit, &now, &user_id],
                )
                .await?
            }
            None => {
                conn.query(
                    r#"
                    WITH due AS (
                        SELECT id
                        FROM outcome_contracts
                        WHERE status = 'open'
                          AND due_at <= $2
                          AND expires_at > $2
                        ORDER BY due_at ASC, created_at ASC
                        LIMIT $1
                        FOR UPDATE SKIP LOCKED
                    )
                    UPDATE outcome_contracts oc
                    SET status = 'evaluating',
                        claimed_at = $2,
                        updated_at = $2
                    FROM due
                    WHERE oc.id = due.id
                    RETURNING
                        oc.id, oc.user_id, oc.actor_id, oc.channel, oc.thread_id, oc.source_kind,
                        oc.source_id, oc.contract_type, oc.status, oc.summary, oc.due_at,
                        oc.expires_at, oc.final_verdict, oc.final_score, oc.evaluation_details,
                        oc.metadata, oc.dedupe_key, oc.claimed_at, oc.evaluated_at,
                        oc.created_at, oc.updated_at
                    "#,
                    &[&limit, &now],
                )
                .await?
            }
        };
        Ok(rows.iter().map(outcome_contract_from_row).collect())
    }

    /// Persist a full outcome contract update.
    pub async fn update_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE outcome_contracts
            SET user_id = $2,
                actor_id = $3,
                channel = $4,
                thread_id = $5,
                source_kind = $6,
                source_id = $7,
                contract_type = $8,
                status = $9,
                summary = $10,
                due_at = $11,
                expires_at = $12,
                final_verdict = $13,
                final_score = $14,
                evaluation_details = $15::jsonb,
                metadata = $16::jsonb,
                dedupe_key = $17,
                claimed_at = $18,
                evaluated_at = $19,
                created_at = $20,
                updated_at = $21
            WHERE id = $1
            "#,
            &[
                &contract.id,
                &contract.user_id,
                &contract.actor_id,
                &contract.channel,
                &contract.thread_id,
                &contract.source_kind,
                &contract.source_id,
                &contract.contract_type,
                &contract.status,
                &contract.summary,
                &contract.due_at,
                &contract.expires_at,
                &contract.final_verdict,
                &contract.final_score,
                &contract.evaluation_details,
                &contract.metadata,
                &contract.dedupe_key,
                &contract.claimed_at,
                &contract.evaluated_at,
                &contract.created_at,
                &contract.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Aggregate outcome counts for Learning Ledger status cards.
    pub async fn outcome_summary_stats(
        &self,
        user_id: &str,
    ) -> Result<OutcomeSummaryStats, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                r#"
                SELECT
                    (SELECT COUNT(*)
                     FROM outcome_contracts
                     WHERE user_id = $1
                       AND status IN ('open', 'evaluating')) AS open_count,
                    (SELECT COUNT(*)
                     FROM outcome_contracts
                     WHERE user_id = $1
                       AND status = 'open'
                       AND due_at <= NOW()
                       AND expires_at > NOW()) AS due_count,
                    (SELECT COUNT(*)
                     FROM outcome_contracts
                     WHERE user_id = $1
                       AND status = 'evaluated'
                       AND COALESCE(evaluated_at, updated_at) >= NOW() - INTERVAL '7 days')
                        AS evaluated_count,
                    (SELECT COALESCE(AVG(CASE WHEN final_verdict = 'negative' THEN 1.0 ELSE 0.0 END), 0.0)
                     FROM outcome_contracts
                     WHERE user_id = $1
                       AND status = 'evaluated'
                       AND COALESCE(evaluated_at, updated_at) >= NOW() - INTERVAL '7 days')
                        AS negative_ratio
                "#,
                &[&user_id],
            )
            .await?;
        Ok(OutcomeSummaryStats {
            open: row.get::<_, i64>("open_count") as u64,
            due: row.get::<_, i64>("due_count") as u64,
            evaluated_last_7d: row.get::<_, i64>("evaluated_count") as u64,
            negative_ratio_last_7d: row
                .try_get::<_, Option<f64>>("negative_ratio")
                .ok()
                .flatten()
                .unwrap_or(0.0),
        })
    }

    /// Return distinct users with due outcome work.
    pub async fn list_users_with_pending_outcome_work(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomePendingUser>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT DISTINCT user_id
                FROM outcome_contracts
                WHERE status = 'open'
                  AND due_at <= $1
                  AND expires_at > $1
                ORDER BY user_id ASC
                "#,
                &[&now],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| OutcomePendingUser {
                user_id: row.get("user_id"),
            })
            .collect())
    }

    /// Return the oldest due and evaluating timestamps for outcome health checks.
    pub async fn outcome_evaluator_health(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutcomeEvaluatorHealth, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                r#"
                SELECT
                    (
                        SELECT MIN(due_at)
                        FROM outcome_contracts
                        WHERE user_id = $1
                          AND status = 'open'
                          AND due_at <= $2
                          AND expires_at > $2
                    ) AS oldest_due_at,
                    (
                        SELECT MIN(COALESCE(claimed_at, updated_at))
                        FROM outcome_contracts
                        WHERE user_id = $1
                          AND status = 'evaluating'
                    ) AS oldest_evaluating_claimed_at
                "#,
                &[&user_id, &now],
            )
            .await?;
        Ok(OutcomeEvaluatorHealth {
            oldest_due_at: row
                .try_get::<_, Option<DateTime<Utc>>>("oldest_due_at")
                .ok()
                .flatten(),
            oldest_evaluating_claimed_at: row
                .try_get::<_, Option<DateTime<Utc>>>("oldest_evaluating_claimed_at")
                .ok()
                .flatten(),
        })
    }

    /// Persist a contract observation, coalescing duplicates by fingerprint.
    pub async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservation,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if observation.id.is_nil() {
            Uuid::new_v4()
        } else {
            observation.id
        };
        let row = conn
            .query_one(
                r#"
                INSERT INTO outcome_observations (
                    id, contract_id, observation_kind, polarity, weight, summary, evidence,
                    fingerprint, observed_at, created_at
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7::jsonb,
                    $8, $9, $10
                )
                ON CONFLICT (contract_id, fingerprint) DO UPDATE
                    SET observed_at = outcome_observations.observed_at
                RETURNING id
                "#,
                &[
                    &id,
                    &observation.contract_id,
                    &observation.observation_kind,
                    &observation.polarity,
                    &observation.weight,
                    &observation.summary,
                    &observation.evidence,
                    &observation.fingerprint,
                    &observation.observed_at,
                    &observation.created_at,
                ],
            )
            .await?;
        Ok(row.get("id"))
    }

    /// List observations for a single contract.
    pub async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservation>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, contract_id, observation_kind, polarity, weight, summary, evidence,
                    fingerprint, observed_at, created_at
                FROM outcome_observations
                WHERE contract_id = $1
                ORDER BY observed_at ASC, id ASC
                "#,
                &[&contract_id],
            )
            .await?;
        Ok(rows.iter().map(outcome_observation_from_row).collect())
    }
}

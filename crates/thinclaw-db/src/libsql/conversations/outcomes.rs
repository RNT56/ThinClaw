use chrono::{DateTime, Utc};
use libsql::params;
use thinclaw_history::OutcomeContract;
use thinclaw_types::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, outcome_contract_from_row};

pub(super) async fn claim_due_for_user_with_lease(
    backend: &LibSqlBackend,
    user_id: &str,
    worker_id: &str,
    limit: i64,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Vec<OutcomeContract>, DatabaseError> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let conn = backend.connect().await?;
    let now_ts = fmt_ts(&now);
    let lease_secs = lease_secs.max(1);
    let lease_expires_at = fmt_ts(&(now + chrono::Duration::seconds(lease_secs)));
    let stale_before = fmt_ts(&(now - chrono::Duration::seconds(lease_secs)));
    let scoped = !user_id.is_empty();
    if scoped {
        conn.execute(
            r#"
            UPDATE outcome_contracts
            SET status = 'expired',
                updated_at = ?1
            WHERE user_id = ?2
              AND status IN ('open', 'evaluating')
              AND evaluated_at IS NULL
              AND expires_at <= ?1
            "#,
            params![now_ts.clone(), user_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    } else {
        conn.execute(
            r#"
            UPDATE outcome_contracts
            SET status = 'expired',
                updated_at = ?1
            WHERE status IN ('open', 'evaluating')
              AND evaluated_at IS NULL
              AND expires_at <= ?1
            "#,
            params![now_ts.clone()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    }

    let mut rows = if scoped {
        conn.query(
            r#"
            SELECT
                id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                contract_type, status, summary, due_at, expires_at, final_verdict,
                final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                claimed_by, lease_expires_at, attempt_count, next_attempt_at,
                evaluated_at, created_at, updated_at
            FROM outcome_contracts
            WHERE user_id = ?2
              AND (
                (status = 'open' AND due_at <= ?1 AND expires_at > ?1
                 AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                OR
                (status = 'evaluating' AND expires_at > ?1
                 AND ((lease_expires_at IS NOT NULL AND lease_expires_at <= ?1)
                      OR (lease_expires_at IS NULL
                          AND (claimed_at IS NULL OR claimed_at <= ?4))))
              )
            ORDER BY due_at ASC, created_at ASC
            LIMIT ?3
            "#,
            params![now_ts.clone(), user_id, limit, stale_before.clone()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    } else {
        conn.query(
            r#"
            SELECT
                id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                contract_type, status, summary, due_at, expires_at, final_verdict,
                final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                claimed_by, lease_expires_at, attempt_count, next_attempt_at,
                evaluated_at, created_at, updated_at
            FROM outcome_contracts
            WHERE (
                (status = 'open' AND due_at <= ?1 AND expires_at > ?1
                 AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                OR
                (status = 'evaluating' AND expires_at > ?1
                 AND ((lease_expires_at IS NOT NULL AND lease_expires_at <= ?1)
                      OR (lease_expires_at IS NULL
                          AND (claimed_at IS NULL OR claimed_at <= ?3))))
            )
            ORDER BY due_at ASC, created_at ASC
            LIMIT ?2
            "#,
            params![now_ts.clone(), limit, stale_before.clone()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    };

    let mut claimed = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    {
        let mut contract = outcome_contract_from_row(&row);
        let affected = conn
            .execute(
                r#"
                UPDATE outcome_contracts
                SET status = 'evaluating',
                    claimed_at = ?2,
                    claimed_by = ?3,
                    lease_expires_at = ?4,
                    attempt_count = attempt_count + 1,
                    next_attempt_at = NULL,
                    updated_at = ?2
                WHERE id = ?1
                  AND (
                    (status = 'open' AND (next_attempt_at IS NULL OR next_attempt_at <= ?2))
                    OR
                    (status = 'evaluating'
                     AND ((lease_expires_at IS NOT NULL AND lease_expires_at <= ?2)
                          OR (lease_expires_at IS NULL
                              AND (claimed_at IS NULL OR claimed_at <= ?5))))
                  )
                "#,
                params![
                    contract.id.to_string(),
                    now_ts.clone(),
                    worker_id,
                    lease_expires_at.clone(),
                    stale_before.clone()
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected > 0 {
            contract.status = "evaluating".to_string();
            contract.claimed_at = Some(now);
            contract.claimed_by = Some(worker_id.to_string());
            contract.lease_expires_at = Some(now + chrono::Duration::seconds(lease_secs));
            contract.attempt_count = contract.attempt_count.saturating_add(1);
            contract.next_attempt_at = None;
            contract.updated_at = now;
            claimed.push(contract);
        }
    }
    Ok(claimed)
}

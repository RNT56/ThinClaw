use chrono::{DateTime, Utc};
use libsql::params;
use thinclaw_history::{OutcomeContract, OutcomeContractQuery};
use thinclaw_types::error::DatabaseError;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_text, outcome_contract_from_row};

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

pub(super) async fn insert_contract(
    backend: &LibSqlBackend,
    contract: &OutcomeContract,
) -> Result<Uuid, DatabaseError> {
    let conn = backend.connect().await?;
    let id = if contract.id.is_nil() {
        Uuid::new_v4()
    } else {
        contract.id
    };
    let affected = conn
        .execute(
            r#"
            INSERT OR IGNORE INTO outcome_contracts (
                id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                contract_type, status, summary, due_at, expires_at, final_verdict,
                final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                claimed_by, lease_expires_at, attempt_count, next_attempt_at,
                evaluated_at, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18,
                ?19, ?20, ?21, ?22,
                ?23, ?24, ?25
            )
            "#,
            params![
                id.to_string(),
                contract.user_id.as_str(),
                contract.actor_id.as_deref(),
                contract.channel.as_deref(),
                contract.thread_id.as_deref(),
                contract.source_kind.as_str(),
                contract.source_id.as_str(),
                contract.contract_type.as_str(),
                contract.status.as_str(),
                contract.summary.as_deref(),
                fmt_ts(&contract.due_at),
                fmt_ts(&contract.expires_at),
                contract.final_verdict.as_deref(),
                contract.final_score,
                contract.evaluation_details.to_string(),
                contract.metadata.to_string(),
                contract.dedupe_key.as_str(),
                contract.claimed_at.as_ref().map(fmt_ts),
                contract.claimed_by.as_deref(),
                contract.lease_expires_at.as_ref().map(fmt_ts),
                contract.attempt_count as i64,
                contract.next_attempt_at.as_ref().map(fmt_ts),
                contract.evaluated_at.as_ref().map(fmt_ts),
                fmt_ts(&contract.created_at),
                fmt_ts(&contract.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    if affected > 0 {
        return Ok(id);
    }

    let mut rows = conn
        .query(
            "SELECT id FROM outcome_contracts WHERE dedupe_key = ?1 LIMIT 1",
            params![contract.dedupe_key.as_str()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    else {
        return Err(DatabaseError::Query(
            "failed to resolve existing outcome contract".to_string(),
        ));
    };
    Ok(get_text(&row, 0).parse().unwrap_or_default())
}

pub(super) async fn get_contract(
    backend: &LibSqlBackend,
    user_id: &str,
    contract_id: Uuid,
) -> Result<Option<OutcomeContract>, DatabaseError> {
    let conn = backend.connect().await?;
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                contract_type, status, summary, due_at, expires_at, final_verdict,
                final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                claimed_by, lease_expires_at, attempt_count, next_attempt_at,
                evaluated_at, created_at, updated_at
            FROM outcome_contracts
            WHERE id = ?1 AND user_id = ?2
            LIMIT 1
            "#,
            params![contract_id.to_string(), user_id],
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
    Ok(Some(outcome_contract_from_row(&row)))
}

pub(super) async fn list_contracts(
    backend: &LibSqlBackend,
    query: &OutcomeContractQuery,
) -> Result<Vec<OutcomeContract>, DatabaseError> {
    if query.limit <= 0 {
        return Ok(Vec::new());
    }
    let conn = backend.connect().await?;
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, user_id, actor_id, channel, thread_id, source_kind, source_id,
                contract_type, status, summary, due_at, expires_at, final_verdict,
                final_score, evaluation_details, metadata, dedupe_key, claimed_at,
                claimed_by, lease_expires_at, attempt_count, next_attempt_at,
                evaluated_at, created_at, updated_at
            FROM outcome_contracts
            WHERE user_id = ?1
              AND (?2 IS NULL OR COALESCE(NULLIF(actor_id, ''), user_id) = ?2)
              AND (?3 IS NULL OR status = ?3)
              AND (?4 IS NULL OR contract_type = ?4)
              AND (?5 IS NULL OR source_kind = ?5)
              AND (?6 IS NULL OR source_id = ?6)
              AND (?7 IS NULL OR thread_id = ?7)
            ORDER BY created_at DESC, rowid DESC
            LIMIT ?8
            "#,
            params![
                query.user_id.as_str(),
                query.actor_id.as_deref(),
                query.status.as_deref(),
                query.contract_type.as_deref(),
                query.source_kind.as_deref(),
                query.source_id.as_deref(),
                query.thread_id.as_deref(),
                query.limit,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let mut contracts = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    {
        contracts.push(outcome_contract_from_row(&row));
    }
    Ok(contracts)
}

pub(super) async fn update_contract(
    backend: &LibSqlBackend,
    contract: &OutcomeContract,
) -> Result<(), DatabaseError> {
    let conn = backend.connect().await?;
    conn.execute(
        r#"
        UPDATE outcome_contracts
        SET user_id = ?2,
            actor_id = ?3,
            channel = ?4,
            thread_id = ?5,
            source_kind = ?6,
            source_id = ?7,
            contract_type = ?8,
            status = ?9,
            summary = ?10,
            due_at = ?11,
            expires_at = ?12,
            final_verdict = ?13,
            final_score = ?14,
            evaluation_details = ?15,
            metadata = ?16,
            dedupe_key = ?17,
            claimed_at = ?18,
            claimed_by = ?19,
            lease_expires_at = ?20,
            attempt_count = ?21,
            next_attempt_at = ?22,
            evaluated_at = ?23,
            created_at = ?24,
            updated_at = ?25
        WHERE id = ?1
        "#,
        params![
            contract.id.to_string(),
            contract.user_id.as_str(),
            contract.actor_id.as_deref(),
            contract.channel.as_deref(),
            contract.thread_id.as_deref(),
            contract.source_kind.as_str(),
            contract.source_id.as_str(),
            contract.contract_type.as_str(),
            contract.status.as_str(),
            contract.summary.as_deref(),
            fmt_ts(&contract.due_at),
            fmt_ts(&contract.expires_at),
            contract.final_verdict.as_deref(),
            contract.final_score,
            contract.evaluation_details.to_string(),
            contract.metadata.to_string(),
            contract.dedupe_key.as_str(),
            contract.claimed_at.as_ref().map(fmt_ts),
            contract.claimed_by.as_deref(),
            contract.lease_expires_at.as_ref().map(fmt_ts),
            contract.attempt_count as i64,
            contract.next_attempt_at.as_ref().map(fmt_ts),
            contract.evaluated_at.as_ref().map(fmt_ts),
            fmt_ts(&contract.created_at),
            fmt_ts(&contract.updated_at),
        ],
    )
    .await
    .map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok(())
}

pub(super) async fn update_claimed_contract(
    backend: &LibSqlBackend,
    contract: &OutcomeContract,
    worker_id: &str,
) -> Result<bool, DatabaseError> {
    let conn = backend.connect().await?;
    let affected = conn
        .execute(
            r#"
            UPDATE outcome_contracts
            SET user_id = ?2,
                actor_id = ?3,
                channel = ?4,
                thread_id = ?5,
                source_kind = ?6,
                source_id = ?7,
                contract_type = ?8,
                status = ?9,
                summary = ?10,
                due_at = ?11,
                expires_at = ?12,
                final_verdict = ?13,
                final_score = ?14,
                evaluation_details = ?15,
                metadata = ?16,
                dedupe_key = ?17,
                claimed_at = ?18,
                claimed_by = ?19,
                lease_expires_at = ?20,
                attempt_count = ?21,
                next_attempt_at = ?22,
                evaluated_at = ?23,
                created_at = ?24,
                updated_at = ?25
            WHERE id = ?1
              AND status = 'evaluating'
              AND claimed_by = ?26
            "#,
            params![
                contract.id.to_string(),
                contract.user_id.as_str(),
                contract.actor_id.as_deref(),
                contract.channel.as_deref(),
                contract.thread_id.as_deref(),
                contract.source_kind.as_str(),
                contract.source_id.as_str(),
                contract.contract_type.as_str(),
                contract.status.as_str(),
                contract.summary.as_deref(),
                fmt_ts(&contract.due_at),
                fmt_ts(&contract.expires_at),
                contract.final_verdict.as_deref(),
                contract.final_score,
                contract.evaluation_details.to_string(),
                contract.metadata.to_string(),
                contract.dedupe_key.as_str(),
                contract.claimed_at.as_ref().map(fmt_ts),
                contract.claimed_by.as_deref(),
                contract.lease_expires_at.as_ref().map(fmt_ts),
                contract.attempt_count as i64,
                contract.next_attempt_at.as_ref().map(fmt_ts),
                contract.evaluated_at.as_ref().map(fmt_ts),
                fmt_ts(&contract.created_at),
                fmt_ts(&contract.updated_at),
                worker_id,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok(affected > 0)
}

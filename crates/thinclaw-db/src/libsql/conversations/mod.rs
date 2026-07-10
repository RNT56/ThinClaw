//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::ConversationStore;
use thinclaw_history::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationSummary,
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, LearningFeedbackRecord, LearningRollbackRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeEvaluatorHealth, OutcomeObservation, OutcomePendingUser,
    OutcomeSummaryStats, SessionSearchHit,
};
use thinclaw_types::error::DatabaseError;
#[async_trait]
impl ConversationStore for LibSqlBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let stable_external_conversation_key = match thread_id {
            Some(thread_id) if !thread_id.is_empty() => format!("{channel}:{thread_id}"),
            _ => format!("{channel}:{id}"),
        };
        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                thread_id, stable_external_conversation_key, metadata
            ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id.to_string(),
                channel,
                user_id,
                id.to_string(),
                ConversationKind::Direct.as_str(),
                opt_text(thread_id),
                stable_external_conversation_key,
                serde_json::json!({}).to_string(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE conversations SET last_activity = ?2 WHERE id = ?1",
            params![id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.add_conversation_message_with_attribution(
            conversation_id,
            role,
            content,
            None,
            None,
            None,
            None,
        )
        .await
    }

    async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO conversation_messages (
                id, conversation_id, role, content, actor_id, actor_display_name,
                raw_sender_id, metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                id.to_string(),
                conversation_id.to_string(),
                role,
                content,
                actor_id,
                actor_display_name,
                raw_sender_id,
                metadata
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        self.touch_conversation(conversation_id).await?;
        Ok(id)
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let stable_external_conversation_key = match thread_id {
            Some(thread_id) if !thread_id.is_empty() => format!("{channel}:{thread_id}"),
            _ => format!("{channel}:{id}"),
        };
        conn.execute(
            r#"
                INSERT INTO conversations (
                    id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                    thread_id, stable_external_conversation_key, metadata
                )
                VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT (id) DO UPDATE SET last_activity = ?9
                "#,
            params![
                id.to_string(),
                channel,
                user_id,
                id.to_string(),
                ConversationKind::Direct.as_str(),
                opt_text(thread_id),
                stable_external_conversation_key,
                serde_json::json!({}).to_string(),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.user_id,
                    c.actor_id,
                    c.conversation_scope_id,
                    c.channel,
                    c.conversation_kind,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    c.stable_external_conversation_key,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                    c.thread_id,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1 AND c.channel = ?2
                ORDER BY c.last_activity DESC
                LIMIT ?3
                "#,
                params![user_id, channel, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            results.push(summary_from_row(&row));
        }
        Ok(results)
    }

    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT c.user_id
                FROM conversations c
                WHERE c.channel = ?1
                  AND c.user_id IS NOT NULL
                  AND trim(c.user_id) <> ''
                GROUP BY c.user_id
                ORDER BY COUNT(*) DESC, MAX(c.last_activity) DESC, c.user_id ASC
                LIMIT 2
                "#,
                params![channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut candidates = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let user_id = row.get::<String>(0).unwrap_or_default();
            if !user_id.trim().is_empty() {
                candidates.push(user_id);
            }
        }

        let Some(primary) = candidates.first() else {
            return Ok(None);
        };

        if primary == "default" && candidates.len() > 1 {
            return Ok(candidates.get(1).cloned());
        }

        Ok(Some(primary.clone()))
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        // Try to find existing
        let mut rows = conn
            .query(
                r#"
                SELECT id FROM conversations
                WHERE user_id = ?1 AND channel = ?2
                  AND json_extract(metadata, '$.thread_type') = 'assistant'
                LIMIT 1
                "#,
                params![user_id, channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row.get(0).unwrap_or_default();
            return id_str
                .parse()
                .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()));
        }

        // Create new
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), channel, user_id, metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), channel, user_id, metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn update_conversation_identity(
        &self,
        id: Uuid,
        principal_id: Option<&str>,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let scope_id = conversation_scope_id.map(|scope| scope.to_string());
        conn.execute(
            r#"
            UPDATE conversations
            SET user_id = COALESCE(?2, user_id),
                actor_id = ?3,
                conversation_scope_id = COALESCE(?4, conversation_scope_id),
                conversation_kind = ?5,
                stable_external_conversation_key = COALESCE(?6, stable_external_conversation_key)
            WHERE id = ?1
            "#,
            params![
                id.to_string(),
                opt_text(principal_id),
                opt_text(actor_id),
                opt_text(scope_id.as_deref()),
                conversation_kind.as_str(),
                opt_text(stable_external_conversation_key),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let patch = serde_json::json!({ "handoff": handoff });
        conn.execute(
            "UPDATE conversations SET metadata = json_patch(coalesce(metadata, '{}'), ?2) WHERE id = ?1",
            params![id.to_string(), patch.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.connect().await?;
        let kind_predicate = if include_group_history {
            "c.conversation_kind IN ('direct', 'group')"
        } else {
            "c.conversation_kind = 'direct'"
        };
        let mut rows = conn
            .query(
                &format!(
                    r#"
                    SELECT
                        c.id,
                        c.user_id,
                        c.actor_id,
                        c.conversation_scope_id,
                        c.channel,
                        c.conversation_kind,
                        c.started_at,
                        c.last_activity,
                        c.metadata,
                        c.stable_external_conversation_key,
                        (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                        c.thread_id,
                        (SELECT substr(m2.content, 1, 100)
                         FROM conversation_messages m2
                         WHERE m2.conversation_id = c.id AND m2.role = 'user'
                         ORDER BY m2.created_at ASC, m2.rowid ASC
                         LIMIT 1
                        ) AS title
                    FROM conversations c
                    WHERE c.user_id = ?1
                      AND (
                        c.actor_id = ?2
                        OR ((c.actor_id IS NULL OR trim(c.actor_id) = '') AND ?2 = ?1)
                      )
                      AND {kind_predicate}
                    ORDER BY c.last_activity DESC
                    LIMIT ?3
                    "#
                ),
                params![principal_id, actor_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            results.push(summary_from_row(&row));
        }
        Ok(results)
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.connect().await?;
        let fetch_limit = limit + 1;
        let cid = conversation_id.to_string();

        let mut rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                    SELECT id, role, content, actor_id, actor_display_name, raw_sender_id, metadata, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1 AND created_at < ?2
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT ?3
                    "#,
                params![cid, fmt_ts(&before_ts), fetch_limit],
            )
            .await
        } else {
            conn.query(
                r#"
                    SELECT id, role, content, actor_id, actor_display_name, raw_sender_id, metadata, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT ?2
                    "#,
                params![cid, fetch_limit],
            )
            .await
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut all = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            all.push(message_from_row(&row));
        }

        let has_more = all.len() as i64 > limit;
        all.truncate(limit as usize);
        all.reverse(); // oldest first
        Ok((all, has_more))
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        // SQLite: use json_patch to merge the key
        let patch = serde_json::json!({ key: value });
        conn.execute(
            "UPDATE conversations SET metadata = json_patch(metadata, ?2) WHERE id = ?1",
            params![id.to_string(), patch.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT metadata FROM conversations WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_json(&row, 0))),
            None => Ok(None),
        }
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id, metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = ?1
                ORDER BY created_at ASC, rowid ASC
                "#,
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            messages.push(message_from_row(&row));
        }
        Ok(messages)
    }

    async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        let query = query.trim();
        if query.is_empty() || limit <= 0 {
            return Ok(Vec::new());
        }

        // FTS5 `MATCH` parses its argument as a query expression, so raw user
        // input containing `-`, `:`, `"`, etc. would be interpreted as operators
        // and error (Postgres tolerates the same input via websearch_to_tsquery).
        // Quote each token so the search stays at parity across backends.
        let match_query = super::fts::sanitize_fts5_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    m.id,
                    c.user_id,
                    c.actor_id,
                    c.channel,
                    c.thread_id,
                    c.conversation_kind,
                    m.role,
                    m.content,
                    substr(m.content, 1, 240) AS excerpt,
                    m.metadata,
                    m.created_at,
                    -bm25(conversation_messages_fts) AS score
                FROM conversation_messages_fts
                JOIN conversation_messages m ON m.rowid = conversation_messages_fts.rowid
                JOIN conversations c ON c.id = m.conversation_id
                WHERE conversation_messages_fts MATCH ?1
                  AND c.user_id = ?2
                  AND (?3 IS NULL OR COALESCE(NULLIF(c.actor_id, ''), c.user_id) = ?3)
                  AND (?4 IS NULL OR c.channel = ?4)
                  AND (?5 IS NULL OR c.thread_id = ?5)
                ORDER BY score DESC, m.created_at DESC, m.rowid DESC
                LIMIT ?6
                "#,
                params![match_query, user_id, actor_id, channel, thread_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut hits = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            hits.push(search_hit_from_row(&row));
        }
        Ok(hits)
    }

    async fn list_conversation_messages_for_learning(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        role: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }

        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    m.id,
                    c.user_id,
                    c.actor_id,
                    c.channel,
                    c.thread_id,
                    c.conversation_kind,
                    m.role,
                    m.content,
                    substr(m.content, 1, 240) AS excerpt,
                    m.metadata,
                    m.created_at,
                    CAST(NULL AS REAL) AS score
                FROM conversation_messages m
                JOIN conversations c ON c.id = m.conversation_id
                WHERE c.user_id = ?1
                  AND (?2 IS NULL OR COALESCE(NULLIF(c.actor_id, ''), c.user_id) = ?2)
                  AND (?3 IS NULL OR c.channel = ?3)
                  AND (?4 IS NULL OR c.thread_id = ?4)
                  AND (?5 IS NULL OR m.role = ?5)
                ORDER BY m.created_at DESC, m.rowid DESC
                LIMIT ?6
                "#,
                params![user_id, actor_id, channel, thread_id, role, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut hits = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            hits.push(search_hit_from_row(&row));
        }
        Ok(hits)
    }

    async fn insert_learning_event(&self, event: &LearningEvent) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if event.id.is_nil() {
            Uuid::new_v4()
        } else {
            event.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_events (
                id, user_id, actor_id, channel, thread_id, conversation_id,
                message_id, job_id, event_type, source, payload, metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                id.to_string(),
                event.user_id.as_str(),
                event.actor_id.as_deref(),
                event.channel.as_deref(),
                event.thread_id.as_deref(),
                event.conversation_id.map(|id| id.to_string()),
                event.message_id.map(|id| id.to_string()),
                event.job_id.map(|id| id.to_string()),
                event.event_type.as_str(),
                event.source.as_str(),
                event.payload.to_string(),
                event
                    .metadata
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
                fmt_ts(&event.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }

        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id,
                    user_id,
                    actor_id,
                    channel,
                    thread_id,
                    conversation_id,
                    message_id,
                    job_id,
                    event_type,
                    source,
                    payload,
                    metadata,
                    created_at
                FROM learning_events
                WHERE user_id = ?1
                  AND (?2 IS NULL OR COALESCE(NULLIF(actor_id, ''), user_id) = ?2)
                  AND (?3 IS NULL OR channel = ?3)
                  AND (?4 IS NULL OR thread_id = ?4)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?5
                "#,
                params![user_id, actor_id, channel, thread_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(learning_event_from_row(&row));
        }
        Ok(events)
    }

    async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if evaluation.id.is_nil() {
            Uuid::new_v4()
        } else {
            evaluation.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_evaluations (
                id, learning_event_id, user_id, evaluator, status, score, details, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                id.to_string(),
                evaluation.learning_event_id.to_string(),
                evaluation.user_id.as_str(),
                evaluation.evaluator.as_str(),
                evaluation.status.as_str(),
                evaluation.score,
                evaluation.details.to_string(),
                fmt_ts(&evaluation.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, learning_event_id, user_id, evaluator, status, score, details, created_at
                FROM learning_evaluations
                WHERE user_id = ?1
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?2
                "#,
                params![user_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut evaluations = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            evaluations.push(learning_evaluation_from_row(&row));
        }
        Ok(evaluations)
    }

    async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if candidate.id.is_nil() {
            Uuid::new_v4()
        } else {
            candidate.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_candidates (
                id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                target_type, target_name, summary, proposal, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                id.to_string(),
                candidate.learning_event_id.map(|value| value.to_string()),
                candidate.user_id.as_str(),
                candidate.candidate_type.as_str(),
                candidate.risk_tier.as_str(),
                candidate.confidence,
                candidate.target_type.as_deref(),
                candidate.target_name.as_deref(),
                candidate.summary.as_deref(),
                candidate.proposal.to_string(),
                fmt_ts(&candidate.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                    target_type, target_name, summary, proposal, created_at
                FROM learning_candidates
                WHERE user_id = ?1
                  AND (?2 IS NULL OR candidate_type = ?2)
                  AND (?3 IS NULL OR risk_tier = ?3)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?4
                "#,
                params![user_id, candidate_type, risk_tier, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut candidates = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            candidates.push(learning_candidate_from_row(&row));
        }
        Ok(candidates)
    }

    async fn update_learning_candidate_proposal(
        &self,
        candidate_id: Uuid,
        proposal: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE learning_candidates
            SET proposal = ?2
            WHERE id = ?1
            "#,
            params![candidate_id.to_string(), proposal.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if version.id.is_nil() {
            Uuid::new_v4()
        } else {
            version.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_artifact_versions (
                id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                status, diff_summary, before_content, after_content, provenance, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                id.to_string(),
                version.candidate_id.map(|value| value.to_string()),
                version.user_id.as_str(),
                version.artifact_type.as_str(),
                version.artifact_name.as_str(),
                version.version_label.as_deref(),
                version.status.as_str(),
                version.diff_summary.as_deref(),
                version.before_content.as_deref(),
                version.after_content.as_deref(),
                version.provenance.to_string(),
                fmt_ts(&version.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                    status, diff_summary, before_content, after_content, provenance, created_at
                FROM learning_artifact_versions
                WHERE user_id = ?1
                  AND (?2 IS NULL OR artifact_type = ?2)
                  AND (?3 IS NULL OR artifact_name = ?3)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?4
                "#,
                params![user_id, artifact_type, artifact_name, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut versions = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            versions.push(learning_artifact_version_from_row(&row));
        }
        Ok(versions)
    }

    async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if feedback.id.is_nil() {
            Uuid::new_v4()
        } else {
            feedback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_feedback (
                id, user_id, target_type, target_id, verdict, note, metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id.to_string(),
                feedback.user_id.as_str(),
                feedback.target_type.as_str(),
                feedback.target_id.as_str(),
                feedback.verdict.as_str(),
                feedback.note.as_deref(),
                feedback.metadata.to_string(),
                fmt_ts(&feedback.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, target_type, target_id, verdict, note, metadata, created_at
                FROM learning_feedback
                WHERE user_id = ?1
                  AND (?2 IS NULL OR target_type = ?2)
                  AND (?3 IS NULL OR target_id = ?3)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?4
                "#,
                params![user_id, target_type, target_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut feedback = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            feedback.push(learning_feedback_from_row(&row));
        }
        Ok(feedback)
    }

    async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if rollback.id.is_nil() {
            Uuid::new_v4()
        } else {
            rollback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_rollbacks (
                id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id.to_string(),
                rollback.user_id.as_str(),
                rollback.artifact_type.as_str(),
                rollback.artifact_name.as_str(),
                rollback.artifact_version_id.map(|value| value.to_string()),
                rollback.reason.as_str(),
                rollback.metadata.to_string(),
                fmt_ts(&rollback.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
                FROM learning_rollbacks
                WHERE user_id = ?1
                  AND (?2 IS NULL OR artifact_type = ?2)
                  AND (?3 IS NULL OR artifact_name = ?3)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?4
                "#,
                params![user_id, artifact_type, artifact_name, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut rollbacks = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            rollbacks.push(learning_rollback_from_row(&row));
        }
        Ok(rollbacks)
    }

    async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if proposal.id.is_nil() {
            Uuid::new_v4()
        } else {
            proposal.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_code_proposals (
                id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
            params![
                id.to_string(),
                proposal.learning_event_id.map(|value| value.to_string()),
                proposal.user_id.as_str(),
                proposal.status.as_str(),
                proposal.title.as_str(),
                proposal.rationale.as_str(),
                serde_json::to_string(&proposal.target_files).unwrap_or_else(|_| "[]".to_string()),
                proposal.diff.as_str(),
                proposal.validation_results.to_string(),
                proposal.rollback_note.as_deref(),
                proposal.confidence,
                proposal.branch_name.as_deref(),
                proposal.pr_url.as_deref(),
                proposal.metadata.to_string(),
                fmt_ts(&proposal.created_at),
                fmt_ts(&proposal.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE id = ?1 AND user_id = ?2
                LIMIT 1
                "#,
                params![proposal_id.to_string(), user_id],
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
        Ok(Some(learning_code_proposal_from_row(&row)))
    }

    async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE user_id = ?1
                  AND (?2 IS NULL OR status = ?2)
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?3
                "#,
                params![user_id, status, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut proposals = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            proposals.push(learning_code_proposal_from_row(&row));
        }
        Ok(proposals)
    }

    async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let metadata_patch = metadata.cloned();
        conn.execute(
            r#"
            UPDATE learning_code_proposals
            SET status = ?2,
                branch_name = COALESCE(?3, branch_name),
                pr_url = COALESCE(?4, pr_url),
                metadata = CASE
                    WHEN ?5 IS NULL THEN metadata
                    ELSE json_patch(coalesce(metadata, '{}'), ?5)
                END,
                updated_at = ?6
            WHERE id = ?1
            "#,
            params![
                proposal_id.to_string(),
                status,
                branch_name,
                pr_url,
                metadata_patch.map(|value| value.to_string()),
                fmt_ts(&Utc::now()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
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

    async fn get_outcome_contract(
        &self,
        user_id: &str,
        contract_id: Uuid,
    ) -> Result<Option<OutcomeContract>, DatabaseError> {
        let conn = self.connect().await?;
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

    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        if query.limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
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

    async fn claim_due_outcome_contracts(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.claim_due_outcome_contracts_with_lease("legacy", limit, now, 300)
            .await
    }

    async fn claim_due_outcome_contracts_for_user(
        &self,
        user_id: &str,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.claim_due_outcome_contracts_for_user_with_lease(user_id, "legacy", limit, now, 300)
            .await
    }

    async fn claim_due_outcome_contracts_with_lease(
        &self,
        worker_id: &str,
        limit: i64,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.claim_due_outcome_contracts_for_user_with_lease("", worker_id, limit, now, lease_secs)
            .await
    }

    async fn claim_due_outcome_contracts_for_user_with_lease(
        &self,
        user_id: &str,
        worker_id: &str,
        limit: i64,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        outcomes::claim_due_for_user_with_lease(self, user_id, worker_id, limit, now, lease_secs)
            .await
    }

    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
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

    async fn update_claimed_outcome_contract(
        &self,
        contract: &OutcomeContract,
        worker_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
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

    async fn outcome_summary_stats(
        &self,
        user_id: &str,
    ) -> Result<OutcomeSummaryStats, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    (SELECT COUNT(*) FROM outcome_contracts
                     WHERE user_id = ?1
                       AND status IN ('open', 'evaluating')) AS open_count,
                    (SELECT COUNT(*) FROM outcome_contracts
                     WHERE user_id = ?1
                       AND status = 'open'
                       AND due_at <= ?2
                       AND expires_at > ?2) AS due_count,
                    (SELECT COUNT(*) FROM outcome_contracts
                     WHERE user_id = ?1
                       AND status = 'evaluated'
                       AND COALESCE(evaluated_at, updated_at) >= ?3) AS evaluated_count,
                    (SELECT COALESCE(AVG(CASE WHEN final_verdict = 'negative' THEN 1.0 ELSE 0.0 END), 0.0)
                     FROM outcome_contracts
                     WHERE user_id = ?1
                       AND status = 'evaluated'
                       AND COALESCE(evaluated_at, updated_at) >= ?3) AS negative_ratio
                "#,
                params![
                    user_id,
                    fmt_ts(&Utc::now()),
                    fmt_ts(&(Utc::now() - chrono::Duration::days(7))),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        else {
            return Ok(OutcomeSummaryStats::default());
        };
        Ok(OutcomeSummaryStats {
            open: row.get::<i64>(0).unwrap_or_default() as u64,
            due: row.get::<i64>(1).unwrap_or_default() as u64,
            evaluated_last_7d: row.get::<i64>(2).unwrap_or_default() as u64,
            negative_ratio_last_7d: row.get::<f64>(3).unwrap_or(0.0),
        })
    }

    async fn list_users_with_pending_outcome_work(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomePendingUser>, DatabaseError> {
        let conn = self.connect().await?;
        let now_ts = fmt_ts(&now);
        let legacy_stale_before = fmt_ts(&(now - chrono::Duration::seconds(300)));
        let mut rows = conn
            .query(
                r#"
                SELECT DISTINCT user_id
                FROM outcome_contracts
                WHERE ((status = 'open'
                        AND due_at <= ?1
                        AND expires_at > ?1
                        AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                       OR (status = 'evaluating'
                           AND expires_at > ?1
                           AND ((lease_expires_at IS NOT NULL AND lease_expires_at <= ?1)
                                OR (lease_expires_at IS NULL
                                    AND (claimed_at IS NULL OR claimed_at <= ?2)))))
                ORDER BY user_id ASC
                "#,
                params![now_ts, legacy_stale_before],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut users = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            users.push(OutcomePendingUser {
                user_id: get_text(&row, 0),
            });
        }
        Ok(users)
    }

    async fn outcome_evaluator_health(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutcomeEvaluatorHealth, DatabaseError> {
        let conn = self.connect().await?;
        let now_ts = fmt_ts(&now);
        let mut rows = conn
            .query(
                r#"
                SELECT
                    (
                        SELECT MIN(due_at)
                        FROM outcome_contracts
                        WHERE user_id = ?1
                          AND status = 'open'
                          AND due_at <= ?2
                          AND expires_at > ?2
                    ) AS oldest_due_at,
                    (
                        SELECT MIN(COALESCE(claimed_at, updated_at))
                        FROM outcome_contracts
                        WHERE user_id = ?1
                          AND status = 'evaluating'
                    ) AS oldest_evaluating_claimed_at
                "#,
                params![user_id, now_ts],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        else {
            return Ok(OutcomeEvaluatorHealth::default());
        };

        Ok(OutcomeEvaluatorHealth {
            oldest_due_at: get_opt_text(&row, 0).and_then(|value| {
                chrono::DateTime::parse_from_rfc3339(&value)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            oldest_evaluating_claimed_at: get_opt_text(&row, 1).and_then(|value| {
                chrono::DateTime::parse_from_rfc3339(&value)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
        })
    }

    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservation,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = if observation.id.is_nil() {
            Uuid::new_v4()
        } else {
            observation.id
        };
        let affected = conn
            .execute(
                r#"
                INSERT OR IGNORE INTO outcome_observations (
                    id, contract_id, observation_kind, polarity, weight, summary, evidence,
                    fingerprint, observed_at, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10
                )
                "#,
                params![
                    id.to_string(),
                    observation.contract_id.to_string(),
                    observation.observation_kind.as_str(),
                    observation.polarity.as_str(),
                    observation.weight,
                    observation.summary.as_deref(),
                    observation.evidence.to_string(),
                    observation.fingerprint.as_str(),
                    fmt_ts(&observation.observed_at),
                    fmt_ts(&observation.created_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected > 0 {
            return Ok(id);
        }

        let mut rows = conn
            .query(
                "SELECT id FROM outcome_observations WHERE contract_id = ?1 AND fingerprint = ?2 LIMIT 1",
                params![
                    observation.contract_id.to_string(),
                    observation.fingerprint.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        else {
            return Err(DatabaseError::Query(
                "failed to resolve existing outcome observation".to_string(),
            ));
        };
        Ok(get_text(&row, 0).parse().unwrap_or_default())
    }

    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservation>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    id, contract_id, observation_kind, polarity, weight, summary, evidence,
                    fingerprint, observed_at, created_at
                FROM outcome_observations
                WHERE contract_id = ?1
                ORDER BY observed_at ASC, rowid ASC
                "#,
                params![contract_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut observations = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            observations.push(outcome_observation_from_row(&row));
        }
        Ok(observations)
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM conversations WHERE id = ?1 AND user_id = ?2",
                libsql::params![conversation_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }

    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT 1
                FROM conversations
                WHERE id = ?1
                  AND user_id = ?2
                  AND (
                    actor_id = ?3
                    OR ((actor_id IS NULL OR trim(actor_id) = '') AND ?3 = ?2)
                  )
                "#,
                params![conversation_id.to_string(), principal_id, actor_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }

    async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        // ON DELETE CASCADE in schema handles conversation_messages automatically
        let rows = conn
            .execute(
                "DELETE FROM conversations WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let rows = conn
            .execute(
                "DELETE FROM conversation_messages WHERE conversation_id = ?1",
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows)
    }
}

mod outcomes;
mod rows;

pub(crate) use rows::*;

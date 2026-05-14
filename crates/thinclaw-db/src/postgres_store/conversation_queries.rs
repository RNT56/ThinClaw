use super::*;
// ==================== Conversation Persistence ====================

pub use thinclaw_history::{ConversationMessage, ConversationSummary};

#[cfg(feature = "postgres")]
pub(super) fn conversation_stable_key(
    channel: &str,
    thread_id: Option<&str>,
    fallback: Uuid,
) -> String {
    match thread_id {
        Some(thread_id) if !thread_id.is_empty() => format!("{channel}:{thread_id}"),
        _ => format!("{channel}:{fallback}"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn conversation_handoff_from_metadata(
    metadata: &serde_json::Value,
) -> Option<ConversationHandoffMetadata> {
    let value = metadata.get("handoff").cloned().or_else(|| {
        let direct = serde_json::json!({
            "last_actor_id": metadata.get("last_actor_id"),
            "task_state": metadata.get("task_state"),
            "last_user_goal": metadata.get("last_user_goal"),
            "handoff_summary": metadata.get("handoff_summary"),
        });
        if direct
            .as_object()
            .map(|m| m.values().any(|v| !v.is_null()))
            .unwrap_or(false)
        {
            Some(direct)
        } else {
            None
        }
    })?;

    serde_json::from_value(value)
        .ok()
        .filter(|handoff: &ConversationHandoffMetadata| !handoff.is_empty())
}

#[allow(dead_code)] // Prepared for conversation handoff persistence path
pub(super) fn conversation_metadata_with_handoff(
    metadata: &serde_json::Value,
    handoff: &ConversationHandoffMetadata,
) -> serde_json::Value {
    let mut merged = metadata.clone();
    if merged.is_null() || !merged.is_object() {
        merged = serde_json::json!({});
    }

    let mut handoff_value = match serde_json::to_value(handoff) {
        Ok(value) => value,
        Err(_) => serde_json::json!({}),
    };
    if handoff_value.is_null() {
        handoff_value = serde_json::json!({});
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert("handoff".to_string(), handoff_value);
    }
    merged
}

#[cfg(feature = "postgres")]
pub(super) fn conversation_summary_from_row(row: &tokio_postgres::Row) -> ConversationSummary {
    let metadata: serde_json::Value = row.get("metadata");
    let thread_type = metadata
        .get("thread_type")
        .and_then(|v: &serde_json::Value| v.as_str())
        .map(String::from);
    let handoff = conversation_handoff_from_metadata(&metadata);
    let conversation_kind = ConversationKind::from_db(
        row.try_get::<_, Option<String>>("conversation_kind")
            .ok()
            .flatten()
            .as_deref(),
    );
    let actor_id = row.try_get::<_, Option<String>>("actor_id").ok().flatten();
    let conversation_scope_id = row
        .try_get::<_, Option<Uuid>>("conversation_scope_id")
        .ok()
        .flatten();
    let stable_external_conversation_key = row
        .try_get::<_, Option<String>>("stable_external_conversation_key")
        .ok()
        .flatten();

    ConversationSummary {
        id: row.get("id"),
        user_id: row.get("user_id"),
        actor_id,
        conversation_scope_id,
        conversation_kind,
        channel: row.get("channel"),
        title: row.get("title"),
        message_count: row.get("message_count"),
        started_at: row.get("started_at"),
        last_activity: row.get("last_activity"),
        thread_type,
        handoff,
        stable_external_conversation_key,
    }
}

#[cfg(feature = "postgres")]
pub(super) fn conversation_message_from_row(row: &tokio_postgres::Row) -> ConversationMessage {
    ConversationMessage {
        id: row.get("id"),
        role: row.get("role"),
        content: row.get("content"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        actor_display_name: row
            .try_get::<_, Option<String>>("actor_display_name")
            .ok()
            .flatten(),
        raw_sender_id: row
            .try_get::<_, Option<String>>("raw_sender_id")
            .ok()
            .flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
impl Store {
    /// Ensure a conversation row exists for a given UUID.
    ///
    /// Idempotent: inserts on first call, bumps `last_activity` on subsequent calls.
    pub async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO conversations (id, channel, user_id, thread_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE SET last_activity = NOW()
            "#,
            &[&id, &channel, &user_id, &thread_id],
        )
        .await?;
        Ok(())
    }

    /// List conversations with a title derived from the first user message.
    pub async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.user_id,
                    c.actor_id,
                    c.conversation_scope_id,
                    c.conversation_kind,
                    c.channel,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    c.stable_external_conversation_key,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                    (SELECT LEFT(m2.content, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = $1 AND c.channel = $2
                ORDER BY c.last_activity DESC
                LIMIT $3
                "#,
                &[&user_id, &channel, &limit],
            )
            .await?;

        Ok(rows.iter().map(conversation_summary_from_row).collect())
    }

    /// Infer the principal that owns the majority of history for a channel.
    ///
    /// When the placeholder `"default"` principal and a real principal both
    /// exist, this prefers the non-default principal so upgraded gateway chat
    /// UIs reconnect to the historical owner automatically.
    pub async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT c.user_id
                FROM conversations c
                WHERE c.channel = $1
                  AND c.user_id IS NOT NULL
                  AND btrim(c.user_id) <> ''
                GROUP BY c.user_id
                ORDER BY COUNT(*) DESC, MAX(c.last_activity) DESC, c.user_id ASC
                LIMIT 2
                "#,
                &[&channel],
            )
            .await?;

        let candidates: Vec<String> = rows
            .iter()
            .filter_map(|row| row.try_get::<_, String>("user_id").ok())
            .filter(|user_id| !user_id.trim().is_empty())
            .collect();

        let Some(primary) = candidates.first() else {
            return Ok(None);
        };

        if primary == "default" && candidates.len() > 1 {
            return Ok(candidates.get(1).cloned());
        }

        Ok(Some(primary.clone()))
    }

    /// Get or create the singleton "assistant" conversation for a user+channel.
    ///
    /// Looks for a conversation where `metadata->>'thread_type' = 'assistant'`.
    /// Creates one if it doesn't exist.
    pub async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;

        // Try to find existing assistant conversation
        let row = conn
            .query_opt(
                r#"
                SELECT id FROM conversations
                WHERE user_id = $1 AND channel = $2 AND metadata->>'thread_type' = 'assistant'
                LIMIT 1
                "#,
                &[&user_id, &channel],
            )
            .await?;

        if let Some(row) = row {
            return Ok(row.get("id"));
        }

        // Create a new assistant conversation
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &conversation_stable_key(channel, Some("assistant"), id),
                &metadata,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Create a conversation with specific metadata.
    pub async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let stable_external_conversation_key = conversation_stable_key(channel, None, id);

        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &stable_external_conversation_key,
                &metadata,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Check whether a conversation belongs to the given user.
    pub async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM conversations WHERE id = $1 AND user_id = $2",
                &[&conversation_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Load messages for a conversation with cursor-based pagination.
    ///
    /// Returns `(messages_oldest_first, has_more)`.
    /// Pass `before` as a cursor to load older messages.
    pub async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.conn().await?;
        let fetch_limit = limit + 1; // Fetch one extra to determine has_more

        let rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1 AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                &[&conversation_id, &before_ts, &fetch_limit],
            )
            .await?
        } else {
            conn.query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&conversation_id, &fetch_limit],
            )
            .await?
        };

        let has_more = rows.len() as i64 > limit;
        let take_count = (rows.len() as i64).min(limit) as usize;

        // Rows come newest-first from DB; reverse so caller gets oldest-first
        let mut messages: Vec<ConversationMessage> = rows
            .iter()
            .take(take_count)
            .map(conversation_message_from_row)
            .collect();
        messages.reverse();

        Ok((messages, has_more))
    }

    /// Merge a single key into a conversation's metadata JSONB.
    pub async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        if key == "handoff" {
            conn.execute(
                "UPDATE conversations SET metadata = jsonb_set(coalesce(metadata, '{}'::jsonb), '{handoff}', $2::jsonb, true) WHERE id = $1",
                &[&id, &value],
            )
            .await?;
        } else {
            let patch = serde_json::json!({ key: value });
            conn.execute(
                "UPDATE conversations SET metadata = metadata || $2 WHERE id = $1",
                &[&id, &patch],
            )
            .await?;
        }
        Ok(())
    }

    /// Read the metadata JSONB for a conversation.
    pub async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT metadata FROM conversations WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get::<_, serde_json::Value>(0)))
    }

    /// Load all messages for a conversation, ordered chronologically.
    pub async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at ASC
                "#,
                &[&conversation_id],
            )
            .await?;

        Ok(rows.iter().map(conversation_message_from_row).collect())
    }

    /// Search conversation messages for a user across transcripts.
    pub async fn search_conversation_messages(
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

        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id AS conversation_id,
                    m.id AS message_id,
                    c.user_id,
                    c.actor_id,
                    c.channel,
                    c.thread_id,
                    c.conversation_kind,
                    m.role,
                    m.content,
                    LEFT(m.content, 240) AS excerpt,
                    COALESCE(m.metadata, '{}'::jsonb) AS metadata,
                    m.created_at,
                    ts_rank_cd(
                        to_tsvector('simple', COALESCE(m.content, '')),
                        websearch_to_tsquery('simple', $2)
                    )::float8 AS score
                FROM conversation_messages m
                JOIN conversations c ON c.id = m.conversation_id
                WHERE c.user_id = $1
                  AND to_tsvector('simple', COALESCE(m.content, ''))
                        @@ websearch_to_tsquery('simple', $2)
                  AND ($3::text IS NULL OR COALESCE(NULLIF(c.actor_id, ''), c.user_id) = $3)
                  AND ($4::text IS NULL OR c.channel = $4)
                  AND ($5::text IS NULL OR c.thread_id = $5)
                ORDER BY score DESC, m.created_at DESC, m.id DESC
                LIMIT $6
                "#,
                &[&user_id, &query, &actor_id, &channel, &thread_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(session_search_hit_from_row).collect())
    }

    /// List conversation messages for learning workflows with bounded scope.
    pub async fn list_conversation_messages_for_learning(
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

        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id AS conversation_id,
                    m.id AS message_id,
                    c.user_id,
                    c.actor_id,
                    c.channel,
                    c.thread_id,
                    c.conversation_kind,
                    m.role,
                    m.content,
                    LEFT(m.content, 240) AS excerpt,
                    COALESCE(m.metadata, '{}'::jsonb) AS metadata,
                    m.created_at,
                    NULL::double precision AS score
                FROM conversation_messages m
                JOIN conversations c ON c.id = m.conversation_id
                WHERE c.user_id = $1
                  AND ($2::text IS NULL OR COALESCE(NULLIF(c.actor_id, ''), c.user_id) = $2)
                  AND ($3::text IS NULL OR c.channel = $3)
                  AND ($4::text IS NULL OR c.thread_id = $4)
                  AND ($5::text IS NULL OR m.role = $5)
                ORDER BY m.created_at DESC, m.id DESC
                LIMIT $6
                "#,
                &[&user_id, &actor_id, &channel, &thread_id, &role, &limit],
            )
            .await?;

        Ok(rows.iter().map(session_search_hit_from_row).collect())
    }
}

//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::db::ConversationStore;
use crate::error::DatabaseError;
use crate::history::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationSummary,
};

fn handoff_from_metadata(metadata: &serde_json::Value) -> Option<ConversationHandoffMetadata> {
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

fn kind_from_row(row: &libsql::Row) -> ConversationKind {
    ConversationKind::from_db(row.get::<String>(5).ok().as_deref())
}

fn summary_from_row(row: &libsql::Row) -> ConversationSummary {
    let metadata = get_json(row, 8);
    ConversationSummary {
        id: row
            .get::<String>(0)
            .unwrap_or_default()
            .parse()
            .unwrap_or_default(),
        user_id: get_text(row, 1),
        actor_id: get_opt_text(row, 2),
        conversation_scope_id: get_opt_text(row, 3).and_then(|s| s.parse().ok()),
        conversation_kind: kind_from_row(row),
        channel: get_text(row, 4),
        title: get_opt_text(row, 12),
        message_count: get_i64(row, 10),
        started_at: get_ts(row, 6),
        last_activity: get_ts(row, 7),
        thread_type: metadata
            .get("thread_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        handoff: handoff_from_metadata(&metadata),
        stable_external_conversation_key: get_opt_text(row, 9),
    }
}

fn message_from_row(row: &libsql::Row) -> ConversationMessage {
    ConversationMessage {
        id: get_text(row, 0).parse().unwrap_or_default(),
        role: get_text(row, 1),
        content: get_text(row, 2),
        actor_id: get_opt_text(row, 3),
        actor_display_name: get_opt_text(row, 4),
        raw_sender_id: get_opt_text(row, 5),
        metadata: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

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
            SET actor_id = ?2,
                conversation_scope_id = COALESCE(?3, conversation_scope_id),
                conversation_kind = ?4,
                stable_external_conversation_key = COALESCE(?5, stable_external_conversation_key)
            WHERE id = ?1
            "#,
            params![
                id.to_string(),
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
                      AND c.actor_id = ?2
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

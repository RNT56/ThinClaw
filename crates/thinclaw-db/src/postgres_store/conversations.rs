#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    // ==================== Conversations ====================

    /// Create a new conversation.
    pub async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let stable_external_conversation_key = conversation_stable_key(channel, thread_id, id);

        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                thread_id, stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, '{}'::jsonb)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &thread_id,
                &stable_external_conversation_key,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Update conversation last activity.
    pub async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE conversations SET last_activity = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;
        Ok(())
    }

    /// Add a message to a conversation.
    pub async fn add_conversation_message(
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

    /// Add a message with actor attribution and message-level metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let metadata_value = metadata.cloned().unwrap_or_else(|| serde_json::json!({}));

        conn.execute(
            r#"
            INSERT INTO conversation_messages (
                id, conversation_id, role, content, actor_id, actor_display_name,
                raw_sender_id, metadata
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            &[
                &id,
                &conversation_id,
                &role,
                &content,
                &actor_id,
                &actor_display_name,
                &raw_sender_id,
                &metadata_value,
            ],
        )
        .await?;

        // The row is already committed. Do not report the append as failed if
        // only the secondary activity timestamp write fails: doing so would
        // leave callers unable to know whether they should retain the turn.
        if let Err(error) = self.touch_conversation(conversation_id).await {
            tracing::warn!(
                conversation = %conversation_id,
                %error,
                "Conversation message persisted but activity timestamp update failed"
            );
        }

        Ok(id)
    }

    /// Persist a trusted model-visible rewrite while retaining the raw user
    /// message content as the transcript/audit source of truth.
    pub async fn set_effective_user_instruction(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        effective_instruction: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let changed = conn
            .execute(
                r#"
                UPDATE conversation_messages
                SET metadata = jsonb_set(
                    jsonb_set(
                        coalesce(metadata, '{}'::jsonb),
                        '{_thinclaw_effective_user_instruction_version}',
                        '1'::jsonb,
                        true
                    ),
                    '{_thinclaw_effective_user_instruction}',
                    to_jsonb($3::text),
                    true
                )
                WHERE conversation_id = $1 AND id = $2 AND role = 'user'
                "#,
                &[&conversation_id, &message_id, &effective_instruction],
            )
            .await?;
        if changed != 1 {
            return Err(DatabaseError::NotFound {
                entity: "conversation user message".to_string(),
                id: message_id.to_string(),
            });
        }
        Ok(())
    }

    /// Delete a conversation and all its messages (cascading via FK).
    pub async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .execute("DELETE FROM conversations WHERE id = $1", &[&id])
            .await?;
        Ok(rows > 0)
    }

    /// Delete all messages from a conversation without deleting the conversation.
    pub async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .execute(
                "DELETE FROM conversation_messages WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await?;
        Ok(rows)
    }

    /// Update the actor-aware identity fields for a conversation.
    pub async fn update_conversation_identity(
        &self,
        id: Uuid,
        principal_id: Option<&str>,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE conversations
            SET user_id = COALESCE($2, user_id),
                actor_id = CASE
                    WHEN $5 = 'group' THEN COALESCE(NULLIF(btrim(actor_id), ''), $3)
                    ELSE $3
                END,
                conversation_scope_id = COALESCE($4, conversation_scope_id),
                conversation_kind = $5,
                stable_external_conversation_key = COALESCE($6, stable_external_conversation_key)
            WHERE id = $1
            "#,
            &[
                &id,
                &principal_id,
                &actor_id,
                &conversation_scope_id,
                &conversation_kind.as_str(),
                &stable_external_conversation_key,
            ],
        )
        .await?;
        Ok(())
    }

    /// Resolve the latest durable conversation addressed by an ingress
    /// identity. This restores native channel room/thread continuity when the
    /// external key is not a ThinClaw UUID.
    #[allow(clippy::too_many_arguments)]
    pub async fn find_latest_conversation_for_ingress(
        &self,
        principal_id: &str,
        actor_id: &str,
        conversation_scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT c.id
                FROM conversations c
                WHERE c.user_id = $1
                  AND c.conversation_kind = $4
                  AND (
                    ($4 = 'direct'
                      AND (
                        c.actor_id = $2
                        OR ((c.actor_id IS NULL OR btrim(c.actor_id) = '') AND $2 = $1)
                      )
                      AND ($6 IS NULL OR (c.channel = $5 AND c.thread_id = $6)))
                    OR
                    ($4 = 'group' AND c.conversation_scope_id = $3)
                  )
                ORDER BY c.last_activity DESC, c.started_at DESC, c.id DESC
                LIMIT 1
                "#,
                &[
                    &principal_id,
                    &actor_id,
                    &conversation_scope_id,
                    &conversation_kind.as_str(),
                    &channel,
                    &external_thread_id,
                ],
            )
            .await?;
        Ok(row.map(|row| row.get("id")))
    }

    /// Update the compact handoff metadata for a conversation.
    pub async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let handoff_value = serde_json::to_value(handoff)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        conn.execute(
            "UPDATE conversations SET metadata = jsonb_set(coalesce(metadata, '{}'::jsonb), '{handoff}', $2::jsonb, true) WHERE id = $1",
            &[&id, &handoff_value],
        )
        .await?;
        Ok(())
    }

    /// List conversations that are linked to an actor across channels.
    ///
    /// When `include_group_history` is false, this intentionally filters to
    /// direct conversations only so automatic recall never pulls group history.
    pub async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.conn().await?;
        let kind_filter = if include_group_history {
            "('direct', 'group')"
        } else {
            "('direct')"
        };
        let rows = conn
            .query(
                &format!(
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
                    WHERE c.user_id = $1
                      AND (
                        (c.conversation_kind = 'direct' AND (
                          c.actor_id = $2
                          OR ((c.actor_id IS NULL OR btrim(c.actor_id) = '') AND $2 = $1)
                        ))
                        OR (c.conversation_kind = 'group' AND (
                          c.actor_id = $2
                          OR EXISTS (
                            SELECT 1 FROM conversation_messages membership
                            WHERE membership.conversation_id = c.id
                              AND membership.actor_id = $2
                          )
                        ))
                      )
                      AND c.conversation_kind IN {}
                    ORDER BY c.last_activity DESC
                    LIMIT $3
                    "#,
                    kind_filter
                ),
                &[&principal_id, &actor_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(conversation_summary_from_row).collect())
    }

    /// Check whether a conversation belongs to the given actor.
    pub async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT 1 FROM conversations c
                WHERE c.id = $1
                  AND c.user_id = $2
                  AND (
                    (c.conversation_kind = 'direct' AND (
                      c.actor_id = $3
                      OR ((c.actor_id IS NULL OR btrim(c.actor_id) = '') AND $3 = $2)
                    ))
                    OR (c.conversation_kind = 'group' AND (
                      c.actor_id = $3
                      OR EXISTS (
                        SELECT 1 FROM conversation_messages membership
                        WHERE membership.conversation_id = c.id
                          AND membership.actor_id = $3
                      )
                    ))
                  )
                "#,
                &[&conversation_id, &principal_id, &actor_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Check complete actor/scope visibility. Direct conversations are actor
    /// private; group conversations are keyed by their stable external scope.
    pub async fn conversation_belongs_to_identity(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
        conversation_scope_id: Uuid,
        conversation_kind: ConversationKind,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT 1 FROM conversations c
                WHERE c.id = $1
                  AND c.user_id = $2
                  AND (
                    ($5 = 'direct'
                      AND c.conversation_kind = 'direct'
                      AND (
                        c.actor_id = $3
                        OR ((c.actor_id IS NULL OR btrim(c.actor_id) = '') AND $3 = $2)
                      ))
                    OR
                    ($5 = 'group'
                      AND c.conversation_kind = 'group'
                      AND c.conversation_scope_id = $4)
                  )
                "#,
                &[
                    &conversation_id,
                    &principal_id,
                    &actor_id,
                    &conversation_scope_id,
                    &conversation_kind.as_str(),
                ],
            )
            .await?;
        Ok(row.is_some())
    }

    // ==================== Jobs ====================
}

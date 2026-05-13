//! Root database adapter for the extracted agent thread-store port.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thinclaw_agent::ports::{AgentScope, ThreadMessage, ThreadRuntimeSnapshot, ThreadStorePort};
use uuid::Uuid;

use crate::db::Database;
use crate::error::DatabaseError;

pub struct RootThreadStorePort {
    store: Arc<dyn Database>,
}

impl RootThreadStorePort {
    pub fn shared(store: Arc<dyn Database>) -> Arc<dyn ThreadStorePort> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl ThreadStorePort for RootThreadStorePort {
    async fn ensure_thread(
        &self,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
        external_thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .ensure_conversation(thread_id, channel, user_id, external_thread_id)
            .await
    }

    async fn load_thread_runtime(
        &self,
        thread_id: Uuid,
    ) -> Result<Option<ThreadRuntimeSnapshot>, DatabaseError> {
        let Some(metadata) = self.store.get_conversation_metadata(thread_id).await? else {
            return Ok(None);
        };
        thinclaw_agent::thread_runtime::decode_thread_runtime(&metadata)
    }

    async fn save_thread_runtime(
        &self,
        thread_id: Uuid,
        runtime: &ThreadRuntimeSnapshot,
    ) -> Result<(), DatabaseError> {
        let value = thinclaw_agent::thread_runtime::encode_thread_runtime(runtime)?;
        self.store
            .update_conversation_metadata_field(
                thread_id,
                thinclaw_agent::thread_runtime::THREAD_RUNTIME_METADATA_KEY,
                &value,
            )
            .await
    }

    async fn append_thread_message(
        &self,
        thread_id: Uuid,
        role: &str,
        content: &str,
        attribution: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message_with_attribution(
                thread_id,
                role,
                content,
                None,
                None,
                None,
                attribution,
            )
            .await
    }

    async fn list_thread_messages(
        &self,
        thread_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<ThreadMessage>, DatabaseError> {
        let (messages, _) = self
            .store
            .list_conversation_messages_paginated(thread_id, before, limit)
            .await?;
        Ok(messages
            .into_iter()
            .map(|message| thread_message_from_history(thread_id, message))
            .collect())
    }

    async fn list_threads_for_recall(
        &self,
        scope: &AgentScope,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<thinclaw_agent::ports::ThreadSummary>, DatabaseError> {
        let summaries = self
            .store
            .list_actor_conversations_for_recall(
                &scope.principal_id,
                &scope.actor_id,
                include_group_history,
                limit,
            )
            .await?;
        Ok(summaries
            .into_iter()
            .map(thread_summary_from_history)
            .collect())
    }
}

fn thread_message_from_history(
    conversation_id: Uuid,
    message: crate::history::ConversationMessage,
) -> ThreadMessage {
    ThreadMessage {
        id: message.id,
        conversation_id,
        role: message.role,
        content: message.content,
        actor_id: message.actor_id,
        actor_display_name: message.actor_display_name,
        raw_sender_id: message.raw_sender_id,
        metadata: message.metadata,
        created_at: message.created_at,
    }
}

fn thread_summary_from_history(
    summary: crate::history::ConversationSummary,
) -> thinclaw_agent::ports::ThreadSummary {
    thinclaw_agent::ports::ThreadSummary {
        id: summary.id,
        user_id: summary.user_id,
        channel: summary.channel,
        thread_id: summary.stable_external_conversation_key,
        title: summary.title.clone(),
        preview: summary.title,
        message_count: summary.message_count,
        updated_at: summary.last_activity,
        metadata: serde_json::json!({
            "conversation_kind": summary.conversation_kind.as_str(),
            "conversation_scope_id": summary.conversation_scope_id.map(|id| id.to_string()),
            "actor_id": summary.actor_id,
            "thread_type": summary.thread_type,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{ConversationKind, ConversationSummary};

    #[test]
    fn thread_summary_adapter_preserves_recall_fields() {
        let id = Uuid::new_v4();
        let scope_id = Uuid::new_v4();
        let now = Utc::now();
        let summary = thread_summary_from_history(ConversationSummary {
            id,
            user_id: "user-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            conversation_scope_id: Some(scope_id),
            conversation_kind: ConversationKind::Direct,
            channel: "web".to_string(),
            title: Some("hello world".to_string()),
            message_count: 3,
            started_at: now,
            last_activity: now,
            thread_type: Some("assistant".to_string()),
            handoff: None,
            stable_external_conversation_key: Some("thread-1".to_string()),
        });

        assert_eq!(summary.id, id);
        assert_eq!(summary.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(summary.preview.as_deref(), Some("hello world"));
        assert_eq!(summary.metadata["conversation_kind"], "direct");
        assert_eq!(
            summary.metadata["conversation_scope_id"],
            scope_id.to_string()
        );
    }
}

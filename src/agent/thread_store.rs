//! Root database adapter for the extracted agent thread-store port.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thinclaw_agent::ports::{AgentScope, ThreadMessage, ThreadRuntimeSnapshot, ThreadStorePort};
use thinclaw_agent::thread_records::{thread_message_from_history, thread_summary_from_history};
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

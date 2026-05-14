use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use thinclaw_channels_core::IncomingMessage;
use uuid::Uuid;

use crate::run_artifact::AgentRunArtifact;
use crate::run_driver::{AgentRunDriver, RunThreadRuntimeLookup};
use crate::session::{Session, Turn};

#[async_trait]
pub trait RunMemorySyncObserver: Send + Sync {
    async fn after_turn_sync(&self, user_id: &str, artifact: &AgentRunArtifact);
}

#[derive(Clone, Default)]
pub struct AgentRunHarness {
    driver: AgentRunDriver,
    runtime_lookup: Option<Arc<dyn RunThreadRuntimeLookup>>,
    memory_sync: Option<Arc<dyn RunMemorySyncObserver>>,
}

impl AgentRunHarness {
    pub fn new() -> Self {
        Self {
            driver: AgentRunDriver::new(),
            runtime_lookup: None,
            memory_sync: None,
        }
    }

    pub fn with_driver(driver: AgentRunDriver) -> Self {
        Self {
            driver,
            runtime_lookup: None,
            memory_sync: None,
        }
    }

    pub fn with_runtime_lookup(mut self, lookup: Option<Arc<dyn RunThreadRuntimeLookup>>) -> Self {
        self.runtime_lookup = lookup;
        self
    }

    pub fn with_memory_sync(mut self, observer: Option<Arc<dyn RunMemorySyncObserver>>) -> Self {
        self.memory_sync = observer;
        self
    }

    pub async fn append_artifact(&self, artifact: &AgentRunArtifact) -> anyhow::Result<()> {
        self.driver
            .append_artifact(artifact)
            .await
            .context("failed to append canonical run artifact")?;

        if let (Some(observer), Some(user_id)) =
            (self.memory_sync.as_ref(), artifact.user_id.as_deref())
        {
            observer.after_turn_sync(user_id, artifact).await;
        }

        Ok(())
    }

    pub async fn record_chat_turn(
        &self,
        llm_provider: &str,
        llm_model: &str,
        session: &Session,
        thread_id: Uuid,
        incoming: &IncomingMessage,
        turn: &Turn,
    ) -> anyhow::Result<AgentRunArtifact> {
        let artifact = self
            .driver
            .record_chat_turn(
                llm_provider,
                llm_model,
                self.runtime_lookup.clone(),
                session,
                thread_id,
                incoming,
                turn,
            )
            .await?;

        if let (Some(observer), Some(user_id)) =
            (self.memory_sync.as_ref(), artifact.user_id.as_deref())
        {
            observer.after_turn_sync(user_id, &artifact).await;
        }

        Ok(artifact)
    }
}

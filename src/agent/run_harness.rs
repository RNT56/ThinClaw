use std::sync::Arc;

use anyhow::Context;
use uuid::Uuid;

use crate::agent::run_artifact::AgentRunArtifact;
use crate::agent::run_driver::AgentRunDriver;

#[derive(Clone, Default)]
pub struct AgentRunHarness {
    driver: AgentRunDriver,
    store: Option<Arc<dyn crate::db::Database>>,
}

impl AgentRunHarness {
    pub fn new(store: Option<Arc<dyn crate::db::Database>>) -> Self {
        Self {
            driver: AgentRunDriver::new(),
            store,
        }
    }

    pub fn with_driver(
        driver: AgentRunDriver,
        store: Option<Arc<dyn crate::db::Database>>,
    ) -> Self {
        Self { driver, store }
    }

    pub async fn append_artifact(&self, artifact: &AgentRunArtifact) -> anyhow::Result<()> {
        self.driver
            .append_artifact(artifact)
            .await
            .context("failed to append canonical run artifact")?;

        if let (Some(store), Some(user_id)) = (self.store.as_ref(), artifact.user_id.as_deref()) {
            let manager = crate::agent::learning::MemoryProviderManager::new(Arc::clone(store));
            manager.after_turn_sync(user_id, artifact).await;
        }

        Ok(())
    }

    pub async fn record_chat_turn(
        &self,
        llm_provider: &str,
        llm_model: &str,
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> anyhow::Result<AgentRunArtifact> {
        let artifact = self
            .driver
            .record_chat_turn(
                llm_provider,
                llm_model,
                self.store.clone(),
                session,
                thread_id,
                incoming,
                turn,
            )
            .await?;

        if let (Some(store), Some(user_id)) = (self.store.as_ref(), artifact.user_id.as_deref()) {
            let manager = crate::agent::learning::MemoryProviderManager::new(Arc::clone(store));
            manager.after_turn_sync(user_id, &artifact).await;
        }

        Ok(artifact)
    }
}

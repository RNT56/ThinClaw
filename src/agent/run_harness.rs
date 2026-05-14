//! Agent run harness compatibility adapter.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

pub use thinclaw_agent::run_harness::RunMemorySyncObserver;

use crate::agent::run_artifact::AgentRunArtifact;
use crate::agent::run_driver::{AgentRunDriver, RootRunThreadRuntimeLookup};

pub struct RootRunMemorySyncObserver {
    store: Arc<dyn crate::db::Database>,
}

impl RootRunMemorySyncObserver {
    pub fn shared(store: Arc<dyn crate::db::Database>) -> Arc<dyn RunMemorySyncObserver> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl RunMemorySyncObserver for RootRunMemorySyncObserver {
    async fn after_turn_sync(&self, user_id: &str, artifact: &AgentRunArtifact) {
        let manager = crate::agent::learning::MemoryProviderManager::new(Arc::clone(&self.store));
        manager.after_turn_sync(user_id, artifact).await;
    }
}

#[derive(Clone, Default)]
pub struct AgentRunHarness {
    inner: thinclaw_agent::run_harness::AgentRunHarness,
}

impl AgentRunHarness {
    pub fn new(store: Option<Arc<dyn crate::db::Database>>) -> Self {
        Self {
            inner: build_inner(thinclaw_agent::run_harness::AgentRunHarness::new(), store),
        }
    }

    pub fn with_driver(
        driver: AgentRunDriver,
        store: Option<Arc<dyn crate::db::Database>>,
    ) -> Self {
        Self {
            inner: build_inner(
                thinclaw_agent::run_harness::AgentRunHarness::with_driver(driver),
                store,
            ),
        }
    }

    pub async fn append_artifact(&self, artifact: &AgentRunArtifact) -> anyhow::Result<()> {
        self.inner.append_artifact(artifact).await
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
        self.inner
            .record_chat_turn(llm_provider, llm_model, session, thread_id, incoming, turn)
            .await
    }
}

fn build_inner(
    inner: thinclaw_agent::run_harness::AgentRunHarness,
    store: Option<Arc<dyn crate::db::Database>>,
) -> thinclaw_agent::run_harness::AgentRunHarness {
    let runtime_lookup = store
        .as_ref()
        .map(|store| RootRunThreadRuntimeLookup::shared(Arc::clone(store)));
    let memory_sync = store.map(RootRunMemorySyncObserver::shared);
    inner
        .with_runtime_lookup(runtime_lookup)
        .with_memory_sync(memory_sync)
}

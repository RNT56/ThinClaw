//! Agent run harness compatibility adapter.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

pub use thinclaw_agent::run_harness::RunMemorySyncObserver;

use crate::agent::run_artifact::AgentRunArtifact;
use crate::agent::run_driver::{AgentRunDriver, RootRunThreadRuntimeLookup};

pub struct RootRunMemorySyncObserver {
    /// Shared provider manager (pooled HTTP client + per-user readiness
    /// cache). Built once by the caller and reused across every turn instead
    /// of constructing a fresh `MemoryProviderManager` (and its 8 provider
    /// adapters) per `after_turn_sync` call.
    manager: Arc<crate::agent::learning::MemoryProviderManager>,
}

impl RootRunMemorySyncObserver {
    /// Build an observer backed by a freshly constructed provider manager.
    ///
    /// Kept for callers that don't already have a shared manager handy; the
    /// `with_manager` constructor should be preferred wherever one exists.
    pub fn shared(store: Arc<dyn crate::db::Database>) -> Arc<dyn RunMemorySyncObserver> {
        Self::with_manager(Arc::new(
            crate::agent::learning::MemoryProviderManager::new(store),
        ))
    }

    /// Build an observer that reuses an existing, already-warmed-up
    /// `MemoryProviderManager` so its readiness cache and pooled HTTP client
    /// stay effective across every recorded turn.
    pub fn with_manager(
        manager: Arc<crate::agent::learning::MemoryProviderManager>,
    ) -> Arc<dyn RunMemorySyncObserver> {
        Arc::new(Self { manager })
    }
}

#[async_trait]
impl RunMemorySyncObserver for RootRunMemorySyncObserver {
    async fn after_turn_sync(&self, user_id: &str, artifact: &AgentRunArtifact) {
        self.manager.after_turn_sync(user_id, artifact).await;
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

    /// Same as [`Self::with_driver`], but reuses an existing, already-warmed
    /// [`crate::agent::learning::MemoryProviderManager`] for the memory-sync
    /// observer instead of constructing a fresh one. Prefer this whenever the
    /// caller already holds a shared manager (for example, the agent loop's
    /// per-`Agent` shared learning orchestrator) so pooled HTTP connections
    /// and the readiness cache stay effective across turns.
    pub fn with_driver_and_memory_manager(
        driver: AgentRunDriver,
        store: Option<Arc<dyn crate::db::Database>>,
        memory_manager: Option<Arc<crate::agent::learning::MemoryProviderManager>>,
    ) -> Self {
        let runtime_lookup = store
            .as_ref()
            .map(|store| RootRunThreadRuntimeLookup::shared(Arc::clone(store)));
        let memory_sync = memory_manager.map(RootRunMemorySyncObserver::with_manager);
        Self {
            inner: thinclaw_agent::run_harness::AgentRunHarness::with_driver(driver)
                .with_runtime_lookup(runtime_lookup)
                .with_memory_sync(memory_sync),
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

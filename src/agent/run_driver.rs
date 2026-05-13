//! Agent run driver compatibility facade.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

pub use thinclaw_agent::run_driver::{AgentRunDriver, RunThreadRuntimeLookup};

pub struct RootRunThreadRuntimeLookup {
    store: Arc<dyn crate::db::Database>,
}

impl RootRunThreadRuntimeLookup {
    pub fn shared(store: Arc<dyn crate::db::Database>) -> Arc<dyn RunThreadRuntimeLookup> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl RunThreadRuntimeLookup for RootRunThreadRuntimeLookup {
    async fn load_thread_runtime(
        &self,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<thinclaw_agent::ports::ThreadRuntimeSnapshot>> {
        let runtime = crate::agent::load_thread_runtime(&self.store, thread_id).await?;
        Ok(
            runtime.map(|runtime| thinclaw_agent::ports::ThreadRuntimeSnapshot {
                prompt_snapshot_hash: runtime.prompt_snapshot_hash,
                ephemeral_overlay_hash: runtime.ephemeral_overlay_hash,
                provider_context_refs: runtime.provider_context_refs,
                ..Default::default()
            }),
        )
    }
}

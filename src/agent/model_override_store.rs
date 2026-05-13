//! Root adapter for agent-owned model override state.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{ModelOverride, ModelOverridePort, ModelOverrideScope};

use crate::error::DatabaseError;
use crate::tools::builtin::llm_tools::ModelOverride as ToolModelOverride;
use crate::tools::builtin::{SharedModelOverride, new_shared_model_override};

pub struct RootModelOverridePort {
    store: SharedModelOverride,
}

impl RootModelOverridePort {
    pub fn new(store: SharedModelOverride) -> Self {
        Self { store }
    }

    pub fn shared(store: SharedModelOverride) -> Arc<dyn ModelOverridePort> {
        Arc::new(Self::new(store))
    }

    pub fn shared_empty() -> (SharedModelOverride, Arc<dyn ModelOverridePort>) {
        let store = new_shared_model_override();
        let port = Self::shared(Arc::clone(&store));
        (store, port)
    }
}

#[async_trait]
impl ModelOverridePort for RootModelOverridePort {
    async fn get_model_override(
        &self,
        scope: &ModelOverrideScope,
    ) -> Result<Option<ModelOverride>, DatabaseError> {
        Ok(self
            .store
            .get(&scope.to_string())
            .await
            .map(model_from_tool))
    }

    async fn set_model_override(
        &self,
        scope: &ModelOverrideScope,
        value: ModelOverride,
    ) -> Result<(), DatabaseError> {
        self.store
            .set(scope.to_string(), tool_from_model(value))
            .await;
        Ok(())
    }

    async fn clear_model_override(&self, scope: &ModelOverrideScope) -> Result<(), DatabaseError> {
        self.store.clear(&scope.to_string()).await;
        Ok(())
    }
}

fn tool_from_model(value: ModelOverride) -> ToolModelOverride {
    ToolModelOverride {
        model_spec: value.model_spec,
        reason: value.reason,
    }
}

fn model_from_tool(value: ToolModelOverride) -> ModelOverride {
    ModelOverride {
        model_spec: value.model_spec,
        reason: value.reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn model_override_port_delegates_to_shared_store() {
        let (store, port) = RootModelOverridePort::shared_empty();
        let scope = ModelOverrideScope::Thread(Uuid::new_v4());

        port.set_model_override(
            &scope,
            ModelOverride {
                model_spec: "openai/gpt-4o".to_string(),
                reason: Some("vision turn".to_string()),
            },
        )
        .await
        .expect("set override");

        let stored = store
            .get(&scope.to_string())
            .await
            .expect("tool store override");
        assert_eq!(stored.model_spec, "openai/gpt-4o");

        let loaded = port
            .get_model_override(&scope)
            .await
            .expect("get override")
            .expect("override exists");
        assert_eq!(loaded.reason.as_deref(), Some("vision turn"));

        port.clear_model_override(&scope)
            .await
            .expect("clear override");
        assert!(store.get(&scope.to_string()).await.is_none());
    }
}

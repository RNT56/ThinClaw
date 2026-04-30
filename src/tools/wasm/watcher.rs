//! Compatibility facade and app registry adapter for the extracted WASM watcher.

use std::sync::Arc;

use async_trait::async_trait;

use crate::tools::registry::ToolRegistry;
use crate::tools::tool::Tool;

pub use thinclaw_tools::wasm::watcher::ToolWatcherConfig;

pub type ToolWatcher = thinclaw_tools::wasm::ToolWatcher<ToolRegistry>;

#[async_trait]
impl thinclaw_tools::wasm::RegistryUnregister for ToolRegistry {
    async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        ToolRegistry::unregister(self, name).await
    }
}

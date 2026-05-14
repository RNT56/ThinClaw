//! Compatibility facade and app registry adapter for the extracted WASM loader.

use std::sync::Arc;

use async_trait::async_trait;

use crate::secrets::SecretsStore;
use crate::tools::execution::HostMediatedToolInvoker;
use crate::tools::registry;
use crate::tools::registry::ToolRegistry;

pub use thinclaw_tools::wasm::loader::{
    DiscoveredTool, LoadResults, WasmLoadError, discover_dev_tools, discover_tools, load_dev_tools,
    resolve_wasm_target_dir, wasm_artifact_path,
};

pub type WasmToolLoader = thinclaw_tools::wasm::WasmToolLoader<ToolRegistry>;

#[async_trait]
impl thinclaw_tools::wasm::WasmToolRegistrar for ToolRegistry {
    type SecretResolver = dyn SecretsStore + Send + Sync;
    type ToolInvoker = HostMediatedToolInvoker;
    type Error = String;

    async fn register_wasm(
        &self,
        reg: thinclaw_tools::wasm::WasmToolRegistration<
            '_,
            Self::SecretResolver,
            Self::ToolInvoker,
        >,
    ) -> Result<(), Self::Error> {
        ToolRegistry::register_wasm(
            self,
            registry::WasmToolRegistration {
                name: reg.name,
                wasm_bytes: reg.wasm_bytes,
                runtime: reg.runtime,
                capabilities: reg.capabilities.into(),
                limits: reg.limits,
                description: reg.description,
                schema: reg.schema,
                secrets_store: reg.secrets,
                oauth_refresh: reg.oauth_refresh,
                tool_invoker: reg.tool_invoker,
            },
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn register_wasm_from_storage(
        &self,
        store: &dyn thinclaw_tools::wasm::WasmToolStore,
        runtime: &Arc<thinclaw_tools::wasm::WasmToolRuntime>,
        user_id: &str,
        name: &str,
        tool_invoker: Option<Arc<Self::ToolInvoker>>,
    ) -> Result<(), Self::Error> {
        ToolRegistry::register_wasm_from_storage(self, store, runtime, user_id, name, tool_invoker)
            .await
            .map_err(|error| error.to_string())
    }
}

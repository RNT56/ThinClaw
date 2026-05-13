//! Compatibility adapter for extracted extension-management tools.

use async_trait::async_trait;

use crate::extensions::manager::AuthRequestContext;
use crate::extensions::{ExtensionKind, ExtensionManager};

pub use thinclaw_tools::builtin::extension_tools::{
    ExtensionManagementPort, ToolActivateTool, ToolAuthRequestContext, ToolAuthTool,
    ToolExtensionKind, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};

fn tool_kind_to_root_kind(kind: ToolExtensionKind) -> ExtensionKind {
    match kind {
        ToolExtensionKind::McpServer => ExtensionKind::McpServer,
        ToolExtensionKind::WasmTool => ExtensionKind::WasmTool,
        ToolExtensionKind::WasmChannel => ExtensionKind::WasmChannel,
    }
}

fn auth_context_to_root_context(context: ToolAuthRequestContext) -> AuthRequestContext {
    AuthRequestContext {
        callback_base_url: context.callback_base_url,
        callback_type: context.callback_type,
        thread_id: context.thread_id,
    }
}

#[async_trait]
impl ExtensionManagementPort for ExtensionManager {
    async fn search(&self, query: &str, discover: bool) -> Result<Vec<serde_json::Value>, String> {
        ExtensionManager::search(self, query, discover)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
            .collect()
    }

    async fn install(
        &self,
        name: &str,
        url: Option<&str>,
        kind_hint: Option<ToolExtensionKind>,
    ) -> Result<serde_json::Value, String> {
        ExtensionManager::install(self, name, url, kind_hint.map(tool_kind_to_root_kind))
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
    }

    async fn auth_with_context(
        &self,
        name: &str,
        context: ToolAuthRequestContext,
    ) -> Result<serde_json::Value, String> {
        ExtensionManager::auth_with_context(self, name, None, auth_context_to_root_context(context))
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
    }

    async fn activate(&self, name: &str) -> Result<serde_json::Value, String> {
        ExtensionManager::activate(self, name)
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
    }

    async fn list(
        &self,
        kind_filter: Option<ToolExtensionKind>,
        include_available: bool,
    ) -> Result<Vec<serde_json::Value>, String> {
        ExtensionManager::list(
            self,
            kind_filter.map(tool_kind_to_root_kind),
            include_available,
        )
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
        .collect()
    }

    async fn remove(&self, name: &str) -> Result<String, String> {
        ExtensionManager::remove(self, name)
            .await
            .map_err(|error| error.to_string())
    }
}

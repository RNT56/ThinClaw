//! Agent-callable tools for managing extensions (MCP servers and WASM tools).
//!
//! These six tools let the LLM search, install, authenticate, activate, list,
//! and remove extensions entirely through conversation.

#![allow(clippy::items_after_test_module)]

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use thinclaw_types::JobContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExtensionKind {
    McpServer,
    WasmTool,
    WasmChannel,
}

#[derive(Debug, Clone, Default)]
pub struct ToolAuthRequestContext {
    pub callback_base_url: Option<String>,
    pub callback_type: Option<String>,
    pub thread_id: Option<String>,
}

#[async_trait]
pub trait ExtensionManagementPort: Send + Sync {
    async fn search(&self, query: &str, discover: bool) -> Result<Vec<serde_json::Value>, String>;
    async fn install(
        &self,
        name: &str,
        url: Option<&str>,
        kind_hint: Option<ToolExtensionKind>,
    ) -> Result<serde_json::Value, String>;
    async fn auth_with_context(
        &self,
        name: &str,
        context: ToolAuthRequestContext,
    ) -> Result<serde_json::Value, String>;
    async fn activate(&self, name: &str) -> Result<serde_json::Value, String>;
    async fn list(
        &self,
        kind_filter: Option<ToolExtensionKind>,
        include_available: bool,
    ) -> Result<Vec<serde_json::Value>, String>;
    async fn remove(&self, name: &str) -> Result<String, String>;
}

fn auth_request_context_from_job(ctx: &JobContext) -> ToolAuthRequestContext {
    let browser_origin = ctx
        .metadata
        .get("browser_origin")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let thread_id = ctx
        .metadata
        .get("thread_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let channel = ctx.metadata.get("channel").and_then(|value| value.as_str());
    let callback_type = if browser_origin.is_some() || matches!(channel, Some("gateway")) {
        Some("web".to_string())
    } else {
        None
    };

    ToolAuthRequestContext {
        callback_base_url: browser_origin,
        callback_type,
        thread_id,
    }
}

fn parse_extension_kind(value: &str) -> Option<ToolExtensionKind> {
    match value {
        "mcp_server" => Some(ToolExtensionKind::McpServer),
        "wasm_tool" => Some(ToolExtensionKind::WasmTool),
        "wasm_channel" => Some(ToolExtensionKind::WasmChannel),
        _ => None,
    }
}

// ── tool_search ──────────────────────────────────────────────────────────

pub struct ToolSearchTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolSearchTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search for available extensions to add new capabilities. Extensions include \
         channels (Telegram, Slack, Discord — for messaging), tools, and MCP servers. \
         Use discover:true to search online if the built-in registry has no results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (name, keyword, or description fragment)"
                },
                "discover": {
                    "type": "boolean",
                    "description": "If true, also search online (slower, 5-15s). Try without first.",
                    "default": false
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let discover = params
            .get("discover")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let results = self
            .manager
            .search(query, discover)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "results": results,
            "count": results.len(),
            "searched_online": discover,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── tool_install ─────────────────────────────────────────────────────────

pub struct ToolInstallTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolInstallTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolInstallTool {
    fn name(&self) -> &str {
        "tool_install"
    }

    fn description(&self) -> &str {
        "Install an extension (channel, tool, or MCP server). \
         Use the name from tool_search results, or provide an explicit URL."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name (from search results or custom)"
                },
                "url": {
                    "type": "string",
                    "description": "Explicit URL (for extensions not in the registry)"
                },
                "kind": {
                    "type": "string",
                    "enum": ["mcp_server", "wasm_tool", "wasm_channel"],
                    "description": "Extension type (auto-detected if omitted)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let url = params.get("url").and_then(|v| v.as_str());

        let kind_hint = params
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(parse_extension_kind);

        let result = self
            .manager
            .install(name, url, kind_hint)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubExtensionPort;

    #[async_trait]
    impl ExtensionManagementPort for StubExtensionPort {
        async fn search(
            &self,
            _query: &str,
            _discover: bool,
        ) -> Result<Vec<serde_json::Value>, String> {
            Ok(Vec::new())
        }

        async fn install(
            &self,
            _name: &str,
            _url: Option<&str>,
            _kind_hint: Option<ToolExtensionKind>,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn auth_with_context(
            &self,
            _name: &str,
            _context: ToolAuthRequestContext,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn activate(&self, _name: &str) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }

        async fn list(
            &self,
            _kind_filter: Option<ToolExtensionKind>,
            _include_available: bool,
        ) -> Result<Vec<serde_json::Value>, String> {
            Ok(Vec::new())
        }

        async fn remove(&self, _name: &str) -> Result<String, String> {
            Ok("removed".to_string())
        }
    }

    fn test_port() -> Arc<dyn ExtensionManagementPort> {
        Arc::new(StubExtensionPort)
    }

    #[test]
    fn auth_request_context_from_gateway_job_metadata() {
        let mut ctx = JobContext::with_user("test-user", "chat", "auth");
        ctx.metadata = serde_json::json!({
            "channel": "gateway",
            "thread_id": "thread-123",
            "browser_origin": "https://chat.example.com",
        });

        let auth_context = auth_request_context_from_job(&ctx);
        assert_eq!(
            auth_context.callback_base_url.as_deref(),
            Some("https://chat.example.com")
        );
        assert_eq!(auth_context.callback_type.as_deref(), Some("web"));
        assert_eq!(auth_context.thread_id.as_deref(), Some("thread-123"));
    }

    #[test]
    fn auth_request_context_without_gateway_metadata() {
        let mut ctx = JobContext::with_user("test-user", "chat", "auth");
        ctx.metadata = serde_json::json!({
            "channel": "repl",
            "thread_id": "thread-123",
        });

        let auth_context = auth_request_context_from_job(&ctx);
        assert_eq!(auth_context.callback_base_url, None);
        assert_eq!(auth_context.callback_type, None);
        assert_eq!(auth_context.thread_id.as_deref(), Some("thread-123"));
    }

    #[test]
    fn tool_search_schema() {
        let tool = ToolSearchTool::new(test_port());
        assert_eq!(tool.name(), "tool_search");
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("query").is_some());
    }

    #[test]
    fn tool_install_schema() {
        let tool = ToolInstallTool::new(test_port());
        assert_eq!(tool.name(), "tool_install");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("url").is_some());
    }

    #[test]
    fn tool_auth_schema_does_not_accept_tokens() {
        let tool = ToolAuthTool::new(test_port());
        assert_eq!(tool.name(), "tool_auth");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("token").is_none());
    }

    #[test]
    fn tool_activate_list_and_remove_schemas() {
        let activate = ToolActivateTool::new(test_port());
        assert_eq!(activate.name(), "tool_activate");
        assert_eq!(
            activate.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );

        let list = ToolListTool::new(test_port());
        assert_eq!(list.name(), "tool_list");
        assert!(list.parameters_schema()["properties"].get("kind").is_some());

        let remove = ToolRemoveTool::new(test_port());
        assert_eq!(remove.name(), "tool_remove");
        assert_eq!(
            remove.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}

// ── tool_auth ────────────────────────────────────────────────────────────

pub struct ToolAuthTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolAuthTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolAuthTool {
    fn name(&self) -> &str {
        "tool_auth"
    }

    fn description(&self) -> &str {
        "Initiate authentication for an extension. For OAuth, returns a URL. \
         For manual auth, returns instructions. The user provides their token \
         through a secure channel, never through this tool."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to authenticate"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;
        let auth_context = auth_request_context_from_job(ctx);

        let result = self
            .manager
            .auth_with_context(name, auth_context)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        // Auto-activate after successful auth so tools are available immediately
        if result.get("status").and_then(|value| value.as_str()) == Some("authenticated") {
            match self.manager.activate(name).await {
                Ok(activate_result) => {
                    let output = serde_json::json!({
                        "status": "authenticated_and_activated",
                        "name": name,
                        "tools_loaded": activate_result.get("tools_loaded").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "message": activate_result.get("message").cloned().unwrap_or(serde_json::Value::Null),
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }
                Err(e) => {
                    tracing::warn!(
                        "Extension '{}' authenticated but activation failed: {}",
                        name,
                        e
                    );
                    let output = serde_json::json!({
                        "status": "authenticated",
                        "name": name,
                        "activation_error": e.to_string(),
                        "message": format!(
                            "Authenticated but activation failed: {}. Try tool_activate.",
                            e
                        ),
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }
            }
        }

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── tool_activate ────────────────────────────────────────────────────────

pub struct ToolActivateTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolActivateTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolActivateTool {
    fn name(&self) -> &str {
        "tool_activate"
    }

    fn description(&self) -> &str {
        "Activate an installed extension — starts channels, loads tools, or connects to MCP servers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to activate"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;
        let auth_context = auth_request_context_from_job(ctx);

        match self.manager.activate(name).await {
            Ok(result) => Ok(ToolOutput::success(result, start.elapsed())),
            Err(activate_err) => {
                let err_str = activate_err;
                let needs_auth = err_str.contains("authentication")
                    || err_str.contains("401")
                    || err_str.contains("Unauthorized")
                    || err_str.contains("not authenticated");

                if !needs_auth {
                    return Err(ToolError::ExecutionFailed(err_str));
                }

                // Activation failed due to missing auth; initiate auth flow
                // so the agent loop can show the auth card.
                match self.manager.auth_with_context(name, auth_context).await {
                    Ok(auth_result)
                        if auth_result.get("status").and_then(|value| value.as_str())
                            == Some("authenticated") =>
                    {
                        // Auth succeeded (e.g. env var was set); retry activation.
                        let result = self
                            .manager
                            .activate(name)
                            .await
                            .map_err(ToolError::ExecutionFailed)?;
                        Ok(ToolOutput::success(result, start.elapsed()))
                    }
                    Ok(auth_result) => {
                        // Auth needs user input (awaiting_token). Return the auth
                        // result so detect_auth_awaiting picks it up.
                        Ok(ToolOutput::success(auth_result, start.elapsed()))
                    }
                    Err(auth_err) => Err(ToolError::ExecutionFailed(format!(
                        "Activation failed ({}), and authentication also failed: {}",
                        err_str, auth_err
                    ))),
                }
            }
        }
    }
}

// ── tool_list ────────────────────────────────────────────────────────────

pub struct ToolListTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolListTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolListTool {
    fn name(&self) -> &str {
        "tool_list"
    }

    fn description(&self) -> &str {
        "List extensions with their authentication and activation status. \
         Set include_available:true to also show registry entries not yet installed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["mcp_server", "wasm_tool", "wasm_channel"],
                    "description": "Filter by extension type (omit to list all)"
                },
                "include_available": {
                    "type": "boolean",
                    "description": "If true, also include registry entries that are not yet installed",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let kind_filter = params
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(parse_extension_kind);

        let include_available = params
            .get("include_available")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let extensions = self
            .manager
            .list(kind_filter, include_available)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "extensions": extensions,
            "count": extensions.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── tool_remove ──────────────────────────────────────────────────────────

pub struct ToolRemoveTool {
    manager: Arc<dyn ExtensionManagementPort>,
}

impl ToolRemoveTool {
    pub fn new(manager: Arc<dyn ExtensionManagementPort>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolRemoveTool {
    fn name(&self) -> &str {
        "tool_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed extension (channel, tool, or MCP server). \
         Unregisters tools and deletes configuration."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to remove"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let message = self
            .manager
            .remove(name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "name": name,
            "message": message,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

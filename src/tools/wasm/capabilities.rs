//! Compatibility facade for WASM capabilities.

use std::collections::HashMap;

pub use thinclaw_tools::wasm::capabilities::{
    EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability, ToolInvokeCapability,
    WorkspaceCapability, WorkspaceReader,
};

/// Root-local compatibility shell around extracted capability DTOs.
///
/// This remains local so existing root/channel conversion impls keep satisfying
/// Rust's orphan rules while the field types and runtime representation live in
/// `thinclaw-tools`.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub workspace_read: Option<WorkspaceCapability>,
    pub http: Option<HttpCapability>,
    pub tool_invoke: Option<ToolInvokeCapability>,
    pub secrets: Option<SecretsCapability>,
}

impl Capabilities {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_workspace_read(mut self, prefixes: Vec<String>) -> Self {
        self.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: prefixes,
            reader: None,
        });
        self
    }

    pub fn with_http(mut self, http: HttpCapability) -> Self {
        self.http = Some(http);
        self
    }

    pub fn with_tool_invoke(mut self, aliases: HashMap<String, String>) -> Self {
        self.tool_invoke = Some(ToolInvokeCapability {
            aliases,
            rate_limit: RateLimitConfig::default(),
        });
        self
    }

    pub fn with_secrets(mut self, allowed: Vec<String>) -> Self {
        self.secrets = Some(SecretsCapability {
            allowed_names: allowed,
        });
        self
    }
}

impl From<Capabilities> for thinclaw_tools::wasm::Capabilities {
    fn from(capabilities: Capabilities) -> Self {
        Self {
            workspace_read: capabilities.workspace_read,
            http: capabilities.http,
            tool_invoke: capabilities.tool_invoke,
            secrets: capabilities.secrets,
        }
    }
}

impl From<thinclaw_tools::wasm::Capabilities> for Capabilities {
    fn from(capabilities: thinclaw_tools::wasm::Capabilities) -> Self {
        Self {
            workspace_read: capabilities.workspace_read,
            http: capabilities.http,
            tool_invoke: capabilities.tool_invoke,
            secrets: capabilities.secrets,
        }
    }
}

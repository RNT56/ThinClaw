//! Compatibility facade for WASM capabilities schema parsing.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::tools::wasm::Capabilities;

#[allow(unused_imports)]
pub use thinclaw_tools::wasm::capabilities_schema::{
    AuthCapabilitySchema, CredentialLocationSchema, CredentialMappingSchema, EndpointPatternSchema,
    HttpCapabilitySchema, OAuthConfigSchema, RateLimitSchema, SecretsCapabilitySchema,
    ToolInvokeCapabilitySchema, ValidationEndpointSchema, WorkspaceCapabilitySchema,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesFile {
    #[serde(default)]
    pub http: Option<HttpCapabilitySchema>,
    #[serde(default)]
    pub secrets: Option<SecretsCapabilitySchema>,
    #[serde(default)]
    pub tool_invoke: Option<ToolInvokeCapabilitySchema>,
    #[serde(default)]
    pub workspace: Option<WorkspaceCapabilitySchema>,
    #[serde(default)]
    pub auth: Option<AuthCapabilitySchema>,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
}

impl CapabilitiesFile {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        thinclaw_tools::wasm::CapabilitiesFile::from_json(json).map(Self::from)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        thinclaw_tools::wasm::CapabilitiesFile::from_bytes(bytes).map(Self::from)
    }

    pub fn to_capabilities(&self) -> Capabilities {
        let extracted = thinclaw_tools::wasm::capabilities_schema::CapabilitiesFile {
            http: self.http.clone(),
            secrets: self.secrets.clone(),
            tool_invoke: self.tool_invoke.clone(),
            workspace: self.workspace.clone(),
            auth: self.auth.clone(),
            capabilities: None,
            config: self.config.clone(),
            hooks: self.hooks.clone(),
        };
        extracted.to_capabilities().into()
    }
}

impl From<&CapabilitiesFile> for thinclaw_tools::wasm::CapabilitiesFile {
    fn from(value: &CapabilitiesFile) -> Self {
        Self {
            http: value.http.clone(),
            secrets: value.secrets.clone(),
            tool_invoke: value.tool_invoke.clone(),
            workspace: value.workspace.clone(),
            auth: value.auth.clone(),
            capabilities: None,
            config: value.config.clone(),
            hooks: value.hooks.clone(),
        }
    }
}

impl From<thinclaw_tools::wasm::CapabilitiesFile> for CapabilitiesFile {
    fn from(value: thinclaw_tools::wasm::CapabilitiesFile) -> Self {
        Self {
            http: value.http,
            secrets: value.secrets,
            tool_invoke: value.tool_invoke,
            workspace: value.workspace,
            auth: value.auth,
            config: value.config,
            hooks: value.hooks,
        }
    }
}

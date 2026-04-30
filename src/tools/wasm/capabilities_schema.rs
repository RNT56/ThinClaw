//! Compatibility facade for WASM capabilities schema parsing.

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
}

impl CapabilitiesFile {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn to_capabilities(&self) -> Capabilities {
        let extracted = thinclaw_tools::wasm::capabilities_schema::CapabilitiesFile {
            http: self.http.clone(),
            secrets: self.secrets.clone(),
            tool_invoke: self.tool_invoke.clone(),
            workspace: self.workspace.clone(),
            auth: self.auth.clone(),
        };
        extracted.to_capabilities().into()
    }
}

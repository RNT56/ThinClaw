//! Compatibility facade for sandbox configuration.

pub use thinclaw_types::sandbox::{
    ResourceLimits, SandboxConfig, SandboxPolicy, default_allowlist,
};

/// Default credential mappings for common APIs.
pub fn default_credential_mappings() -> Vec<crate::secrets::CredentialMapping> {
    use crate::secrets::CredentialMapping;

    vec![
        CredentialMapping::bearer("OPENAI_API_KEY", "api.openai.com"),
        CredentialMapping::header("ANTHROPIC_API_KEY", "x-api-key", "api.anthropic.com"),
        CredentialMapping::bearer("NEARAI_API_KEY", "api.near.ai"),
    ]
}

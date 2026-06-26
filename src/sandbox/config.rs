//! Compatibility facade for sandbox configuration.

pub use thinclaw_types::sandbox::{
    ResourceLimits, SandboxConfig, SandboxPolicy, default_allowlist,
};

/// Default credential mappings for common APIs.
///
/// Credential injection only applies to the plaintext `http://` forward path in
/// the sandbox proxy; HTTPS hosts are reached via `CONNECT` and tunneled
/// opaquely, so `AllowWithCredentials` never injects anything for them. HTTPS
/// credential delivery is handled out-of-band by the orchestrator's
/// `/worker/{id}/credentials` endpoint instead. There are currently no
/// plaintext-`http://` hosts that require default credential injection, so this
/// returns an empty vec. Live callers in `NetworkProxyBuilder` still depend on
/// this function existing.
pub fn default_credential_mappings() -> Vec<crate::secrets::CredentialMapping> {
    Vec::new()
}

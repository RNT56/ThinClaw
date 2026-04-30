//! WASM OAuth compatibility facade.

pub use thinclaw_tools::wasm::oauth::{
    GOOGLE_OAUTH_TOKEN, LEGACY_GMAIL_OAUTH_TOKEN, OAuthPkcePair, OAuthRefreshConfig,
    ResolvedOAuthConfig, WasmOAuthTokenExchange, WasmToolAuthCheck, WasmToolAuthMode,
    WasmToolAuthStatus, WasmToolAuthorizationRequest, WasmToolOAuthError, WasmToolOAuthFlow,
    build_authorization_url, canonical_secret_name, is_google_secret_name, refresh_secret_name,
    scopes_secret_name, shared_auth_provider,
};

use crate::tools::wasm::CapabilitiesFile;

pub fn resolve_oauth_refresh_config(cap_file: &CapabilitiesFile) -> Option<OAuthRefreshConfig> {
    let extracted = thinclaw_tools::wasm::CapabilitiesFile::from(cap_file);
    thinclaw_tools::wasm::resolve_oauth_refresh_config(&extracted)
}

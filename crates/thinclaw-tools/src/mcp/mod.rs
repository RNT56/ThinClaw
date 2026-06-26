//! Root-independent Model Context Protocol helpers.

use std::net::SocketAddr;

use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned};

pub mod auth;
pub mod client;
pub mod config;
pub mod protocol;
pub mod session;
pub mod stdio;

/// Best-effort DNS-rebind pin for an outbound MCP/OAuth `url` (F-02).
///
/// Returns the URL host plus the socket addresses it resolved to at validation
/// time, for the caller to pass to [`reqwest::ClientBuilder::resolve_to_addrs`]
/// so the eventual connection targets an address that already passed validation
/// instead of re-resolving (and possibly rebinding to a private address) at
/// connect time.
///
/// Returns `None` when there is nothing to pin or validation fails — an
/// IP-literal host, or an operator-configured local server (loopback/private)
/// that the SSRF blocklist legitimately rejects — so the caller falls back to an
/// unpinned client with no behavior change for local MCP servers. `require_https`
/// is intentionally off here; the HTTPS policy for MCP URLs is enforced at config
/// time (`McpServerConfig::validate`, honoring `allow_local_http`).
pub(crate) fn pinned_addrs_for(url: &str) -> Option<(String, Vec<SocketAddr>)> {
    let options = OutboundUrlGuardOptions {
        require_https: false,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    match validate_outbound_url_pinned(url, &options) {
        Ok(guarded) if !guarded.pinned_addrs.is_empty() => {
            let host = guarded.url.host_str()?.to_string();
            Some((host, guarded.pinned_addrs))
        }
        _ => None,
    }
}

/// Build a `reqwest` client from `builder`, pinning `url`'s host to its
/// validated addresses when possible (F-02). On a build failure the `fallback`
/// closure supplies an unpinned client so callers never hard-fail on the pin.
pub(crate) fn build_pinned(
    builder: reqwest::ClientBuilder,
    url: &str,
    fallback: impl FnOnce() -> reqwest::Client,
) -> reqwest::Client {
    match pinned_addrs_for(url) {
        Some((host, addrs)) => builder
            .resolve_to_addrs(&host, &addrs)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, host, url, "failed to build pinned MCP/OAuth client; using unpinned client");
                fallback()
            }),
        None => builder.build().unwrap_or_else(|_| fallback()),
    }
}

pub use auth::{
    AccessToken, AuthError, AuthorizationServerMetadata, ClientRegistrationRequest,
    ClientRegistrationResponse, DEFAULT_OAUTH_CALLBACK_PORT, OAuthDiscoveryBundle,
    OAuthFlowOptions, PkceChallenge, ProtectedResourceMetadata, authorize_mcp_server,
    authorize_mcp_server_with_options, bind_callback_listener, build_authorization_url,
    discover_authorization_server, discover_full_oauth_metadata, discover_oauth_bundle,
    discover_oauth_endpoints, discover_protected_resource, exchange_code_for_token,
    find_available_port, get_access_token, is_authenticated, refresh_access_token, register_client,
    store_client_id, store_tokens,
};
pub use client::{McpClient, McpInteractionKind, McpPendingInteraction};
pub use config::{
    ConfigError, McpCapabilityPolicy, McpConfigProvider, McpConfigStore, McpLoggingLevel,
    McpRuntimeHealth, McpServerConfig, McpServersFile, McpTransport, OAuthConfig,
    load_mcp_servers_from, save_mcp_servers_to,
};
pub use protocol::{
    CallToolResult, CancelledNotification, ClientCapabilities, ClientElicitationCapability,
    ClientRootsCapability, ClientSamplingCapability, ClientSamplingToolsCapability,
    CompleteArgument, CompleteResult, ContentBlock, ElicitationCreateRequest, ExecutionTimeHint,
    GetPromptResult, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, LoggingMessageNotification, McpError, McpIcon,
    McpNotification, McpPrompt, McpPromptArgument, McpPromptMessage, McpRequest, McpResource,
    McpResourceContents, McpResourceTemplate, McpResponse, McpTool, McpToolAnnotations,
    McpToolExecution, McpTransportMessage, PROTOCOL_VERSION, ProgressNotification, PromptContent,
    ReadResourceResult, ResourceUpdatedNotification, SamplingCreateMessageRequest,
    SamplingCreateMessageResult, SamplingResponseContent,
};
pub use session::{McpSession, McpSessionManager};
pub use stdio::{McpInboundHandler, StdioTransport};

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn local_and_literal_urls_are_not_pinned() {
        // Loopback / private hosts are operator-trusted local MCP servers; the
        // SSRF blocklist rejects them, so pinning must fall back to None (the
        // caller then builds an unpinned client — no regression for local
        // servers). IP-literal hosts have nothing to pin. Invalid URLs → None.
        assert!(pinned_addrs_for("http://127.0.0.1:8080").is_none());
        assert!(pinned_addrs_for("http://localhost:8080/mcp").is_none());
        assert!(pinned_addrs_for("https://192.168.0.10").is_none());
        assert!(pinned_addrs_for("http://10.1.2.3:9000").is_none());
        assert!(pinned_addrs_for("not-a-url").is_none());
    }
}

//! Root-independent Model Context Protocol helpers.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use thinclaw_tools_core::{
    GuardedUrl, OutboundUrlGuardOptions, ToolError, is_public_outbound_ip,
    validate_outbound_url_pinned_async,
};

pub mod auth;
pub mod client;
pub mod config;
pub mod protocol;
pub mod session;
pub mod stdio;

type ResolvedAddressPin = Option<(String, Vec<SocketAddr>)>;
type GuardedMcpUrl = (GuardedUrl, ResolvedAddressPin);

/// Best-effort DNS-rebind pin for an outbound MCP/OAuth `url` (F-02).
///
/// Returns the URL host plus the socket addresses it resolved to at validation
/// time, for the caller to pass to [`reqwest::ClientBuilder::resolve_to_addrs`]
/// so the eventual connection targets an address that already passed validation
/// instead of re-resolving (and possibly rebinding to a private address) at
/// connect time.
///
/// Validation errors are never converted into an unpinned client. Explicit
/// local MCP endpoints are supported only when `allow_local` is true; hostnames
/// in that mode are restricted to the `.localhost` namespace and pinned to
/// loopback addresses.
pub(crate) async fn pinned_addrs_for(
    url: &str,
    allow_local: bool,
) -> Result<GuardedMcpUrl, ToolError> {
    let options = OutboundUrlGuardOptions {
        require_https: false,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    let guarded = match validate_outbound_url_pinned_async(url, &options).await {
        Ok(guarded) => guarded,
        Err(public_error) if allow_local => validate_local_url_pinned(url)
            .await
            .map_err(|_| public_error)?,
        Err(error) => return Err(error),
    };
    let pin = if guarded.pinned_addrs.is_empty() {
        None
    } else {
        let host = guarded.url.host_str().ok_or_else(|| {
            ToolError::InvalidParameters("MCP URL does not contain a host".to_string())
        })?;
        Some((host.to_string(), guarded.pinned_addrs.clone()))
    };
    Ok((guarded, pin))
}

async fn validate_local_url_pinned(url: &str) -> Result<GuardedUrl, ToolError> {
    let parsed = url::Url::parse(url)
        .map_err(|error| ToolError::InvalidParameters(format!("invalid MCP URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ToolError::NotAuthorized(
            "local MCP URLs must use HTTP(S) without embedded credentials".to_string(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("MCP URL is missing a host".to_string()))?;
    let port = parsed
        .port_or_known_default()
        .unwrap_or_else(|| if parsed.scheme() == "https" { 443 } else { 80 });
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_public_outbound_ip(ip) || !is_safe_local_mcp_ip(ip) {
            return Err(ToolError::NotAuthorized(
                "local MCP override requires a private or loopback address".to_string(),
            ));
        }
        return Ok(GuardedUrl {
            url: parsed,
            pinned_addrs: Vec::new(),
        });
    }
    let canonical_host = host.trim_end_matches('.').to_ascii_lowercase();
    if canonical_host != "localhost" && !canonical_host.ends_with(".localhost") {
        return Err(ToolError::NotAuthorized(
            "local MCP hostnames must use the .localhost namespace".to_string(),
        ));
    }
    let resolved = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| {
        ToolError::ExternalService(
            "local MCP hostname did not resolve within 5 seconds".to_string(),
        )
    })?
    .map_err(|error| {
        ToolError::ExternalService(format!("failed to resolve local MCP host: {error}"))
    })?;
    let mut addresses = Vec::new();
    for address in resolved {
        if addresses.len() >= 64 {
            return Err(ToolError::ExternalService(
                "local MCP hostname resolved to more than 64 addresses".to_string(),
            ));
        }
        addresses.push(address);
    }
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(ToolError::NotAuthorized(
            "local MCP hostname did not resolve exclusively to loopback".to_string(),
        ));
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(GuardedUrl {
        url: parsed,
        pinned_addrs: addresses,
    })
}

fn is_safe_local_mcp_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
    }
}

/// Build a redirect-disabled `reqwest` client pinned to the validated endpoint.
/// Both URL-validation and client-build errors fail closed.
pub(crate) async fn build_pinned(
    builder: reqwest::ClientBuilder,
    url: &str,
    allow_local: bool,
) -> Result<reqwest::Client, ToolError> {
    let (_, pin) = pinned_addrs_for(url, allow_local).await?;
    let mut builder = builder
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if let Some((host, addrs)) = pin {
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    builder.build().map_err(|error| {
        ToolError::ExternalService(format!("failed to build pinned MCP HTTP client: {error}"))
    })
}

pub use auth::{
    AccessToken, AuthError, AuthorizationServerMetadata, ClientRegistrationRequest,
    ClientRegistrationResponse, DEFAULT_OAUTH_CALLBACK_PORT, OAuthDiscoveryBundle,
    OAuthFlowOptions, PkceChallenge, PreparedMcpAuthorization, ProtectedResourceMetadata,
    authorize_mcp_server, authorize_mcp_server_with_options, bind_callback_listener,
    build_authorization_url, complete_mcp_authorization, discover_authorization_server,
    discover_full_oauth_metadata, discover_oauth_bundle, discover_oauth_endpoints,
    discover_protected_resource, exchange_code_for_token, find_available_port, get_access_token,
    is_authenticated, prepare_mcp_authorization, refresh_access_token, register_client,
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

    #[tokio::test]
    async fn local_and_literal_urls_are_not_pinned() {
        assert!(
            pinned_addrs_for("http://127.0.0.1:8080", false)
                .await
                .is_err()
        );
        assert!(
            pinned_addrs_for("http://127.0.0.1:8080", true)
                .await
                .is_ok()
        );
        assert!(
            pinned_addrs_for("http://localhost:8080/mcp", true)
                .await
                .is_ok()
        );
        assert!(pinned_addrs_for("https://192.168.0.10", true).await.is_ok());
        assert!(pinned_addrs_for("http://10.1.2.3:9000", true).await.is_ok());
        assert!(pinned_addrs_for("not-a-url", true).await.is_err());
        assert!(
            pinned_addrs_for("http://user:secret@localhost:8080", true)
                .await
                .is_err()
        );
    }
}

//! Compatibility facade for MCP OAuth authentication.
//!
//! Root-independent OAuth discovery, token exchange, and secret persistence
//! live in `thinclaw-tools`. This module supplies app-specific callback
//! binding, browser opening, and user guidance.

use std::sync::Arc;

use tokio::net::TcpListener;

use crate::cli::oauth_defaults::{self, OAUTH_CALLBACK_PORT};
use crate::secrets::SecretsStore;
use crate::tools::mcp::config::McpServerConfig;

pub use thinclaw_tools::mcp::auth::{
    AccessToken, AuthError, AuthorizationServerMetadata, ClientRegistrationRequest,
    ClientRegistrationResponse, DEFAULT_OAUTH_CALLBACK_PORT, OAuthDiscoveryBundle,
    OAuthFlowOptions, PkceChallenge, ProtectedResourceMetadata, bind_callback_listener,
    build_authorization_url, discover_authorization_server, discover_full_oauth_metadata,
    discover_oauth_bundle, discover_oauth_endpoints, discover_protected_resource,
    exchange_code_for_token, get_access_token, is_authenticated, refresh_access_token,
    register_client, store_client_id, store_tokens,
};

/// Perform the MCP OAuth flow with ThinClaw's CLI/browser callback behavior.
pub async fn authorize_mcp_server(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<AccessToken, AuthError> {
    let callback_host = oauth_defaults::callback_host();
    if oauth_defaults::ssh_or_headless_detected() {
        oauth_defaults::print_ssh_callback_hint();
    }

    thinclaw_tools::mcp::auth::authorize_mcp_server_with_options(
        server_config,
        secrets,
        user_id,
        OAuthFlowOptions {
            callback_host,
            callback_port: OAUTH_CALLBACK_PORT,
            success_html: Arc::new(|server_name| oauth_defaults::landing_html(server_name, true)),
            failure_html: Arc::new(|server_name| oauth_defaults::landing_html(server_name, false)),
            open_authorization_url: Arc::new(|auth_url| {
                open::that(auth_url).map_err(|error| error.to_string())
            }),
            on_manual_authorization_url: Arc::new(|auth_url| {
                oauth_defaults::print_ssh_callback_hint();
                println!("  Please open this URL manually:");
                println!("  {}", auth_url);
            }),
            on_remote_plain_http_callback: Arc::new(|host, port| {
                println!(
                    "Warning: MCP OAuth callback is using plain HTTP to a remote host ({host})."
                );
                println!("         Authorization codes will be transmitted unencrypted.");
                println!("         Consider SSH port forwarding instead:");
                println!("           ssh -L {port}:127.0.0.1:{port} user@{host}");
            }),
        },
    )
    .await
}

/// Bind the OAuth callback listener on ThinClaw's shared fixed port.
pub async fn find_available_port() -> Result<(TcpListener, u16), AuthError> {
    let listener = oauth_defaults::bind_callback_listener()
        .await
        .map_err(|_| AuthError::PortUnavailable)?;
    Ok((listener, OAUTH_CALLBACK_PORT))
}

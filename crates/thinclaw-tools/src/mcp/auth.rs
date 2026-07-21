//! OAuth 2.1 authentication for MCP servers.
//!
//! Implements the MCP Authorization specification using OAuth 2.1 with PKCE.
//! See: https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/authorization/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::StreamExt;
use rand::Rng;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use thinclaw_secrets::{CreateSecretParams, SecretAccessContext, SecretError, SecretsStore};

use crate::mcp::config::McpServerConfig;

/// Default OAuth callback port used by ThinClaw flows.
pub const DEFAULT_OAUTH_CALLBACK_PORT: u16 = 9876;
const MAX_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_OAUTH_ERROR_BYTES: usize = 16 * 1024;
const MAX_AUTHORIZATION_URL_BYTES: usize = 16 * 1024;
const MAX_OAUTH_PARAMETER_BYTES: usize = 8 * 1024;
const MAX_OAUTH_EXTRA_PARAMETERS: usize = 32;
const MAX_OAUTH_SECRET_BYTES: usize = 64 * 1024;
const MAX_OAUTH_TOKEN_LIFETIME_SECS: u64 = 365 * 24 * 60 * 60;
const MAX_OAUTH_METADATA_ITEMS: usize = 256;
const MAX_OAUTH_CALLBACK_LINE_BYTES: usize = 16 * 1024;
const MAX_OAUTH_CALLBACK_HTML_BYTES: usize = 64 * 1024;
const MAX_OAUTH_CALLBACK_CONNECTIONS: usize = 16;
const OAUTH_CALLBACK_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

pub type HtmlRenderer = Arc<dyn Fn(&str) -> String + Send + Sync>;
pub type AuthorizationUrlOpener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
pub type AuthorizationUrlReporter = Arc<dyn Fn(&str) + Send + Sync>;
pub type RemotePlainHttpCallbackReporter = Arc<dyn Fn(&str, u16) + Send + Sync>;

/// Host-application hooks for interactive OAuth authorization.
pub struct OAuthFlowOptions {
    pub callback_host: String,
    pub callback_port: u16,
    pub success_html: HtmlRenderer,
    pub failure_html: HtmlRenderer,
    pub open_authorization_url: AuthorizationUrlOpener,
    pub on_manual_authorization_url: AuthorizationUrlReporter,
    pub on_remote_plain_http_callback: RemotePlainHttpCallbackReporter,
}

impl Default for OAuthFlowOptions {
    fn default() -> Self {
        Self {
            callback_host: "127.0.0.1".to_string(),
            callback_port: DEFAULT_OAUTH_CALLBACK_PORT,
            success_html: Arc::new(|server_name| {
                format!(
                    "<html><body><h1>Authorization complete</h1><p>You can return to ThinClaw for {}.</p></body></html>",
                    html_escape(server_name)
                )
            }),
            failure_html: Arc::new(|server_name| {
                format!(
                    "<html><body><h1>Authorization failed</h1><p>ThinClaw could not authorize {}.</p></body></html>",
                    html_escape(server_name)
                )
            }),
            open_authorization_url: Arc::new(|_| Ok(())),
            on_manual_authorization_url: Arc::new(|auth_url| {
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
        }
    }
}

impl OAuthFlowOptions {
    pub fn redirect_uri(&self) -> String {
        let host = canonical_loopback_host(&self.callback_host)
            .unwrap_or_else(|| self.callback_host.clone());
        format!("http://{}:{}/callback", host, self.callback_port)
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn canonical_loopback_host(host: &str) -> Option<String> {
    if host.eq_ignore_ascii_case("localhost") {
        return Some("127.0.0.1".to_string());
    }
    match host.parse::<std::net::IpAddr>().ok()? {
        std::net::IpAddr::V4(ip) if ip.is_loopback() => Some(ip.to_string()),
        std::net::IpAddr::V6(ip) if ip.is_loopback() => Some(format!("[{ip}]")),
        _ => None,
    }
}

async fn read_bounded_response(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!("response exceeded the {max_bytes}-byte limit"));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| format!("failed to read response: {}", error.without_url()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!("response exceeded the {max_bytes}-byte limit"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn decode_bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let body = read_bounded_response(response, MAX_OAUTH_RESPONSE_BYTES).await?;
    serde_json::from_slice(&body).map_err(|error| format!("invalid JSON response: {error}"))
}

async fn bounded_oauth_error_code(response: reqwest::Response) -> Option<String> {
    let body = read_bounded_response(response, MAX_OAUTH_ERROR_BYTES)
        .await
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let code = value.get("error")?.as_str()?;
    (code.len() <= 128
        && !code.is_empty()
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
    .then(|| code.to_string())
}

fn status_with_oauth_code(status: reqwest::StatusCode, code: Option<String>) -> String {
    match code {
        Some(code) => format!("HTTP {status} (OAuth error `{code}`)"),
        None => format!("HTTP {status}"),
    }
}

fn redacted_oauth_endpoint(value: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(value) else {
        return "<invalid-url>".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

async fn validate_trusted_oauth_endpoint(
    value: &str,
    allow_local: bool,
    label: &str,
) -> Result<(), AuthError> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| AuthError::DiscoveryFailed(format!("{label} is not a valid URL")))?;
    if value.len() > MAX_AUTHORIZATION_URL_BYTES
        || !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AuthError::DiscoveryFailed(format!(
            "{label} is malformed or oversized"
        )));
    }

    let is_public = super::pinned_addrs_for(value, false).await.is_ok();
    if is_public {
        if parsed.scheme() != "https" {
            return Err(AuthError::DiscoveryFailed(format!(
                "Public {label} must use HTTPS"
            )));
        }
        return Ok(());
    }
    if !allow_local {
        return Err(AuthError::DiscoveryFailed(format!(
            "{label} is not a permitted public URL"
        )));
    }
    super::pinned_addrs_for(value, true)
        .await
        .map(|_| ())
        .map_err(|_| AuthError::DiscoveryFailed(format!("{label} is not a permitted local URL")))
}

async fn validate_authorization_endpoint(value: &str, allow_local: bool) -> Result<(), AuthError> {
    validate_trusted_oauth_endpoint(value, allow_local, "authorization endpoint").await
}

fn validate_oauth_parameter(value: &str, label: &str, max_bytes: usize) -> Result<(), AuthError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(AuthError::DiscoveryFailed(format!(
            "{label} is empty, malformed, or oversized"
        )));
    }
    Ok(())
}

fn validate_oauth_items(values: &[String], label: &str) -> Result<(), AuthError> {
    if values.len() > MAX_OAUTH_METADATA_ITEMS {
        return Err(AuthError::DiscoveryFailed(format!(
            "{label} contains too many entries"
        )));
    }
    for value in values {
        validate_oauth_parameter(value, label, MAX_OAUTH_PARAMETER_BYTES)?;
    }
    Ok(())
}

fn validate_oauth_extra(
    values: &HashMap<String, serde_json::Value>,
    label: &str,
) -> Result<(), AuthError> {
    if values.len() > MAX_OAUTH_METADATA_ITEMS
        || values
            .keys()
            .any(|key| key.is_empty() || key.len() > 256 || key.chars().any(char::is_control))
    {
        return Err(AuthError::DiscoveryFailed(format!(
            "{label} contains malformed or excessive extension metadata"
        )));
    }
    Ok(())
}

fn validate_redirect_uri(value: &str) -> Result<(), AuthError> {
    validate_oauth_parameter(value, "OAuth redirect URI", MAX_AUTHORIZATION_URL_BYTES)?;
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        AuthError::DiscoveryFailed("OAuth redirect URI is not a valid URL".to_string())
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(AuthError::DiscoveryFailed(
            "OAuth redirect URI must not contain credentials or a fragment".to_string(),
        ));
    }
    match parsed.scheme() {
        "https" => Ok(()),
        "http"
            if parsed.host().is_some_and(|host| match host {
                url::Host::Domain(domain) => domain
                    .trim_end_matches('.')
                    .eq_ignore_ascii_case("localhost"),
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
            }) =>
        {
            Ok(())
        }
        _ => Err(AuthError::DiscoveryFailed(
            "OAuth redirect URI must use HTTPS, except for an HTTP loopback callback".to_string(),
        )),
    }
}

/// OAuth authorization error.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Server does not support OAuth authorization")]
    NotSupported,

    #[error("Failed to discover authorization endpoints: {0}")]
    DiscoveryFailed(String),

    #[error("Authorization denied by user")]
    AuthorizationDenied,

    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),

    #[error("Token expired and refresh failed: {0}")]
    RefreshFailed(String),

    #[error("No access token available")]
    NoToken,

    #[error("Timeout waiting for authorization callback")]
    Timeout,

    #[error("Could not bind to callback port")]
    PortUnavailable,

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Secrets error: {0}")]
    Secrets(String),
}

/// OAuth protected resource metadata.
/// Discovered from /.well-known/oauth-protected-resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The protected resource identifier.
    pub resource: String,

    /// Optional user-facing resource name.
    #[serde(default)]
    pub resource_name: Option<String>,

    /// Authorization servers that can issue tokens for this resource.
    #[serde(default)]
    pub authorization_servers: Vec<String>,

    /// Scopes supported by this resource.
    #[serde(default)]
    pub scopes_supported: Vec<String>,

    /// Supported bearer token presentation methods.
    #[serde(default)]
    pub bearer_methods_supported: Vec<String>,

    /// Optional documentation URL for the protected resource.
    #[serde(default)]
    pub resource_documentation: Option<String>,

    /// Optional metadata bag preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// OAuth authorization server metadata.
/// Discovered from /.well-known/oauth-authorization-server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// Authorization server issuer.
    pub issuer: String,

    /// Authorization endpoint URL.
    pub authorization_endpoint: String,

    /// Token endpoint URL.
    pub token_endpoint: String,

    /// Dynamic client registration endpoint (if DCR is supported).
    #[serde(default)]
    pub registration_endpoint: Option<String>,

    /// Supported response types.
    #[serde(default)]
    pub response_types_supported: Vec<String>,

    /// Supported grant types.
    #[serde(default)]
    pub grant_types_supported: Vec<String>,

    /// Supported code challenge methods.
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,

    /// Scopes supported by this server.
    #[serde(default)]
    pub scopes_supported: Vec<String>,

    /// Revocation endpoint.
    #[serde(default)]
    pub revocation_endpoint: Option<String>,

    /// Optional pushed authorization request endpoint.
    #[serde(default)]
    pub pushed_authorization_request_endpoint: Option<String>,

    /// Optional metadata bag preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Combined discovery result for a protected resource and its authorization server.
#[derive(Debug, Clone)]
pub struct OAuthDiscoveryBundle {
    pub protected_resource: ProtectedResourceMetadata,
    pub authorization_server: AuthorizationServerMetadata,
}

/// Dynamic Client Registration request.
#[derive(Debug, Clone, Serialize)]
pub struct ClientRegistrationRequest {
    /// Human-readable client name.
    pub client_name: String,

    /// Redirect URIs for OAuth callbacks.
    pub redirect_uris: Vec<String>,

    /// Grant types the client will use.
    pub grant_types: Vec<String>,

    /// Response types the client will use.
    pub response_types: Vec<String>,

    /// Token endpoint authentication method.
    pub token_endpoint_auth_method: String,
}

/// Dynamic Client Registration response.
#[derive(Clone, Deserialize)]
pub struct ClientRegistrationResponse {
    /// The assigned client ID.
    pub client_id: String,

    /// Client secret (if issued).
    #[serde(default)]
    pub client_secret: Option<String>,

    /// When the client secret expires (if applicable).
    #[serde(default)]
    pub client_secret_expires_at: Option<u64>,

    /// Registration access token for managing the registration.
    #[serde(default)]
    pub registration_access_token: Option<String>,

    /// Registration client URI for managing the registration.
    #[serde(default)]
    pub registration_client_uri: Option<String>,
}

impl std::fmt::Debug for ClientRegistrationResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientRegistrationResponse")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("client_secret_expires_at", &self.client_secret_expires_at)
            .field(
                "registration_access_token",
                &self
                    .registration_access_token
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field("registration_client_uri", &"[REDACTED URL]")
            .finish()
    }
}

/// Access token with optional refresh token and expiry.
#[derive(Clone)]
pub struct AccessToken {
    /// The access token value.
    pub access_token: String,

    /// Token type (usually "Bearer").
    pub token_type: String,

    /// Seconds until expiration (if provided).
    pub expires_in: Option<u64>,

    /// Refresh token for obtaining new access tokens.
    pub refresh_token: Option<String>,

    /// Scopes granted.
    pub scope: Option<String>,
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccessToken")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scope", &self.scope)
            .finish()
    }
}

/// Token response from the authorization server.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenResponse")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scope", &self.scope)
            .finish()
    }
}

/// PKCE verifier and challenge pair.
#[derive(Clone)]
pub struct PkceChallenge {
    /// Code verifier (high-entropy random string).
    pub verifier: String,
    /// Code challenge (S256 hash of verifier).
    pub challenge: String,
}

impl std::fmt::Debug for PkceChallenge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PkceChallenge")
            .field("verifier", &"[REDACTED]")
            .field("challenge", &self.challenge)
            .finish()
    }
}

/// Opaque state for a single MCP OAuth authorization transaction.
///
/// The token endpoint, dynamically registered client ID, PKCE verifier, and
/// protected-resource binding are captured at preparation time. Keeping them
/// together prevents callback handlers from rediscovering a different endpoint
/// or losing the verifier between the authorization and token-exchange steps.
pub struct PreparedMcpAuthorization {
    authorization_url: String,
    client_id: String,
    token_url: String,
    pkce: Option<PkceChallenge>,
    resource: String,
    dynamically_registered: bool,
}

impl std::fmt::Debug for PreparedMcpAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedMcpAuthorization")
            .field("authorization_url", &"[REDACTED URL]")
            .field("client_id", &self.client_id)
            .field("token_url", &"[REDACTED URL]")
            .field("pkce", &self.pkce)
            .field("resource", &"[REDACTED URL]")
            .field("dynamically_registered", &self.dynamically_registered)
            .finish()
    }
}

impl PreparedMcpAuthorization {
    /// URL that the user must open to authorize this transaction.
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }
}

/// Captured OAuth callback payload.
#[derive(Clone)]
struct AuthorizationCallback {
    code: String,
}

fn validate_protected_resource_metadata(
    metadata: &ProtectedResourceMetadata,
) -> Result<(), AuthError> {
    validate_oauth_parameter(
        &metadata.resource,
        "protected resource identifier",
        MAX_AUTHORIZATION_URL_BYTES,
    )?;
    if metadata.resource_name.as_ref().is_some_and(|value| {
        value.len() > MAX_OAUTH_PARAMETER_BYTES || value.chars().any(char::is_control)
    }) || metadata
        .resource_documentation
        .as_ref()
        .is_some_and(|value| {
            value.len() > MAX_AUTHORIZATION_URL_BYTES || value.chars().any(char::is_control)
        })
    {
        return Err(AuthError::DiscoveryFailed(
            "Protected-resource metadata contains malformed or oversized text".to_string(),
        ));
    }
    validate_oauth_items(
        &metadata.authorization_servers,
        "authorization server metadata",
    )?;
    validate_oauth_items(&metadata.scopes_supported, "supported OAuth scopes")?;
    validate_oauth_items(
        &metadata.bearer_methods_supported,
        "supported bearer methods",
    )?;
    validate_oauth_extra(&metadata.extra, "protected-resource metadata")?;
    Ok(())
}

fn normalized_issuer(value: &str) -> Result<String, AuthError> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| AuthError::DiscoveryFailed("OAuth issuer is not a valid URL".to_string()))?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AuthError::DiscoveryFailed(
            "OAuth issuer must not contain credentials, a query, or a fragment".to_string(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

async fn validate_authorization_server_metadata(
    metadata: &AuthorizationServerMetadata,
    expected_issuer: &str,
    allow_local: bool,
) -> Result<(), AuthError> {
    validate_oauth_parameter(
        &metadata.issuer,
        "OAuth issuer",
        MAX_AUTHORIZATION_URL_BYTES,
    )?;
    if normalized_issuer(&metadata.issuer)? != normalized_issuer(expected_issuer)? {
        return Err(AuthError::DiscoveryFailed(
            "Discovered OAuth issuer does not match the requested authorization server".to_string(),
        ));
    }
    validate_trusted_oauth_endpoint(
        &metadata.authorization_endpoint,
        allow_local,
        "authorization endpoint",
    )
    .await?;
    validate_trusted_oauth_endpoint(&metadata.token_endpoint, allow_local, "token endpoint")
        .await?;
    if let Some(value) = metadata.registration_endpoint.as_deref() {
        validate_trusted_oauth_endpoint(value, allow_local, "registration endpoint").await?;
    }
    for (label, value) in [
        (
            "revocation endpoint",
            metadata.revocation_endpoint.as_deref(),
        ),
        (
            "pushed authorization request endpoint",
            metadata.pushed_authorization_request_endpoint.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_trusted_oauth_endpoint(value, allow_local, label).await?;
        }
    }
    validate_oauth_items(
        &metadata.response_types_supported,
        "supported OAuth response types",
    )?;
    validate_oauth_items(
        &metadata.grant_types_supported,
        "supported OAuth grant types",
    )?;
    validate_oauth_items(
        &metadata.code_challenge_methods_supported,
        "supported PKCE methods",
    )?;
    validate_oauth_items(&metadata.scopes_supported, "supported OAuth scopes")?;
    validate_oauth_extra(&metadata.extra, "authorization-server metadata")?;
    if !metadata.response_types_supported.is_empty()
        && !metadata
            .response_types_supported
            .iter()
            .any(|value| value == "code")
    {
        return Err(AuthError::NotSupported);
    }
    if !metadata.code_challenge_methods_supported.is_empty()
        && !metadata
            .code_challenge_methods_supported
            .iter()
            .any(|value| value.eq_ignore_ascii_case("S256"))
    {
        return Err(AuthError::NotSupported);
    }
    Ok(())
}

fn validate_client_registration_response(
    registration: &ClientRegistrationResponse,
) -> Result<(), AuthError> {
    validate_oauth_parameter(
        &registration.client_id,
        "dynamically registered client ID",
        MAX_OAUTH_PARAMETER_BYTES,
    )?;
    for (label, value) in [
        ("client secret", registration.client_secret.as_deref()),
        (
            "registration access token",
            registration.registration_access_token.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_OAUTH_SECRET_BYTES
                || value.chars().any(char::is_control)
        }) {
            return Err(AuthError::DiscoveryFailed(format!(
                "DCR {label} is malformed or oversized"
            )));
        }
    }
    if registration
        .registration_client_uri
        .as_ref()
        .is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_AUTHORIZATION_URL_BYTES
                || value.chars().any(char::is_control)
        })
    {
        return Err(AuthError::DiscoveryFailed(
            "DCR registration client URI is malformed or oversized".to_string(),
        ));
    }
    Ok(())
}

fn access_token_from_response(token: TokenResponse) -> Result<AccessToken, String> {
    if token.access_token.is_empty()
        || token.access_token.len() > MAX_OAUTH_SECRET_BYTES
        || token.access_token.chars().any(char::is_control)
        || !token.token_type.eq_ignore_ascii_case("Bearer")
        || token.refresh_token.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_OAUTH_SECRET_BYTES
                || value.chars().any(char::is_control)
        })
        || token.scope.as_ref().is_some_and(|value| {
            value.len() > MAX_OAUTH_PARAMETER_BYTES || value.chars().any(char::is_control)
        })
        || token
            .expires_in
            .is_some_and(|seconds| seconds > MAX_OAUTH_TOKEN_LIFETIME_SECS)
    {
        return Err("token response contains malformed or oversized fields".to_string());
    }
    Ok(AccessToken {
        access_token: token.access_token,
        token_type: "Bearer".to_string(),
        expires_in: token.expires_in,
        refresh_token: token.refresh_token,
        scope: token.scope,
    })
}

fn validate_access_token(token: &AccessToken) -> Result<(), AuthError> {
    if token.access_token.is_empty()
        || token.access_token.len() > MAX_OAUTH_SECRET_BYTES
        || token.access_token.chars().any(char::is_control)
        || !token.token_type.eq_ignore_ascii_case("Bearer")
        || token.refresh_token.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_OAUTH_SECRET_BYTES
                || value.chars().any(char::is_control)
        })
        || token.scope.as_ref().is_some_and(|value| {
            value.len() > MAX_OAUTH_PARAMETER_BYTES || value.chars().any(char::is_control)
        })
        || token
            .expires_in
            .is_some_and(|seconds| seconds > MAX_OAUTH_TOKEN_LIFETIME_SECS)
    {
        return Err(AuthError::Secrets(
            "refusing to persist malformed or oversized OAuth token fields".to_string(),
        ));
    }
    Ok(())
}

fn origin_from_server_url(server_url: &str) -> Result<String, AuthError> {
    let parsed = reqwest::Url::parse(server_url)
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid server URL: {}", e)))?;
    Ok(parsed.origin().ascii_serialization())
}

fn resolve_resource_identifier(server_url: &str, metadata: &ProtectedResourceMetadata) -> String {
    if !metadata.resource.trim().is_empty() {
        metadata.resource.clone()
    } else {
        origin_from_server_url(server_url).unwrap_or_else(|_| server_url.to_string())
    }
}

async fn fetch_authorization_server_metadata(
    client: &reqwest::Client,
    metadata_url: &str,
    expected_issuer: &str,
    allow_local: bool,
) -> Result<AuthorizationServerMetadata, AuthError> {
    let response = client
        .get(metadata_url)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.without_url().to_string()))?;

    if !response.status().is_success() {
        return Err(AuthError::DiscoveryFailed(format!(
            "HTTP {} from {}",
            response.status(),
            redacted_oauth_endpoint(metadata_url)
        )));
    }

    let metadata: AuthorizationServerMetadata = decode_bounded_json(response)
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid metadata: {}", e)))?;
    validate_authorization_server_metadata(&metadata, expected_issuer, allow_local).await?;
    Ok(metadata)
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair.
    pub fn generate() -> Self {
        let mut verifier_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        Self {
            verifier,
            challenge,
        }
    }
}

/// Discover protected resource metadata from an MCP server.
pub async fn discover_protected_resource(
    server_url: &str,
) -> Result<ProtectedResourceMetadata, AuthError> {
    discover_protected_resource_with_policy(server_url, false).await
}

async fn discover_protected_resource_with_policy(
    server_url: &str,
    allow_local: bool,
) -> Result<ProtectedResourceMetadata, AuthError> {
    // Parse the server URL to extract the origin (scheme + host + port)
    // The .well-known endpoints are always at the root of the origin, not under any path
    let origin = origin_from_server_url(server_url)?;

    // Try the well-known endpoint at the origin root
    let well_known_url = format!("{}/.well-known/oauth-protected-resource", origin);

    // F-02: pin the connection to the validated address for the discovery host.
    let client = super::build_pinned(
        reqwest::Client::builder().timeout(Duration::from_secs(10)),
        &well_known_url,
        allow_local,
    )
    .await
    .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;

    let response = client
        .get(&well_known_url)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.without_url().to_string()))?;

    if !response.status().is_success() {
        return Err(AuthError::NotSupported);
    }

    let mut metadata: ProtectedResourceMetadata = decode_bounded_json(response)
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid metadata: {}", e)))?;
    if metadata.resource.trim().is_empty() {
        metadata.resource = origin;
    }
    validate_protected_resource_metadata(&metadata)?;
    Ok(metadata)
}

/// Discover authorization server metadata.
pub async fn discover_authorization_server(
    auth_server_url: &str,
) -> Result<AuthorizationServerMetadata, AuthError> {
    discover_authorization_server_with_policy(auth_server_url, false).await
}

async fn discover_authorization_server_with_policy(
    auth_server_url: &str,
    allow_local: bool,
) -> Result<AuthorizationServerMetadata, AuthError> {
    validate_trusted_oauth_endpoint(auth_server_url, allow_local, "authorization server").await?;
    normalized_issuer(auth_server_url)?;
    // F-02: pin to the validated address for the authorization-server host. Both
    // metadata URLs below share this host, so one pin covers them.
    let client = super::build_pinned(
        reqwest::Client::builder().timeout(Duration::from_secs(10)),
        auth_server_url,
        allow_local,
    )
    .await
    .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;

    let base_url = auth_server_url.trim_end_matches('/');
    let metadata_urls = [
        format!("{}/.well-known/oauth-authorization-server", base_url),
        format!("{}/.well-known/openid-configuration", base_url),
    ];

    let mut last_error = None;
    for metadata_url in metadata_urls {
        match fetch_authorization_server_metadata(
            &client,
            &metadata_url,
            auth_server_url,
            allow_local,
        )
        .await
        {
            Ok(metadata) => return Ok(metadata),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AuthError::DiscoveryFailed("Unable to discover authorization server metadata".to_string())
    }))
}

/// Discover OAuth endpoints for an MCP server.
///
/// First checks if endpoints are explicitly configured, then falls back to discovery.
pub async fn discover_oauth_endpoints(
    server_config: &McpServerConfig,
) -> Result<(String, String), AuthError> {
    server_config
        .validate()
        .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;
    let allow_local = server_config.allow_local_http;
    let oauth = server_config
        .oauth
        .as_ref()
        .ok_or(AuthError::NotSupported)?;

    // If endpoints are explicitly configured, use them
    if let (Some(auth_url), Some(token_url)) = (&oauth.authorization_url, &oauth.token_url) {
        validate_authorization_endpoint(auth_url, allow_local).await?;
        validate_trusted_oauth_endpoint(token_url, allow_local, "token endpoint").await?;
        return Ok((auth_url.clone(), token_url.clone()));
    }

    // Try to discover from the server
    let resource_meta =
        discover_protected_resource_with_policy(&server_config.url, allow_local).await?;

    // Get the first authorization server
    let auth_server_url = resource_meta
        .authorization_servers
        .first()
        .ok_or_else(|| AuthError::DiscoveryFailed("No authorization servers listed".to_string()))?;

    // Discover the authorization server metadata
    let auth_meta = discover_authorization_server_with_policy(auth_server_url, allow_local).await?;

    Ok((auth_meta.authorization_endpoint, auth_meta.token_endpoint))
}

/// Discover full OAuth metadata including DCR support.
///
/// Returns authorization server metadata which includes registration_endpoint if DCR is supported.
pub async fn discover_full_oauth_metadata(
    server_url: &str,
) -> Result<AuthorizationServerMetadata, AuthError> {
    Ok(discover_oauth_bundle(server_url)
        .await?
        .authorization_server)
}

/// Discover both protected-resource and authorization-server metadata.
pub async fn discover_oauth_bundle(server_url: &str) -> Result<OAuthDiscoveryBundle, AuthError> {
    discover_oauth_bundle_with_policy(server_url, false).await
}

async fn discover_oauth_bundle_with_policy(
    server_url: &str,
    allow_local: bool,
) -> Result<OAuthDiscoveryBundle, AuthError> {
    let resource_meta = discover_protected_resource_with_policy(server_url, allow_local).await?;
    let auth_server_url = resource_meta
        .authorization_servers
        .first()
        .ok_or_else(|| AuthError::DiscoveryFailed("No authorization servers listed".to_string()))?;
    let auth_meta = discover_authorization_server_with_policy(auth_server_url, allow_local).await?;
    Ok(OAuthDiscoveryBundle {
        protected_resource: resource_meta,
        authorization_server: auth_meta,
    })
}

/// Perform Dynamic Client Registration with an authorization server.
///
/// This allows clients to register themselves at runtime without pre-configured credentials.
pub async fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<ClientRegistrationResponse, AuthError> {
    register_client_with_policy(registration_endpoint, redirect_uri, false).await
}

async fn register_client_with_policy(
    registration_endpoint: &str,
    redirect_uri: &str,
    allow_local: bool,
) -> Result<ClientRegistrationResponse, AuthError> {
    // F-02: pin the connection to the validated address for the registration host.
    let client = super::build_pinned(
        reqwest::Client::builder().timeout(Duration::from_secs(30)),
        registration_endpoint,
        allow_local,
    )
    .await
    .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;

    let request = ClientRegistrationRequest {
        client_name: "ThinClaw".to_string(),
        redirect_uris: vec![redirect_uri.to_string()],
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        token_endpoint_auth_method: "none".to_string(), // Public client (no secret)
    };

    let response = client
        .post(registration_endpoint)
        .json(&request)
        .send()
        .await
        .map_err(|e| {
            AuthError::DiscoveryFailed(format!("DCR request failed: {}", e.without_url()))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let code = bounded_oauth_error_code(response).await;
        return Err(AuthError::DiscoveryFailed(format!(
            "DCR failed: {}",
            status_with_oauth_code(status, code)
        )));
    }

    let registration: ClientRegistrationResponse = decode_bounded_json(response)
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid DCR response: {}", e)))?;
    validate_client_registration_response(&registration)?;
    Ok(registration)
}

/// Prepare a single MCP OAuth transaction for either a blocking loopback flow
/// or a host application's asynchronous callback route.
pub async fn prepare_mcp_authorization(
    server_config: &McpServerConfig,
    redirect_uri: &str,
    state: &str,
) -> Result<PreparedMcpAuthorization, AuthError> {
    server_config
        .validate()
        .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;
    validate_redirect_uri(redirect_uri)?;
    validate_oauth_parameter(state, "OAuth state", MAX_OAUTH_PARAMETER_BYTES)?;
    let allow_local = server_config.allow_local_http;

    let (
        client_id,
        authorization_endpoint,
        token_url,
        scopes,
        extra_params,
        resource,
        dynamically_registered,
    ) = if let Some(oauth) = &server_config.oauth {
        // Explicit endpoints can operate without discovery. Discovery remains
        // useful for default scopes and the protected-resource identifier, but
        // failure cannot replace explicitly trusted configuration.
        let bundle = discover_oauth_bundle_with_policy(&server_config.url, allow_local)
            .await
            .ok();
        let (authorization_endpoint, token_url) = if let (Some(auth_url), Some(token_url)) =
            (&oauth.authorization_url, &oauth.token_url)
        {
            (auth_url.clone(), token_url.clone())
        } else {
            let bundle = bundle.as_ref().ok_or(AuthError::NotSupported)?;
            (
                bundle.authorization_server.authorization_endpoint.clone(),
                bundle.authorization_server.token_endpoint.clone(),
            )
        };
        let scopes = if oauth.scopes.is_empty() {
            bundle
                .as_ref()
                .map(|bundle| {
                    if bundle.protected_resource.scopes_supported.is_empty() {
                        bundle.authorization_server.scopes_supported.clone()
                    } else {
                        bundle.protected_resource.scopes_supported.clone()
                    }
                })
                .unwrap_or_default()
        } else {
            oauth.scopes.clone()
        };
        let resource = oauth.resource.clone().unwrap_or_else(|| {
            bundle
                .as_ref()
                .map(|bundle| {
                    resolve_resource_identifier(&server_config.url, &bundle.protected_resource)
                })
                .unwrap_or_else(|| server_config.url.clone())
        });
        (
            oauth.client_id.clone(),
            authorization_endpoint,
            token_url,
            scopes,
            oauth.extra_params.clone(),
            resource,
            false,
        )
    } else {
        let bundle = discover_oauth_bundle_with_policy(&server_config.url, allow_local).await?;
        let auth_meta = bundle.authorization_server.clone();
        let registration_endpoint = auth_meta
            .registration_endpoint
            .as_deref()
            .ok_or(AuthError::NotSupported)?;
        let registration =
            register_client_with_policy(registration_endpoint, redirect_uri, allow_local).await?;
        let scopes = if bundle.protected_resource.scopes_supported.is_empty() {
            auth_meta.scopes_supported.clone()
        } else {
            bundle.protected_resource.scopes_supported.clone()
        };
        let resource = resolve_resource_identifier(&server_config.url, &bundle.protected_resource);
        (
            registration.client_id,
            auth_meta.authorization_endpoint,
            auth_meta.token_endpoint,
            scopes,
            HashMap::new(),
            resource,
            true,
        )
    };

    validate_oauth_parameter(&client_id, "OAuth client ID", MAX_OAUTH_PARAMETER_BYTES)?;
    validate_oauth_parameter(
        &resource,
        "OAuth protected resource",
        MAX_AUTHORIZATION_URL_BYTES,
    )?;
    validate_authorization_endpoint(&authorization_endpoint, allow_local).await?;
    validate_trusted_oauth_endpoint(&token_url, allow_local, "token endpoint").await?;

    // ThinClaw is a public client and therefore always uses S256 PKCE. This
    // remains true for explicitly configured clients as well as DCR clients.
    let pkce = PkceChallenge::generate();
    let authorization_url = build_authorization_url(
        &authorization_endpoint,
        &client_id,
        redirect_uri,
        &scopes,
        Some(&pkce),
        Some(state),
        Some(&resource),
        &extra_params,
    )?;

    Ok(PreparedMcpAuthorization {
        authorization_url,
        client_id,
        token_url,
        pkce: Some(pkce),
        resource,
        dynamically_registered,
    })
}

/// Complete a transaction returned by [`prepare_mcp_authorization`], persist
/// its tokens, and retain a DCR client ID for future refreshes.
pub async fn complete_mcp_authorization(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    prepared: &PreparedMcpAuthorization,
    code: &str,
    redirect_uri: &str,
) -> Result<AccessToken, AuthError> {
    server_config
        .validate()
        .map_err(|error| AuthError::TokenExchangeFailed(error.to_string()))?;
    validate_redirect_uri(redirect_uri)
        .map_err(|error| AuthError::TokenExchangeFailed(error.to_string()))?;
    if code.is_empty()
        || code.len() > MAX_OAUTH_PARAMETER_BYTES
        || code.chars().any(char::is_control)
    {
        return Err(AuthError::TokenExchangeFailed(
            "authorization code is malformed or oversized".to_string(),
        ));
    }

    let token = exchange_code_for_token_with_policy(
        &prepared.token_url,
        &prepared.client_id,
        code,
        redirect_uri,
        prepared.pkce.as_ref(),
        Some(&prepared.resource),
        server_config.allow_local_http,
    )
    .await?;
    if prepared.dynamically_registered {
        store_client_id(secrets, user_id, server_config, &prepared.client_id).await?;
    }
    store_tokens(secrets, user_id, server_config, &token).await?;
    Ok(token)
}

/// Perform the OAuth 2.1 authorization flow for an MCP server.
///
/// Supports two modes:
/// 1. Pre-configured OAuth: Uses the client_id from server config
/// 2. Dynamic Client Registration: Discovers and registers with the server automatically
///
/// Flow:
/// 1. Discovers authorization endpoints from the server
/// 2. If no client_id configured, attempts Dynamic Client Registration (DCR)
/// 3. Generates PKCE challenge
/// 4. Opens browser for user authorization
/// 5. Receives callback with authorization code
/// 6. Exchanges code for access token
/// 7. Stores token securely
pub async fn authorize_mcp_server(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<AccessToken, AuthError> {
    authorize_mcp_server_with_options(server_config, secrets, user_id, OAuthFlowOptions::default())
        .await
}

/// Perform the OAuth 2.1 authorization flow with host-provided callback/browser behavior.
pub async fn authorize_mcp_server_with_options(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    options: OAuthFlowOptions,
) -> Result<AccessToken, AuthError> {
    server_config
        .validate()
        .map_err(|error| AuthError::DiscoveryFailed(error.to_string()))?;
    if options.callback_port == 0 || !is_loopback_host(&options.callback_host) {
        (options.on_remote_plain_http_callback)(&options.callback_host, options.callback_port);
        return Err(AuthError::DiscoveryFailed(
            "OAuth callbacks over plain HTTP must bind to loopback; use SSH port forwarding or an HTTPS callback proxy"
                .to_string(),
        ));
    }
    // Find an available port for the callback first (needed for DCR)
    let (listener, _port) =
        bind_callback_listener(&options.callback_host, options.callback_port).await?;
    let redirect_uri = options.redirect_uri();

    let state = uuid::Uuid::new_v4().to_string();
    let prepared = prepare_mcp_authorization(server_config, &redirect_uri, &state).await?;

    // Open browser
    println!("  Opening browser for {} login...", server_config.name);
    if let Err(e) = (options.open_authorization_url)(prepared.authorization_url()) {
        println!("  Could not open browser: {}", e);
        (options.on_manual_authorization_url)(prepared.authorization_url());
    }

    println!("  Waiting for authorization...");

    // Wait for callback
    let callback = wait_for_authorization_callback(
        listener,
        &server_config.name,
        Some(&state),
        options.success_html.clone(),
        options.failure_html.clone(),
    )
    .await?;

    println!("  Exchanging code for token...");

    // Exchange code for token
    complete_mcp_authorization(
        server_config,
        secrets,
        user_id,
        &prepared,
        &callback.code,
        &redirect_uri,
    )
    .await
}

/// Bind the OAuth callback listener on the shared fixed port.
pub async fn find_available_port() -> Result<(TcpListener, u16), AuthError> {
    bind_callback_listener("127.0.0.1", DEFAULT_OAUTH_CALLBACK_PORT).await
}

/// Bind the OAuth callback listener on the requested fixed port.
pub async fn bind_callback_listener(
    host: &str,
    port: u16,
) -> Result<(TcpListener, u16), AuthError> {
    if port == 0 || !is_loopback_host(host) {
        return Err(AuthError::DiscoveryFailed(
            "OAuth callback listener must use a non-zero port on a loopback host".to_string(),
        ));
    }
    let canonical = canonical_loopback_host(host).ok_or_else(|| {
        AuthError::DiscoveryFailed("OAuth callback listener host is invalid".to_string())
    })?;
    let listener = TcpListener::bind(format!("{canonical}:{port}"))
        .await
        .map_err(|_| AuthError::PortUnavailable)?;
    Ok((listener, port))
}

/// Build the authorization URL with all required parameters.
#[allow(clippy::too_many_arguments)]
pub fn build_authorization_url(
    base_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    pkce: Option<&PkceChallenge>,
    state: Option<&str>,
    resource: Option<&str>,
    extra_params: &HashMap<String, String>,
) -> Result<String, AuthError> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| {
        AuthError::DiscoveryFailed(format!("Invalid authorization URL: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || base_url.len() > MAX_AUTHORIZATION_URL_BYTES
        || client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
        || redirect_uri.len() > MAX_OAUTH_PARAMETER_BYTES
        || scopes.len() > MAX_OAUTH_METADATA_ITEMS
        || extra_params.len() > MAX_OAUTH_EXTRA_PARAMETERS
    {
        return Err(AuthError::DiscoveryFailed(
            "Authorization endpoint or parameters are malformed or oversized".to_string(),
        ));
    }
    validate_redirect_uri(redirect_uri)?;
    if state.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_OAUTH_PARAMETER_BYTES
            || value.chars().any(char::is_control)
    }) || pkce.is_some_and(|value| {
        value.challenge.is_empty()
            || value.challenge.len() > 128
            || !value
                .challenge
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return Err(AuthError::DiscoveryFailed(
            "OAuth state or PKCE challenge is malformed or oversized".to_string(),
        ));
    }

    const RESERVED: &[&str] = &[
        "client_id",
        "response_type",
        "redirect_uri",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "state",
        "resource",
    ];
    let existing = url
        .query_pairs()
        .filter(|(key, _)| !RESERVED.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if existing.len() > MAX_OAUTH_EXTRA_PARAMETERS
        || existing.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
        })
    {
        return Err(AuthError::DiscoveryFailed(
            "Authorization endpoint query is malformed or excessive".to_string(),
        ));
    }
    url.set_query(None);
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in existing {
            pairs.append_pair(&key, &value);
        }
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("response_type", "code");
        pairs.append_pair("redirect_uri", redirect_uri);
        if !scopes.is_empty() {
            let scope = scopes.join(" ");
            if scope.len() > MAX_OAUTH_PARAMETER_BYTES || scope.chars().any(char::is_control) {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth scope list is malformed or oversized".to_string(),
                ));
            }
            pairs.append_pair("scope", &scope);
        }
        if let Some(pkce) = pkce {
            pairs.append_pair("code_challenge", &pkce.challenge);
            pairs.append_pair("code_challenge_method", "S256");
        }
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
        if let Some(resource) = resource {
            if resource.is_empty()
                || resource.len() > MAX_AUTHORIZATION_URL_BYTES
                || resource.chars().any(char::is_control)
            {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth resource parameter is oversized".to_string(),
                ));
            }
            pairs.append_pair("resource", resource);
        }
        for (key, value) in extra_params {
            if RESERVED.contains(&key.as_str()) {
                continue;
            }
            if key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
            {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth extra parameter is malformed or oversized".to_string(),
                ));
            }
            pairs.append_pair(key, value);
        }
    }
    let result = url.to_string();
    if result.len() > MAX_AUTHORIZATION_URL_BYTES {
        return Err(AuthError::DiscoveryFailed(
            "Authorization URL exceeds the output limit".to_string(),
        ));
    }
    Ok(result)
}

/// Compare two OAuth `state` values in constant time.
///
/// Avoids leaking the expected `state` through a timing side channel when
/// validating the loopback callback. Uses `subtle::ConstantTimeEq`, the same
/// primitive as the WASM tool OAuth flow in `crate::wasm::oauth` and
/// `cli::oauth_defaults`, rather than a hand-rolled comparator.
fn oauth_state_matches(expected: &str, received: &str) -> bool {
    use subtle::ConstantTimeEq;
    // `ct_eq` is constant-time only across equal-length inputs; the explicit
    // length check guards the differing-length case (the byte comparison is
    // skipped, but the lengths themselves are not secret).
    expected.len() == received.len() && expected.as_bytes().ct_eq(received.as_bytes()).into()
}

/// Wait for the authorization callback and validate an optional state nonce.
async fn write_callback_response(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let body = if body.len() <= MAX_OAUTH_CALLBACK_HTML_BYTES {
        body
    } else {
        "<html><body>OAuth callback completed.</body></html>"
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(
        OAUTH_CALLBACK_CONNECTION_TIMEOUT,
        socket.write_all(response.as_bytes()),
    )
    .await;
    let _ = socket.shutdown().await;
}

async fn handle_authorization_callback_connection(
    mut socket: tokio::net::TcpStream,
    server_name: String,
    expected_state: Option<String>,
    success_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
    failure_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
) -> Result<Option<AuthorizationCallback>, AuthError> {
    let request_line = tokio::time::timeout(OAUTH_CALLBACK_CONNECTION_TIMEOUT, async {
        let reader = BufReader::new(&mut socket);
        let mut limited = reader.take((MAX_OAUTH_CALLBACK_LINE_BYTES + 1) as u64);
        let mut request_line = String::new();
        let bytes = limited
            .read_line(&mut request_line)
            .await
            .map_err(|error| AuthError::Http(error.to_string()))?;
        Ok::<_, AuthError>((request_line, bytes))
    })
    .await;

    let Ok(Ok((request_line, bytes))) = request_line else {
        write_callback_response(&mut socket, "408 Request Timeout", "").await;
        return Ok(None);
    };
    if bytes == 0
        || request_line.len() > MAX_OAUTH_CALLBACK_LINE_BYTES
        || !request_line.ends_with('\n')
    {
        write_callback_response(&mut socket, "414 URI Too Long", "").await;
        return Ok(None);
    }

    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    };
    if method != "GET" || !matches!(version, "HTTP/1.0" | "HTTP/1.1") || parts.next().is_some() {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" || query.len() > MAX_OAUTH_CALLBACK_LINE_BYTES {
        write_callback_response(&mut socket, "404 Not Found", "").await;
        return Ok(None);
    }

    let mut params = HashMap::new();
    let mut item_count = 0usize;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        item_count = item_count.saturating_add(1);
        if item_count > MAX_OAUTH_EXTRA_PARAMETERS
            || key.is_empty()
            || key.len() > 128
            || value.len() > MAX_OAUTH_PARAMETER_BYTES
            || params
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            write_callback_response(&mut socket, "400 Bad Request", "").await;
            return Ok(None);
        }
    }

    // Validate state before acting on either an error or a code. Otherwise any
    // local process could cancel the real flow with `/callback?error=...`.
    if expected_state.as_deref().is_some_and(|expected| {
        !params
            .get("state")
            .is_some_and(|received| oauth_state_matches(expected, received))
    }) {
        write_callback_response(&mut socket, "400 Bad Request", &failure_html(&server_name)).await;
        return Ok(None);
    }

    if params.contains_key("error") {
        write_callback_response(&mut socket, "400 Bad Request", &failure_html(&server_name)).await;
        return Err(AuthError::AuthorizationDenied);
    }

    let Some(code) = params.remove("code") else {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    };
    if code.is_empty()
        || code.len() > MAX_OAUTH_PARAMETER_BYTES
        || code.chars().any(char::is_control)
    {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    }

    write_callback_response(&mut socket, "200 OK", &success_html(&server_name)).await;
    Ok(Some(AuthorizationCallback { code }))
}

async fn wait_for_authorization_callback(
    listener: TcpListener,
    server_name: &str,
    expected_state: Option<&str>,
    success_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
    failure_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
) -> Result<AuthorizationCallback, AuthError> {
    let expected_state = expected_state.map(str::to_string);
    let server_name = server_name.to_string();
    tokio::time::timeout(Duration::from_secs(300), async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept(), if handlers.len() < MAX_OAUTH_CALLBACK_CONNECTIONS => {
                    let (socket, _) = accepted
                        .map_err(|error| AuthError::Http(error.to_string()))?;
                    handlers.spawn(handle_authorization_callback_connection(
                        socket,
                        server_name.clone(),
                        expected_state.clone(),
                        Arc::clone(&success_html),
                        Arc::clone(&failure_html),
                    ));
                }
                completed = handlers.join_next(), if !handlers.is_empty() => {
                    match completed {
                        Some(Ok(Ok(Some(callback)))) => return Ok(callback),
                        Some(Ok(Err(error))) => return Err(error),
                        Some(Ok(Ok(None))) | Some(Err(_)) | None => {}
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| AuthError::Timeout)?
}

/// Exchange the authorization code for an access token.
pub async fn exchange_code_for_token(
    token_url: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    pkce: Option<&PkceChallenge>,
    resource: Option<&str>,
) -> Result<AccessToken, AuthError> {
    exchange_code_for_token_with_policy(
        token_url,
        client_id,
        code,
        redirect_uri,
        pkce,
        resource,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn exchange_code_for_token_with_policy(
    token_url: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    pkce: Option<&PkceChallenge>,
    resource: Option<&str>,
    allow_local: bool,
) -> Result<AccessToken, AuthError> {
    validate_trusted_oauth_endpoint(token_url, allow_local, "token endpoint")
        .await
        .map_err(|error| AuthError::TokenExchangeFailed(error.to_string()))?;
    validate_redirect_uri(redirect_uri)
        .map_err(|error| AuthError::TokenExchangeFailed(error.to_string()))?;
    for (label, value) in [("OAuth client ID", client_id), ("authorization code", code)] {
        if value.is_empty()
            || value.len() > MAX_OAUTH_PARAMETER_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(AuthError::TokenExchangeFailed(format!(
                "{label} is malformed or oversized"
            )));
        }
    }
    if pkce.is_some_and(|pkce| {
        pkce.verifier.is_empty()
            || pkce.verifier.len() > 128
            || !pkce.verifier.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
            })
    }) || resource.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_AUTHORIZATION_URL_BYTES
            || value.chars().any(char::is_control)
    }) {
        return Err(AuthError::TokenExchangeFailed(
            "PKCE verifier or protected resource is malformed or oversized".to_string(),
        ));
    }

    // F-02: pin the connection to the validated address for the token host.
    let client = super::build_pinned(
        reqwest::Client::builder().timeout(Duration::from_secs(30)),
        token_url,
        allow_local,
    )
    .await
    .map_err(|error| AuthError::TokenExchangeFailed(error.to_string()))?;

    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client_id.to_string()),
    ];

    if let Some(pkce) = pkce {
        params.push(("code_verifier", pkce.verifier.clone()));
    }

    if let Some(resource) = resource {
        params.push(("resource", resource.to_string()));
    }

    let response = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| AuthError::TokenExchangeFailed(e.without_url().to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let code = bounded_oauth_error_code(response).await;
        return Err(AuthError::TokenExchangeFailed(status_with_oauth_code(
            status, code,
        )));
    }

    let token_response: TokenResponse = decode_bounded_json(response)
        .await
        .map_err(|e| AuthError::TokenExchangeFailed(format!("Invalid response: {}", e)))?;

    access_token_from_response(token_response)
        .map_err(|error| AuthError::TokenExchangeFailed(format!("Invalid response: {error}")))
}

/// Store access and refresh tokens securely.
pub async fn store_tokens(
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    server_config: &McpServerConfig,
    token: &AccessToken,
) -> Result<(), AuthError> {
    validate_access_token(token)?;
    // Persist the continuity credential before publishing a replacement access
    // token. If storage is only partially successful, callers continue to see
    // the previous access token rather than a new token with missing refresh
    // state.
    if let Some(ref refresh_token) = token.refresh_token {
        let params =
            CreateSecretParams::new(server_config.refresh_token_secret_name(), refresh_token)
                .with_provider(format!("mcp:{}", server_config.name));

        secrets
            .create(user_id, params)
            .await
            .map_err(|e| AuthError::Secrets(e.to_string()))?;
    }

    let mut params =
        CreateSecretParams::new(server_config.token_secret_name(), &token.access_token)
            .with_provider(format!("mcp:{}", server_config.name));
    if let Some(expires_in) = token.expires_in {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64);
        params = params.with_expiry(expires_at);
    }

    secrets
        .create(user_id, params)
        .await
        .map_err(|e| AuthError::Secrets(e.to_string()))?;

    Ok(())
}

/// Store the DCR client ID for future token refresh.
pub async fn store_client_id(
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    server_config: &McpServerConfig,
    client_id: &str,
) -> Result<(), AuthError> {
    if client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
    {
        return Err(AuthError::Secrets(
            "refusing to persist a malformed or oversized OAuth client ID".to_string(),
        ));
    }
    let params = CreateSecretParams::new(server_config.client_id_secret_name(), client_id)
        .with_provider(format!("mcp:{}", server_config.name));

    secrets
        .create(user_id, params)
        .await
        .map(|_| ())
        .map_err(|e| AuthError::Secrets(e.to_string()))
}

/// Get the client ID for a server (from config or stored DCR).
async fn get_client_id(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<String, AuthError> {
    // First check if OAuth is configured with a client_id
    if let Some(ref oauth) = server_config.oauth {
        return Ok(oauth.client_id.clone());
    }

    // Otherwise try to get the DCR client_id from secrets
    match secrets
        .get_for_injection(
            user_id,
            &server_config.client_id_secret_name(),
            SecretAccessContext::new("mcp.auth", "oauth_client_id"),
        )
        .await
    {
        Ok(client_id) => Ok(client_id.expose().to_string()),
        Err(SecretError::NotFound(_)) => Err(AuthError::RefreshFailed(
            "No client ID found. Please re-authenticate.".to_string(),
        )),
        Err(e) => Err(AuthError::Secrets(e.to_string())),
    }
}

/// Get the stored access token for an MCP server.
pub async fn get_access_token(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<Option<String>, AuthError> {
    match secrets
        .get_for_injection(
            user_id,
            &server_config.token_secret_name(),
            SecretAccessContext::new("mcp.auth", "oauth_access_token"),
        )
        .await
    {
        Ok(token) => Ok(Some(token.expose().to_string())),
        Err(SecretError::NotFound(_)) => Ok(None),
        Err(e) => Err(AuthError::Secrets(e.to_string())),
    }
}

/// Check if a server has valid authentication.
///
/// Returns true if:
/// - A valid access token is stored (regardless of how it was obtained)
/// - The server doesn't require authentication at all
pub async fn is_authenticated(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> bool {
    // Read metadata rather than using `exists`: all backends intentionally keep
    // expired rows for refresh/audit purposes, so mere row existence does not
    // mean the credential is currently usable.
    secrets
        .get(user_id, &server_config.token_secret_name())
        .await
        .is_ok()
}

/// Refresh an access token using the refresh token.
///
/// Works with both pre-configured OAuth and Dynamic Client Registration (DCR).
/// For DCR, retrieves the client_id from stored secrets.
pub async fn refresh_access_token(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<AccessToken, AuthError> {
    server_config
        .validate()
        .map_err(|error| AuthError::RefreshFailed(error.to_string()))?;
    let allow_local = server_config.allow_local_http;
    // Get client_id (from config or stored DCR)
    let client_id = get_client_id(server_config, secrets, user_id).await?;
    if client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
    {
        return Err(AuthError::RefreshFailed(
            "stored OAuth client ID is malformed or oversized".to_string(),
        ));
    }

    // Get the refresh token
    let refresh_token = secrets
        .get_for_injection(
            user_id,
            &server_config.refresh_token_secret_name(),
            SecretAccessContext::new("mcp.auth", "oauth_refresh_token"),
        )
        .await
        .map_err(|e| AuthError::RefreshFailed(format!("No refresh token: {}", e)))?;
    if refresh_token.expose().is_empty()
        || refresh_token.expose().len() > MAX_OAUTH_SECRET_BYTES
        || refresh_token.expose().chars().any(char::is_control)
    {
        return Err(AuthError::RefreshFailed(
            "stored refresh token is malformed or oversized".to_string(),
        ));
    }

    // Discover the token endpoint
    let (token_url, resource) = if let Some(ref oauth) = server_config.oauth {
        if let Some(ref url) = oauth.token_url {
            (
                url.clone(),
                oauth
                    .resource
                    .clone()
                    .or_else(|| Some(server_config.url.clone())),
            )
        } else {
            // Discover from server
            let bundle = discover_oauth_bundle_with_policy(&server_config.url, allow_local).await?;
            (
                bundle.authorization_server.token_endpoint,
                oauth.resource.clone().or_else(|| {
                    Some(resolve_resource_identifier(
                        &server_config.url,
                        &bundle.protected_resource,
                    ))
                }),
            )
        }
    } else {
        // DCR - always discover
        let bundle = discover_oauth_bundle_with_policy(&server_config.url, allow_local).await?;
        (
            bundle.authorization_server.token_endpoint,
            Some(resolve_resource_identifier(
                &server_config.url,
                &bundle.protected_resource,
            )),
        )
    };

    validate_trusted_oauth_endpoint(&token_url, allow_local, "token endpoint")
        .await
        .map_err(|error| AuthError::RefreshFailed(error.to_string()))?;
    if resource.as_ref().is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_AUTHORIZATION_URL_BYTES
            || value.chars().any(char::is_control)
    }) {
        return Err(AuthError::RefreshFailed(
            "OAuth protected resource is malformed or oversized".to_string(),
        ));
    }

    // F-02: pin the connection to the validated address for the token host.
    let client = super::build_pinned(
        reqwest::Client::builder().timeout(Duration::from_secs(30)),
        &token_url,
        allow_local,
    )
    .await
    .map_err(|error| AuthError::RefreshFailed(error.to_string()))?;

    let params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.expose().to_string()),
        ("client_id", client_id),
    ];
    let mut params = params;
    if let Some(resource) = resource {
        params.push(("resource", resource));
    }

    let response = client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| AuthError::RefreshFailed(e.without_url().to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let code = bounded_oauth_error_code(response).await;
        return Err(AuthError::RefreshFailed(status_with_oauth_code(
            status, code,
        )));
    }

    let token_response: TokenResponse = decode_bounded_json(response)
        .await
        .map_err(|e| AuthError::RefreshFailed(format!("Invalid response: {}", e)))?;

    let token = access_token_from_response(token_response)
        .map_err(|error| AuthError::RefreshFailed(format!("Invalid response: {error}")))?;

    // Store the new tokens
    store_tokens(secrets, user_id, server_config, &token).await?;

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_challenge_generation() {
        let pkce = PkceChallenge::generate();

        // Verifier should be base64url encoded
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.verifier.contains('+'));
        assert!(!pkce.verifier.contains('/'));
        assert!(!pkce.verifier.contains('='));

        // Challenge should be different from verifier
        assert_ne!(pkce.verifier, pkce.challenge);

        // Two challenges should be different
        let pkce2 = PkceChallenge::generate();
        assert_ne!(pkce.verifier, pkce2.verifier);
    }

    #[test]
    fn test_oauth_state_matches_constant_time() {
        let state = "a3f1c9d2e4b5a6071829304a5b6c7d8e";
        assert!(oauth_state_matches(state, state));
        assert!(!oauth_state_matches(state, "wrong"));
        assert!(!oauth_state_matches(state, ""));
        // Same length, single-byte difference must be rejected.
        let mut tampered = state.to_string();
        tampered.replace_range(0..1, if state.starts_with('a') { "b" } else { "a" });
        assert!(!oauth_state_matches(state, &tampered));
        // A longer received value with the expected as a prefix must be rejected.
        assert!(!oauth_state_matches(state, &format!("{state}extra")));
    }

    #[test]
    fn test_build_authorization_url() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &["read".to_string(), "write".to_string()],
            None,
            None,
            None,
            &HashMap::new(),
        )
        .unwrap();

        assert!(url.starts_with("https://auth.example.com/authorize?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "scope")
                .map(|(_, value)| value.into_owned()),
            Some("read write".to_string())
        );
    }

    #[test]
    fn test_build_authorization_url_with_pkce() {
        let pkce = PkceChallenge::generate();
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            Some(&pkce),
            None,
            None,
            &HashMap::new(),
        )
        .unwrap();

        assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn test_build_authorization_url_with_extra_params() {
        let mut extra = HashMap::new();
        extra.insert("owner".to_string(), "user".to_string());
        extra.insert("state".to_string(), "abc123".to_string());

        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            None,
            None,
            &extra,
        )
        .unwrap();

        assert!(url.contains("owner=user"));
        assert!(!url.contains("state=abc123"));
    }

    #[test]
    fn test_build_authorization_url_preserves_generated_state() {
        let mut extra = HashMap::new();
        extra.insert("state".to_string(), "override".to_string());
        extra.insert(
            "resource".to_string(),
            "https://wrong.example.com".to_string(),
        );

        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            Some("expected-state"),
            Some("https://resource.example.com"),
            &extra,
        )
        .unwrap();

        assert!(url.contains("state=expected-state"));
        assert!(url.contains("resource=https%3A%2F%2Fresource.example.com"));
        assert!(!url.contains("override"));
        assert!(!url.contains("wrong.example.com"));
    }

    #[test]
    fn authorization_url_rejects_unsafe_inputs_and_reserved_query_overrides() {
        let empty = HashMap::new();
        for base in [
            "javascript:alert(1)",
            "https://user:secret@auth.example.com/authorize",
            "https://auth.example.com/authorize#fragment",
        ] {
            assert!(
                build_authorization_url(
                    base,
                    "client",
                    "http://127.0.0.1:9876/callback",
                    &[],
                    None,
                    Some("state"),
                    None,
                    &empty,
                )
                .is_err()
            );
        }
        assert!(
            build_authorization_url(
                "https://auth.example.com/authorize",
                "client",
                "http://public.example.com/callback",
                &[],
                None,
                Some("state"),
                None,
                &empty,
            )
            .is_err()
        );
        assert!(
            build_authorization_url(
                "https://auth.example.com/authorize",
                "client",
                "https://app.example.com/callback",
                &[],
                None,
                Some("bad\nstate"),
                None,
                &empty,
            )
            .is_err()
        );

        let url = build_authorization_url(
            "https://auth.example.com/authorize?state=attacker&client_id=attacker&tenant=safe",
            "expected-client",
            "https://app.example.com/callback",
            &[],
            None,
            Some("expected-state"),
            None,
            &empty,
        )
        .unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let pairs = parsed.query_pairs().into_owned().collect::<Vec<_>>();
        assert_eq!(
            pairs
                .iter()
                .filter(|(key, _)| key == "state")
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["expected-state"]
        );
        assert_eq!(
            pairs
                .iter()
                .filter(|(key, _)| key == "client_id")
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["expected-client"]
        );
        assert!(
            pairs
                .iter()
                .any(|pair| pair == &("tenant".into(), "safe".into()))
        );
    }

    #[test]
    fn token_response_fields_are_strictly_validated() {
        let valid = TokenResponse {
            access_token: "access".to_string(),
            token_type: "bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("refresh".to_string()),
            scope: Some("read write".to_string()),
        };
        assert_eq!(
            access_token_from_response(valid).unwrap().token_type,
            "Bearer"
        );

        let wrong_type = TokenResponse {
            access_token: "access".to_string(),
            token_type: "mac".to_string(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        assert!(access_token_from_response(wrong_type).is_err());
        let injected = TokenResponse {
            access_token: "access\r\nX-Injected: yes".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        assert!(access_token_from_response(injected).is_err());

        let implausible_expiry = TokenResponse {
            access_token: "access".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(MAX_OAUTH_TOKEN_LIFETIME_SECS + 1),
            refresh_token: None,
            scope: None,
        };
        assert!(access_token_from_response(implausible_expiry).is_err());
    }

    #[tokio::test]
    async fn stored_access_tokens_preserve_expiry_and_expired_rows_are_not_authenticated() {
        use secrecy::SecretString;
        use thinclaw_secrets::{InMemorySecretsStore, SecretsCrypto};

        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from("mcp-test-master-key-32-bytes-long!!")).unwrap(),
        );
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));
        let config = McpServerConfig::new("expiry-test", "https://mcp.example.com");
        let before = chrono::Utc::now();
        let token = AccessToken {
            access_token: "access".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(120),
            refresh_token: Some("refresh".to_string()),
            scope: None,
        };

        store_tokens(&secrets, "user", &config, &token)
            .await
            .unwrap();
        let stored = secrets
            .get("user", &config.token_secret_name())
            .await
            .unwrap();
        let expires_at = stored.expires_at.expect("expiry must be persisted");
        assert!(expires_at >= before + chrono::Duration::seconds(119));
        assert!(expires_at <= chrono::Utc::now() + chrono::Duration::seconds(121));
        assert!(is_authenticated(&config, &secrets, "user").await);

        secrets
            .create(
                "user",
                CreateSecretParams::new(config.token_secret_name(), "expired")
                    .with_expiry(chrono::Utc::now() - chrono::Duration::seconds(1)),
            )
            .await
            .unwrap();
        assert!(!is_authenticated(&config, &secrets, "user").await);
    }

    #[tokio::test]
    async fn callback_slowloris_and_wrong_state_do_not_cancel_valid_flow() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let callback = tokio::spawn(wait_for_authorization_callback(
            listener,
            "test",
            Some("expected-state"),
            Arc::new(|_| "success".to_string()),
            Arc::new(|_| "failure".to_string()),
        ));

        // One peer deliberately never finishes its request line. A valid
        // callback must still be processed by another bounded handler.
        let _slow = tokio::net::TcpStream::connect(address).await.unwrap();

        let mut wrong = tokio::net::TcpStream::connect(address).await.unwrap();
        wrong
            .write_all(
                b"GET /callback?error=denied&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .unwrap();
        let mut wrong_response = Vec::new();
        wrong.read_to_end(&mut wrong_response).await.unwrap();
        assert!(wrong_response.starts_with(b"HTTP/1.1 400"));

        let mut valid = tokio::net::TcpStream::connect(address).await.unwrap();
        valid
            .write_all(
                b"GET /callback?code=valid-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), callback)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.code, "valid-code");
    }
}

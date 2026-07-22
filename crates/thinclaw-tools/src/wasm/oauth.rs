use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use rand::Rng;
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use thinclaw_secrets::{CreateSecretParams, SecretError, SecretsStore};
use thinclaw_tools_core::{
    GuardedUrl, OutboundUrlGuardOptions, validate_outbound_url_pinned_async,
};

use crate::wasm::{AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema};

pub const GOOGLE_OAUTH_TOKEN: &str = "google_oauth_token";
pub const LEGACY_GMAIL_OAUTH_TOKEN: &str = "gmail_oauth_token";

const MAX_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_OAUTH_ERROR_BYTES: usize = 16 * 1024;
const MAX_OAUTH_URL_BYTES: usize = 16 * 1024;
const MAX_OAUTH_PARAMETER_BYTES: usize = 8 * 1024;
const MAX_OAUTH_SECRET_BYTES: usize = 64 * 1024;
const MAX_OAUTH_ITEMS: usize = 256;
const MAX_OAUTH_EXTRA_PARAMETERS: usize = 32;

pub(super) async fn read_bounded_response(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, WasmToolOAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(WasmToolOAuthError::InvalidResponse(format!(
            "response exceeded the {max_bytes}-byte limit"
        )));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            WasmToolOAuthError::InvalidResponse(error.without_url().to_string())
        })?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(WasmToolOAuthError::InvalidResponse(format!(
                "response exceeded the {max_bytes}-byte limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn decode_bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, WasmToolOAuthError> {
    let body = read_bounded_response(response, MAX_OAUTH_RESPONSE_BYTES).await?;
    serde_json::from_slice(&body)
        .map_err(|error| WasmToolOAuthError::InvalidResponse(error.to_string()))
}

pub(super) async fn bounded_oauth_error(response: reqwest::Response) -> Option<String> {
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

pub(super) async fn validate_public_oauth_endpoint(
    value: &str,
    label: &str,
) -> Result<GuardedUrl, WasmToolOAuthError> {
    if value.is_empty() || value.len() > MAX_OAUTH_URL_BYTES {
        return Err(WasmToolOAuthError::Capabilities(format!(
            "{label} is empty or oversized"
        )));
    }
    let options = OutboundUrlGuardOptions {
        require_https: true,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    let guarded = validate_outbound_url_pinned_async(value, &options)
        .await
        .map_err(|error| WasmToolOAuthError::Capabilities(error.to_string()))?;
    if !guarded.url.username().is_empty()
        || guarded.url.password().is_some()
        || guarded.url.fragment().is_some()
    {
        return Err(WasmToolOAuthError::Capabilities(format!(
            "{label} contains credentials or a fragment"
        )));
    }
    Ok(guarded)
}

pub(super) fn oauth_client_for(
    endpoint: &GuardedUrl,
    timeout: Duration,
) -> Result<reqwest::Client, WasmToolOAuthError> {
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .redirect(Policy::none())
        .no_proxy();
    if !endpoint.pinned_addrs.is_empty() {
        let host = endpoint.url.host_str().ok_or_else(|| {
            WasmToolOAuthError::Capabilities("OAuth endpoint has no host".to_string())
        })?;
        builder = builder.resolve_to_addrs(host, &endpoint.pinned_addrs);
    }
    Ok(builder.build()?)
}

fn validate_redirect_uri(value: &str) -> Result<(), WasmToolOAuthError> {
    let parsed = url::Url::parse(value)
        .map_err(|error| WasmToolOAuthError::Capabilities(error.to_string()))?;
    if value.is_empty()
        || value.len() > MAX_OAUTH_URL_BYTES
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(WasmToolOAuthError::Capabilities(
            "OAuth redirect URI is malformed or oversized".to_string(),
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
        _ => Err(WasmToolOAuthError::Capabilities(
            "OAuth redirect URI must use HTTPS, except for an HTTP loopback callback".to_string(),
        )),
    }
}

fn validate_oauth_config_shape(
    auth: &AuthCapabilitySchema,
    oauth: &OAuthConfigSchema,
    client_id: &str,
    client_secret: Option<&str>,
    scopes: &[String],
) -> Result<(), WasmToolOAuthError> {
    for (label, endpoint) in [
        ("authorization endpoint", oauth.authorization_url.as_str()),
        ("token endpoint", oauth.token_url.as_str()),
    ] {
        let parsed = url::Url::parse(endpoint).map_err(|error| {
            WasmToolOAuthError::Capabilities(format!("invalid {label}: {error}"))
        })?;
        if endpoint.is_empty()
            || endpoint.len() > MAX_OAUTH_URL_BYTES
            || parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(WasmToolOAuthError::Capabilities(format!(
                "{label} must be a bounded HTTPS URL without credentials or fragments"
            )));
        }
    }
    if auth.secret_name.is_empty()
        || auth.secret_name.len() > 128
        || !auth
            .secret_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
        || client_secret.is_some_and(|secret| {
            secret.is_empty()
                || secret.len() > MAX_OAUTH_SECRET_BYTES
                || secret.chars().any(char::is_control)
        })
        || scopes.len() > MAX_OAUTH_ITEMS
        || scopes.iter().any(|scope| {
            scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
        })
        || oauth.extra_params.len() > MAX_OAUTH_EXTRA_PARAMETERS
        || oauth.extra_params.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
        })
        || oauth.access_token_field.is_empty()
        || oauth.access_token_field.len() > 128
        || !oauth
            .access_token_field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || (!oauth.use_pkce && client_secret.is_none())
    {
        return Err(WasmToolOAuthError::Capabilities(
            "OAuth capability contains malformed or oversized fields, or disables PKCE for a public client"
                .to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_oauth_secret_value(
    value: &str,
    label: &str,
) -> Result<(), WasmToolOAuthError> {
    if value.is_empty()
        || value.len() > MAX_OAUTH_SECRET_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(WasmToolOAuthError::InvalidResponse(format!(
            "{label} is empty, malformed, or oversized"
        )));
    }
    Ok(())
}

pub(super) fn validate_bearer_token_type(
    token_data: &serde_json::Value,
) -> Result<(), WasmToolOAuthError> {
    if token_data
        .get("token_type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.eq_ignore_ascii_case("Bearer"))
    {
        return Err(WasmToolOAuthError::InvalidResponse(
            "OAuth token type must be Bearer".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn oauth_expiry_from_response(
    token_data: &serde_json::Value,
) -> Result<Option<DateTime<Utc>>, WasmToolOAuthError> {
    let Some(value) = token_data.get("expires_in") else {
        return Ok(None);
    };
    let seconds = value.as_u64().ok_or_else(|| {
        WasmToolOAuthError::InvalidResponse("expires_in must be an unsigned integer".to_string())
    })?;
    let seconds = i64::try_from(seconds)
        .map_err(|_| WasmToolOAuthError::InvalidResponse("expires_in is too large".to_string()))?;
    Utc::now()
        .checked_add_signed(chrono::Duration::seconds(seconds))
        .map(Some)
        .ok_or_else(|| {
            WasmToolOAuthError::InvalidResponse(
                "expires_in overflows the timestamp range".to_string(),
            )
        })
}

#[cfg(feature = "wasm-runtime")]
pub(super) async fn validate_oauth_refresh_config(
    config: &OAuthRefreshConfig,
) -> Result<GuardedUrl, WasmToolOAuthError> {
    if config.client_id.is_empty()
        || config.client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || config.client_id.chars().any(char::is_control)
        || config.client_secret.as_ref().is_some_and(|secret| {
            secret.is_empty()
                || secret.len() > MAX_OAUTH_SECRET_BYTES
                || secret.chars().any(char::is_control)
        })
        || config.secret_name.is_empty()
        || config.secret_name.len() > 128
        || !config
            .secret_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || config.provider.as_ref().is_some_and(|provider| {
            provider.is_empty() || provider.len() > 128 || provider.chars().any(char::is_control)
        })
    {
        return Err(WasmToolOAuthError::Capabilities(
            "OAuth refresh configuration is malformed or oversized".to_string(),
        ));
    }
    validate_public_oauth_endpoint(&config.token_url, "token endpoint").await
}

pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

const GOOGLE_CLIENT_ID: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
const GOOGLE_CLIENT_SECRET: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};
const GITHUB_CLIENT_ID: &str = match option_env!("THINCLAW_GITHUB_CLIENT_ID") {
    Some(v) => v,
    None => "Ov23liIronClawGHApp01",
};
const GITHUB_CLIENT_SECRET: &str = match option_env!("THINCLAW_GITHUB_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};
const NOTION_CLIENT_ID: &str = match option_env!("THINCLAW_NOTION_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
const NOTION_CLIENT_SECRET: &str = match option_env!("THINCLAW_NOTION_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

pub fn builtin_credentials(secret_name: &str) -> Option<OAuthCredentials> {
    match secret_name {
        "google_oauth_token" | "gmail_oauth_token"
            if !GOOGLE_CLIENT_ID.is_empty() && !GOOGLE_CLIENT_SECRET.is_empty() =>
        {
            Some(OAuthCredentials {
                client_id: GOOGLE_CLIENT_ID,
                client_secret: GOOGLE_CLIENT_SECRET,
            })
        }
        "github_oauth_token"
            if !GITHUB_CLIENT_ID.is_empty() && !GITHUB_CLIENT_SECRET.is_empty() =>
        {
            Some(OAuthCredentials {
                client_id: GITHUB_CLIENT_ID,
                client_secret: GITHUB_CLIENT_SECRET,
            })
        }
        "notion_oauth_token"
            if !NOTION_CLIENT_ID.is_empty() && !NOTION_CLIENT_SECRET.is_empty() =>
        {
            Some(OAuthCredentials {
                client_id: NOTION_CLIENT_ID,
                client_secret: NOTION_CLIENT_SECRET,
            })
        }
        _ => None,
    }
}

fn resolve_oauth_client_pair(
    oauth: &OAuthConfigSchema,
    builtin: Option<&OAuthCredentials>,
) -> Result<(String, Option<String>), WasmToolOAuthError> {
    let configured_client_id = oauth.client_id.clone().or_else(|| {
        oauth
            .client_id_env
            .as_ref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|value| !value.trim().is_empty())
    });
    let configured_client_secret = oauth.client_secret.clone().or_else(|| {
        oauth
            .client_secret_env
            .as_ref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|value| !value.trim().is_empty())
    });

    match (configured_client_id, configured_client_secret) {
        (Some(client_id), client_secret) => Ok((client_id, client_secret)),
        (None, Some(_)) => Err(WasmToolOAuthError::MissingClientId),
        (None, None) => builtin
            .map(|credentials| {
                (
                    credentials.client_id.to_string(),
                    (!credentials.client_secret.trim().is_empty())
                        .then(|| credentials.client_secret.to_string()),
                )
            })
            .ok_or(WasmToolOAuthError::MissingClientId),
    }
}

/// Configuration needed to refresh an expired OAuth access token.
#[derive(Clone)]
pub struct OAuthRefreshConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub secret_name: String,
    pub provider: Option<String>,
}

impl std::fmt::Debug for OAuthRefreshConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthRefreshConfig")
            .field("token_url", &"[REDACTED URL]")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("secret_name", &self.secret_name)
            .field("provider", &self.provider)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmToolAuthMode {
    None,
    OAuth,
    ManualToken,
}

impl WasmToolAuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OAuth => "oauth",
            Self::ManualToken => "manual_token",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmToolAuthStatus {
    NoAuthRequired,
    Authenticated,
    AwaitingAuthorization,
    AwaitingToken,
    NeedsReauth,
    InsufficientScope,
}

impl WasmToolAuthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoAuthRequired => "no_auth_required",
            Self::Authenticated => "authenticated",
            Self::AwaitingAuthorization => "awaiting_authorization",
            Self::AwaitingToken => "awaiting_token",
            Self::NeedsReauth => "needs_reauth",
            Self::InsufficientScope => "insufficient_scope",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WasmToolAuthCheck {
    pub auth_mode: WasmToolAuthMode,
    pub auth_status: WasmToolAuthStatus,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

impl WasmToolAuthCheck {
    pub fn no_auth_required() -> Self {
        Self {
            auth_mode: WasmToolAuthMode::None,
            auth_status: WasmToolAuthStatus::NoAuthRequired,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
        }
    }

    pub fn authenticated(mode: WasmToolAuthMode, provider: Option<String>) -> Self {
        Self {
            auth_mode: mode,
            auth_status: WasmToolAuthStatus::Authenticated,
            shared_auth_provider: provider,
            missing_scopes: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct ResolvedOAuthConfig {
    pub oauth: OAuthConfigSchema,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub required_scopes: Vec<String>,
    pub shared_auth_provider: Option<String>,
}

impl std::fmt::Debug for ResolvedOAuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedOAuthConfig")
            .field("authorization_url", &"[REDACTED URL]")
            .field("token_url", &"[REDACTED URL]")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("required_scopes", &self.required_scopes)
            .field("shared_auth_provider", &self.shared_auth_provider)
            .finish()
    }
}

#[derive(Clone)]
pub struct OAuthPkcePair {
    pub verifier: String,
    pub challenge: String,
}

impl std::fmt::Debug for OAuthPkcePair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthPkcePair")
            .field("verifier", &"[REDACTED]")
            .field("challenge", &self.challenge)
            .finish()
    }
}

#[derive(Clone)]
pub struct WasmToolAuthorizationRequest {
    pub auth_url: String,
    pub callback_type: String,
    pub redirect_uri: String,
    pub code_verifier: Option<String>,
    pub auth_mode: WasmToolAuthMode,
    pub auth_status: WasmToolAuthStatus,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

impl std::fmt::Debug for WasmToolAuthorizationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WasmToolAuthorizationRequest")
            .field("auth_url", &"[REDACTED]")
            .field("callback_type", &self.callback_type)
            .field("redirect_uri", &"[REDACTED URL]")
            .field(
                "code_verifier",
                &self.code_verifier.as_ref().map(|_| "[REDACTED]"),
            )
            .field("auth_mode", &self.auth_mode)
            .field("auth_status", &self.auth_status)
            .field("shared_auth_provider", &self.shared_auth_provider)
            .field("missing_scopes", &self.missing_scopes)
            .finish()
    }
}

#[derive(Clone)]
pub struct WasmOAuthTokenExchange {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub granted_scopes: Vec<String>,
    pub raw: serde_json::Value,
}

impl std::fmt::Debug for WasmOAuthTokenExchange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WasmOAuthTokenExchange")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("granted_scopes", &self.granted_scopes)
            .field("raw", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WasmToolOAuthError {
    #[error("Missing OAuth client_id configuration")]
    MissingClientId,

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Secrets error: {0}")]
    Secrets(#[from] SecretError),

    #[error("Capabilities error: {0}")]
    Capabilities(String),

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Invalid OAuth response: {0}")]
    InvalidResponse(String),
}

pub struct WasmToolOAuthFlow<'a> {
    secrets: &'a (dyn SecretsStore + Send + Sync),
    user_id: &'a str,
    tools_dir: &'a Path,
}

impl<'a> WasmToolOAuthFlow<'a> {
    pub fn new(
        secrets: &'a (dyn SecretsStore + Send + Sync),
        user_id: &'a str,
        tools_dir: &'a Path,
    ) -> Self {
        Self {
            secrets,
            user_id,
            tools_dir,
        }
    }

    pub async fn combined_oauth_config(
        &self,
        auth: &AuthCapabilitySchema,
    ) -> Result<Option<ResolvedOAuthConfig>, WasmToolOAuthError> {
        let Some(oauth) = auth.oauth.as_ref() else {
            return Ok(None);
        };

        let mut required_scopes = self
            .collect_shared_scopes(&auth.secret_name, &oauth.scopes)
            .await;
        required_scopes.sort();
        required_scopes.dedup();

        let builtin = builtin_credentials(&canonical_secret_name(&auth.secret_name));

        let (client_id, client_secret) = resolve_oauth_client_pair(oauth, builtin.as_ref())?;

        let mut merged_oauth = oauth.clone();
        merged_oauth.scopes = required_scopes.clone();
        validate_oauth_config_shape(
            auth,
            &merged_oauth,
            &client_id,
            client_secret.as_deref(),
            &required_scopes,
        )?;

        Ok(Some(ResolvedOAuthConfig {
            oauth: merged_oauth,
            client_id,
            client_secret,
            required_scopes,
            shared_auth_provider: shared_auth_provider(auth),
        }))
    }

    pub async fn check_auth_status(
        &self,
        auth: &AuthCapabilitySchema,
    ) -> Result<WasmToolAuthCheck, WasmToolOAuthError> {
        let Some(resolved) = self.combined_oauth_config(auth).await? else {
            let mode = if auth.instructions.is_some()
                || auth.setup_url.is_some()
                || auth.env_var.is_some()
            {
                WasmToolAuthMode::ManualToken
            } else {
                WasmToolAuthMode::None
            };

            if mode == WasmToolAuthMode::None {
                return Ok(WasmToolAuthCheck::no_auth_required());
            }

            if self
                .find_access_token_secret_name(&auth.secret_name)
                .await?
                .is_some()
            {
                return Ok(WasmToolAuthCheck::authenticated(
                    mode,
                    shared_auth_provider(auth),
                ));
            }

            return Ok(WasmToolAuthCheck {
                auth_mode: mode,
                auth_status: WasmToolAuthStatus::AwaitingToken,
                shared_auth_provider: shared_auth_provider(auth),
                missing_scopes: Vec::new(),
            });
        };

        let Some(existing_secret_name) = self
            .find_access_token_secret_name(&auth.secret_name)
            .await?
        else {
            return Ok(WasmToolAuthCheck {
                auth_mode: WasmToolAuthMode::OAuth,
                auth_status: WasmToolAuthStatus::AwaitingAuthorization,
                shared_auth_provider: resolved.shared_auth_provider,
                missing_scopes: Vec::new(),
            });
        };

        let granted_scopes = match self
            .load_granted_scopes_for_secret(&existing_secret_name)
            .await?
        {
            Some(scopes) => Some(scopes),
            None if is_google_secret_name(&auth.secret_name) => {
                let access_token = self
                    .secrets
                    .get_for_injection(
                        self.user_id,
                        &existing_secret_name,
                        thinclaw_secrets::SecretAccessContext::new("wasm.oauth", "scope_discovery"),
                    )
                    .await?;
                match discover_google_token_scopes(access_token.expose()).await {
                    Ok(scopes) if !scopes.is_empty() => {
                        self.store_scope_metadata(auth, &scopes).await?;
                        Some(scopes)
                    }
                    _ => None,
                }
            }
            None => None,
        };

        if !resolved.required_scopes.is_empty() {
            let Some(granted_scopes) = granted_scopes else {
                return Ok(WasmToolAuthCheck {
                    auth_mode: WasmToolAuthMode::OAuth,
                    auth_status: WasmToolAuthStatus::NeedsReauth,
                    shared_auth_provider: resolved.shared_auth_provider,
                    missing_scopes: resolved.required_scopes.clone(),
                });
            };

            let missing_scopes = difference(&resolved.required_scopes, &granted_scopes);
            if !missing_scopes.is_empty() {
                return Ok(WasmToolAuthCheck {
                    auth_mode: WasmToolAuthMode::OAuth,
                    auth_status: WasmToolAuthStatus::NeedsReauth,
                    shared_auth_provider: resolved.shared_auth_provider,
                    missing_scopes,
                });
            }
        }

        if existing_secret_name != canonical_secret_name(&auth.secret_name) {
            self.migrate_google_secret_alias(auth, &existing_secret_name)
                .await?;
        }

        Ok(WasmToolAuthCheck::authenticated(
            WasmToolAuthMode::OAuth,
            resolved.shared_auth_provider,
        ))
    }

    pub async fn prepare_authorization(
        &self,
        auth: &AuthCapabilitySchema,
        redirect_uri: &str,
        callback_type: impl Into<String>,
        state_nonce: Option<&str>,
    ) -> Result<WasmToolAuthorizationRequest, WasmToolOAuthError> {
        let resolved = self.combined_oauth_config(auth).await?.ok_or_else(|| {
            WasmToolOAuthError::Capabilities("Missing auth.oauth block".to_string())
        })?;
        validate_redirect_uri(redirect_uri)?;
        let state_nonce = state_nonce.ok_or_else(|| {
            WasmToolOAuthError::Capabilities("OAuth state nonce is required".to_string())
        })?;
        if state_nonce.is_empty()
            || state_nonce.len() > MAX_OAUTH_PARAMETER_BYTES
            || state_nonce.chars().any(char::is_control)
        {
            return Err(WasmToolOAuthError::Capabilities(
                "OAuth state nonce is malformed or oversized".to_string(),
            ));
        }
        validate_public_oauth_endpoint(&resolved.oauth.authorization_url, "authorization endpoint")
            .await?;
        let current = self.check_auth_status(auth).await?;
        let pkce = resolved.oauth.use_pkce.then(generate_pkce_pair);

        let auth_url = build_authorization_url(
            &resolved.oauth,
            &resolved.client_id,
            redirect_uri,
            &resolved.required_scopes,
            pkce.as_ref(),
            Some(state_nonce),
        )?;

        Ok(WasmToolAuthorizationRequest {
            auth_url,
            callback_type: callback_type.into(),
            redirect_uri: redirect_uri.to_string(),
            code_verifier: pkce.map(|pair| pair.verifier),
            auth_mode: WasmToolAuthMode::OAuth,
            auth_status: if current.auth_status == WasmToolAuthStatus::NeedsReauth {
                WasmToolAuthStatus::NeedsReauth
            } else {
                WasmToolAuthStatus::AwaitingAuthorization
            },
            shared_auth_provider: resolved.shared_auth_provider,
            missing_scopes: current.missing_scopes,
        })
    }

    pub async fn exchange_code(
        &self,
        auth: &AuthCapabilitySchema,
        redirect_uri: &str,
        code: &str,
        code_verifier: Option<&str>,
    ) -> Result<WasmOAuthTokenExchange, WasmToolOAuthError> {
        let resolved = self.combined_oauth_config(auth).await?.ok_or_else(|| {
            WasmToolOAuthError::Capabilities("Missing auth.oauth block".to_string())
        })?;
        validate_redirect_uri(redirect_uri)?;
        if code.is_empty()
            || code.len() > MAX_OAUTH_PARAMETER_BYTES
            || code.chars().any(char::is_control)
            || (resolved.oauth.use_pkce
                && code_verifier.is_none_or(|verifier| {
                    verifier.len() < 43
                        || verifier.len() > 128
                        || !verifier.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(byte, b'-' | b'.' | b'_' | b'~')
                        })
                }))
            || (!resolved.oauth.use_pkce && code_verifier.is_some())
        {
            return Err(WasmToolOAuthError::TokenExchange(
                "authorization code or PKCE verifier is malformed".to_string(),
            ));
        }
        let endpoint =
            validate_public_oauth_endpoint(&resolved.oauth.token_url, "token endpoint").await?;
        let client = oauth_client_for(&endpoint, Duration::from_secs(30))?;

        let mut token_params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
        ];

        if let Some(verifier) = code_verifier {
            token_params.push(("code_verifier", verifier.to_string()));
        }

        let mut request = client.post(endpoint.url.clone());
        if let Some(ref secret) = resolved.client_secret {
            request = request.basic_auth(&resolved.client_id, Some(secret));
        } else {
            token_params.push(("client_id", resolved.client_id.clone()));
        }

        let response = request.form(&token_params).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let code = bounded_oauth_error(response).await;
            return Err(WasmToolOAuthError::TokenExchange(format!(
                "HTTP {status}{}",
                code.map(|code| format!(" (OAuth error `{code}`)"))
                    .unwrap_or_default()
            )));
        }

        let token_data: serde_json::Value = decode_bounded_json(response).await?;
        let access_token = token_data
            .get(&resolved.oauth.access_token_field)
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                WasmToolOAuthError::InvalidResponse(format!(
                    "missing {} field",
                    resolved.oauth.access_token_field
                ))
            })?;
        validate_oauth_secret_value(access_token, "access token")?;
        validate_bearer_token_type(&token_data)?;
        let access_token = access_token.to_string();

        let refresh_token = token_data
            .get("refresh_token")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        if let Some(value) = refresh_token.as_deref() {
            validate_oauth_secret_value(value, "refresh token")?;
        }
        let expires_at = oauth_expiry_from_response(&token_data)?;
        let granted_scopes = granted_scopes_from_response(&token_data, &resolved.required_scopes);
        if granted_scopes.len() > MAX_OAUTH_ITEMS
            || granted_scopes.iter().any(|scope| {
                scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
            })
        {
            return Err(WasmToolOAuthError::InvalidResponse(
                "granted scopes are malformed or excessive".to_string(),
            ));
        }

        Ok(WasmOAuthTokenExchange {
            access_token,
            refresh_token,
            expires_at,
            granted_scopes,
            raw: token_data,
        })
    }

    pub async fn store_token_exchange(
        &self,
        auth: &AuthCapabilitySchema,
        token: &WasmOAuthTokenExchange,
    ) -> Result<(), WasmToolOAuthError> {
        let canonical_name = canonical_secret_name(&auth.secret_name);
        if token.access_token.is_empty()
            || token.access_token.len() > MAX_OAUTH_SECRET_BYTES
            || token.access_token.chars().any(char::is_control)
            || token.refresh_token.as_ref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > MAX_OAUTH_SECRET_BYTES
                    || value.chars().any(char::is_control)
            })
            || token.granted_scopes.len() > MAX_OAUTH_ITEMS
            || token.granted_scopes.iter().any(|scope| {
                scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
            })
        {
            return Err(WasmToolOAuthError::InvalidResponse(
                "refusing to persist malformed or oversized OAuth fields".to_string(),
            ));
        }

        let resolved = self.combined_oauth_config(auth).await?.ok_or_else(|| {
            WasmToolOAuthError::Capabilities("Missing auth.oauth block".to_string())
        })?;
        if resolved.client_id.is_empty()
            || resolved.client_id.len() > MAX_OAUTH_SECRET_BYTES
            || resolved.client_id.chars().any(char::is_control)
            || resolved.client_secret.as_ref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > MAX_OAUTH_SECRET_BYTES
                    || value.chars().any(char::is_control)
            })
        {
            return Err(WasmToolOAuthError::Capabilities(
                "OAuth client credentials are malformed or oversized".to_string(),
            ));
        }

        // Persist supporting client/refresh/scope state before publishing the new
        // access token. A partial failure therefore leaves the old access token
        // authoritative instead of exposing a new token with incomplete refresh
        // metadata.
        let provider = auth.provider.clone().or_else(|| shared_auth_provider(auth));
        let mut client_id_params =
            CreateSecretParams::new(client_id_secret_name(&canonical_name), &resolved.client_id);
        if let Some(ref provider) = provider {
            client_id_params = client_id_params.with_provider(provider.clone());
        }
        self.secrets.create(self.user_id, client_id_params).await?;

        if let Some(client_secret) = resolved.client_secret.as_deref() {
            let mut client_secret_params =
                CreateSecretParams::new(client_secret_secret_name(&canonical_name), client_secret);
            if let Some(ref provider) = provider {
                client_secret_params = client_secret_params.with_provider(provider.clone());
            }
            self.secrets
                .create(self.user_id, client_secret_params)
                .await?;
        } else {
            self.secrets
                .delete(self.user_id, &client_secret_secret_name(&canonical_name))
                .await?;
        }

        if let Some(refresh_token) = token.refresh_token.as_deref() {
            let mut refresh_params =
                CreateSecretParams::new(refresh_secret_name(&canonical_name), refresh_token);
            if let Some(ref provider) = provider {
                refresh_params = refresh_params.with_provider(provider.clone());
            }
            self.secrets.create(self.user_id, refresh_params).await?;
        }

        if !token.granted_scopes.is_empty() {
            self.store_scope_metadata(auth, &token.granted_scopes)
                .await?;
        }

        let mut access_params = CreateSecretParams::new(&canonical_name, &token.access_token);
        if let Some(ref provider) = provider {
            access_params = access_params.with_provider(provider.clone());
        }
        if let Some(expires_at) = token.expires_at.as_ref().cloned() {
            access_params = access_params.with_expiry(expires_at);
        }
        self.secrets.create(self.user_id, access_params).await?;

        Ok(())
    }

    pub async fn store_manual_token(
        &self,
        auth: &AuthCapabilitySchema,
        token: &str,
    ) -> Result<(), WasmToolOAuthError> {
        if token.is_empty()
            || token.len() > MAX_OAUTH_SECRET_BYTES
            || token.chars().any(char::is_control)
        {
            return Err(WasmToolOAuthError::InvalidResponse(
                "manual token is empty, malformed, or oversized".to_string(),
            ));
        }
        if auth.secret_name.is_empty()
            || auth.secret_name.len() > 128
            || !auth
                .secret_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(WasmToolOAuthError::Capabilities(
                "secret name is malformed".to_string(),
            ));
        }
        let canonical_name = canonical_secret_name(&auth.secret_name);
        let mut params = CreateSecretParams::new(canonical_name, token);
        if let Some(ref provider) = auth.provider {
            params = params.with_provider(provider.clone());
        } else if let Some(provider) = shared_auth_provider(auth) {
            params = params.with_provider(provider);
        }
        self.secrets.create(self.user_id, params).await?;
        Ok(())
    }

    pub async fn store_scope_metadata(
        &self,
        auth: &AuthCapabilitySchema,
        granted_scopes: &[String],
    ) -> Result<(), WasmToolOAuthError> {
        if granted_scopes.len() > MAX_OAUTH_ITEMS
            || granted_scopes.iter().any(|scope| {
                scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
            })
        {
            return Err(WasmToolOAuthError::InvalidResponse(
                "scope metadata is malformed or excessive".to_string(),
            ));
        }
        let canonical_name = canonical_secret_name(&auth.secret_name);
        let json = serde_json::to_string(&normalize_scopes(granted_scopes.iter().cloned()))
            .map_err(|error| WasmToolOAuthError::InvalidResponse(error.to_string()))?;
        let mut params = CreateSecretParams::new(scopes_secret_name(canonical_name), json);
        if let Some(ref provider) = auth.provider {
            params = params.with_provider(provider.clone());
        } else if let Some(provider) = shared_auth_provider(auth) {
            params = params.with_provider(provider);
        }
        self.secrets.create(self.user_id, params).await?;
        Ok(())
    }

    async fn collect_shared_scopes(
        &self,
        secret_name: &str,
        base_scopes: &[String],
    ) -> Vec<String> {
        let mut scopes: HashSet<String> = base_scopes.iter().cloned().collect();
        let secret_names: HashSet<String> = secret_lookup_names(secret_name).into_iter().collect();

        let Ok(mut entries) = tokio::fs::read_dir(self.tools_dir).await else {
            return normalize_scopes(scopes);
        };

        let mut inspected = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            inspected = inspected.saturating_add(1);
            if inspected > 1024 || scopes.len() >= MAX_OAUTH_ITEMS {
                break;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .ends_with(".capabilities.json")
            {
                continue;
            }

            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_file() || file_type.is_symlink() {
                continue;
            }
            let Ok(file) = tokio::fs::File::open(&path).await else {
                continue;
            };
            let Ok(metadata) = file.metadata().await else {
                continue;
            };
            if metadata.len() > MAX_OAUTH_RESPONSE_BYTES as u64 {
                continue;
            }
            let mut content = Vec::with_capacity(metadata.len() as usize);
            if file
                .take((MAX_OAUTH_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut content)
                .await
                .is_err()
                || content.len() > MAX_OAUTH_RESPONSE_BYTES
            {
                continue;
            }
            let Ok(caps) = CapabilitiesFile::from_bytes(&content) else {
                continue;
            };
            let Some(other_auth) = caps.auth else {
                continue;
            };
            if !secret_names.contains(other_auth.secret_name.as_str()) {
                continue;
            }
            if let Some(other_oauth) = other_auth.oauth {
                for scope in other_oauth.scopes {
                    if scopes.len() >= MAX_OAUTH_ITEMS {
                        break;
                    }
                    if !scope.is_empty()
                        && scope.len() <= 1024
                        && !scope.chars().any(char::is_control)
                    {
                        scopes.insert(scope);
                    }
                }
            }
        }

        normalize_scopes(scopes)
    }

    async fn find_access_token_secret_name(
        &self,
        secret_name: &str,
    ) -> Result<Option<String>, WasmToolOAuthError> {
        for candidate in secret_lookup_names(secret_name) {
            match self.secrets.get(self.user_id, &candidate).await {
                Ok(_) => return Ok(Some(candidate)),
                Err(SecretError::Expired) => {
                    if self
                        .secrets
                        .exists(self.user_id, &refresh_secret_name(&candidate))
                        .await
                        .unwrap_or(false)
                    {
                        return Ok(Some(candidate));
                    }
                }
                Err(SecretError::NotFound(_)) => continue,
                Err(error) => return Err(error.into()),
            }
        }

        Ok(None)
    }

    async fn load_granted_scopes_for_secret(
        &self,
        secret_name: &str,
    ) -> Result<Option<Vec<String>>, WasmToolOAuthError> {
        for candidate in secret_lookup_names(secret_name) {
            match self
                .secrets
                .get_for_injection(
                    self.user_id,
                    &scopes_secret_name(&candidate),
                    thinclaw_secrets::SecretAccessContext::new("wasm.oauth", "scope_metadata_read"),
                )
                .await
            {
                Ok(value) => return Ok(Some(parse_scopes_value(value.expose()))),
                Err(SecretError::NotFound(_)) | Err(SecretError::Expired) => continue,
                Err(error) => return Err(error.into()),
            }
        }

        Ok(None)
    }

    async fn migrate_google_secret_alias(
        &self,
        auth: &AuthCapabilitySchema,
        existing_secret_name: &str,
    ) -> Result<(), WasmToolOAuthError> {
        if !is_google_secret_name(existing_secret_name) {
            return Ok(());
        }

        let canonical = canonical_secret_name(existing_secret_name);
        if existing_secret_name == canonical {
            return Ok(());
        }

        let existing = self
            .secrets
            .get_for_injection(
                self.user_id,
                existing_secret_name,
                thinclaw_secrets::SecretAccessContext::new("wasm.oauth", "secret_alias_migration"),
            )
            .await?;
        let mut params = CreateSecretParams::new(&canonical, existing.expose());
        if let Some(ref provider) = auth.provider {
            params = params.with_provider(provider.clone());
        } else {
            params = params.with_provider("google");
        }
        self.secrets.create(self.user_id, params).await?;

        if let Ok(refresh) = self
            .secrets
            .get_for_injection(
                self.user_id,
                &refresh_secret_name(existing_secret_name),
                thinclaw_secrets::SecretAccessContext::new(
                    "wasm.oauth",
                    "refresh_secret_alias_migration",
                ),
            )
            .await
        {
            let refresh_params =
                CreateSecretParams::new(refresh_secret_name(&canonical), refresh.expose())
                    .with_provider("google");
            self.secrets.create(self.user_id, refresh_params).await?;
        }

        if let Some(scopes) = self
            .load_granted_scopes_for_secret(existing_secret_name)
            .await?
        {
            self.store_scope_metadata(auth, &scopes).await?;
        }

        Ok(())
    }
}

pub fn build_authorization_url(
    oauth: &OAuthConfigSchema,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    pkce: Option<&OAuthPkcePair>,
    state_nonce: Option<&str>,
) -> Result<String, WasmToolOAuthError> {
    validate_redirect_uri(redirect_uri)?;
    let state_nonce = state_nonce.ok_or_else(|| {
        WasmToolOAuthError::Capabilities("OAuth state nonce is required".to_string())
    })?;
    if oauth.use_pkce && pkce.is_none() {
        return Err(WasmToolOAuthError::Capabilities(
            "S256 PKCE is required by this OAuth capability".to_string(),
        ));
    }
    if !oauth.use_pkce && pkce.is_some() {
        return Err(WasmToolOAuthError::Capabilities(
            "PKCE parameters were supplied to a capability that disables PKCE".to_string(),
        ));
    }
    if client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
        || state_nonce.is_empty()
        || state_nonce.len() > MAX_OAUTH_PARAMETER_BYTES
        || state_nonce.chars().any(char::is_control)
        || scopes.len() > MAX_OAUTH_ITEMS
        || oauth.extra_params.len() > MAX_OAUTH_EXTRA_PARAMETERS
        || pkce.is_some_and(|pkce| {
            pkce.challenge.is_empty()
                || pkce.challenge.len() > 128
                || !pkce
                    .challenge
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return Err(WasmToolOAuthError::Capabilities(
            "authorization parameters are malformed or oversized".to_string(),
        ));
    }
    let scope = scopes.join(" ");
    if scope.len() > MAX_OAUTH_PARAMETER_BYTES
        || scopes.iter().any(|value| {
            value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
        })
    {
        return Err(WasmToolOAuthError::Capabilities(
            "OAuth scopes are malformed or oversized".to_string(),
        ));
    }

    let mut url = url::Url::parse(&oauth.authorization_url).map_err(|error| {
        WasmToolOAuthError::Capabilities(format!("invalid authorization endpoint: {error}"))
    })?;
    if oauth.authorization_url.len() > MAX_OAUTH_URL_BYTES
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(WasmToolOAuthError::Capabilities(
            "authorization endpoint is malformed or oversized".to_string(),
        ));
    }
    const RESERVED: &[&str] = &[
        "client_id",
        "response_type",
        "redirect_uri",
        "scope",
        "state",
        "code_challenge",
        "code_challenge_method",
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
        return Err(WasmToolOAuthError::Capabilities(
            "authorization endpoint query is malformed or excessive".to_string(),
        ));
    }
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in existing {
            query.append_pair(&key, &value);
        }
        query.append_pair("client_id", client_id);
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", redirect_uri);
        if !scope.is_empty() {
            query.append_pair("scope", &scope);
        }
        query.append_pair("state", state_nonce);
        if let Some(pkce) = pkce {
            query.append_pair("code_challenge", &pkce.challenge);
            query.append_pair("code_challenge_method", "S256");
        }
        for (key, value) in &oauth.extra_params {
            if RESERVED.contains(&key.as_str()) {
                continue;
            }
            if key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
            {
                return Err(WasmToolOAuthError::Capabilities(
                    "OAuth extra parameter is malformed or oversized".to_string(),
                ));
            }
            query.append_pair(key, value);
        }
    }
    let result = url.to_string();
    if result.len() > MAX_OAUTH_URL_BYTES {
        return Err(WasmToolOAuthError::Capabilities(
            "authorization URL exceeds the output limit".to_string(),
        ));
    }
    Ok(result)
}

pub fn generate_pkce_pair() -> OAuthPkcePair {
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    OAuthPkcePair {
        verifier,
        challenge,
    }
}

pub fn resolve_oauth_refresh_config(cap_file: &CapabilitiesFile) -> Option<OAuthRefreshConfig> {
    let auth = cap_file.auth.as_ref()?;
    let oauth = auth.oauth.as_ref()?;
    let builtin = builtin_credentials(&canonical_secret_name(&auth.secret_name));

    let (client_id, client_secret) = resolve_oauth_client_pair(oauth, builtin.as_ref()).ok()?;

    Some(OAuthRefreshConfig {
        token_url: oauth.token_url.clone(),
        client_id,
        client_secret,
        secret_name: canonical_secret_name(&auth.secret_name),
        provider: auth.provider.clone().or_else(|| shared_auth_provider(auth)),
    })
}

pub fn canonical_secret_name(secret_name: &str) -> String {
    if is_google_secret_name(secret_name) {
        GOOGLE_OAUTH_TOKEN.to_string()
    } else {
        secret_name.to_string()
    }
}

pub fn refresh_secret_name(secret_name: impl AsRef<str>) -> String {
    format!(
        "{}_refresh_token",
        canonical_secret_name(secret_name.as_ref())
    )
}

pub fn client_id_secret_name(secret_name: impl AsRef<str>) -> String {
    format!("{}_client_id", canonical_secret_name(secret_name.as_ref()))
}

pub fn client_secret_secret_name(secret_name: impl AsRef<str>) -> String {
    format!(
        "{}_client_secret",
        canonical_secret_name(secret_name.as_ref())
    )
}

pub fn scopes_secret_name(secret_name: impl AsRef<str>) -> String {
    format!("{}_scopes", canonical_secret_name(secret_name.as_ref()))
}

pub fn shared_auth_provider(auth: &AuthCapabilitySchema) -> Option<String> {
    auth.provider.clone().or_else(|| {
        if is_google_secret_name(&auth.secret_name) {
            Some("google".to_string())
        } else {
            None
        }
    })
}

pub fn is_google_secret_name(secret_name: &str) -> bool {
    matches!(secret_name, GOOGLE_OAUTH_TOKEN | LEGACY_GMAIL_OAUTH_TOKEN)
}

fn secret_lookup_names(secret_name: &str) -> Vec<String> {
    if is_google_secret_name(secret_name) {
        vec![
            GOOGLE_OAUTH_TOKEN.to_string(),
            LEGACY_GMAIL_OAUTH_TOKEN.to_string(),
        ]
    } else {
        vec![secret_name.to_string()]
    }
}

fn granted_scopes_from_response(
    token_data: &serde_json::Value,
    fallback_scopes: &[String],
) -> Vec<String> {
    if let Some(scope_value) = token_data.get("scope").and_then(|value| value.as_str()) {
        return parse_scopes_value(scope_value);
    }

    if let Some(scope_values) = token_data.get("scopes").and_then(|value| value.as_array()) {
        return normalize_scopes(
            scope_values
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>(),
        );
    }

    normalize_scopes(fallback_scopes.iter().cloned())
}

fn parse_scopes_value(value: &str) -> Vec<String> {
    if value.trim().is_empty() {
        return Vec::new();
    }

    if let Ok(json_scopes) = serde_json::from_str::<Vec<String>>(value) {
        return normalize_scopes(json_scopes);
    }

    normalize_scopes(
        value
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    )
}

fn difference(required_scopes: &[String], granted_scopes: &[String]) -> Vec<String> {
    let granted: HashSet<&str> = granted_scopes.iter().map(String::as_str).collect();
    required_scopes
        .iter()
        .filter(|scope| !granted.contains(scope.as_str()))
        .cloned()
        .collect()
}

fn normalize_scopes<I>(scopes: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut values = scopes
        .into_iter()
        .filter(|scope| !scope.trim().is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

async fn discover_google_token_scopes(token: &str) -> Result<Vec<String>, WasmToolOAuthError> {
    if token.is_empty()
        || token.len() > MAX_OAUTH_SECRET_BYTES
        || token.chars().any(char::is_control)
    {
        return Err(WasmToolOAuthError::InvalidResponse(
            "access token is malformed or oversized".to_string(),
        ));
    }
    let endpoint = validate_public_oauth_endpoint(
        "https://oauth2.googleapis.com/tokeninfo",
        "Google tokeninfo endpoint",
    )
    .await?;
    let client = oauth_client_for(&endpoint, Duration::from_secs(10))?;
    let response = client.get(endpoint.url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(WasmToolOAuthError::TokenExchange(format!(
            "tokeninfo returned {}",
            response.status()
        )));
    }

    let json: serde_json::Value = decode_bounded_json(response).await?;
    let scopes = json
        .get("scope")
        .and_then(|value| value.as_str())
        .map(parse_scopes_value)
        .unwrap_or_default();
    if scopes.len() > MAX_OAUTH_ITEMS
        || scopes.iter().any(|scope| {
            scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
        })
    {
        return Err(WasmToolOAuthError::InvalidResponse(
            "tokeninfo scope response is malformed or excessive".to_string(),
        ));
    }
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use secrecy::SecretString;
    use thinclaw_secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};

    use super::*;

    #[test]
    fn canonical_google_secret_is_shared() {
        assert_eq!(
            canonical_secret_name(GOOGLE_OAUTH_TOKEN),
            GOOGLE_OAUTH_TOKEN
        );
        assert_eq!(
            canonical_secret_name(LEGACY_GMAIL_OAUTH_TOKEN),
            GOOGLE_OAUTH_TOKEN
        );
    }

    #[test]
    fn parse_scopes_supports_json_and_space_delimited() {
        assert_eq!(
            parse_scopes_value(r#"["b","a","b"]"#),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            parse_scopes_value("b a b"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn build_authorization_url_includes_state_and_pkce() {
        let oauth = OAuthConfigSchema {
            authorization_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            access_token_field: "access_token".to_string(),
            scopes: vec![],
            use_pkce: true,
            extra_params: HashMap::from([("access_type".to_string(), "offline".to_string())]),
            ..OAuthConfigSchema::default()
        };

        let pkce = OAuthPkcePair {
            verifier: "verifier".to_string(),
            challenge: "challenge".to_string(),
        };
        let url = build_authorization_url(
            &oauth,
            "client-id",
            "http://localhost/callback",
            &["scope-a".to_string(), "scope-b".to_string()],
            Some(&pkce),
            Some("nonce"),
        )
        .unwrap();

        assert!(url.contains("client_id=client-id"));
        assert!(url.contains("state=nonce"));
        assert!(url.contains("code_challenge=challenge"));
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "scope")
                .map(|(_, value)| value.into_owned()),
            Some("scope-a scope-b".to_string())
        );
        assert!(url.contains("access_type=offline"));
    }

    #[tokio::test]
    async fn token_exchange_persists_refresh_client_and_scope_metadata() {
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .expect("test crypto"),
        );
        let store = InMemorySecretsStore::new(crypto);
        store
            .create(
                "user",
                CreateSecretParams::new(
                    client_secret_secret_name(GOOGLE_OAUTH_TOKEN),
                    "stale-secret",
                ),
            )
            .await
            .expect("seed stale client secret");
        let tools = tempfile::tempdir().expect("tools directory");
        let flow = WasmToolOAuthFlow::new(&store, "user", tools.path());
        let auth = AuthCapabilitySchema {
            secret_name: GOOGLE_OAUTH_TOKEN.to_string(),
            provider: Some("google".to_string()),
            oauth: Some(OAuthConfigSchema {
                authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token_url: "https://oauth2.googleapis.com/token".to_string(),
                client_id: Some("public-client-id".to_string()),
                scopes: vec!["scope-b".to_string(), "scope-a".to_string()],
                use_pkce: true,
                access_token_field: "access_token".to_string(),
                ..OAuthConfigSchema::default()
            }),
            ..AuthCapabilitySchema::default()
        };
        let token = WasmOAuthTokenExchange {
            access_token: "access-value".to_string(),
            refresh_token: Some("refresh-value".to_string()),
            expires_at: None,
            granted_scopes: vec!["scope-b".to_string(), "scope-a".to_string()],
            raw: serde_json::json!({}),
        };

        flow.store_token_exchange(&auth, &token)
            .await
            .expect("persist exchange");

        for (name, expected) in [
            (GOOGLE_OAUTH_TOKEN.to_string(), "access-value"),
            (refresh_secret_name(GOOGLE_OAUTH_TOKEN), "refresh-value"),
            (
                client_id_secret_name(GOOGLE_OAUTH_TOKEN),
                "public-client-id",
            ),
        ] {
            let value = store
                .get_decrypted("user", &name)
                .await
                .expect("stored value");
            assert_eq!(value.expose(), expected);
        }
        assert!(
            !store
                .exists("user", &client_secret_secret_name(GOOGLE_OAUTH_TOKEN))
                .await
                .expect("client-secret existence")
        );
        let scopes = store
            .get_decrypted("user", &scopes_secret_name(GOOGLE_OAUTH_TOKEN))
            .await
            .expect("scope metadata");
        assert_eq!(
            parse_scopes_value(scopes.expose()),
            vec!["scope-a", "scope-b"]
        );
    }
}

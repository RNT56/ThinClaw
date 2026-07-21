//! OAuth 2.0 token manager with PKCE support.
//!
//! Shared infrastructure for Google Drive, Dropbox, OneDrive providers.
//! Uses Authorization Code + PKCE flow (no client secret needed for desktop apps).
//!
//! # Flow
//! 1. Generate PKCE code verifier + challenge
//! 2. Build authorization URL — user opens in browser
//! 3. Listen on localhost for redirect with auth code
//! 4. Exchange auth code for access + refresh tokens
//! 5. Auto-refresh when access token expires

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

use super::provider::CloudError;

const OAUTH_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(30);
const OAUTH_CONNECT_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const OAUTH_CALLBACK_TIMEOUT: StdDuration = StdDuration::from_secs(5 * 60);
const OAUTH_CALLBACK_READ_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const MAX_OAUTH_CALLBACK_REQUEST_BYTES: usize = 16 * 1024;
const MAX_OAUTH_CALLBACK_ATTEMPTS: usize = 16;
const MAX_OAUTH_TOKEN_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_OAUTH_TOKEN_BYTES: usize = 64 * 1024;
const MAX_OAUTH_CODE_BYTES: usize = 16 * 1024;
const MAX_OAUTH_KEYCHAIN_BYTES: usize = 256 * 1024;

// ── OAuth Configuration ──────────────────────────────────────────────────

/// Provider-specific OAuth parameters.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// OAuth provider name (for logging / keychain storage)
    pub provider_name: String,
    /// OAuth client ID (embedded in app or user-provided)
    pub client_id: String,
    /// Authorization endpoint URL
    pub auth_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// Revocation endpoint URL (optional — not all providers support it)
    pub revoke_url: Option<String>,
    /// Requested scopes (space-separated)
    pub scopes: String,
    /// Redirect URI (localhost listener)
    pub redirect_uri: String,
}

impl OAuthConfig {
    /// Google Drive OAuth config.
    ///
    /// Requires a Google Cloud Console project with the Drive API enabled.
    /// Client ID is for "Desktop" application type (no client secret needed with PKCE).
    pub fn google_drive(client_id: String) -> Self {
        Self {
            provider_name: "google_drive".to_string(),
            client_id,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            revoke_url: Some("https://oauth2.googleapis.com/revoke".to_string()),
            scopes: "https://www.googleapis.com/auth/drive.file".to_string(),
            redirect_uri: "http://127.0.0.1:19246/oauth/callback".to_string(),
        }
    }

    /// Dropbox OAuth config.
    pub fn dropbox(client_id: String) -> Self {
        Self {
            provider_name: "dropbox".to_string(),
            client_id,
            auth_url: "https://www.dropbox.com/oauth2/authorize".to_string(),
            token_url: "https://api.dropboxapi.com/oauth2/token".to_string(),
            revoke_url: Some("https://api.dropboxapi.com/2/auth/token/revoke".to_string()),
            scopes: String::new(), // Dropbox doesn't use scopes
            redirect_uri: "http://127.0.0.1:19246/oauth/callback".to_string(),
        }
    }

    /// OneDrive / Microsoft Graph OAuth config.
    pub fn onedrive(client_id: String) -> Self {
        Self {
            provider_name: "onedrive".to_string(),
            client_id,
            auth_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize"
                .to_string(),
            token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".to_string(),
            revoke_url: None, // Microsoft uses logout endpoint instead
            scopes: "Files.ReadWrite offline_access".to_string(),
            redirect_uri: "http://127.0.0.1:19246/oauth/callback".to_string(),
        }
    }
}

// ── PKCE ─────────────────────────────────────────────────────────────────

/// PKCE code verifier (43-128 chars of [A-Za-z0-9-._~]).
fn generate_code_verifier() -> String {
    // 96 random bytes encode to exactly 128 unpadded base64url characters,
    // satisfying RFC 7636 without modulo bias.
    random_urlsafe_token(96)
}

/// PKCE code challenge = BASE64URL(SHA256(code_verifier)).
fn compute_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash)
}

/// BASE64URL encoding (no padding, URL-safe alphabet).
fn base64_url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn random_urlsafe_token(bytes: usize) -> String {
    let mut random = vec![0u8; bytes];
    OsRng.fill_bytes(&mut random);
    base64_url_encode(&random)
}

// ── Token Types ──────────────────────────────────────────────────────────

/// OAuth tokens returned from the token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    /// Bearer access token
    pub access_token: String,
    /// Refresh token (for obtaining new access tokens)
    pub refresh_token: Option<String>,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// When the access token expires
    pub expires_at: DateTime<Utc>,
    /// Granted scopes (may differ from requested)
    pub scope: Option<String>,
}

impl OAuthTokens {
    /// Check if the access token is expired or about to expire (< 5 min remaining).
    pub fn is_expired(&self) -> bool {
        Utc::now() + ChronoDuration::minutes(5) >= self.expires_at
    }
}

/// Raw token response from the OAuth provider.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

// ── OAuth Manager ────────────────────────────────────────────────────────

/// Manages the OAuth 2.0 PKCE flow and token lifecycle.
pub struct OAuthManager {
    config: OAuthConfig,
    http: reqwest::Client,
    /// Serializes refresh-token rotation. Without this, concurrent provider
    /// requests can exchange the same refresh token and overwrite each other.
    refresh_lock: Mutex<()>,
}

/// One pending loopback authorization. The verifier never crosses the IPC
/// boundary and is zeroized when the flow completes, expires, or is dropped.
pub(crate) struct PendingOAuthFlow {
    provider: String,
    oauth_config: Option<OAuthConfig>,
    code_verifier: Option<Zeroizing<String>>,
    callback_rx: Option<oneshot::Receiver<Result<String, CloudError>>>,
    callback_task: Option<JoinHandle<()>>,
    created_at: Instant,
}

pub(crate) struct OAuthFlowStart {
    pub(crate) auth_url: String,
    pub(crate) flow_id: String,
    pub(crate) pending: PendingOAuthFlow,
}

pub(crate) struct OAuthCallback {
    pub(crate) code: String,
    pub(crate) code_verifier: Zeroizing<String>,
    pub(crate) oauth_config: OAuthConfig,
}

impl PendingOAuthFlow {
    pub(crate) fn provider_is(&self, provider: &str) -> bool {
        self.provider == provider
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= OAUTH_CALLBACK_TIMEOUT
    }

    pub(crate) async fn wait(mut self) -> Result<OAuthCallback, CloudError> {
        let callback_rx = self.callback_rx.take().ok_or_else(|| {
            CloudError::AuthFailed("OAuth callback flow was already consumed".to_string())
        })?;
        let code = callback_rx.await.map_err(|_| {
            CloudError::AuthFailed("OAuth callback listener stopped unexpectedly".to_string())
        })??;
        if let Some(task) = self.callback_task.take() {
            task.await.map_err(|error| {
                CloudError::AuthFailed(format!("OAuth callback task failed: {error}"))
            })?;
        }
        let code_verifier = self.code_verifier.take().ok_or_else(|| {
            CloudError::AuthFailed("OAuth PKCE verifier was already consumed".to_string())
        })?;
        let oauth_config = self.oauth_config.take().ok_or_else(|| {
            CloudError::AuthFailed("OAuth configuration was already consumed".to_string())
        })?;
        Ok(OAuthCallback {
            code,
            code_verifier,
            oauth_config,
        })
    }
}

impl Drop for PendingOAuthFlow {
    fn drop(&mut self) {
        if let Some(task) = self.callback_task.take() {
            task.abort();
        }
    }
}

impl OAuthManager {
    /// Create a new OAuth manager for the given provider.
    pub fn new(config: OAuthConfig) -> Result<Self, CloudError> {
        validate_oauth_config(&config)?;
        let http = reqwest::Client::builder()
            .connect_timeout(OAUTH_CONNECT_TIMEOUT)
            .timeout(OAUTH_HTTP_TIMEOUT)
            // Token and revocation endpoints are fixed, trusted URLs. Refuse
            // redirects so credentials cannot be forwarded to another origin.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ThinClaw-Desktop/cloud-oauth")
            .build()
            .map_err(|error| {
                CloudError::Provider(format!("Failed to create OAuth HTTP client: {error}"))
            })?;
        Ok(Self {
            config,
            http,
            refresh_lock: Mutex::new(()),
        })
    }

    /// Generate the authorization URL and PKCE verifier.
    ///
    /// Returns `(authorization_url, code_verifier, state)`.
    fn authorize_url(&self) -> Result<(String, String, String), CloudError> {
        let verifier = generate_code_verifier();
        let challenge = compute_code_challenge(&verifier);
        let state = random_urlsafe_token(32);
        let mut url = reqwest::Url::parse(&self.config.auth_url).map_err(|error| {
            CloudError::Provider(format!("Invalid OAuth authorization URL: {error}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", &self.config.redirect_uri)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state);
            if !self.config.scopes.is_empty() {
                query.append_pair("scope", &self.config.scopes);
            }
            if self.config.provider_name == "dropbox" {
                query.append_pair("token_access_type", "offline");
            }
            if self.config.provider_name == "google_drive" {
                query
                    .append_pair("access_type", "offline")
                    .append_pair("prompt", "consent");
            }
        }

        info!("[oauth/{}] Generated auth URL", self.config.provider_name);
        Ok((url.to_string(), verifier, state))
    }

    /// Bind the configured loopback callback before returning the URL. This
    /// guarantees that the redirect target is live and keeps PKCE/state data
    /// entirely inside the backend.
    pub(crate) async fn start_authorization(
        &self,
        provider: String,
    ) -> Result<OAuthFlowStart, CloudError> {
        let redirect = reqwest::Url::parse(&self.config.redirect_uri).map_err(|error| {
            CloudError::Provider(format!("Invalid OAuth redirect URL: {error}"))
        })?;
        let port = redirect
            .port()
            .ok_or_else(|| CloudError::Provider("OAuth redirect port is missing".to_string()))?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                CloudError::AuthFailed(format!(
                    "Cannot start the local OAuth callback on 127.0.0.1:{port}: {error}"
                ))
            })?;
        let expected_path = redirect.path().to_string();
        let (auth_url, code_verifier, state) = self.authorize_url()?;
        let (callback_tx, callback_rx) = oneshot::channel();
        let callback_task = tokio::spawn(async move {
            let result = wait_for_oauth_callback(listener, &expected_path, &state).await;
            let _ = callback_tx.send(result);
        });

        Ok(OAuthFlowStart {
            auth_url,
            flow_id: random_urlsafe_token(24),
            pending: PendingOAuthFlow {
                provider,
                oauth_config: Some(self.config.clone()),
                code_verifier: Some(Zeroizing::new(code_verifier)),
                callback_rx: Some(callback_rx),
                callback_task: Some(callback_task),
                created_at: Instant::now(),
            },
        })
    }

    /// Exchange the authorization code for tokens.
    ///
    /// Called after the user completes the OAuth flow and the redirect
    /// listener captures the `code` parameter.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<OAuthTokens, CloudError> {
        validate_authorization_code(code)?;
        validate_code_verifier(verifier)?;
        info!(
            "[oauth/{}] Exchanging authorization code for tokens",
            self.config.provider_name
        );

        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.config.redirect_uri),
            ("client_id", &self.config.client_id),
            ("code_verifier", verifier),
        ];

        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Token request failed: {}", e)))?;

        let token_resp = decode_token_response(resp, "Token exchange").await?;
        let tokens = self.token_response_to_tokens(token_resp)?;

        info!(
            "[oauth/{}] Token exchange successful. Expires at: {}",
            self.config.provider_name, tokens.expires_at
        );

        Ok(tokens)
    }

    /// Refresh the access token using the refresh token.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<OAuthTokens, CloudError> {
        validate_token_value("refresh token", refresh_token)?;
        debug!(
            "[oauth/{}] Refreshing access token",
            self.config.provider_name
        );

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.config.client_id),
        ];

        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Refresh request failed: {}", e)))?;

        let mut token_resp = decode_token_response(resp, "Token refresh").await?;

        // Some providers don't return a new refresh token on refresh
        if token_resp.refresh_token.is_none() {
            token_resp.refresh_token = Some(refresh_token.to_string());
        }

        let tokens = self.token_response_to_tokens(token_resp)?;

        debug!(
            "[oauth/{}] Token refreshed. New expiry: {}",
            self.config.provider_name, tokens.expires_at
        );

        Ok(tokens)
    }

    /// Revoke the refresh token (sign out).
    pub async fn revoke(&self, token: &str) -> Result<(), CloudError> {
        validate_token_value("OAuth token", token)?;
        if let Some(revoke_url) = &self.config.revoke_url {
            info!("[oauth/{}] Revoking token", self.config.provider_name);

            let resp = if self.config.provider_name == "google_drive" {
                // Google uses POST with token in body
                self.http
                    .post(revoke_url)
                    .form(&[("token", token)])
                    .send()
                    .await
            } else {
                // Dropbox uses POST with Authorization header
                self.http.post(revoke_url).bearer_auth(token).send().await
            };

            match resp {
                Ok(r) if r.status().is_success() => {
                    info!("[oauth/{}] Token revoked", self.config.provider_name);
                }
                Ok(r) => {
                    warn!(
                        "[oauth/{}] Revoke returned HTTP {}, continuing",
                        self.config.provider_name,
                        r.status()
                    );
                }
                Err(e) => {
                    warn!(
                        "[oauth/{}] Revoke request failed: {}, continuing",
                        self.config.provider_name, e
                    );
                }
            }
        }

        Ok(())
    }

    // ── Keychain Storage ─────────────────────────────────────────────────

    #[cfg(target_os = "macos")]
    fn keychain_service(&self) -> String {
        format!("com.thinclaw.desktop.oauth.{}", self.config.provider_name)
    }

    #[cfg(target_os = "macos")]
    fn legacy_keychain_service(&self) -> String {
        format!("com.scrappy.oauth.{}", self.config.provider_name)
    }

    /// Save tokens to macOS Keychain.
    #[cfg(target_os = "macos")]
    pub fn save_tokens_to_keychain(&self, tokens: &OAuthTokens) -> Result<(), CloudError> {
        use security_framework::passwords::set_generic_password;

        let service = self.keychain_service();
        let account = "tokens";

        let json = serde_json::to_vec(tokens)
            .map_err(|e| CloudError::Provider(format!("Token serialization failed: {}", e)))?;
        if json.len() > MAX_OAUTH_KEYCHAIN_BYTES {
            return Err(CloudError::Provider(format!(
                "OAuth token record exceeds {MAX_OAUTH_KEYCHAIN_BYTES} bytes"
            )));
        }

        // `set_generic_password` creates or updates atomically. Deleting first
        // creates a credential-loss window if the replacement write fails.
        set_generic_password(&service, account, &json)
            .map_err(|e| CloudError::Provider(format!("Keychain save failed: {}", e)))?;

        debug!(
            "[oauth/{}] Tokens saved to Keychain",
            self.config.provider_name
        );
        Ok(())
    }

    /// Load tokens from macOS Keychain.
    #[cfg(target_os = "macos")]
    pub fn load_tokens_from_keychain(&self) -> Result<Option<OAuthTokens>, CloudError> {
        match self.load_tokens_from_service(&self.keychain_service())? {
            Some(tokens) => Ok(Some(tokens)),
            None => self.load_tokens_from_service(&self.legacy_keychain_service()),
        }
    }

    #[cfg(target_os = "macos")]
    fn load_tokens_from_service(&self, service: &str) -> Result<Option<OAuthTokens>, CloudError> {
        use security_framework::passwords::get_generic_password;

        match get_generic_password(service, "tokens") {
            Ok(data) => {
                if data.len() > MAX_OAUTH_KEYCHAIN_BYTES {
                    return Err(CloudError::Provider(format!(
                        "OAuth token record exceeds {MAX_OAUTH_KEYCHAIN_BYTES} bytes"
                    )));
                }
                let tokens: OAuthTokens = serde_json::from_slice(&data).map_err(|e| {
                    CloudError::Provider(format!("Token deserialization failed: {}", e))
                })?;
                validate_stored_tokens(&tokens)?;
                debug!(
                    "[oauth/{}] Loaded tokens from Keychain (expires: {})",
                    self.config.provider_name, tokens.expires_at
                );
                Ok(Some(tokens))
            }
            Err(e) => {
                let code = e.code();
                if code == -25300 {
                    // errSecItemNotFound
                    Ok(None)
                } else {
                    Err(CloudError::Provider(format!(
                        "Keychain read failed: {} (code: {})",
                        e, code
                    )))
                }
            }
        }
    }

    /// Delete tokens from macOS Keychain (on sign-out or revocation).
    #[cfg(target_os = "macos")]
    pub fn delete_tokens_from_keychain(&self) -> Result<(), CloudError> {
        use security_framework::passwords::delete_generic_password;

        let service = self.keychain_service();
        match delete_generic_password(&service, "tokens") {
            Ok(()) => {}
            Err(error) if error.code() == -25300 => {}
            Err(error) => {
                return Err(CloudError::Provider(format!(
                    "Keychain token delete failed: {error}"
                )))
            }
        }
        info!(
            "[oauth/{}] Tokens deleted from Keychain",
            self.config.provider_name
        );
        Ok(())
    }

    // Non-macOS stubs
    #[cfg(not(target_os = "macos"))]
    pub fn save_tokens_to_keychain(&self, _tokens: &OAuthTokens) -> Result<(), CloudError> {
        Err(CloudError::Provider(
            "Keychain only supported on macOS".into(),
        ))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn load_tokens_from_keychain(&self) -> Result<Option<OAuthTokens>, CloudError> {
        Err(CloudError::Provider(
            "Keychain only supported on macOS".into(),
        ))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn delete_tokens_from_keychain(&self) -> Result<(), CloudError> {
        Err(CloudError::Provider(
            "Keychain only supported on macOS".into(),
        ))
    }

    /// Get a valid access token, auto-refreshing if expired.
    ///
    /// Loads from Keychain, refreshes if needed, saves updated tokens back.
    pub async fn get_valid_token(&self) -> Result<String, CloudError> {
        let initial = self.load_tokens_from_keychain()?.ok_or_else(|| {
            CloudError::AuthFailed("No OAuth tokens found — please sign in".into())
        })?;

        if !initial.is_expired() {
            return Ok(initial.access_token);
        }

        // Re-check after obtaining the lock because another request may have
        // completed a refresh while this one was waiting.
        let _refresh_guard = self.refresh_lock.lock().await;
        let tokens = self.load_tokens_from_keychain()?.ok_or_else(|| {
            CloudError::AuthFailed("No OAuth tokens found — please sign in".into())
        })?;
        if !tokens.is_expired() {
            return Ok(tokens.access_token);
        }

        let previous_scope = tokens.scope.clone();
        let refresh = tokens.refresh_token.ok_or_else(|| {
            CloudError::AuthFailed("Access token expired and no refresh token available".into())
        })?;

        let mut new_tokens = self.refresh_token(&refresh).await?;
        if new_tokens.scope.is_none() {
            new_tokens.scope = previous_scope;
        }
        self.save_tokens_to_keychain(&new_tokens)?;

        Ok(new_tokens.access_token)
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn token_response_to_tokens(&self, resp: TokenResponse) -> Result<OAuthTokens, CloudError> {
        validate_token_value("access token", &resp.access_token)?;
        if let Some(refresh_token) = resp.refresh_token.as_deref() {
            validate_token_value("refresh token", refresh_token)?;
        }
        let token_type = resp.token_type.unwrap_or_else(|| "Bearer".to_string());
        if !token_type.eq_ignore_ascii_case("bearer") {
            return Err(CloudError::AuthFailed(format!(
                "Unsupported OAuth token type '{token_type}'"
            )));
        }
        if let Some(scope) = resp.scope.as_deref() {
            if scope.len() > MAX_OAUTH_TOKEN_BYTES || scope.chars().any(char::is_control) {
                return Err(CloudError::AuthFailed(
                    "OAuth scope response is invalid".to_string(),
                ));
            }
        }
        let expires_in = resp.expires_in.unwrap_or(3600);
        if !(1..=31_536_000).contains(&expires_in) {
            return Err(CloudError::AuthFailed(
                "OAuth token expiry is outside the supported range".to_string(),
            ));
        }
        let expires_at = Utc::now()
            .checked_add_signed(ChronoDuration::seconds(expires_in))
            .ok_or_else(|| CloudError::AuthFailed("OAuth token expiry overflowed".to_string()))?;

        Ok(OAuthTokens {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            token_type: "Bearer".to_string(),
            expires_at,
            scope: resp.scope,
        })
    }
}

/// Resolve the OAuth client identity once at flow start. Placeholder IDs make
/// the UI appear usable while guaranteeing a provider-side failure, so
/// providers without a bundled client fail with an actionable error instead.
pub(crate) fn config_for_provider(provider: &str) -> Result<OAuthConfig, CloudError> {
    let client_id = match provider {
        "gdrive" => std::env::var("GOOGLE_CLIENT_ID")
            .ok()
            .or_else(|| std::env::var("GOOGLE_OAUTH_CLIENT_ID").ok())
            .or_else(|| option_env!("THINCLAW_GOOGLE_CLIENT_ID").map(str::to_string))
            .or_else(|| {
                thinclaw_core::cli::oauth_defaults::builtin_credentials("google_oauth_token")
                    .map(|credentials| credentials.client_id.to_string())
            })
            .map(OAuthConfig::google_drive),
        "dropbox" => std::env::var("DROPBOX_CLIENT_ID")
            .ok()
            .or_else(|| option_env!("THINCLAW_DROPBOX_CLIENT_ID").map(str::to_string))
            .map(OAuthConfig::dropbox),
        "onedrive" => std::env::var("ONEDRIVE_CLIENT_ID")
            .ok()
            .or_else(|| option_env!("THINCLAW_ONEDRIVE_CLIENT_ID").map(str::to_string))
            .map(OAuthConfig::onedrive),
        other => {
            return Err(CloudError::Provider(format!(
                "OAuth is not supported for provider '{other}'"
            )))
        }
    };

    client_id.ok_or_else(|| {
        let variable = match provider {
            "dropbox" => "DROPBOX_CLIENT_ID",
            "onedrive" => "ONEDRIVE_CLIENT_ID",
            _ => "the provider OAuth client ID",
        };
        CloudError::Provider(format!(
            "Cloud OAuth for '{provider}' is not configured in this build; set {variable}"
        ))
    })
}

async fn decode_token_response(
    response: reqwest::Response,
    context: &str,
) -> Result<TokenResponse, CloudError> {
    let status = response.status();
    if !status.is_success() {
        let detail = super::provider::bounded_error_body(response).await;
        return Err(CloudError::AuthFailed(format!(
            "{context} failed (HTTP {status}): {detail}"
        )));
    }
    thinclaw_core::http_response::bounded_json(response, MAX_OAUTH_TOKEN_RESPONSE_BYTES)
        .await
        .map_err(|error| CloudError::AuthFailed(format!("{context} response is invalid: {error}")))
}

fn validate_oauth_config(config: &OAuthConfig) -> Result<(), CloudError> {
    if config.provider_name.is_empty()
        || config.provider_name.len() > 64
        || !config
            .provider_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(CloudError::Provider(
            "OAuth provider name is invalid".to_string(),
        ));
    }
    if config.client_id.is_empty()
        || config.client_id.len() > 4_096
        || config.client_id.chars().any(char::is_control)
    {
        return Err(CloudError::Provider(
            "OAuth client ID is invalid".to_string(),
        ));
    }
    if config.scopes.len() > 16 * 1024 || config.scopes.chars().any(char::is_control) {
        return Err(CloudError::Provider(
            "OAuth scope list is invalid".to_string(),
        ));
    }

    validate_https_oauth_endpoint("authorization", &config.auth_url)?;
    validate_https_oauth_endpoint("token", &config.token_url)?;
    if let Some(revoke_url) = config.revoke_url.as_deref() {
        validate_https_oauth_endpoint("revocation", revoke_url)?;
    }

    let redirect = reqwest::Url::parse(&config.redirect_uri)
        .map_err(|error| CloudError::Provider(format!("Invalid OAuth redirect URL: {error}")))?;
    if redirect.scheme() != "http"
        || redirect.host_str() != Some("127.0.0.1")
        || redirect.port().is_none()
        || redirect.path() != "/oauth/callback"
        || redirect.query().is_some()
        || redirect.fragment().is_some()
        || !redirect.username().is_empty()
        || redirect.password().is_some()
    {
        return Err(CloudError::Provider(
            "OAuth redirect must be an exact loopback HTTP callback URL".to_string(),
        ));
    }
    Ok(())
}

fn validate_https_oauth_endpoint(label: &str, value: &str) -> Result<(), CloudError> {
    let url = reqwest::Url::parse(value)
        .map_err(|error| CloudError::Provider(format!("Invalid OAuth {label} URL: {error}")))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(CloudError::Provider(format!(
            "OAuth {label} URL must be a credential-free HTTPS URL"
        )));
    }
    Ok(())
}

fn validate_authorization_code(code: &str) -> Result<(), CloudError> {
    if code.is_empty() || code.len() > MAX_OAUTH_CODE_BYTES || code.chars().any(char::is_control) {
        return Err(CloudError::AuthFailed(
            "OAuth authorization code is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_code_verifier(verifier: &str) -> Result<(), CloudError> {
    if !(43..=128).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(CloudError::AuthFailed(
            "OAuth PKCE verifier is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_token_value(label: &str, value: &str) -> Result<(), CloudError> {
    if value.is_empty()
        || value.len() > MAX_OAUTH_TOKEN_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(CloudError::AuthFailed(format!("OAuth {label} is invalid")));
    }
    Ok(())
}

fn validate_stored_tokens(tokens: &OAuthTokens) -> Result<(), CloudError> {
    validate_token_value("access token", &tokens.access_token)?;
    if let Some(refresh_token) = tokens.refresh_token.as_deref() {
        validate_token_value("refresh token", refresh_token)?;
    }
    if !tokens.token_type.eq_ignore_ascii_case("bearer") {
        return Err(CloudError::AuthFailed(
            "Stored OAuth token type is not Bearer".to_string(),
        ));
    }
    if let Some(scope) = tokens.scope.as_deref() {
        if scope.len() > MAX_OAUTH_TOKEN_BYTES || scope.chars().any(char::is_control) {
            return Err(CloudError::AuthFailed(
                "Stored OAuth scope is invalid".to_string(),
            ));
        }
    }
    Ok(())
}

enum ParsedCallback {
    Ignore,
    Complete(Result<String, CloudError>),
}

async fn wait_for_oauth_callback(
    listener: TcpListener,
    expected_path: &str,
    expected_state: &str,
) -> Result<String, CloudError> {
    let deadline = Instant::now() + OAUTH_CALLBACK_TIMEOUT;
    for _ in 0..MAX_OAUTH_CALLBACK_ATTEMPTS {
        let (mut stream, peer) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| CloudError::AuthFailed("OAuth authorization timed out".to_string()))?
            .map_err(|error| {
                CloudError::AuthFailed(format!("OAuth callback accept failed: {error}"))
            })?;
        if !peer.ip().is_loopback() {
            continue;
        }

        let target = match read_callback_target(&mut stream).await {
            Ok(target) => target,
            Err(_) => {
                let _ = write_callback_response(
                    &mut stream,
                    "400 Bad Request",
                    "The OAuth callback request was invalid.",
                )
                .await;
                continue;
            }
        };
        match parse_callback(&target, expected_path, expected_state) {
            ParsedCallback::Ignore => {
                let _ = write_callback_response(
                    &mut stream,
                    "400 Bad Request",
                    "This callback does not match the active OAuth sign-in.",
                )
                .await;
            }
            ParsedCallback::Complete(Ok(code)) => {
                let _ = write_callback_response(
                    &mut stream,
                    "200 OK",
                    "Authorization complete. You can close this tab and return to ThinClaw.",
                )
                .await;
                return Ok(code);
            }
            ParsedCallback::Complete(Err(error)) => {
                let _ = write_callback_response(
                    &mut stream,
                    "400 Bad Request",
                    "Authorization was denied or could not be completed. Return to ThinClaw for details.",
                )
                .await;
                return Err(error);
            }
        }
    }

    Err(CloudError::AuthFailed(
        "OAuth callback received too many invalid requests".to_string(),
    ))
}

async fn read_callback_target(stream: &mut TcpStream) -> Result<String, CloudError> {
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = tokio::time::timeout(OAUTH_CALLBACK_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| CloudError::AuthFailed("OAuth callback read timed out".to_string()))?
            .map_err(|error| {
                CloudError::AuthFailed(format!("OAuth callback read failed: {error}"))
            })?;
        if read == 0 {
            return Err(CloudError::AuthFailed(
                "OAuth callback closed before sending headers".to_string(),
            ));
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_OAUTH_CALLBACK_REQUEST_BYTES {
            return Err(CloudError::AuthFailed(
                "OAuth callback request is too large".to_string(),
            ));
        }
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = std::str::from_utf8(&request)
        .map_err(|_| CloudError::AuthFailed("OAuth callback is not valid UTF-8".to_string()))?;
    let request_line = request
        .split("\r\n")
        .next()
        .ok_or_else(|| CloudError::AuthFailed("OAuth callback has no request line".to_string()))?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if method != "GET"
        || !target.starts_with('/')
        || target.starts_with("//")
        || !version.starts_with("HTTP/1.")
        || parts.next().is_some()
    {
        return Err(CloudError::AuthFailed(
            "OAuth callback request line is invalid".to_string(),
        ));
    }
    Ok(target.to_string())
}

fn parse_callback(target: &str, expected_path: &str, expected_state: &str) -> ParsedCallback {
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return ParsedCallback::Ignore;
    };
    if url.path() != expected_path || url.fragment().is_some() {
        return ParsedCallback::Ignore;
    }

    let state = match unique_query_value(&url, "state") {
        Ok(Some(state)) if constant_time_eq(state.as_bytes(), expected_state.as_bytes()) => state,
        _ => return ParsedCallback::Ignore,
    };
    drop(state);

    match unique_query_value(&url, "error") {
        Err(()) => return ParsedCallback::Ignore,
        Ok(Some(error)) => {
            let description = unique_query_value(&url, "error_description")
                .ok()
                .flatten()
                .unwrap_or_default();
            let error = sanitize_oauth_error(&error);
            let description = sanitize_oauth_error(&description);
            return ParsedCallback::Complete(Err(CloudError::AuthFailed(format!(
                "OAuth provider returned '{error}': {description}"
            ))));
        }
        Ok(None) => {}
    }

    match unique_query_value(&url, "code") {
        Ok(Some(code)) if validate_authorization_code(&code).is_ok() => {
            ParsedCallback::Complete(Ok(code))
        }
        _ => ParsedCallback::Complete(Err(CloudError::AuthFailed(
            "OAuth callback did not contain a valid authorization code".to_string(),
        ))),
    }
}

fn unique_query_value(url: &reqwest::Url, name: &str) -> Result<Option<String>, ()> {
    let mut values = url.query_pairs().filter(|(key, _)| key == name);
    let first = values.next().map(|(_, value)| value.into_owned());
    if values.next().is_some() {
        Err(())
    } else {
        Ok(first)
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (&left, &right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

fn sanitize_oauth_error(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

async fn write_callback_response(
    stream: &mut TcpStream,
    status: &str,
    message: &str,
) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>ThinClaw OAuth</title><p>{message}</p>"
    );
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_verifier_length() {
        let verifier = generate_code_verifier();
        assert_eq!(verifier.len(), 128);
        // All chars must be in the allowed set
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric()
            || c == '-'
            || c == '.'
            || c == '_'
            || c == '~'));
    }

    #[test]
    fn test_pkce_challenge_deterministic() {
        let challenge1 = compute_code_challenge("test_verifier_123");
        let challenge2 = compute_code_challenge("test_verifier_123");
        assert_eq!(challenge1, challenge2);

        // Different verifier → different challenge
        let challenge3 = compute_code_challenge("different_verifier");
        assert_ne!(challenge1, challenge3);
    }

    #[test]
    fn test_pkce_challenge_format() {
        let challenge = compute_code_challenge("test");
        // BASE64URL: only [A-Za-z0-9_-], no padding
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn test_authorize_url_google() {
        let config = OAuthConfig::google_drive("test-client-id".to_string());
        let manager = OAuthManager::new(config).unwrap();
        let (url, verifier, state) = manager.authorize_url().unwrap();

        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("scope="));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(verifier.len() == 128);
        assert!(url.contains(&format!("state={state}")));
    }

    #[test]
    fn test_authorize_url_dropbox() {
        let config = OAuthConfig::dropbox("dropbox-client-id".to_string());
        let manager = OAuthManager::new(config).unwrap();
        let (url, _verifier, _state) = manager.authorize_url().unwrap();

        assert!(url.starts_with("https://www.dropbox.com/oauth2/authorize"));
        assert!(url.contains("token_access_type=offline"));
        // Dropbox doesn't use scopes — no &scope= in URL
    }

    #[test]
    fn test_authorize_url_onedrive() {
        let config = OAuthConfig::onedrive("ms-client-id".to_string());
        let manager = OAuthManager::new(config).unwrap();
        let (url, _verifier, _state) = manager.authorize_url().unwrap();

        assert!(
            url.starts_with("https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize")
        );
        assert!(url.contains("scope="));
        assert!(url.contains("Files.ReadWrite"));
    }

    #[test]
    fn test_token_expiry_check() {
        let expired = OAuthTokens {
            access_token: "expired".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() - ChronoDuration::hours(1),
            scope: None,
        };
        assert!(expired.is_expired());

        let about_to_expire = OAuthTokens {
            access_token: "soon".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() + ChronoDuration::minutes(3), // < 5 min threshold
            scope: None,
        };
        assert!(about_to_expire.is_expired());

        let fresh = OAuthTokens {
            access_token: "fresh".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() + ChronoDuration::hours(1),
            scope: None,
        };
        assert!(!fresh.is_expired());
    }

    #[test]
    fn test_token_serialization_roundtrip() {
        let tokens = OAuthTokens {
            access_token: "ya29.test".to_string(),
            refresh_token: Some("1//refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() + ChronoDuration::hours(1),
            scope: Some("drive.file".to_string()),
        };

        let json = serde_json::to_vec(&tokens).unwrap();
        let restored: OAuthTokens = serde_json::from_slice(&json).unwrap();

        assert_eq!(restored.access_token, tokens.access_token);
        assert_eq!(restored.refresh_token, tokens.refresh_token);
        assert_eq!(restored.token_type, tokens.token_type);
        assert_eq!(restored.scope, tokens.scope);
    }

    #[test]
    fn callback_requires_unique_matching_state() {
        assert!(matches!(
            parse_callback(
                "/oauth/callback?state=expected&code=abc",
                "/oauth/callback",
                "expected"
            ),
            ParsedCallback::Complete(Ok(code)) if code == "abc"
        ));
        assert!(matches!(
            parse_callback(
                "/oauth/callback?state=wrong&code=abc",
                "/oauth/callback",
                "expected"
            ),
            ParsedCallback::Ignore
        ));
        assert!(matches!(
            parse_callback(
                "/oauth/callback?state=expected&state=expected&code=abc",
                "/oauth/callback",
                "expected"
            ),
            ParsedCallback::Ignore
        ));
    }

    #[test]
    fn callback_rejects_duplicate_or_missing_codes() {
        assert!(matches!(
            parse_callback(
                "/oauth/callback?state=expected&code=a&code=b",
                "/oauth/callback",
                "expected"
            ),
            ParsedCallback::Complete(Err(_))
        ));
        assert!(matches!(
            parse_callback(
                "/oauth/callback?state=expected",
                "/oauth/callback",
                "expected"
            ),
            ParsedCallback::Complete(Err(_))
        ));
    }

    #[test]
    fn oauth_config_rejects_non_loopback_redirects() {
        let mut config = OAuthConfig::google_drive("client".to_string());
        config.redirect_uri = "https://example.com/oauth/callback".to_string();
        assert!(OAuthManager::new(config).is_err());
    }

    #[test]
    fn token_response_validation_rejects_wrong_type_and_expiry() {
        let manager = OAuthManager::new(OAuthConfig::google_drive("client".to_string())).unwrap();
        let wrong_type = TokenResponse {
            access_token: "access".to_string(),
            refresh_token: None,
            token_type: Some("MAC".to_string()),
            expires_in: Some(3600),
            scope: None,
        };
        assert!(manager.token_response_to_tokens(wrong_type).is_err());

        let invalid_expiry = TokenResponse {
            access_token: "access".to_string(),
            refresh_token: None,
            token_type: Some("Bearer".to_string()),
            expires_in: Some(-1),
            scope: None,
        };
        assert!(manager.token_response_to_tokens(invalid_expiry).is_err());
    }
}

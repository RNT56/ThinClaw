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

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use super::provider::CloudError;

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
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
        .chars()
        .collect();
    (0..128)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
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
        Utc::now() + Duration::minutes(5) >= self.expires_at
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
}

impl OAuthManager {
    /// Create a new OAuth manager for the given provider.
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Generate the authorization URL and PKCE verifier.
    ///
    /// Returns `(authorization_url, code_verifier)`.
    /// The caller should:
    /// 1. Open `authorization_url` in the user's browser
    /// 2. Start the localhost redirect listener
    /// 3. Pass the received `code` + `code_verifier` to `exchange_code()`
    pub fn authorize_url(&self) -> (String, String) {
        let verifier = generate_code_verifier();
        let challenge = compute_code_challenge(&verifier);

        // Generate random state for CSRF protection
        let state: String = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            (0..32)
                .map(|_| rng.gen_range(b'a'..=b'z') as char)
                .collect()
        };

        let mut url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
            self.config.auth_url,
            urlencoding::encode(&self.config.client_id),
            urlencoding::encode(&self.config.redirect_uri),
            urlencoding::encode(&challenge),
            urlencoding::encode(&state),
        );

        if !self.config.scopes.is_empty() {
            url.push_str(&format!(
                "&scope={}",
                urlencoding::encode(&self.config.scopes)
            ));
        }

        // Dropbox-specific: token_access_type=offline for refresh tokens
        if self.config.provider_name == "dropbox" {
            url.push_str("&token_access_type=offline");
        }

        // Google-specific: access_type=offline & prompt=consent for refresh tokens
        if self.config.provider_name == "google_drive" {
            url.push_str("&access_type=offline&prompt=consent");
        }

        info!(
            "[oauth/{}] Generated auth URL (state={}...)",
            self.config.provider_name,
            &state[..8]
        );

        (url, verifier)
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

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::AuthFailed(format!(
                "Token exchange failed (HTTP {}): {}",
                status, body
            )));
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Token parse failed: {}", e)))?;

        let tokens = self.token_response_to_tokens(token_resp);

        info!(
            "[oauth/{}] Token exchange successful. Expires at: {}",
            self.config.provider_name, tokens.expires_at
        );

        Ok(tokens)
    }

    /// Refresh the access token using the refresh token.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<OAuthTokens, CloudError> {
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

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::AuthFailed(format!(
                "Token refresh failed (HTTP {}): {}",
                status, body
            )));
        }

        let mut token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Refresh parse failed: {}", e)))?;

        // Some providers don't return a new refresh token on refresh
        if token_resp.refresh_token.is_none() {
            token_resp.refresh_token = Some(refresh_token.to_string());
        }

        let tokens = self.token_response_to_tokens(token_resp);

        debug!(
            "[oauth/{}] Token refreshed. New expiry: {}",
            self.config.provider_name, tokens.expires_at
        );

        Ok(tokens)
    }

    /// Revoke the refresh token (sign out).
    pub async fn revoke(&self, token: &str) -> Result<(), CloudError> {
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

    fn keychain_service(&self) -> String {
        format!("com.thinclaw.desktop.oauth.{}", self.config.provider_name)
    }

    fn legacy_keychain_service(&self) -> String {
        format!("com.scrappy.oauth.{}", self.config.provider_name)
    }

    /// Save tokens to macOS Keychain.
    #[cfg(target_os = "macos")]
    pub fn save_tokens_to_keychain(&self, tokens: &OAuthTokens) -> Result<(), CloudError> {
        use security_framework::passwords::{delete_generic_password, set_generic_password};

        let service = self.keychain_service();
        let account = "tokens";

        let json = serde_json::to_vec(tokens)
            .map_err(|e| CloudError::Provider(format!("Token serialization failed: {}", e)))?;

        // New writes use the ThinClaw service. Legacy Scrappy OAuth tokens are
        // read-only fallback and are intentionally never deleted automatically.
        let _ = delete_generic_password(&service, account);
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
                let tokens: OAuthTokens = serde_json::from_slice(&data).map_err(|e| {
                    CloudError::Provider(format!("Token deserialization failed: {}", e))
                })?;
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
        let _ = delete_generic_password(&service, "tokens");
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
        let tokens = self.load_tokens_from_keychain()?.ok_or_else(|| {
            CloudError::AuthFailed("No OAuth tokens found — please sign in".into())
        })?;

        if !tokens.is_expired() {
            return Ok(tokens.access_token);
        }

        // Token expired — refresh it
        let refresh = tokens.refresh_token.ok_or_else(|| {
            CloudError::AuthFailed("Access token expired and no refresh token available".into())
        })?;

        let new_tokens = self.refresh_token(&refresh).await?;
        self.save_tokens_to_keychain(&new_tokens)?;

        Ok(new_tokens.access_token)
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn token_response_to_tokens(&self, resp: TokenResponse) -> OAuthTokens {
        let expires_in = resp.expires_in.unwrap_or(3600);
        let expires_at = Utc::now() + Duration::seconds(expires_in);

        OAuthTokens {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            token_type: resp.token_type.unwrap_or_else(|| "Bearer".to_string()),
            expires_at,
            scope: resp.scope,
        }
    }
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
        let manager = OAuthManager::new(config);
        let (url, verifier) = manager.authorize_url();

        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("scope="));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(verifier.len() == 128);
    }

    #[test]
    fn test_authorize_url_dropbox() {
        let config = OAuthConfig::dropbox("dropbox-client-id".to_string());
        let manager = OAuthManager::new(config);
        let (url, _verifier) = manager.authorize_url();

        assert!(url.starts_with("https://www.dropbox.com/oauth2/authorize"));
        assert!(url.contains("token_access_type=offline"));
        // Dropbox doesn't use scopes — no &scope= in URL
    }

    #[test]
    fn test_authorize_url_onedrive() {
        let config = OAuthConfig::onedrive("ms-client-id".to_string());
        let manager = OAuthManager::new(config);
        let (url, _verifier) = manager.authorize_url();

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
            expires_at: Utc::now() - Duration::hours(1),
            scope: None,
        };
        assert!(expired.is_expired());

        let about_to_expire = OAuthTokens {
            access_token: "soon".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() + Duration::minutes(3), // < 5 min threshold
            scope: None,
        };
        assert!(about_to_expire.is_expired());

        let fresh = OAuthTokens {
            access_token: "fresh".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Utc::now() + Duration::hours(1),
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
            expires_at: Utc::now() + Duration::hours(1),
            scope: Some("drive.file".to_string()),
        };

        let json = serde_json::to_vec(&tokens).unwrap();
        let restored: OAuthTokens = serde_json::from_slice(&json).unwrap();

        assert_eq!(restored.access_token, tokens.access_token);
        assert_eq!(restored.refresh_token, tokens.refresh_token);
        assert_eq!(restored.token_type, tokens.token_type);
        assert_eq!(restored.scope, tokens.scope);
    }
}

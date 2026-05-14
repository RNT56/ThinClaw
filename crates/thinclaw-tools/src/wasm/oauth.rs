use std::collections::HashSet;
use std::path::Path;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rand::RngCore;
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};

use thinclaw_secrets::{CreateSecretParams, SecretError, SecretsStore};

use crate::wasm::{AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema};

pub const GOOGLE_OAUTH_TOKEN: &str = "google_oauth_token";
pub const LEGACY_GMAIL_OAUTH_TOKEN: &str = "gmail_oauth_token";

pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

const GOOGLE_CLIENT_ID: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "564604149681-efo25d43rs85v0tibdepsmdv5dsrhhr0.apps.googleusercontent.com",
};
const GOOGLE_CLIENT_SECRET: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_SECRET") {
    Some(v) => v,
    None => "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",
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
        "google_oauth_token" | "gmail_oauth_token" => Some(OAuthCredentials {
            client_id: GOOGLE_CLIENT_ID,
            client_secret: GOOGLE_CLIENT_SECRET,
        }),
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

/// Configuration needed to refresh an expired OAuth access token.
#[derive(Debug, Clone)]
pub struct OAuthRefreshConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub secret_name: String,
    pub provider: Option<String>,
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

#[derive(Debug, Clone)]
pub struct ResolvedOAuthConfig {
    pub oauth: OAuthConfigSchema,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub required_scopes: Vec<String>,
    pub shared_auth_provider: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthPkcePair {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct WasmOAuthTokenExchange {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub granted_scopes: Vec<String>,
    pub raw: serde_json::Value,
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

        let client_id = oauth
            .client_id
            .clone()
            .or_else(|| {
                oauth
                    .client_id_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))
            .ok_or(WasmToolOAuthError::MissingClientId)?;

        let client_secret = oauth
            .client_secret
            .clone()
            .or_else(|| {
                oauth
                    .client_secret_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()))
            .filter(|value| !value.trim().is_empty());

        let mut merged_oauth = oauth.clone();
        merged_oauth.scopes = required_scopes.clone();

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
        let current = self.check_auth_status(auth).await?;
        let pkce = if resolved.oauth.use_pkce {
            Some(generate_pkce_pair())
        } else {
            None
        };

        let auth_url = build_authorization_url(
            &resolved.oauth,
            &resolved.client_id,
            redirect_uri,
            &resolved.required_scopes,
            pkce.as_ref(),
            state_nonce,
        );

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

        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()?;

        let mut token_params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
        ];

        if let Some(verifier) = code_verifier {
            token_params.push(("code_verifier", verifier.to_string()));
        }

        let mut request = client.post(&resolved.oauth.token_url);
        if let Some(ref secret) = resolved.client_secret {
            request = request.basic_auth(&resolved.client_id, Some(secret));
        } else {
            token_params.push(("client_id", resolved.client_id.clone()));
        }

        let response = request.form(&token_params).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(WasmToolOAuthError::TokenExchange(format!(
                "{} - {}",
                status, body
            )));
        }

        let token_data: serde_json::Value = response.json().await?;
        let access_token = token_data
            .get(&resolved.oauth.access_token_field)
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                WasmToolOAuthError::InvalidResponse(format!(
                    "missing {} field",
                    resolved.oauth.access_token_field
                ))
            })?
            .to_string();

        let refresh_token = token_data
            .get("refresh_token")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        let expires_at = token_data
            .get("expires_in")
            .and_then(|value| value.as_u64())
            .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
        let granted_scopes = granted_scopes_from_response(&token_data, &resolved.required_scopes);

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
        let mut access_params = CreateSecretParams::new(&canonical_name, &token.access_token);
        if let Some(ref provider) = auth.provider {
            access_params = access_params.with_provider(provider.clone());
        } else if let Some(provider) = shared_auth_provider(auth) {
            access_params = access_params.with_provider(provider);
        }
        if let Some(expires_at) = token.expires_at.as_ref().cloned() {
            access_params = access_params.with_expiry(expires_at);
        }
        self.secrets.create(self.user_id, access_params).await?;

        if let Some(refresh_token) = token.refresh_token.as_deref() {
            let mut refresh_params =
                CreateSecretParams::new(refresh_secret_name(&canonical_name), refresh_token);
            if let Some(ref provider) = auth.provider {
                refresh_params = refresh_params.with_provider(provider.clone());
            } else if let Some(provider) = shared_auth_provider(auth) {
                refresh_params = refresh_params.with_provider(provider);
            }
            self.secrets.create(self.user_id, refresh_params).await?;
        }

        if !token.granted_scopes.is_empty() {
            self.store_scope_metadata(auth, &token.granted_scopes)
                .await?;
        }

        Ok(())
    }

    pub async fn store_manual_token(
        &self,
        auth: &AuthCapabilitySchema,
        token: &str,
    ) -> Result<(), WasmToolOAuthError> {
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

        while let Ok(Some(entry)) = entries.next_entry().await {
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

            let Ok(content) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            let Ok(caps) = CapabilitiesFile::from_json(&content) else {
                continue;
            };
            let Some(other_auth) = caps.auth else {
                continue;
            };
            if !secret_names.contains(other_auth.secret_name.as_str()) {
                continue;
            }
            if let Some(other_oauth) = other_auth.oauth {
                scopes.extend(other_oauth.scopes);
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
) -> String {
    let mut params = vec![
        ("client_id".to_string(), client_id.to_string()),
        ("response_type".to_string(), "code".to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
    ];

    if !scopes.is_empty() {
        params.push(("scope".to_string(), scopes.join(" ")));
    }

    if let Some(state_nonce) = state_nonce {
        params.push(("state".to_string(), state_nonce.to_string()));
    }

    if let Some(pkce) = pkce {
        params.push(("code_challenge".to_string(), pkce.challenge.clone()));
        params.push(("code_challenge_method".to_string(), "S256".to_string()));
    }

    for (key, value) in &oauth.extra_params {
        params.push((key.clone(), value.clone()));
    }

    let query = params
        .into_iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(&key),
                urlencoding::encode(&value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");

    format!("{}?{}", oauth.authorization_url, query)
}

pub fn generate_pkce_pair() -> OAuthPkcePair {
    let mut verifier_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
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

    let client_id = oauth
        .client_id
        .clone()
        .or_else(|| {
            oauth
                .client_id_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| {
            builtin
                .as_ref()
                .map(|credentials| credentials.client_id.to_string())
        })?;

    let client_secret = oauth
        .client_secret
        .clone()
        .or_else(|| {
            oauth
                .client_secret_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| {
            builtin
                .as_ref()
                .map(|credentials| credentials.client_secret.to_string())
        })
        .filter(|value| !value.trim().is_empty());

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
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()?;
    let response = client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&[("access_token", token)])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(WasmToolOAuthError::TokenExchange(format!(
            "tokeninfo returned {}",
            response.status()
        )));
    }

    let json: serde_json::Value = response.json().await?;
    let scopes = json
        .get("scope")
        .and_then(|value| value.as_str())
        .map(parse_scopes_value)
        .unwrap_or_default();
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
        );

        assert!(url.contains("client_id=client-id"));
        assert!(url.contains("state=nonce"));
        assert!(url.contains("code_challenge=challenge"));
        assert!(url.contains("scope=scope-a%20scope-b"));
        assert!(url.contains("access_type=offline"));
    }
}

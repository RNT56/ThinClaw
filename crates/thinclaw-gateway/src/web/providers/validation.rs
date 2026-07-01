//! Provider API-key validation, masking, fingerprinting, and status helpers.

use axum::http::StatusCode;

pub const PROVIDER_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_RUNTIME_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_SECRETS_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_CREDENTIAL_SPEC_NOT_FOUND_STATUS: StatusCode = StatusCode::NOT_FOUND;
pub const PROVIDER_SENSITIVE_ROUTE_FORBIDDEN_STATUS: StatusCode = StatusCode::FORBIDDEN;
pub fn provider_store_unavailable_status() -> StatusCode {
    PROVIDER_STORE_UNAVAILABLE_STATUS
}

pub fn provider_runtime_unavailable_status() -> StatusCode {
    PROVIDER_RUNTIME_UNAVAILABLE_STATUS
}

pub fn provider_secrets_store_unavailable_status() -> StatusCode {
    PROVIDER_SECRETS_STORE_UNAVAILABLE_STATUS
}

pub fn provider_credential_spec_not_found_status() -> StatusCode {
    PROVIDER_CREDENTIAL_SPEC_NOT_FOUND_STATUS
}

pub fn provider_sensitive_route_forbidden_status() -> StatusCode {
    PROVIDER_SENSITIVE_ROUTE_FORBIDDEN_STATUS
}

pub fn provider_credentials_not_configured_message(display_name: impl AsRef<str>) -> String {
    format!("{} credentials are not configured", display_name.as_ref())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderApiKeyError {
    #[error("provider API key is required")]
    Missing,
    #[error("provider API key contains control characters")]
    InvalidCharacters,
}

impl ProviderApiKeyError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Missing | Self::InvalidCharacters => StatusCode::BAD_REQUEST,
        }
    }
}

pub fn validate_provider_api_key(raw: Option<&str>) -> Result<String, ProviderApiKeyError> {
    let api_key = raw.unwrap_or("").trim().to_string();
    if api_key.is_empty() {
        return Err(ProviderApiKeyError::Missing);
    }
    if api_key
        .chars()
        .any(|ch| ch.is_control() || ch == '\n' || ch == '\r')
    {
        return Err(ProviderApiKeyError::InvalidCharacters);
    }
    Ok(api_key)
}

pub fn mask_provider_key(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        "****".to_string()
    } else {
        format!(
            "{}...{}",
            chars.iter().take(4).collect::<String>(),
            chars
                .iter()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        )
    }
}

pub fn provider_key_fingerprint(value: &str) -> String {
    let key = blake3::derive_key(
        "thinclaw.provider-vault.fingerprint.v1",
        b"local-display-only",
    );
    let hash = blake3::keyed_hash(&key, value.as_bytes());
    hex::encode(&hash.as_bytes()[..12])
}

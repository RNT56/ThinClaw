//! Cross-domain DTOs shared by several gateway APIs.

use serde::Serialize;

/// Information about an available LLM model.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ModelInfo {
    pub name: String,
    pub is_primary: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
    /// Auth URL to open (when activation requires OAuth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    /// Setup URL to open for manual token flows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    /// Auth mode (`oauth`, `manual_token`, `secrets`, `none`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    /// Detailed auth status (`awaiting_authorization`, `needs_reauth`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_status: Option<String>,
    /// Whether the extension is waiting for a manual token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaiting_token: Option<bool>,
    /// Instructions for manual token entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Shared auth provider for grouped credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    /// Missing scopes when reauth is required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// Whether the channel was successfully activated after setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    /// Whether a gateway restart is needed (activation failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_restart: Option<bool>,
}

impl ActionResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            auth_url: None,
            setup_url: None,
            auth_mode: None,
            auth_status: None,
            awaiting_token: None,
            instructions: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            activated: None,
            needs_restart: None,
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            auth_url: None,
            setup_url: None,
            auth_mode: None,
            auth_status: None,
            awaiting_token: None,
            instructions: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            activated: None,
            needs_restart: None,
        }
    }
}

// --- Health ---

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HealthResponse {
    pub status: &'static str,
    pub channel: &'static str,
}

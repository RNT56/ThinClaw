//! Extension listing, setup, registry, and extension-auth DTOs.

use serde::{Deserialize, Serialize};
use thinclaw_types::IntegrationSetupStatus;

#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub active: bool,
    pub tools: Vec<String>,
    /// Whether this extension has configurable secrets (setup schema).
    #[serde(default)]
    pub needs_setup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// WASM channel activation status: "installed", "configured", "active", "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_status: Option<String>,
    /// Human-readable error when activation_status is "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    /// Channel-specific runtime diagnostics for live transport debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_diagnostics: Option<serde_json::Value>,
    /// Whether the UI can request an explicit reconnect for this extension.
    #[serde(default)]
    pub reconnect_supported: bool,
    /// Normalized setup/auth state used by WebUI, onboarding, CLI, and TUI.
    pub setup: IntegrationSetupStatus,
}

#[derive(Debug, Serialize)]
pub struct ExtensionListResponse {
    pub extensions: Vec<ExtensionInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionRequest {
    pub name: String,
    pub url: Option<String>,
    pub kind: Option<String>,
}

// --- Extension Setup ---

#[derive(Debug, Serialize)]
pub struct ExtensionSetupResponse {
    pub name: String,
    pub kind: String,
    pub mode: String,
    pub auth_status: String,
    pub fields: Vec<SecretFieldInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    /// Whether this secret is already stored.
    pub provided: bool,
    /// Whether the secret will be auto-generated if left empty.
    pub auto_generate: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionSetupRequest {
    pub secrets: std::collections::HashMap<String, String>,
}

// --- Registry ---

#[derive(Debug, Serialize)]
pub struct RegistryEntryInfo {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub installed: bool,
}

#[derive(Debug, Serialize)]
pub struct RegistrySearchResponse {
    pub entries: Vec<RegistryEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RegistrySearchQuery {
    pub query: Option<String>,
}

// --- Auth Token ---

/// Request to submit an auth token for an extension (dedicated endpoint).
#[derive(Debug, Deserialize)]
pub struct AuthTokenRequest {
    pub extension_name: String,
    pub token: String,
}

/// Request to cancel an in-progress auth flow.
#[derive(Debug, Deserialize)]
pub struct AuthCancelRequest {
    pub extension_name: String,
}

#[derive(Debug, Deserialize)]
pub struct NostrPrivateKeyRequest {
    #[serde(default)]
    pub private_key: Option<String>,
}

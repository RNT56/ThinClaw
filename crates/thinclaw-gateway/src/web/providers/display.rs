//! Provider response DTOs and display/shaping helpers.

use super::*;
use thinclaw_llm_core::RoutingRule;
use thinclaw_settings::{AdvisorAutoEscalationMode, ProvidersSettings};

/// Response entry for GET /api/providers.
#[derive(serde::Serialize)]
pub struct ProviderInfo {
    pub slug: String,
    pub display_name: String,
    pub api_style: String,
    pub default_model: String,
    pub default_context_size: u32,
    pub has_key: bool,
    #[serde(default)]
    pub credential_ready: bool,
    pub env_key_name: String,
    pub auth_kind: String,
    #[serde(default)]
    pub auth_mode: String,
    #[serde(default)]
    pub oauth_supported: bool,
    #[serde(default)]
    pub oauth_available: bool,
    #[serde(default)]
    pub oauth_source_label: Option<String>,
    #[serde(default)]
    pub oauth_source_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ProviderCredentialMetadata>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ProviderCredentialMetadata {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_version: Option<i32>,
}

#[derive(serde::Serialize)]
pub struct ProvidersListResponse {
    pub providers: Vec<ProviderInfo>,
}

#[derive(Debug)]
pub struct ProviderInfoInput {
    pub slug: String,
    pub display_name: String,
    pub api_style: String,
    pub default_model: String,
    pub default_context_size: u32,
    pub has_key: bool,
    pub credential_ready: bool,
    pub env_key_name: String,
    pub auth_kind: String,
    pub auth_mode: String,
    pub oauth_supported: bool,
    pub oauth_available: bool,
    pub oauth_source_label: Option<String>,
    pub oauth_source_location: Option<String>,
    pub setup_url: Option<String>,
    pub tier: Option<String>,
    pub credential: Option<ProviderCredentialMetadata>,
}

pub fn provider_info(input: ProviderInfoInput) -> ProviderInfo {
    ProviderInfo {
        slug: input.slug,
        display_name: input.display_name,
        api_style: input.api_style,
        default_model: input.default_model,
        default_context_size: input.default_context_size,
        has_key: input.has_key,
        credential_ready: input.credential_ready,
        env_key_name: input.env_key_name,
        auth_kind: input.auth_kind,
        auth_mode: input.auth_mode,
        oauth_supported: input.oauth_supported,
        oauth_available: input.oauth_available,
        oauth_source_label: input.oauth_source_label,
        oauth_source_location: input.oauth_source_location,
        setup_url: input.setup_url,
        tier: input.tier,
        credential: input.credential,
    }
}

pub fn providers_list_response(mut providers: Vec<ProviderInfo>) -> ProvidersListResponse {
    providers.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    ProvidersListResponse { providers }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOauthUiSourceInput {
    pub available: bool,
    pub source_label: String,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOauthUiState {
    pub supported: bool,
    pub available: bool,
    pub source_label: Option<String>,
    pub source_location: Option<String>,
}

pub fn provider_oauth_ui_state(source: Option<ProviderOauthUiSourceInput>) -> ProviderOauthUiState {
    match source {
        Some(source) => ProviderOauthUiState {
            supported: true,
            available: source.available,
            source_label: Some(source.source_label),
            source_location: Some(source.source_location),
        },
        None => ProviderOauthUiState {
            supported: false,
            available: false,
            source_label: None,
            source_location: None,
        },
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProviderConfigEntry {
    pub slug: String,
    pub display_name: String,
    pub api_style: String,
    pub default_model: String,
    pub env_key_name: String,
    #[serde(default)]
    pub has_key: bool,
    #[serde(default)]
    pub credential_ready: bool,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_mode: String,
    #[serde(default)]
    pub oauth_supported: bool,
    #[serde(default)]
    pub oauth_available: bool,
    #[serde(default)]
    pub oauth_source_label: Option<String>,
    #[serde(default)]
    pub oauth_source_location: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub preferred_cheap: bool,
    #[serde(default)]
    pub discovery_supported: bool,
    pub primary_model: Option<String>,
    pub cheap_model: Option<String>,
    pub suggested_primary_model: Option<String>,
    pub suggested_cheap_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ProvidersConfigResponse {
    pub routing_enabled: bool,
    pub routing_mode: String,
    pub cascade_enabled: bool,
    pub tool_phase_synthesis_enabled: bool,
    pub tool_phase_primary_thinking_enabled: bool,
    pub compatible_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub bedrock_region: Option<String>,
    pub bedrock_proxy_url: Option<String>,
    pub llama_cpp_server_url: Option<String>,
    pub primary_provider: Option<String>,
    pub primary_model: Option<String>,
    pub preferred_cheap_provider: Option<String>,
    pub cheap_model: Option<String>,
    #[serde(default)]
    pub primary_pool_order: Vec<String>,
    #[serde(default)]
    pub cheap_pool_order: Vec<String>,
    pub fallback_chain: Vec<String>,
    pub policy_rules: Vec<RoutingRule>,
    pub providers: Vec<ProviderConfigEntry>,
    pub runtime_revision: Option<u64>,
    pub last_reload_error: Option<String>,
    pub advisor_max_calls: u32,
    pub advisor_auto_escalation_mode: AdvisorAutoEscalationMode,
    pub advisor_escalation_prompt: Option<String>,
    #[serde(default)]
    pub advisor_ready: bool,
    #[serde(default)]
    pub advisor_disabled_reason: Option<String>,
    #[serde(default)]
    pub executor_target: Option<String>,
    #[serde(default)]
    pub advisor_target: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub derived_defaults: ProvidersSettings,
    pub persisted: ProvidersSettings,
    pub effective: ProvidersSettings,
}

#[derive(serde::Serialize)]
pub struct ProviderModelsResponse {
    pub slug: String,
    pub display_name: String,
    pub discovery_supported: bool,
    pub discovery_status: String,
    pub error: Option<String>,
    pub current_primary_model: Option<String>,
    pub current_cheap_model: Option<String>,
    pub suggested_primary_model: Option<String>,
    pub suggested_cheap_model: Option<String>,
    pub models: Vec<ProviderModelOption>,
}

#[derive(Debug, Clone)]
pub struct ProviderModelsResponseInput {
    pub slug: String,
    pub display_name: String,
    pub discovery_supported: bool,
    pub discovery_status: String,
    pub error: Option<String>,
    pub current_primary_model: Option<String>,
    pub current_cheap_model: Option<String>,
    pub suggested_primary_model: Option<String>,
    pub suggested_cheap_model: Option<String>,
    pub models: Vec<ProviderModelOption>,
}

pub fn provider_models_response(input: ProviderModelsResponseInput) -> ProviderModelsResponse {
    ProviderModelsResponse {
        slug: input.slug,
        display_name: input.display_name,
        discovery_supported: input.discovery_supported,
        discovery_status: input.discovery_status,
        error: input.error,
        current_primary_model: input.current_primary_model,
        current_cheap_model: input.current_cheap_model,
        suggested_primary_model: input.suggested_primary_model,
        suggested_cheap_model: input.suggested_cheap_model,
        models: input.models,
    }
}

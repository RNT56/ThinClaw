//! Root-independent provider gateway policies.

use axum::http::StatusCode;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use thinclaw_llm_core::RoutingRule;
use thinclaw_settings::{
    AdvisorAutoEscalationMode, ProviderCredentialMode, ProviderModelSlots, ProvidersSettings,
    RoutingMode, Settings,
};

pub const PROVIDER_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_RUNTIME_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_SECRETS_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const PROVIDER_CREDENTIAL_SPEC_NOT_FOUND_STATUS: StatusCode = StatusCode::NOT_FOUND;
pub const PROVIDER_SENSITIVE_ROUTE_FORBIDDEN_STATUS: StatusCode = StatusCode::FORBIDDEN;
pub const PROVIDERS_ENABLED_SETTING_KEY: &str = "providers.enabled";
pub const PROVIDERS_FALLBACK_CHAIN_SETTING_KEY: &str = "providers.fallback_chain";

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

#[derive(serde::Deserialize)]
pub struct ProvidersConfigWriteRequest {
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
    #[serde(default = "default_advisor_max_calls_api")]
    pub advisor_max_calls: u32,
    #[serde(default)]
    pub advisor_auto_escalation_mode: AdvisorAutoEscalationMode,
    #[serde(default)]
    pub advisor_escalation_prompt: Option<String>,
    #[serde(default)]
    pub auto_fix: bool,
}

fn default_advisor_max_calls_api() -> u32 {
    4
}

pub fn apply_providers_config_write(settings: &mut Settings, body: &ProvidersConfigWriteRequest) {
    settings.providers.smart_routing_enabled = body.routing_enabled;
    settings.providers.routing_mode = routing_mode_from_api(body.routing_mode.as_str());
    settings.providers.smart_routing_cascade = body.cascade_enabled;
    settings.providers.tool_phase_synthesis_enabled = body.tool_phase_synthesis_enabled;
    settings.providers.tool_phase_primary_thinking_enabled =
        body.tool_phase_primary_thinking_enabled;
    settings.openai_compatible_base_url = trimmed_optional_model(body.compatible_base_url.as_ref());
    settings.ollama_base_url = trimmed_optional_model(body.ollama_base_url.as_ref());
    settings.bedrock_region = trimmed_optional_model(body.bedrock_region.as_ref());
    settings.bedrock_proxy_url = trimmed_optional_model(body.bedrock_proxy_url.as_ref());
    settings.llama_cpp_server_url = trimmed_optional_model(body.llama_cpp_server_url.as_ref());
    settings.providers.primary = body.primary_provider.clone();
    settings.providers.primary_model = trimmed_optional_model(body.primary_model.as_ref());
    settings.providers.preferred_cheap_provider = body.preferred_cheap_provider.clone();
    settings.providers.cheap_model = trimmed_optional_model(body.cheap_model.as_ref());
    settings.providers.primary_pool_order = body.primary_pool_order.clone();
    settings.providers.cheap_pool_order = body.cheap_pool_order.clone();
    settings.providers.fallback_chain = body.fallback_chain.clone();
    settings.providers.policy_rules = body.policy_rules.clone();
    settings.providers.advisor_max_calls = body.advisor_max_calls;
    settings.providers.advisor_auto_escalation_mode = body.advisor_auto_escalation_mode;
    settings.providers.advisor_escalation_prompt =
        trimmed_optional_model(body.advisor_escalation_prompt.as_ref());
    settings.providers.enabled = body
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| provider.slug.clone())
        .collect();

    settings.providers.provider_credential_modes.clear();
    let previous_provider_models = settings.providers.provider_models.clone();
    let previous_allowed_models = settings.providers.allowed_models.clone();
    settings.providers.allowed_models.clear();
    settings.providers.provider_models.clear();

    for provider in &body.providers {
        let auth_mode = provider_credential_mode_from_api(provider.auth_mode.as_str());
        if provider.oauth_supported && auth_mode == ProviderCredentialMode::ExternalOAuthSync {
            settings
                .providers
                .provider_credential_modes
                .insert(provider.slug.clone(), auth_mode);
        }

        let previous_slots =
            previous_provider_models
                .get(&provider.slug)
                .map(|slots| ProviderModelSlotsSnapshot {
                    primary: slots.primary.clone(),
                    cheap: slots.cheap.clone(),
                });
        let input = SavedProviderModelInput {
            default_model: provider.default_model.clone(),
            enabled: provider.enabled,
            primary: provider.primary,
            preferred_cheap: provider.preferred_cheap,
            primary_model: provider.primary_model.clone(),
            cheap_model: provider.cheap_model.clone(),
            suggested_primary_model: provider.suggested_primary_model.clone(),
            suggested_cheap_model: provider.suggested_cheap_model.clone(),
        };
        let resolved = resolve_saved_provider_models(
            &input,
            previous_slots.as_ref(),
            previous_allowed_models
                .get(&provider.slug)
                .map(Vec::as_slice),
        );

        if resolved.should_persist_slots {
            settings.providers.provider_models.insert(
                provider.slug.clone(),
                ProviderModelSlots {
                    primary: resolved.primary_model.clone(),
                    cheap: resolved.cheap_model.clone(),
                },
            );
        }

        if provider.primary {
            settings.providers.primary = Some(provider.slug.clone());
            settings.providers.primary_model = resolved.primary_model.clone();
        }
        if provider.preferred_cheap {
            settings.providers.preferred_cheap_provider = Some(provider.slug.clone());
        }
        if provider.enabled
            && let Some(model) = resolved.primary_model.as_deref()
        {
            settings
                .providers
                .allowed_models
                .insert(provider.slug.clone(), vec![model.to_string()]);
        }
    }

    let enabled_set: HashSet<String> = settings.providers.enabled.iter().cloned().collect();
    settings.providers.primary = settings
        .providers
        .primary
        .take()
        .filter(|slug| enabled_set.contains(slug));
    settings.providers.preferred_cheap_provider = settings
        .providers
        .preferred_cheap_provider
        .take()
        .filter(|slug| enabled_set.contains(slug));
    settings.providers.primary_pool_order =
        unique_enabled_provider_order(&settings.providers.primary_pool_order, &enabled_set);
    settings.providers.cheap_pool_order =
        unique_enabled_provider_order(&settings.providers.cheap_pool_order, &enabled_set);
    settings
        .providers
        .fallback_chain
        .retain(|entry| route_target_is_available_for_enabled_providers(entry, &enabled_set));

    if let Some(primary_slug) = settings.providers.primary.clone() {
        settings.providers.primary_model = settings
            .providers
            .provider_models
            .get(&primary_slug)
            .and_then(|slots| slots.primary.clone())
            .or(settings.providers.primary_model.clone());
    }

    if let Some(preferred_cheap_slug) = settings.providers.preferred_cheap_provider.clone() {
        settings.providers.cheap_model = settings
            .providers
            .provider_models
            .get(&preferred_cheap_slug)
            .and_then(|slots| {
                slots
                    .cheap
                    .as_ref()
                    .map(|model| format!("{preferred_cheap_slug}/{model}"))
            })
            .or(settings.providers.cheap_model.clone());
    } else if settings.providers.cheap_model.is_none() {
        settings.providers.cheap_model =
            settings
                .providers
                .provider_models
                .iter()
                .find_map(|(slug, slots)| {
                    enabled_set
                        .contains(slug)
                        .then(|| slots.cheap.as_ref().map(|model| format!("{slug}/{model}")))
                        .flatten()
                });
    }

    let explicit_provider_oauth = settings
        .providers
        .provider_credential_modes
        .values()
        .any(|mode| *mode == ProviderCredentialMode::ExternalOAuthSync);
    settings.providers.oauth_sync_enabled =
        explicit_provider_oauth || !settings.providers.oauth_sync_sources.is_empty();
}

fn routing_mode_from_api(value: &str) -> RoutingMode {
    match value {
        "cheap_split" => RoutingMode::CheapSplit,
        "advisor_executor" | "advisor" => RoutingMode::AdvisorExecutor,
        "policy" => RoutingMode::Policy,
        _ => RoutingMode::PrimaryOnly,
    }
}

fn provider_credential_mode_from_api(value: &str) -> ProviderCredentialMode {
    match value {
        "oauth_sync" => ProviderCredentialMode::ExternalOAuthSync,
        _ => ProviderCredentialMode::ApiKey,
    }
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

#[derive(serde::Deserialize)]
pub struct RouteSimulateRequest {
    pub prompt: String,
    #[serde(default)]
    pub has_vision: bool,
    #[serde(default)]
    pub has_tools: bool,
    #[serde(default)]
    pub requires_streaming: bool,
}

#[derive(serde::Serialize)]
pub struct RouteSimulateResponse {
    pub target: String,
    pub reason: String,
    #[serde(default)]
    pub fallback_chain: Vec<String>,
    #[serde(default)]
    pub candidate_list: Vec<String>,
    #[serde(default)]
    pub rejections: Vec<String>,
    #[serde(default)]
    pub score_breakdown: Vec<RouteSimulateScore>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct RouteSimulateScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

#[derive(Debug, Clone)]
pub struct RouteSimulateResponseInput {
    pub target: String,
    pub reason: String,
    pub fallback_chain: Vec<String>,
    pub candidate_list: Vec<String>,
    pub rejections: Vec<String>,
    pub score_breakdown: Vec<RouteSimulateScoreInput>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RouteSimulateScoreInput {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

pub fn route_simulate_response(input: RouteSimulateResponseInput) -> RouteSimulateResponse {
    RouteSimulateResponse {
        target: input.target,
        reason: input.reason,
        fallback_chain: input.fallback_chain,
        candidate_list: input.candidate_list,
        rejections: input.rejections,
        score_breakdown: input
            .score_breakdown
            .into_iter()
            .map(route_simulate_score)
            .collect(),
        diagnostics: input.diagnostics,
    }
}

pub fn route_simulate_score(input: RouteSimulateScoreInput) -> RouteSimulateScore {
    RouteSimulateScore {
        target: input.target,
        telemetry_key: input.telemetry_key,
        quality: input.quality,
        cost: input.cost,
        latency: input.latency,
        health: input.health,
        policy_bias: input.policy_bias,
        composite: input.composite,
    }
}

#[derive(serde::Deserialize)]
pub struct ProviderKeyRequest {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderKeyCredentialResponse {
    pub source: String,
    pub provider: String,
    pub masked_preview: Option<String>,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderKeyMutationResponse {
    pub status: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ProviderKeyCredentialResponse>,
}

pub fn provider_key_saved_response(
    display_name: &str,
    provider: impl Into<String>,
    masked_preview: Option<String>,
    fingerprint: Option<String>,
) -> ProviderKeyMutationResponse {
    ProviderKeyMutationResponse {
        status: "ok".to_string(),
        message: format!("Credentials saved for {display_name}"),
        credential: Some(ProviderKeyCredentialResponse {
            source: "local_encrypted".to_string(),
            provider: provider.into(),
            masked_preview,
            fingerprint,
        }),
    }
}

pub fn provider_key_save_partial_failure_response(
    display_name: &str,
    error: impl std::fmt::Display,
) -> ProviderKeyMutationResponse {
    ProviderKeyMutationResponse {
        status: "partial_failure".to_string(),
        message: format!(
            "{display_name} credentials were saved, but the live LLM runtime could not be reloaded: {error}"
        ),
        credential: None,
    }
}

pub fn provider_key_deleted_response(display_name: &str) -> ProviderKeyMutationResponse {
    ProviderKeyMutationResponse {
        status: "ok".to_string(),
        message: format!("Credentials removed for {display_name}"),
        credential: None,
    }
}

pub fn provider_key_delete_partial_failure_response(
    display_name: &str,
    error: impl std::fmt::Display,
) -> ProviderKeyMutationResponse {
    ProviderKeyMutationResponse {
        status: "partial_failure".to_string(),
        message: format!(
            "{display_name} credentials were removed, but the live LLM runtime could not be reloaded: {error}"
        ),
        credential: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredentialSpec {
    pub display_name: String,
    pub secret_name: String,
    pub default_model: String,
}

impl ProviderCredentialSpec {
    pub fn api_key(
        display_name: impl Into<String>,
        secret_name: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            display_name: display_name.into(),
            secret_name: secret_name.into(),
            default_model: default_model.into(),
        }
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }
}

pub fn fallback_provider_credential_spec(slug: &str) -> Option<ProviderCredentialSpec> {
    match slug {
        "openai_compatible" => Some(ProviderCredentialSpec::api_key(
            "OpenAI-compatible",
            "llm_compatible_api_key",
            "default",
        )),
        "bedrock" => Some(ProviderCredentialSpec::api_key(
            "AWS Bedrock",
            "llm_bedrock_api_key",
            "anthropic.claude-3-sonnet-20240229-v1:0",
        )),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIdentity {
    pub display_name: String,
    pub default_model: String,
}

impl ProviderIdentity {
    pub fn new(display_name: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            default_model: default_model.into(),
        }
    }
}

pub fn provider_identity(
    slug: &str,
    catalog_identity: Option<ProviderIdentity>,
) -> ProviderIdentity {
    if let Some(identity) = catalog_identity {
        return identity;
    }

    if let Some(spec) = fallback_provider_credential_spec(slug) {
        return ProviderIdentity::new(spec.display_name, spec.default_model);
    }

    match slug {
        "ollama" => ProviderIdentity::new("Ollama", "llama3"),
        "llama_cpp" => ProviderIdentity::new("llama.cpp", "llama-local"),
        other => ProviderIdentity::new(other, "default"),
    }
}

pub fn provider_supports_model_discovery(slug: &str, catalog_provider_exists: bool) -> bool {
    catalog_provider_exists
        || matches!(
            slug,
            "ollama" | "openai_compatible" | "bedrock" | "llama_cpp"
        )
}

pub fn suggested_cheap_model_from_catalog(
    default_model: &str,
    catalog_suggested_cheap_model: Option<&str>,
) -> Option<String> {
    catalog_suggested_cheap_model
        .map(str::to_string)
        .or_else(|| (!default_model.is_empty()).then(|| default_model.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticProviderEntryInput {
    pub slug: String,
    pub display_name: String,
    pub api_style: String,
    pub default_model: String,
    pub env_key_name: String,
    pub has_key: bool,
    pub auth_required: bool,
    pub oauth_supported: bool,
    pub discovery_supported: bool,
    pub suggested_primary_model: Option<String>,
    pub suggested_cheap_model: Option<String>,
    pub setup_url: Option<String>,
    pub tier: Option<String>,
}

pub fn synthetic_provider_entry(
    input: SyntheticProviderEntryInput,
    providers_settings: &ProvidersSettings,
    settings: &Settings,
) -> ProviderConfigEntry {
    ProviderConfigEntry {
        slug: input.slug.clone(),
        display_name: input.display_name,
        api_style: input.api_style,
        default_model: input.default_model.clone(),
        env_key_name: input.env_key_name,
        has_key: input.has_key,
        credential_ready: input.has_key,
        auth_required: input.auth_required,
        auth_mode: "api_key".to_string(),
        oauth_supported: input.oauth_supported,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        enabled: providers_settings
            .enabled
            .iter()
            .any(|enabled| enabled == &input.slug),
        primary: providers_settings.primary.as_deref() == Some(input.slug.as_str()),
        preferred_cheap: providers_settings.preferred_cheap_provider.as_deref()
            == Some(input.slug.as_str()),
        discovery_supported: input.discovery_supported,
        primary_model: provider_primary_model_for_slug(
            settings,
            providers_settings,
            &input.slug,
            &input.default_model,
        ),
        cheap_model: provider_cheap_model_for_slug(
            settings,
            providers_settings,
            &input.slug,
            &input.default_model,
            input.suggested_cheap_model.as_deref(),
        ),
        suggested_primary_model: input.suggested_primary_model,
        suggested_cheap_model: input.suggested_cheap_model,
        setup_url: input.setup_url,
        tier: input.tier,
    }
}

pub fn provider_auth_mode(
    providers_settings: &ProvidersSettings,
    slug: &str,
) -> ProviderCredentialMode {
    providers_settings
        .provider_credential_modes
        .get(slug)
        .copied()
        .unwrap_or_default()
}

pub fn provider_primary_model_for_slug(
    settings: &Settings,
    providers_settings: &ProvidersSettings,
    slug: &str,
    default_model: &str,
) -> Option<String> {
    providers_settings
        .provider_models
        .get(slug)
        .and_then(|slots| slots.primary.clone())
        .or_else(|| {
            if providers_settings.primary.as_deref() == Some(slug) {
                providers_settings.primary_model.clone()
            } else {
                providers_settings
                    .allowed_models
                    .get(slug)
                    .and_then(|models| models.first().cloned())
            }
        })
        .or_else(|| {
            if matches!(
                settings.llm_backend.as_deref(),
                Some(current) if current == slug || (slug == "openrouter" && current == "openai_compatible")
            ) {
                settings.selected_model.clone()
            } else {
                None
            }
        })
        .or_else(|| {
            if providers_settings.enabled.iter().any(|enabled| enabled == slug) {
                Some(default_model.to_string())
            } else {
                None
            }
        })
}

pub fn provider_cheap_model_for_slug(
    settings: &Settings,
    providers_settings: &ProvidersSettings,
    slug: &str,
    default_model: &str,
    suggested_cheap_model: Option<&str>,
) -> Option<String> {
    providers_settings
        .provider_models
        .get(slug)
        .and_then(|slots| slots.cheap.clone())
        .or_else(|| {
            providers_settings
                .cheap_model
                .as_deref()
                .and_then(|spec| spec.split_once('/'))
                .and_then(|(cheap_slug, model)| (cheap_slug == slug).then(|| model.to_string()))
        })
        .or_else(|| suggested_cheap_model.map(str::to_string))
        .or_else(|| (!default_model.is_empty()).then(|| default_model.to_string()))
        .or_else(|| {
            provider_primary_model_for_slug(settings, providers_settings, slug, default_model)
        })
}

pub fn sync_legacy_llm_settings(settings: &mut Settings) {
    match settings.providers.primary.as_deref() {
        Some("openai") => settings.llm_backend = Some("openai".to_string()),
        Some("anthropic") => settings.llm_backend = Some("anthropic".to_string()),
        Some("ollama") => settings.llm_backend = Some("ollama".to_string()),
        Some("gemini") => settings.llm_backend = Some("gemini".to_string()),
        Some("tinfoil") => settings.llm_backend = Some("tinfoil".to_string()),
        Some("bedrock") => settings.llm_backend = Some("bedrock".to_string()),
        Some("llama_cpp") => settings.llm_backend = Some("llama_cpp".to_string()),
        Some("openrouter") => {
            settings.llm_backend = Some("openai_compatible".to_string());
            settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        }
        Some("openai_compatible") => {
            settings.llm_backend = Some("openai_compatible".to_string());
        }
        _ => {
            settings.llm_backend = None;
        }
    }

    settings.selected_model = settings.providers.primary_model.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderModelOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    pub source: String,
    pub recommended_primary: bool,
    pub recommended_cheap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProviderModel {
    pub id: String,
    pub name: String,
    pub is_chat: bool,
    pub context_length: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderModelSlotsSnapshot {
    pub primary: Option<String>,
    pub cheap: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProviderModelInput {
    pub default_model: String,
    pub enabled: bool,
    pub primary: bool,
    pub preferred_cheap: bool,
    pub primary_model: Option<String>,
    pub cheap_model: Option<String>,
    pub suggested_primary_model: Option<String>,
    pub suggested_cheap_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSavedProviderModels {
    pub primary_model: Option<String>,
    pub cheap_model: Option<String>,
    pub should_persist_slots: bool,
}

pub fn provider_model_options_from_discovery(
    slug: &str,
    default_model: &str,
    discovered: Vec<DiscoveredProviderModel>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    catalog_suggested_cheap_model: Option<&str>,
) -> (
    Vec<ProviderModelOption>,
    Option<String>,
    Option<String>,
    bool,
) {
    let mut discovered_map = BTreeMap::new();
    for model in discovered.into_iter().filter(|model| {
        if slug == "openai" {
            is_openai_chat_model(&model.id)
        } else {
            model.is_chat
        }
    }) {
        discovered_map.entry(model.id.clone()).or_insert(model);
    }

    let has_live_models = !discovered_map.is_empty();
    let current_primary_model =
        current_primary_model.filter(|model| discovered_map.contains_key(*model));
    let current_cheap_model =
        current_cheap_model.filter(|model| discovered_map.contains_key(*model));
    let preferred_default_model = (!default_model.is_empty()
        && discovered_map.contains_key(default_model))
    .then(|| default_model.to_string());
    let suggested_provider_cheap = catalog_suggested_cheap_model
        .map(str::to_string)
        .filter(|model| discovered_map.contains_key(model.as_str()));

    let suggested_primary_model = current_primary_model
        .map(str::to_string)
        .or_else(|| preferred_default_model.clone())
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| primary_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                None
            } else {
                Some(default_model.to_string())
            }
        });

    let suggested_cheap_model = current_cheap_model
        .map(str::to_string)
        .or(suggested_provider_cheap)
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| cheap_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                suggested_primary_model.clone()
            } else {
                catalog_suggested_cheap_model
                    .map(str::to_string)
                    .or_else(|| suggested_primary_model.clone())
            }
        });

    let mut model_ids = BTreeSet::new();
    let mut ordered_ids = Vec::new();
    for id in discovered_map.keys() {
        if model_ids.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }

    ordered_ids.sort_by(|a, b| {
        if matches!(slug, "openai" | "minimax" | "cohere") {
            let priority = |model: &String| match slug {
                "openai" => openai_model_priority(model),
                "minimax" => minimax_model_priority(model),
                "cohere" => cohere_model_priority(model),
                _ => usize::MAX,
            };
            priority(a).cmp(&priority(b))
        } else {
            model_display_rank(
                a,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            )
            .cmp(&model_display_rank(
                b,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            ))
            .reverse()
            .then_with(|| a.cmp(b))
        }
    });

    let models = ordered_ids
        .into_iter()
        .map(|id| {
            let discovered = discovered_map.get(&id);
            ProviderModelOption {
                id: id.clone(),
                label: discovered
                    .map(|model| model.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| id.clone()),
                context_length: discovered.and_then(|model| model.context_length),
                source: if discovered.is_some() {
                    "discovered".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model.as_deref() == Some(id.as_str()),
                recommended_cheap: suggested_cheap_model.as_deref() == Some(id.as_str()),
            }
        })
        .collect();

    (
        models,
        suggested_primary_model,
        suggested_cheap_model,
        has_live_models,
    )
}

pub fn fallback_provider_model_options(
    default_model: &str,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
    fallback_models: impl IntoIterator<Item = (String, String)>,
) -> Vec<ProviderModelOption> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for id in [
        current_primary_model,
        current_cheap_model,
        suggested_primary_model,
        suggested_cheap_model,
        Some(default_model),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(id.to_string()) {
            models.push(ProviderModelOption {
                id: id.to_string(),
                label: id.to_string(),
                context_length: None,
                source: if id == default_model {
                    "default".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model == Some(id),
                recommended_cheap: suggested_cheap_model == Some(id),
            });
        }
    }

    for (static_id, label) in fallback_models {
        if seen.insert(static_id.clone()) {
            models.push(ProviderModelOption {
                id: static_id,
                label,
                context_length: None,
                source: "curated".to_string(),
                recommended_primary: false,
                recommended_cheap: false,
            });
        }
    }

    if models.is_empty() && !default_model.is_empty() {
        models.push(ProviderModelOption {
            id: default_model.to_string(),
            label: default_model.to_string(),
            context_length: None,
            source: "default".to_string(),
            recommended_primary: true,
            recommended_cheap: suggested_cheap_model == Some(default_model),
        });
    }

    models
}

pub fn static_fallback_provider_models(slug: &str) -> Vec<(String, String)> {
    match slug {
        "anthropic" => vec![
            (
                "claude-opus-4-7".to_string(),
                "Claude Opus 4.7 (recommended)".to_string(),
            ),
            (
                "claude-opus-4-6".to_string(),
                "Claude Opus 4.6 (latest)".to_string(),
            ),
            (
                "claude-sonnet-4-6".to_string(),
                "Claude Sonnet 4.6".to_string(),
            ),
            ("claude-opus-4-5".to_string(), "Claude Opus 4.5".to_string()),
            (
                "claude-sonnet-4-5".to_string(),
                "Claude Sonnet 4.5".to_string(),
            ),
            (
                "claude-haiku-4-5".to_string(),
                "Claude Haiku 4.5 (fast)".to_string(),
            ),
        ],
        "openai" => vec![
            (
                "gpt-5.3-codex".to_string(),
                "GPT-5.3 Codex (latest)".to_string(),
            ),
            ("gpt-5.2-codex".to_string(), "GPT-5.2 Codex".to_string()),
            ("gpt-5.2".to_string(), "GPT-5.2".to_string()),
            (
                "gpt-5.1-codex-mini".to_string(),
                "GPT-5.1 Codex Mini (fast)".to_string(),
            ),
            ("gpt-5".to_string(), "GPT-5".to_string()),
            ("gpt-5-mini".to_string(), "GPT-5 Mini".to_string()),
            ("gpt-4.1".to_string(), "GPT-4.1".to_string()),
            ("gpt-4.1-mini".to_string(), "GPT-4.1 Mini".to_string()),
            (
                "o4-mini".to_string(),
                "o4-mini (fast reasoning)".to_string(),
            ),
            ("o3".to_string(), "o3 (reasoning)".to_string()),
        ],
        "gemini" => vec![
            ("gemini-2.5-pro".to_string(), "Gemini 2.5 Pro".to_string()),
            (
                "gemini-2.5-flash".to_string(),
                "Gemini 2.5 Flash".to_string(),
            ),
            (
                "gemini-2.5-flash-lite".to_string(),
                "Gemini 2.5 Flash Lite".to_string(),
            ),
        ],
        "groq" => vec![
            (
                "llama-3.3-70b-versatile".to_string(),
                "Llama 3.3 70B".to_string(),
            ),
            (
                "llama-3.1-8b-instant".to_string(),
                "Llama 3.1 8B Instant".to_string(),
            ),
        ],
        "mistral" => vec![
            (
                "mistral-large-latest".to_string(),
                "Mistral Large".to_string(),
            ),
            (
                "mistral-small-latest".to_string(),
                "Mistral Small".to_string(),
            ),
        ],
        "xai" => vec![
            ("grok-3".to_string(), "Grok 3".to_string()),
            ("grok-3-mini".to_string(), "Grok 3 Mini".to_string()),
        ],
        "deepseek" => vec![
            ("deepseek-chat".to_string(), "DeepSeek Chat".to_string()),
            (
                "deepseek-reasoner".to_string(),
                "DeepSeek Reasoner".to_string(),
            ),
        ],
        "openrouter" => vec![
            (
                "anthropic/claude-sonnet-4-20250514".to_string(),
                "Claude Sonnet 4 (via OR)".to_string(),
            ),
            (
                "openai/gpt-5.3-codex".to_string(),
                "GPT-5.3 Codex (via OR)".to_string(),
            ),
            (
                "google/gemini-2.5-flash".to_string(),
                "Gemini 2.5 Flash (via OR)".to_string(),
            ),
        ],
        "together" => vec![
            (
                "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string(),
                "Llama 3.3 70B Turbo".to_string(),
            ),
            (
                "meta-llama/Llama-3.1-8B-Instruct-Turbo".to_string(),
                "Llama 3.1 8B Turbo".to_string(),
            ),
        ],
        "cerebras" => vec![("llama-3.3-70b".to_string(), "Llama 3.3 70B".to_string())],
        "nvidia" => vec![(
            "meta/llama-3.3-70b-instruct".to_string(),
            "Llama 3.3 70B".to_string(),
        )],
        "minimax" => vec![
            ("MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
            ("MiniMax-M2.5".to_string(), "MiniMax M2.5".to_string()),
            (
                "MiniMax-M2.5-highspeed".to_string(),
                "MiniMax M2.5 Highspeed".to_string(),
            ),
            ("MiniMax-M2.1".to_string(), "MiniMax M2.1".to_string()),
            (
                "MiniMax-M2.1-highspeed".to_string(),
                "MiniMax M2.1 Highspeed".to_string(),
            ),
            ("MiniMax-M2".to_string(), "MiniMax M2".to_string()),
        ],
        "cohere" => vec![
            ("command-a-03-2025".to_string(), "Command A".to_string()),
            (
                "command-r-plus-08-2024".to_string(),
                "Command R+".to_string(),
            ),
            ("command-r-08-2024".to_string(), "Command R".to_string()),
            ("command-r7b-12-2024".to_string(), "Command R7B".to_string()),
        ],
        "tinfoil" => vec![("kimi-k2-5".to_string(), "Kimi K2.5".to_string())],
        _ => vec![],
    }
}

pub fn provider_fallback_model_catalog(
    slug: &str,
    dynamic_models: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let dynamic: Vec<_> = dynamic_models.into_iter().collect();
    if dynamic.is_empty() {
        static_fallback_provider_models(slug)
    } else {
        dynamic
    }
}

pub fn trimmed_optional_model(value: Option<&String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub fn resolve_saved_provider_models(
    provider: &SavedProviderModelInput,
    previous_slots: Option<&ProviderModelSlotsSnapshot>,
    previous_allowed_models: Option<&[String]>,
) -> ResolvedSavedProviderModels {
    let previous_primary_model = previous_slots
        .and_then(|slots| slots.primary.clone())
        .or_else(|| previous_allowed_models.and_then(|models| models.first().cloned()));
    let previous_cheap_model = previous_slots.and_then(|slots| slots.cheap.clone());
    let incoming_primary_model = trimmed_optional_model(provider.primary_model.as_ref());
    let incoming_cheap_model = trimmed_optional_model(provider.cheap_model.as_ref());
    let suggested_primary_model = trimmed_optional_model(provider.suggested_primary_model.as_ref())
        .or_else(|| previous_primary_model.clone())
        .or_else(|| {
            if provider.enabled || provider.primary {
                Some(provider.default_model.clone())
            } else {
                None
            }
        });
    let primary_model = incoming_primary_model
        .clone()
        .or_else(|| previous_primary_model.clone())
        .or_else(|| suggested_primary_model.clone());
    let suggested_cheap_model = trimmed_optional_model(provider.suggested_cheap_model.as_ref())
        .or_else(|| previous_cheap_model.clone())
        .or_else(|| primary_model.clone());
    let cheap_model = incoming_cheap_model
        .clone()
        .or_else(|| previous_cheap_model.clone())
        .or_else(|| suggested_cheap_model.clone())
        .or_else(|| primary_model.clone());
    let should_persist_slots = provider.enabled
        || provider.primary
        || provider.preferred_cheap
        || incoming_primary_model.is_some()
        || incoming_cheap_model.is_some()
        || previous_slots.is_some();

    ResolvedSavedProviderModels {
        primary_model,
        cheap_model,
        should_persist_slots,
    }
}

pub fn stale_provider_namespace_keys(
    previous: &std::collections::HashMap<String, serde_json::Value>,
    next: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    const PROVIDER_OBJECT_PREFIXES: &[&str] =
        &["providers.allowed_models.", "providers.provider_models."];

    previous
        .keys()
        .filter(|key| {
            PROVIDER_OBJECT_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
                && !next.contains_key(*key)
        })
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSettingListMutation {
    pub key: &'static str,
    pub value: Vec<String>,
}

pub fn provider_auto_enable_setting_updates(
    enabled: Option<serde_json::Value>,
    fallback_chain: Option<serde_json::Value>,
    slug: &str,
    default_model: &str,
) -> Vec<ProviderSettingListMutation> {
    let mut updates = Vec::new();

    let mut enabled_list: Vec<String> = enabled
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if !enabled_list.iter().any(|entry| entry == slug) {
        enabled_list.push(slug.to_string());
        updates.push(ProviderSettingListMutation {
            key: PROVIDERS_ENABLED_SETTING_KEY,
            value: enabled_list,
        });
    }

    let mut fallback_chain_list: Vec<String> = fallback_chain
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let provider_prefix = format!("{slug}/");
    if !fallback_chain_list
        .iter()
        .any(|entry| entry.starts_with(&provider_prefix))
    {
        fallback_chain_list.push(format!("{slug}/{default_model}"));
        updates.push(ProviderSettingListMutation {
            key: PROVIDERS_FALLBACK_CHAIN_SETTING_KEY,
            value: fallback_chain_list,
        });
    }

    updates
}

pub fn provider_auto_disable_setting_updates(
    enabled: Option<serde_json::Value>,
    fallback_chain: Option<serde_json::Value>,
    slug: &str,
) -> Vec<ProviderSettingListMutation> {
    let mut updates = Vec::new();

    if let Some(mut enabled_list) =
        enabled.and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
    {
        let before = enabled_list.len();
        enabled_list.retain(|entry| entry != slug);
        if enabled_list.len() != before {
            updates.push(ProviderSettingListMutation {
                key: PROVIDERS_ENABLED_SETTING_KEY,
                value: enabled_list,
            });
        }
    }

    if let Some(mut fallback_chain_list) =
        fallback_chain.and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
    {
        let provider_prefix = format!("{slug}/");
        let before = fallback_chain_list.len();
        fallback_chain_list.retain(|entry| !entry.starts_with(&provider_prefix));
        if fallback_chain_list.len() != before {
            updates.push(ProviderSettingListMutation {
                key: PROVIDERS_FALLBACK_CHAIN_SETTING_KEY,
                value: fallback_chain_list,
            });
        }
    }

    updates
}

pub fn unique_enabled_provider_order(entries: &[String], enabled: &HashSet<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for entry in entries {
        if enabled.contains(entry) && !unique.iter().any(|existing| existing == entry) {
            unique.push(entry.clone());
        }
    }
    unique
}

pub fn route_target_is_available_for_enabled_providers(
    target: &str,
    enabled: &HashSet<String>,
) -> bool {
    if matches!(target, "primary" | "cheap") {
        return true;
    }
    if let Some(slug) = target
        .strip_suffix("@primary")
        .or_else(|| target.strip_suffix("@cheap"))
    {
        return enabled.contains(slug);
    }
    if let Some((slug, _)) = target.split_once('/') {
        return enabled.contains(slug);
    }
    false
}

pub fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");
    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");
    is_chat_family && !is_non_chat_variant
}

pub fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

pub fn minimax_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "minimax-m2.7",
        "minimax-m2.5",
        "minimax-m2.5-highspeed",
        "minimax-m2.1",
        "minimax-m2.1-highspeed",
        "minimax-m2",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }
    if id.contains("m2.7") {
        return EXACT_PRIORITY.len();
    }
    if id.contains("m2.5") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.contains("m2.5") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 2;
    }
    if id.contains("m2.1") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 3;
    }
    if id.contains("m2.1") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 4;
    }
    if id.contains("m2") {
        return EXACT_PRIORITY.len() + 5;
    }
    EXACT_PRIORITY.len() + 50
}

pub fn cohere_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "command-a-03-2025",
        "command-r-plus-08-2024",
        "command-r-08-2024",
        "command-r7b-12-2024",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }
    if id.starts_with("command-a") {
        return EXACT_PRIORITY.len();
    }
    if id.starts_with("command-r-plus") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.starts_with("command-r") {
        return EXACT_PRIORITY.len() + 2;
    }
    EXACT_PRIORITY.len() + 50
}

pub fn primary_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("pro")
        || lower.contains("sonnet")
        || lower.contains("opus")
        || lower.contains("command-a")
        || lower.contains("4o")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score += 40;
    }
    if lower.contains("m2.7") {
        score += 52;
    } else if lower.contains("m2.5") && !lower.contains("highspeed") {
        score += 48;
    } else if lower.contains("m2.1") && !lower.contains("highspeed") {
        score += 44;
    } else if lower.contains("command-r-plus") {
        score += 34;
    }
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("8b")
        || lower.contains("instant")
    {
        score -= 18;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score -= 14;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

pub fn cheap_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("flash")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("instant")
        || lower.contains("8b")
    {
        score += 45;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score += 42;
    }
    if lower.contains("pro")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("command-a")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score -= 18;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

pub fn model_display_rank(
    model: &str,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
) -> i32 {
    let mut score = primary_model_rank(model).max(cheap_model_rank(model));
    if suggested_primary_model == Some(model) {
        score += 60;
    }
    if suggested_cheap_model == Some(model) {
        score += 50;
    }
    if current_primary_model == Some(model) {
        score += 40;
    }
    if current_cheap_model == Some(model) {
        score += 35;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_entry(slug: &str) -> ProviderConfigEntry {
        ProviderConfigEntry {
            slug: slug.to_string(),
            display_name: slug.to_string(),
            api_style: "openai".to_string(),
            default_model: format!("{slug}-default"),
            env_key_name: format!("{}_API_KEY", slug.to_ascii_uppercase()),
            has_key: false,
            credential_ready: false,
            auth_required: true,
            auth_mode: "api_key".to_string(),
            oauth_supported: false,
            oauth_available: false,
            oauth_source_label: None,
            oauth_source_location: None,
            enabled: false,
            primary: false,
            preferred_cheap: false,
            discovery_supported: true,
            primary_model: None,
            cheap_model: None,
            suggested_primary_model: None,
            suggested_cheap_model: None,
            setup_url: None,
            tier: None,
        }
    }

    fn write_request(providers: Vec<ProviderConfigEntry>) -> ProvidersConfigWriteRequest {
        ProvidersConfigWriteRequest {
            routing_enabled: true,
            routing_mode: "primary_only".to_string(),
            cascade_enabled: true,
            tool_phase_synthesis_enabled: false,
            tool_phase_primary_thinking_enabled: true,
            compatible_base_url: None,
            ollama_base_url: None,
            bedrock_region: None,
            bedrock_proxy_url: None,
            llama_cpp_server_url: None,
            primary_provider: None,
            primary_model: None,
            preferred_cheap_provider: None,
            cheap_model: None,
            primary_pool_order: Vec::new(),
            cheap_pool_order: Vec::new(),
            fallback_chain: Vec::new(),
            policy_rules: Vec::new(),
            providers,
            advisor_max_calls: 4,
            advisor_auto_escalation_mode: AdvisorAutoEscalationMode::default(),
            advisor_escalation_prompt: None,
            auto_fix: false,
        }
    }

    #[test]
    fn provider_api_key_validation_trims_and_accepts_plain_values() {
        assert_eq!(
            validate_provider_api_key(Some(" sk-test ")),
            Ok("sk-test".to_string())
        );
    }

    #[test]
    fn provider_api_key_validation_rejects_empty_values() {
        let err = validate_provider_api_key(Some("  ")).unwrap_err();
        assert_eq!(err, ProviderApiKeyError::Missing);
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn provider_api_key_validation_rejects_control_characters() {
        let err = validate_provider_api_key(Some("sk-test\nnext")).unwrap_err();
        assert_eq!(err, ProviderApiKeyError::InvalidCharacters);
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn provider_status_helpers_preserve_existing_statuses_and_messages() {
        assert_eq!(
            provider_store_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            provider_runtime_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            provider_secrets_store_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            provider_credential_spec_not_found_status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            provider_sensitive_route_forbidden_status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            provider_credentials_not_configured_message("OpenAI"),
            "OpenAI credentials are not configured"
        );
    }

    #[test]
    fn provider_key_masking_hides_short_values() {
        assert_eq!(mask_provider_key("short"), "****");
        assert_eq!(mask_provider_key("12345678"), "****");
    }

    #[test]
    fn provider_key_masking_keeps_edges_for_long_values() {
        assert_eq!(mask_provider_key("sk-1234567890"), "sk-1...7890");
    }

    #[test]
    fn provider_key_masking_is_unicode_safe() {
        assert_eq!(mask_provider_key("🔑abcd1234🔒"), "🔑abc...234🔒");
    }

    #[test]
    fn provider_key_fingerprint_is_stable_and_short() {
        let first = provider_key_fingerprint("sk-test");
        let second = provider_key_fingerprint("sk-test");
        assert_eq!(first, second);
        assert_eq!(first.len(), 24);
        assert_ne!(first, provider_key_fingerprint("sk-other"));
    }

    #[test]
    fn providers_list_response_sorts_by_display_name() {
        let response = providers_list_response(vec![
            provider_info(ProviderInfoInput {
                slug: "z".to_string(),
                display_name: "Zed".to_string(),
                api_style: "openai".to_string(),
                default_model: "z-model".to_string(),
                default_context_size: 1000,
                has_key: false,
                credential_ready: false,
                env_key_name: "Z_KEY".to_string(),
                auth_kind: "api_key".to_string(),
                auth_mode: "api_key".to_string(),
                oauth_supported: false,
                oauth_available: false,
                oauth_source_label: None,
                oauth_source_location: None,
                setup_url: None,
                tier: None,
                credential: None,
            }),
            provider_info(ProviderInfoInput {
                slug: "a".to_string(),
                display_name: "Alpha".to_string(),
                api_style: "openai".to_string(),
                default_model: "a-model".to_string(),
                default_context_size: 1000,
                has_key: true,
                credential_ready: true,
                env_key_name: "A_KEY".to_string(),
                auth_kind: "api_key".to_string(),
                auth_mode: "api_key".to_string(),
                oauth_supported: false,
                oauth_available: false,
                oauth_source_label: None,
                oauth_source_location: None,
                setup_url: None,
                tier: None,
                credential: None,
            }),
        ]);

        assert_eq!(response.providers[0].slug, "a");
        assert_eq!(response.providers[1].slug, "z");
        assert_eq!(
            serde_json::to_value(&response).unwrap()["providers"][0]["display_name"],
            serde_json::json!("Alpha")
        );
    }

    #[test]
    fn provider_oauth_ui_state_shapes_supported_and_unsupported_sources() {
        let supported = provider_oauth_ui_state(Some(ProviderOauthUiSourceInput {
            available: true,
            source_label: "Codex".to_string(),
            source_location: "~/.codex/auth.json".to_string(),
        }));

        assert_eq!(
            supported,
            ProviderOauthUiState {
                supported: true,
                available: true,
                source_label: Some("Codex".to_string()),
                source_location: Some("~/.codex/auth.json".to_string()),
            }
        );
        assert_eq!(
            provider_oauth_ui_state(None),
            ProviderOauthUiState {
                supported: false,
                available: false,
                source_label: None,
                source_location: None,
            }
        );
    }

    #[test]
    fn provider_models_response_preserves_existing_json_shape() {
        let response = provider_models_response(ProviderModelsResponseInput {
            slug: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            discovery_supported: true,
            discovery_status: "discovered".to_string(),
            error: None,
            current_primary_model: Some("gpt-5".to_string()),
            current_cheap_model: Some("gpt-5-mini".to_string()),
            suggested_primary_model: Some("gpt-5".to_string()),
            suggested_cheap_model: Some("gpt-5-mini".to_string()),
            models: vec![ProviderModelOption {
                id: "gpt-5".to_string(),
                label: "GPT-5".to_string(),
                context_length: Some(400_000),
                source: "discovered".to_string(),
                recommended_primary: true,
                recommended_cheap: false,
            }],
        });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "slug": "openai",
                "display_name": "OpenAI",
                "discovery_supported": true,
                "discovery_status": "discovered",
                "error": null,
                "current_primary_model": "gpt-5",
                "current_cheap_model": "gpt-5-mini",
                "suggested_primary_model": "gpt-5",
                "suggested_cheap_model": "gpt-5-mini",
                "models": [{
                    "id": "gpt-5",
                    "label": "GPT-5",
                    "context_length": 400000,
                    "source": "discovered",
                    "recommended_primary": true,
                    "recommended_cheap": false,
                }],
            })
        );
    }

    #[test]
    fn route_simulate_response_preserves_existing_json_shape() {
        let response = route_simulate_response(RouteSimulateResponseInput {
            target: "openai/gpt-5".to_string(),
            reason: "best candidate".to_string(),
            fallback_chain: vec!["anthropic/claude".to_string()],
            candidate_list: vec!["openai/gpt-5".to_string()],
            rejections: vec!["ollama/local: unhealthy".to_string()],
            score_breakdown: vec![RouteSimulateScoreInput {
                target: "openai/gpt-5".to_string(),
                telemetry_key: Some("openai".to_string()),
                quality: 0.9,
                cost: 0.2,
                latency: 0.4,
                health: 1.0,
                policy_bias: 0.1,
                composite: 0.8,
            }],
            diagnostics: vec!["routing enabled".to_string()],
        });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "target": "openai/gpt-5",
                "reason": "best candidate",
                "fallback_chain": ["anthropic/claude"],
                "candidate_list": ["openai/gpt-5"],
                "rejections": ["ollama/local: unhealthy"],
                "score_breakdown": [{
                    "target": "openai/gpt-5",
                    "telemetry_key": "openai",
                    "quality": 0.9,
                    "cost": 0.2,
                    "latency": 0.4,
                    "health": 1.0,
                    "policy_bias": 0.1,
                    "composite": 0.8,
                }],
                "diagnostics": ["routing enabled"],
            })
        );
    }

    #[test]
    fn provider_key_saved_response_preserves_existing_json_shape() {
        let api_key = "sk-1234567890";
        let response = provider_key_saved_response(
            "OpenAI",
            "openai",
            Some(mask_provider_key(api_key)),
            Some(provider_key_fingerprint(api_key)),
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "status": "ok",
                "message": "Credentials saved for OpenAI",
                "credential": {
                    "source": "local_encrypted",
                    "provider": "openai",
                    "masked_preview": "sk-1...7890",
                    "fingerprint": provider_key_fingerprint(api_key),
                }
            })
        );
    }

    #[test]
    fn provider_key_delete_response_preserves_existing_json_shape() {
        let response = provider_key_deleted_response("OpenAI");

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "status": "ok",
                "message": "Credentials removed for OpenAI",
            })
        );
    }

    #[test]
    fn provider_key_partial_failure_responses_preserve_existing_json_shape() {
        let save = provider_key_save_partial_failure_response("OpenAI", "reload failed");
        let delete = provider_key_delete_partial_failure_response("OpenAI", "reload failed");

        assert_eq!(
            serde_json::to_value(save).unwrap(),
            serde_json::json!({
                "status": "partial_failure",
                "message": "OpenAI credentials were saved, but the live LLM runtime could not be reloaded: reload failed",
            })
        );
        assert_eq!(
            serde_json::to_value(delete).unwrap(),
            serde_json::json!({
                "status": "partial_failure",
                "message": "OpenAI credentials were removed, but the live LLM runtime could not be reloaded: reload failed",
            })
        );
    }

    #[test]
    fn fallback_provider_credential_specs_cover_synthetic_providers() {
        let compat = fallback_provider_credential_spec("openai_compatible").unwrap();
        assert_eq!(compat.display_name(), "OpenAI-compatible");
        assert_eq!(compat.secret_name, "llm_compatible_api_key");
        assert_eq!(compat.default_model(), "default");

        let bedrock = fallback_provider_credential_spec("bedrock").unwrap();
        assert_eq!(bedrock.display_name(), "AWS Bedrock");
        assert_eq!(bedrock.secret_name, "llm_bedrock_api_key");
        assert_eq!(
            bedrock.default_model(),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );

        assert!(fallback_provider_credential_spec("unknown").is_none());
    }

    #[test]
    fn provider_identity_prefers_catalog_and_falls_back_for_synthetic_providers() {
        let catalog = provider_identity(
            "cataloged",
            Some(ProviderIdentity::new("Catalog Provider", "catalog-model")),
        );
        assert_eq!(catalog.display_name, "Catalog Provider");
        assert_eq!(catalog.default_model, "catalog-model");

        let compat = provider_identity("openai_compatible", None);
        assert_eq!(compat.display_name, "OpenAI-compatible");
        assert_eq!(compat.default_model, "default");

        let ollama = provider_identity("ollama", None);
        assert_eq!(ollama.display_name, "Ollama");
        assert_eq!(ollama.default_model, "llama3");

        let unknown = provider_identity("custom", None);
        assert_eq!(unknown.display_name, "custom");
        assert_eq!(unknown.default_model, "default");
    }

    #[test]
    fn provider_discovery_support_accepts_catalog_or_synthetic_providers() {
        assert!(provider_supports_model_discovery("cataloged", true));
        assert!(provider_supports_model_discovery("bedrock", false));
        assert!(provider_supports_model_discovery("llama_cpp", false));
        assert!(!provider_supports_model_discovery("custom", false));
    }

    #[test]
    fn suggested_cheap_model_prefers_catalog_then_default_model() {
        assert_eq!(
            suggested_cheap_model_from_catalog("primary-model", Some("cheap-model")).as_deref(),
            Some("cheap-model")
        );
        assert_eq!(
            suggested_cheap_model_from_catalog("primary-model", None).as_deref(),
            Some("primary-model")
        );
        assert_eq!(suggested_cheap_model_from_catalog("", None), None);
    }

    #[test]
    fn synthetic_provider_entry_applies_provider_settings() {
        let settings = Settings::default();
        let providers = ProvidersSettings {
            enabled: vec!["openai_compatible".to_string()],
            primary: Some("openai_compatible".to_string()),
            primary_model: Some("compat-primary".to_string()),
            preferred_cheap_provider: Some("openai_compatible".to_string()),
            cheap_model: Some("openai_compatible/compat-cheap".to_string()),
            ..ProvidersSettings::default()
        };

        let entry = synthetic_provider_entry(
            SyntheticProviderEntryInput {
                slug: "openai_compatible".to_string(),
                display_name: "OpenAI-compatible".to_string(),
                api_style: "openai_compatible".to_string(),
                default_model: "default".to_string(),
                env_key_name: "LLM_API_KEY".to_string(),
                has_key: true,
                auth_required: false,
                oauth_supported: false,
                discovery_supported: true,
                suggested_primary_model: Some("default".to_string()),
                suggested_cheap_model: Some("cheap-default".to_string()),
                setup_url: None,
                tier: None,
            },
            &providers,
            &settings,
        );

        assert!(entry.enabled);
        assert!(entry.primary);
        assert!(entry.preferred_cheap);
        assert!(entry.discovery_supported);
        assert_eq!(entry.primary_model.as_deref(), Some("compat-primary"));
        assert_eq!(entry.cheap_model.as_deref(), Some("compat-cheap"));
        assert_eq!(
            entry.suggested_cheap_model.as_deref(),
            Some("cheap-default")
        );
    }

    #[test]
    fn provider_auth_mode_uses_explicit_mode_or_api_key_default() {
        let mut providers = ProvidersSettings::default();
        providers.provider_credential_modes.insert(
            "openai".to_string(),
            ProviderCredentialMode::ExternalOAuthSync,
        );

        assert_eq!(
            provider_auth_mode(&providers, "openai"),
            ProviderCredentialMode::ExternalOAuthSync
        );
        assert_eq!(
            provider_auth_mode(&providers, "anthropic"),
            ProviderCredentialMode::ApiKey
        );
    }

    #[test]
    fn provider_config_write_applies_trimmed_fields_and_filters_disabled_targets() {
        let mut settings = Settings::default();
        let mut openai = config_entry("openai");
        openai.enabled = true;
        openai.primary = true;
        openai.oauth_supported = true;
        openai.auth_mode = "oauth_sync".to_string();
        openai.primary_model = Some(" gpt-5 ".to_string());
        openai.cheap_model = Some(" gpt-5-mini ".to_string());
        openai.suggested_primary_model = Some("gpt-5".to_string());
        openai.suggested_cheap_model = Some("gpt-5-mini".to_string());

        let mut gemini = config_entry("gemini");
        gemini.preferred_cheap = true;
        gemini.primary_model = Some("gemini-pro".to_string());
        gemini.cheap_model = Some("gemini-flash".to_string());

        let mut body = write_request(vec![openai, gemini]);
        body.routing_mode = "advisor".to_string();
        body.cascade_enabled = false;
        body.compatible_base_url = Some(" https://example.test/v1 ".to_string());
        body.ollama_base_url = Some("   ".to_string());
        body.primary_provider = Some("openai".to_string());
        body.preferred_cheap_provider = Some("gemini".to_string());
        body.primary_pool_order = vec![
            "gemini".to_string(),
            "openai".to_string(),
            "openai".to_string(),
        ];
        body.cheap_pool_order = body.primary_pool_order.clone();
        body.fallback_chain = vec![
            "openai/gpt-5".to_string(),
            "gemini/gemini-flash".to_string(),
            "primary".to_string(),
            "openai@cheap".to_string(),
            "unknown".to_string(),
        ];
        body.advisor_escalation_prompt = Some(" escalate carefully ".to_string());

        apply_providers_config_write(&mut settings, &body);

        assert_eq!(
            settings.providers.routing_mode,
            RoutingMode::AdvisorExecutor
        );
        assert!(!settings.providers.smart_routing_cascade);
        assert_eq!(
            settings.openai_compatible_base_url.as_deref(),
            Some("https://example.test/v1")
        );
        assert_eq!(settings.ollama_base_url, None);
        assert_eq!(
            settings.providers.advisor_escalation_prompt.as_deref(),
            Some("escalate carefully")
        );
        assert_eq!(settings.providers.enabled, vec!["openai"]);
        assert_eq!(settings.providers.primary.as_deref(), Some("openai"));
        assert_eq!(settings.providers.preferred_cheap_provider, None);
        assert_eq!(settings.providers.primary_pool_order, vec!["openai"]);
        assert_eq!(settings.providers.cheap_pool_order, vec!["openai"]);
        assert_eq!(
            settings.providers.fallback_chain,
            vec!["openai/gpt-5", "primary", "openai@cheap"]
        );
        assert_eq!(
            settings.providers.provider_credential_modes.get("openai"),
            Some(&ProviderCredentialMode::ExternalOAuthSync)
        );
        assert!(settings.providers.oauth_sync_enabled);
        assert_eq!(
            settings
                .providers
                .allowed_models
                .get("openai")
                .and_then(|models| models.first())
                .map(String::as_str),
            Some("gpt-5")
        );
        assert_eq!(
            settings
                .providers
                .provider_models
                .get("openai")
                .and_then(|slots| slots.cheap.as_deref()),
            Some("gpt-5-mini")
        );
        assert_eq!(
            settings.providers.cheap_model.as_deref(),
            Some("openai/gpt-5-mini")
        );
    }

    #[test]
    fn provider_config_write_uses_previous_provider_models_for_blank_inputs() {
        let mut settings = Settings::default();
        settings.providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("previous-primary".to_string()),
                cheap: Some("previous-cheap".to_string()),
            },
        );
        settings
            .providers
            .allowed_models
            .insert("openai".to_string(), vec!["previous-allowed".to_string()]);

        let mut openai = config_entry("openai");
        openai.enabled = true;
        openai.primary = true;
        openai.primary_model = Some("   ".to_string());
        openai.cheap_model = Some(" ".to_string());

        let body = write_request(vec![openai]);

        apply_providers_config_write(&mut settings, &body);

        let slots = settings.providers.provider_models.get("openai").unwrap();
        assert_eq!(slots.primary.as_deref(), Some("previous-primary"));
        assert_eq!(slots.cheap.as_deref(), Some("previous-cheap"));
        assert_eq!(
            settings.providers.primary_model.as_deref(),
            Some("previous-primary")
        );
        assert_eq!(
            settings.providers.cheap_model.as_deref(),
            Some("openai/previous-cheap")
        );
        assert_eq!(
            settings
                .providers
                .allowed_models
                .get("openai")
                .and_then(|models| models.first())
                .map(String::as_str),
            Some("previous-primary")
        );
    }

    #[test]
    fn provider_primary_model_resolution_preserves_slot_precedence() {
        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            selected_model: Some("legacy-openrouter-model".to_string()),
            ..Settings::default()
        };
        let mut providers = ProvidersSettings {
            primary: Some("openrouter".to_string()),
            primary_model: Some("configured-primary".to_string()),
            enabled: vec!["openrouter".to_string()],
            ..ProvidersSettings::default()
        };

        assert_eq!(
            provider_primary_model_for_slug(&settings, &providers, "openrouter", "default")
                .as_deref(),
            Some("configured-primary")
        );

        providers.primary_model = None;
        assert_eq!(
            provider_primary_model_for_slug(&settings, &providers, "openrouter", "default")
                .as_deref(),
            Some("legacy-openrouter-model")
        );
    }

    #[test]
    fn provider_cheap_model_resolution_prefers_slot_then_global_target() {
        let settings = Settings::default();
        let mut providers = ProvidersSettings {
            cheap_model: Some("gemini/gemini-cheap-global".to_string()),
            enabled: vec!["gemini".to_string()],
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "gemini".to_string(),
            thinclaw_settings::ProviderModelSlots {
                primary: Some("gemini-primary-slot".to_string()),
                cheap: Some("gemini-cheap-slot".to_string()),
            },
        );

        assert_eq!(
            provider_cheap_model_for_slug(
                &settings,
                &providers,
                "gemini",
                "gemini-default",
                Some("gemini-suggested-cheap"),
            )
            .as_deref(),
            Some("gemini-cheap-slot")
        );

        providers.provider_models.clear();
        assert_eq!(
            provider_cheap_model_for_slug(
                &settings,
                &providers,
                "gemini",
                "gemini-default",
                Some("gemini-suggested-cheap"),
            )
            .as_deref(),
            Some("gemini-cheap-global")
        );
    }

    #[test]
    fn sync_legacy_llm_settings_projects_primary_provider_and_model() {
        let mut settings = Settings::default();
        settings.providers.primary = Some("openrouter".to_string());
        settings.providers.primary_model = Some("anthropic/claude-sonnet".to_string());

        sync_legacy_llm_settings(&mut settings);

        assert_eq!(settings.llm_backend.as_deref(), Some("openai_compatible"));
        assert_eq!(
            settings.openai_compatible_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(
            settings.selected_model.as_deref(),
            Some("anthropic/claude-sonnet")
        );
    }

    #[test]
    fn sync_legacy_llm_settings_clears_unknown_primary_and_missing_model() {
        let mut settings = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            ..Settings::default()
        };
        settings.providers.primary = Some("unknown".to_string());

        sync_legacy_llm_settings(&mut settings);

        assert_eq!(settings.llm_backend, None);
        assert_eq!(settings.selected_model, None);
    }

    #[test]
    fn provider_model_options_keep_live_chat_models_only() {
        let discovered = vec![
            DiscoveredProviderModel {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                is_chat: true,
                context_length: Some(128_000),
            },
            DiscoveredProviderModel {
                id: "text-embedding-3-small".to_string(),
                name: "Embedding".to_string(),
                is_chat: false,
                context_length: None,
            },
            DiscoveredProviderModel {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini".to_string(),
                is_chat: true,
                context_length: Some(128_000),
            },
        ];

        let (models, suggested_primary, suggested_cheap, has_live_models) =
            provider_model_options_from_discovery(
                "openai",
                "gpt-4o",
                discovered,
                Some("gpt-legacy"),
                None,
                Some("gpt-4o-mini"),
            );

        let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();
        assert!(has_live_models);
        assert_eq!(ids, vec!["gpt-4o", "gpt-4o-mini"]);
        assert_eq!(suggested_primary.as_deref(), Some("gpt-4o"));
        assert_eq!(suggested_cheap.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn fallback_model_options_dedupe_configured_and_curated_models() {
        let models = fallback_provider_model_options(
            "default-model",
            Some("primary-model"),
            Some("cheap-model"),
            Some("primary-model"),
            Some("cheap-model"),
            vec![
                ("cheap-model".to_string(), "Cheap".to_string()),
                ("curated-model".to_string(), "Curated".to_string()),
            ],
        );

        let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "primary-model",
                "cheap-model",
                "default-model",
                "curated-model"
            ]
        );
        assert_eq!(models[0].source, "configured");
        assert!(models[0].recommended_primary);
        assert!(models[1].recommended_cheap);
    }

    #[test]
    fn static_fallback_provider_models_cover_known_providers_only() {
        let openai = static_fallback_provider_models("openai");
        assert_eq!(
            openai.first().map(|entry| entry.0.as_str()),
            Some("gpt-5.3-codex")
        );
        assert!(openai.iter().any(|entry| entry.0 == "gpt-5-mini"));
        assert!(static_fallback_provider_models("unknown").is_empty());
    }

    #[test]
    fn provider_fallback_model_catalog_prefers_dynamic_models() {
        let dynamic = provider_fallback_model_catalog(
            "openai",
            vec![("custom-model".to_string(), "Custom Model".to_string())],
        );
        assert_eq!(
            dynamic,
            vec![("custom-model".to_string(), "Custom Model".to_string())]
        );

        let static_fallback =
            provider_fallback_model_catalog("openai", Vec::<(String, String)>::new());
        assert_eq!(
            static_fallback.first().map(|entry| entry.0.as_str()),
            Some("gpt-5.3-codex")
        );
    }

    #[test]
    fn saved_provider_models_prefer_incoming_then_previous_values() {
        let input = SavedProviderModelInput {
            default_model: "gemini-2.5-flash".to_string(),
            enabled: true,
            primary: false,
            preferred_cheap: false,
            primary_model: Some(" gemini-3.1-pro ".to_string()),
            cheap_model: None,
            suggested_primary_model: Some("gemini-2.5-flash".to_string()),
            suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
        };
        let previous = ProviderModelSlotsSnapshot {
            primary: Some("gemini-1.5-pro".to_string()),
            cheap: Some("gemini-1.5-flash".to_string()),
        };

        let resolved = resolve_saved_provider_models(&input, Some(&previous), None);
        assert_eq!(resolved.primary_model.as_deref(), Some("gemini-3.1-pro"));
        assert_eq!(resolved.cheap_model.as_deref(), Some("gemini-1.5-flash"));
        assert!(resolved.should_persist_slots);
    }

    #[test]
    fn stale_provider_namespace_keys_only_include_removed_provider_objects() {
        let previous = std::collections::HashMap::from([
            (
                "providers.allowed_models.openai".to_string(),
                serde_json::json!(["gpt-4o"]),
            ),
            (
                "providers.provider_models.openai.primary".to_string(),
                serde_json::json!("gpt-4o"),
            ),
            (
                "providers.enabled".to_string(),
                serde_json::json!(["openai"]),
            ),
            ("selected_model".to_string(), serde_json::json!("gpt-4o")),
        ]);
        let next = std::collections::HashMap::from([
            ("providers.enabled".to_string(), serde_json::json!([])),
            ("selected_model".to_string(), serde_json::json!(null)),
        ]);

        let mut stale = stale_provider_namespace_keys(&previous, &next);
        stale.sort();
        assert_eq!(
            stale,
            vec![
                "providers.allowed_models.openai",
                "providers.provider_models.openai.primary"
            ]
        );
    }

    #[test]
    fn provider_auto_enable_setting_updates_append_missing_entries() {
        let updates = provider_auto_enable_setting_updates(
            Some(serde_json::json!(["openai"])),
            Some(serde_json::json!(["openai/gpt-4o"])),
            "gemini",
            "gemini-2.5-flash",
        );

        assert_eq!(
            updates,
            vec![
                ProviderSettingListMutation {
                    key: PROVIDERS_ENABLED_SETTING_KEY,
                    value: vec!["openai".to_string(), "gemini".to_string()],
                },
                ProviderSettingListMutation {
                    key: PROVIDERS_FALLBACK_CHAIN_SETTING_KEY,
                    value: vec![
                        "openai/gpt-4o".to_string(),
                        "gemini/gemini-2.5-flash".to_string()
                    ],
                },
            ]
        );
    }

    #[test]
    fn provider_auto_enable_setting_updates_skip_existing_entries() {
        let updates = provider_auto_enable_setting_updates(
            Some(serde_json::json!(["gemini"])),
            Some(serde_json::json!(["gemini/gemini-1.5-flash"])),
            "gemini",
            "gemini-2.5-flash",
        );

        assert!(updates.is_empty());
    }

    #[test]
    fn provider_auto_enable_setting_updates_default_malformed_lists() {
        let updates = provider_auto_enable_setting_updates(
            Some(serde_json::json!("not-a-list")),
            Some(serde_json::json!(null)),
            "anthropic",
            "claude-sonnet-4-6",
        );

        assert_eq!(
            updates,
            vec![
                ProviderSettingListMutation {
                    key: PROVIDERS_ENABLED_SETTING_KEY,
                    value: vec!["anthropic".to_string()],
                },
                ProviderSettingListMutation {
                    key: PROVIDERS_FALLBACK_CHAIN_SETTING_KEY,
                    value: vec!["anthropic/claude-sonnet-4-6".to_string()],
                },
            ]
        );
    }

    #[test]
    fn provider_auto_disable_setting_updates_remove_matching_entries() {
        let updates = provider_auto_disable_setting_updates(
            Some(serde_json::json!(["openai", "gemini", "anthropic"])),
            Some(serde_json::json!([
                "openai/gpt-4o",
                "gemini/gemini-2.5-flash",
                "gemini@primary"
            ])),
            "gemini",
        );

        assert_eq!(
            updates,
            vec![
                ProviderSettingListMutation {
                    key: PROVIDERS_ENABLED_SETTING_KEY,
                    value: vec!["openai".to_string(), "anthropic".to_string()],
                },
                ProviderSettingListMutation {
                    key: PROVIDERS_FALLBACK_CHAIN_SETTING_KEY,
                    value: vec!["openai/gpt-4o".to_string(), "gemini@primary".to_string()],
                },
            ]
        );
    }

    #[test]
    fn provider_auto_disable_setting_updates_ignore_missing_or_malformed_lists() {
        let updates = provider_auto_disable_setting_updates(
            Some(serde_json::json!("not-a-list")),
            None,
            "openai",
        );

        assert!(updates.is_empty());
    }

    #[test]
    fn route_target_availability_uses_enabled_provider_slugs() {
        let enabled =
            std::collections::HashSet::from(["anthropic".to_string(), "openai".to_string()]);

        assert!(route_target_is_available_for_enabled_providers(
            "primary", &enabled
        ));
        assert!(route_target_is_available_for_enabled_providers(
            "anthropic@primary",
            &enabled
        ));
        assert!(route_target_is_available_for_enabled_providers(
            "openai/gpt-4o",
            &enabled
        ));
        assert!(!route_target_is_available_for_enabled_providers(
            "gemini@cheap",
            &enabled
        ));
    }

    #[test]
    fn unique_enabled_provider_order_dedupes_and_filters() {
        let enabled =
            std::collections::HashSet::from(["openai".to_string(), "anthropic".to_string()]);
        let ordered = unique_enabled_provider_order(
            &[
                "gemini".to_string(),
                "openai".to_string(),
                "anthropic".to_string(),
                "openai".to_string(),
            ],
            &enabled,
        );
        assert_eq!(ordered, vec!["openai", "anthropic"]);
    }
}

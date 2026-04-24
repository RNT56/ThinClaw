use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::identity_helpers::{GatewayAuthSource, GatewayRequestIdentity};
use crate::channels::web::rate_limiter::RateLimiter;
use crate::channels::web::server::GatewayState;

/// Response for GET /api/providers — lists all catalog providers with key status.
#[derive(serde::Serialize)]
pub(crate) struct ProviderInfo {
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) api_style: String,
    pub(crate) default_model: String,
    pub(crate) default_context_size: u32,
    pub(crate) has_key: bool,
    #[serde(default)]
    pub(crate) credential_ready: bool,
    pub(crate) env_key_name: String,
    pub(crate) auth_kind: String,
    #[serde(default)]
    pub(crate) auth_mode: String,
    #[serde(default)]
    pub(crate) oauth_supported: bool,
    #[serde(default)]
    pub(crate) oauth_available: bool,
    #[serde(default)]
    pub(crate) oauth_source_label: Option<String>,
    #[serde(default)]
    pub(crate) oauth_source_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) credential: Option<ProviderCredentialMetadata>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub(crate) struct ProviderCredentialMetadata {
    pub(crate) source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) masked_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) key_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) encryption_version: Option<i32>,
}

#[derive(serde::Serialize)]
pub(crate) struct ProvidersListResponse {
    pub(crate) providers: Vec<ProviderInfo>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct ProviderConfigEntry {
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) api_style: String,
    pub(crate) default_model: String,
    pub(crate) env_key_name: String,
    #[serde(default)]
    pub(crate) has_key: bool,
    #[serde(default)]
    pub(crate) credential_ready: bool,
    #[serde(default)]
    pub(crate) auth_required: bool,
    #[serde(default)]
    pub(crate) auth_mode: String,
    #[serde(default)]
    pub(crate) oauth_supported: bool,
    #[serde(default)]
    pub(crate) oauth_available: bool,
    #[serde(default)]
    pub(crate) oauth_source_label: Option<String>,
    #[serde(default)]
    pub(crate) oauth_source_location: Option<String>,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) primary: bool,
    #[serde(default)]
    pub(crate) preferred_cheap: bool,
    #[serde(default)]
    pub(crate) discovery_supported: bool,
    pub(crate) primary_model: Option<String>,
    pub(crate) cheap_model: Option<String>,
    pub(crate) suggested_primary_model: Option<String>,
    pub(crate) suggested_cheap_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tier: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ProvidersConfigResponse {
    pub(crate) routing_enabled: bool,
    pub(crate) routing_mode: String,
    pub(crate) cascade_enabled: bool,
    pub(crate) tool_phase_synthesis_enabled: bool,
    pub(crate) tool_phase_primary_thinking_enabled: bool,
    pub(crate) compatible_base_url: Option<String>,
    pub(crate) ollama_base_url: Option<String>,
    pub(crate) bedrock_region: Option<String>,
    pub(crate) bedrock_proxy_url: Option<String>,
    pub(crate) llama_cpp_server_url: Option<String>,
    pub(crate) primary_provider: Option<String>,
    pub(crate) primary_model: Option<String>,
    pub(crate) preferred_cheap_provider: Option<String>,
    pub(crate) cheap_model: Option<String>,
    #[serde(default)]
    pub(crate) primary_pool_order: Vec<String>,
    #[serde(default)]
    pub(crate) cheap_pool_order: Vec<String>,
    pub(crate) fallback_chain: Vec<String>,
    pub(crate) policy_rules: Vec<crate::llm::routing_policy::RoutingRule>,
    pub(crate) providers: Vec<ProviderConfigEntry>,
    pub(crate) runtime_revision: Option<u64>,
    pub(crate) last_reload_error: Option<String>,
    pub(crate) advisor_max_calls: u32,
    pub(crate) advisor_auto_escalation_mode: crate::settings::AdvisorAutoEscalationMode,
    pub(crate) advisor_escalation_prompt: Option<String>,
    #[serde(default)]
    pub(crate) advisor_ready: bool,
    #[serde(default)]
    pub(crate) advisor_disabled_reason: Option<String>,
    #[serde(default)]
    pub(crate) executor_target: Option<String>,
    #[serde(default)]
    pub(crate) advisor_target: Option<String>,
    #[serde(default)]
    pub(crate) diagnostics: Vec<String>,
    pub(crate) derived_defaults: crate::settings::ProvidersSettings,
    pub(crate) persisted: crate::settings::ProvidersSettings,
    pub(crate) effective: crate::settings::ProvidersSettings,
}

#[derive(serde::Deserialize)]
pub(crate) struct ProvidersConfigWriteRequest {
    pub(crate) routing_enabled: bool,
    pub(crate) routing_mode: String,
    pub(crate) cascade_enabled: bool,
    pub(crate) tool_phase_synthesis_enabled: bool,
    pub(crate) tool_phase_primary_thinking_enabled: bool,
    pub(crate) compatible_base_url: Option<String>,
    pub(crate) ollama_base_url: Option<String>,
    pub(crate) bedrock_region: Option<String>,
    pub(crate) bedrock_proxy_url: Option<String>,
    pub(crate) llama_cpp_server_url: Option<String>,
    pub(crate) primary_provider: Option<String>,
    pub(crate) primary_model: Option<String>,
    pub(crate) preferred_cheap_provider: Option<String>,
    pub(crate) cheap_model: Option<String>,
    #[serde(default)]
    pub(crate) primary_pool_order: Vec<String>,
    #[serde(default)]
    pub(crate) cheap_pool_order: Vec<String>,
    pub(crate) fallback_chain: Vec<String>,
    pub(crate) policy_rules: Vec<crate::llm::routing_policy::RoutingRule>,
    pub(crate) providers: Vec<ProviderConfigEntry>,
    #[serde(default = "default_advisor_max_calls_api")]
    pub(crate) advisor_max_calls: u32,
    #[serde(default)]
    pub(crate) advisor_auto_escalation_mode: crate::settings::AdvisorAutoEscalationMode,
    #[serde(default)]
    pub(crate) advisor_escalation_prompt: Option<String>,
    #[serde(default)]
    pub(crate) auto_fix: bool,
}

fn default_advisor_max_calls_api() -> u32 {
    4
}

#[derive(serde::Serialize)]
pub(crate) struct ProviderModelOption {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) context_length: Option<u32>,
    pub(crate) source: String,
    pub(crate) recommended_primary: bool,
    pub(crate) recommended_cheap: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct ProviderModelsResponse {
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) discovery_supported: bool,
    pub(crate) discovery_status: String,
    pub(crate) error: Option<String>,
    pub(crate) current_primary_model: Option<String>,
    pub(crate) current_cheap_model: Option<String>,
    pub(crate) suggested_primary_model: Option<String>,
    pub(crate) suggested_cheap_model: Option<String>,
    pub(crate) models: Vec<ProviderModelOption>,
}

#[derive(serde::Deserialize)]
pub(crate) struct RouteSimulateRequest {
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) has_vision: bool,
    #[serde(default)]
    pub(crate) has_tools: bool,
    #[serde(default)]
    pub(crate) requires_streaming: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct RouteSimulateResponse {
    pub(crate) target: String,
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) fallback_chain: Vec<String>,
    #[serde(default)]
    pub(crate) candidate_list: Vec<String>,
    #[serde(default)]
    pub(crate) rejections: Vec<String>,
    #[serde(default)]
    pub(crate) score_breakdown: Vec<RouteSimulateScore>,
    #[serde(default)]
    pub(crate) diagnostics: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct RouteSimulateScore {
    pub(crate) target: String,
    pub(crate) telemetry_key: Option<String>,
    pub(crate) quality: f64,
    pub(crate) cost: f64,
    pub(crate) latency: f64,
    pub(crate) health: f64,
    pub(crate) policy_bias: f64,
    pub(crate) composite: f64,
}

struct ProviderOauthUiState {
    supported: bool,
    available: bool,
    source_label: Option<String>,
    source_location: Option<String>,
}

fn provider_auth_mode(
    providers_settings: &crate::settings::ProvidersSettings,
    slug: &str,
) -> crate::settings::ProviderCredentialMode {
    providers_settings
        .provider_credential_modes
        .get(slug)
        .copied()
        .unwrap_or_default()
}

fn provider_oauth_ui_state(slug: &str) -> ProviderOauthUiState {
    if let Some(kind) = crate::llm::credential_sync::provider_oauth_source_kind(slug) {
        return ProviderOauthUiState {
            supported: true,
            available: crate::llm::credential_sync::oauth_source_available(kind),
            source_label: Some(crate::llm::credential_sync::oauth_source_label(kind).to_string()),
            source_location: Some(crate::llm::credential_sync::oauth_source_location_hint(
                kind,
            )),
        };
    }

    ProviderOauthUiState {
        supported: false,
        available: false,
        source_label: None,
        source_location: None,
    }
}

pub(crate) async fn providers_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<ProvidersListResponse>, StatusCode> {
    let catalog = crate::config::provider_catalog::catalog();
    let secrets = state.secrets_store.as_ref();

    let mut providers = Vec::new();
    let mut entries: Vec<_> = catalog.iter().collect();
    entries.sort_by_key(|(slug, _)| *slug);

    for (slug, endpoint) in entries {
        let has_env = crate::config::helpers::optional_env(&endpoint.env_key_name)
            .ok()
            .flatten()
            .is_some();
        let has_secret = if let Some(ss) = secrets {
            ss.exists(&request_identity.principal_id, &endpoint.secret_name)
                .await
                .unwrap_or(false)
        } else {
            false
        };
        let credential = provider_credential_metadata(
            secrets,
            &request_identity.principal_id,
            &endpoint.secret_name,
            &endpoint.env_key_name,
        )
        .await;
        let oauth = provider_oauth_ui_state(slug);

        let api_style_str = match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::OpenAi => "openai",
            crate::config::provider_catalog::ApiStyle::Anthropic => "anthropic",
            crate::config::provider_catalog::ApiStyle::OpenAiCompatible => "openai_compatible",
            crate::config::provider_catalog::ApiStyle::Ollama => "ollama",
        };

        providers.push(ProviderInfo {
            slug: slug.to_string(),
            display_name: endpoint.display_name.to_string(),
            api_style: api_style_str.to_string(),
            default_model: endpoint.default_model.to_string(),
            default_context_size: endpoint.default_context_size,
            has_key: has_env || has_secret,
            credential_ready: has_env || has_secret,
            env_key_name: endpoint.env_key_name.to_string(),
            auth_kind: if oauth.supported {
                "api_key_or_external_oauth_sync".to_string()
            } else {
                "api_key".to_string()
            },
            auth_mode: "api_key".to_string(),
            oauth_supported: oauth.supported,
            oauth_available: oauth.available,
            oauth_source_label: oauth.source_label,
            oauth_source_location: oauth.source_location,
            setup_url: endpoint.setup_url.clone(),
            tier: endpoint.tier.clone(),
            credential,
        });
    }

    let compat_has_key = crate::config::helpers::optional_env("LLM_API_KEY")
        .ok()
        .flatten()
        .is_some()
        || secret_exists(
            secrets,
            &request_identity.principal_id,
            "llm_compatible_api_key",
        )
        .await;
    providers.push(ProviderInfo {
        slug: "openai_compatible".to_string(),
        display_name: "OpenAI-compatible".to_string(),
        api_style: "openai_compatible".to_string(),
        default_model: "default".to_string(),
        default_context_size: 128_000,
        has_key: compat_has_key,
        credential_ready: compat_has_key,
        env_key_name: "LLM_API_KEY".to_string(),
        auth_kind: "api_key".to_string(),
        auth_mode: "api_key".to_string(),
        oauth_supported: false,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        setup_url: None,
        tier: None,
        credential: provider_credential_metadata(
            secrets,
            &request_identity.principal_id,
            "llm_compatible_api_key",
            "LLM_API_KEY",
        )
        .await,
    });

    let bedrock_has_key = crate::config::helpers::optional_env("BEDROCK_API_KEY")
        .ok()
        .flatten()
        .is_some()
        || crate::config::helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
            .ok()
            .flatten()
            .is_some()
        || secret_exists(
            secrets,
            &request_identity.principal_id,
            "llm_bedrock_api_key",
        )
        .await
        || crate::config::helpers::optional_env("BEDROCK_PROXY_API_KEY")
            .ok()
            .flatten()
            .is_some()
        || secret_exists(
            secrets,
            &request_identity.principal_id,
            "llm_bedrock_proxy_api_key",
        )
        .await;
    providers.push(ProviderInfo {
        slug: "bedrock".to_string(),
        display_name: "AWS Bedrock".to_string(),
        api_style: "bedrock".to_string(),
        default_model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
        default_context_size: 200_000,
        has_key: bedrock_has_key,
        credential_ready: bedrock_has_key,
        env_key_name: "BEDROCK_API_KEY".to_string(),
        auth_kind: "api_key".to_string(),
        auth_mode: "api_key".to_string(),
        oauth_supported: false,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        setup_url: None,
        tier: None,
        credential: provider_credential_metadata(
            secrets,
            &request_identity.principal_id,
            "llm_bedrock_api_key",
            "BEDROCK_API_KEY",
        )
        .await,
    });

    providers.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    Ok(Json(ProvidersListResponse { providers }))
}

pub(crate) async fn providers_config_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<ProvidersConfigResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let map = store
        .get_all_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to load provider settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let settings = crate::settings::Settings::from_db_map(&map);
    let providers_settings = crate::llm::normalize_providers_settings(&settings);
    let diagnostics = crate::llm::validate_providers_settings(&settings.providers);
    let derived_defaults = crate::llm::derive_runtime_defaults(&settings);
    let persisted = settings.providers.clone();
    let runtime_status = state.llm_runtime.as_ref().map(|runtime| runtime.status());
    let secrets = state.secrets_store.as_ref();
    let providers = build_routing_provider_entries(
        &request_identity.principal_id,
        &settings,
        &persisted,
        secrets,
    )
    .await;

    Ok(Json(ProvidersConfigResponse {
        routing_enabled: providers_settings.smart_routing_enabled,
        routing_mode: providers_settings.routing_mode.as_str().to_string(),
        cascade_enabled: providers_settings.smart_routing_cascade,
        tool_phase_synthesis_enabled: providers_settings.tool_phase_synthesis_enabled,
        tool_phase_primary_thinking_enabled: providers_settings.tool_phase_primary_thinking_enabled,
        compatible_base_url: settings.openai_compatible_base_url.clone(),
        ollama_base_url: settings.ollama_base_url.clone(),
        bedrock_region: settings.bedrock_region.clone(),
        bedrock_proxy_url: settings.bedrock_proxy_url.clone(),
        llama_cpp_server_url: settings.llama_cpp_server_url.clone(),
        primary_provider: providers_settings.primary.clone(),
        primary_model: providers_settings.primary_model.clone(),
        preferred_cheap_provider: providers_settings.preferred_cheap_provider.clone(),
        cheap_model: providers_settings.cheap_model.clone(),
        primary_pool_order: providers_settings.primary_pool_order.clone(),
        cheap_pool_order: providers_settings.cheap_pool_order.clone(),
        fallback_chain: providers_settings.fallback_chain.clone(),
        policy_rules: providers_settings.policy_rules.clone(),
        providers,
        runtime_revision: runtime_status.as_ref().map(|status| status.revision),
        last_reload_error: runtime_status
            .as_ref()
            .and_then(|status| status.last_error.clone()),
        advisor_max_calls: providers_settings.advisor_max_calls,
        advisor_auto_escalation_mode: providers_settings.advisor_auto_escalation_mode,
        advisor_escalation_prompt: providers_settings.advisor_escalation_prompt.clone(),
        advisor_ready: runtime_status
            .as_ref()
            .map(|status| status.advisor_ready)
            .unwrap_or(false),
        advisor_disabled_reason: runtime_status
            .as_ref()
            .and_then(|status| status.advisor_disabled_reason.clone()),
        executor_target: runtime_status
            .as_ref()
            .and_then(|status| status.executor_target.clone()),
        advisor_target: runtime_status
            .as_ref()
            .and_then(|status| status.advisor_target.clone()),
        diagnostics,
        derived_defaults,
        persisted,
        effective: providers_settings.clone(),
    }))
}

pub(crate) async fn build_routing_provider_entries(
    user_id: &str,
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Vec<ProviderConfigEntry> {
    let mut providers = Vec::new();
    let mut entries: Vec<_> = crate::config::provider_catalog::catalog().iter().collect();
    entries.sort_by_key(|(slug, _)| *slug);

    for (slug, endpoint) in entries {
        let has_env = crate::config::helpers::optional_env(&endpoint.env_key_name)
            .ok()
            .flatten()
            .is_some();
        let has_secret = secret_exists(secrets, user_id, &endpoint.secret_name).await;
        let auth_mode = provider_auth_mode(providers_settings, slug);
        let oauth = provider_oauth_ui_state(slug);
        let primary_model = provider_primary_model_for_slug(
            settings,
            providers_settings,
            slug,
            &endpoint.default_model,
        );
        let cheap_model = provider_cheap_model_for_slug(
            settings,
            providers_settings,
            slug,
            &endpoint.default_model,
        );
        providers.push(ProviderConfigEntry {
            slug: (*slug).to_string(),
            display_name: endpoint.display_name.to_string(),
            api_style: match endpoint.api_style {
                crate::config::provider_catalog::ApiStyle::OpenAi => "openai",
                crate::config::provider_catalog::ApiStyle::Anthropic => "anthropic",
                crate::config::provider_catalog::ApiStyle::OpenAiCompatible => "openai_compatible",
                crate::config::provider_catalog::ApiStyle::Ollama => "ollama",
            }
            .to_string(),
            default_model: endpoint.default_model.to_string(),
            env_key_name: endpoint.env_key_name.to_string(),
            has_key: has_env || has_secret,
            credential_ready: if auth_mode
                == crate::settings::ProviderCredentialMode::ExternalOAuthSync
            {
                oauth.available
            } else {
                has_env || has_secret
            },
            auth_required: true,
            auth_mode: match auth_mode {
                crate::settings::ProviderCredentialMode::ApiKey => "api_key",
                crate::settings::ProviderCredentialMode::ExternalOAuthSync => "oauth_sync",
            }
            .to_string(),
            oauth_supported: oauth.supported,
            oauth_available: oauth.available,
            oauth_source_label: oauth.source_label,
            oauth_source_location: oauth.source_location,
            enabled: providers_settings
                .enabled
                .iter()
                .any(|enabled| enabled == slug),
            primary: providers_settings.primary.as_deref() == Some(slug),
            preferred_cheap: providers_settings.preferred_cheap_provider.as_deref() == Some(slug),
            discovery_supported: provider_supports_model_discovery(slug),
            primary_model: primary_model.clone(),
            cheap_model: cheap_model.clone(),
            suggested_primary_model: primary_model
                .or_else(|| Some(endpoint.default_model.to_string())),
            suggested_cheap_model: cheap_model
                .or_else(|| suggested_cheap_model_for_slug(slug, &endpoint.default_model)),
            setup_url: endpoint.setup_url.clone(),
            tier: endpoint.tier.clone(),
        });
    }

    providers.push(synthetic_provider_entry(
        "ollama",
        "Ollama",
        "ollama",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("ollama"))
            .unwrap_or("llama3"),
        "OLLAMA_BASE_URL",
        providers_settings,
        settings,
        true,
        false,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "openai_compatible",
        "OpenAI-compatible",
        "openai_compatible",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("openai_compatible"))
            .unwrap_or("default"),
        "LLM_API_KEY",
        providers_settings,
        settings,
        settings.openai_compatible_base_url.is_some()
            || crate::config::helpers::optional_env("LLM_BASE_URL")
                .ok()
                .flatten()
                .is_some()
            || crate::config::helpers::optional_env("LLM_API_KEY")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_compatible_api_key").await,
        false,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "bedrock",
        "AWS Bedrock",
        "bedrock",
        "anthropic.claude-3-sonnet-20240229-v1:0",
        "BEDROCK_API_KEY",
        providers_settings,
        settings,
        crate::config::helpers::optional_env("BEDROCK_API_KEY")
            .ok()
            .flatten()
            .is_some()
            || crate::config::helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_bedrock_api_key").await
            || crate::config::helpers::optional_env("BEDROCK_PROXY_API_KEY")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_bedrock_proxy_api_key").await,
        false,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "llama_cpp",
        "llama.cpp",
        "llama_cpp",
        "llama-local",
        "",
        providers_settings,
        settings,
        true,
        false,
        false,
    ));

    providers.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    providers
}

fn synthetic_provider_entry(
    slug: &str,
    display_name: &str,
    api_style: &str,
    default_model: &str,
    env_key_name: &str,
    providers_settings: &crate::settings::ProvidersSettings,
    settings: &crate::settings::Settings,
    has_key: bool,
    auth_required: bool,
    oauth_supported: bool,
) -> ProviderConfigEntry {
    ProviderConfigEntry {
        slug: slug.to_string(),
        display_name: display_name.to_string(),
        api_style: api_style.to_string(),
        default_model: default_model.to_string(),
        env_key_name: env_key_name.to_string(),
        has_key,
        credential_ready: has_key,
        auth_required,
        auth_mode: "api_key".to_string(),
        oauth_supported,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        enabled: providers_settings
            .enabled
            .iter()
            .any(|enabled| enabled == slug),
        primary: providers_settings.primary.as_deref() == Some(slug),
        preferred_cheap: providers_settings.preferred_cheap_provider.as_deref() == Some(slug),
        discovery_supported: provider_supports_model_discovery(slug),
        primary_model: provider_primary_model_for_slug(
            settings,
            providers_settings,
            slug,
            default_model,
        ),
        cheap_model: provider_cheap_model_for_slug(
            settings,
            providers_settings,
            slug,
            default_model,
        ),
        suggested_primary_model: Some(default_model.to_string()),
        suggested_cheap_model: suggested_cheap_model_for_slug(slug, default_model),
        setup_url: None,
        tier: None,
    }
}

fn provider_primary_model_for_slug(
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
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

fn provider_cheap_model_for_slug(
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    slug: &str,
    default_model: &str,
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
        .or_else(|| suggested_cheap_model_for_slug(slug, default_model))
        .or_else(|| {
            provider_primary_model_for_slug(settings, providers_settings, slug, default_model)
        })
}

fn suggested_cheap_model_for_slug(slug: &str, default_model: &str) -> Option<String> {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        if let Some(ref cheap) = endpoint.suggested_cheap_model {
            return Some(cheap.clone());
        }
    }
    if !default_model.is_empty() {
        Some(default_model.to_string())
    } else {
        None
    }
}

fn provider_supports_model_discovery(slug: &str) -> bool {
    crate::config::provider_catalog::endpoint_for(slug).is_some()
        || matches!(
            slug,
            "ollama" | "openai_compatible" | "bedrock" | "llama_cpp"
        )
}

pub(crate) async fn build_provider_models_response(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> ProviderModelsResponse {
    let (display_name, default_model) = provider_identity(slug);
    let current_primary_model =
        provider_primary_model_for_slug(settings, providers_settings, slug, default_model.as_str());
    let current_cheap_model =
        provider_cheap_model_for_slug(settings, providers_settings, slug, default_model.as_str());
    let discovery_supported = provider_supports_model_discovery(slug);

    if !discovery_supported {
        let suggested_primary_model = current_primary_model
            .clone()
            .or_else(|| Some(default_model.clone()));
        let suggested_cheap_model = current_cheap_model
            .clone()
            .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
        return ProviderModelsResponse {
            slug: slug.to_string(),
            display_name,
            discovery_supported: false,
            discovery_status: "unsupported".to_string(),
            error: None,
            current_primary_model: current_primary_model.clone(),
            current_cheap_model: current_cheap_model.clone(),
            suggested_primary_model: suggested_primary_model.clone(),
            suggested_cheap_model: suggested_cheap_model.clone(),
            models: fallback_provider_model_options(
                slug,
                default_model.as_str(),
                current_primary_model.as_deref(),
                current_cheap_model.as_deref(),
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
            ),
        };
    }

    match discover_provider_models(user_id, slug, settings, secrets).await {
        Ok(result) => {
            let (
                discovered_models,
                suggested_primary_model,
                suggested_cheap_model,
                has_live_models,
            ) = provider_model_options_from_discovery(
                slug,
                default_model.as_str(),
                result.models,
                current_primary_model.as_deref(),
                current_cheap_model.as_deref(),
            );
            if result.error.is_some() || !has_live_models {
                let fallback_primary_model = current_primary_model
                    .clone()
                    .or_else(|| Some(default_model.clone()));
                let fallback_cheap_model = current_cheap_model
                    .clone()
                    .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
                ProviderModelsResponse {
                    slug: slug.to_string(),
                    display_name,
                    discovery_supported: true,
                    discovery_status: "fallback".to_string(),
                    error: result.error,
                    current_primary_model: current_primary_model.clone(),
                    current_cheap_model: current_cheap_model.clone(),
                    suggested_primary_model: fallback_primary_model.clone(),
                    suggested_cheap_model: fallback_cheap_model.clone(),
                    models: fallback_provider_model_options(
                        slug,
                        default_model.as_str(),
                        current_primary_model.as_deref(),
                        current_cheap_model.as_deref(),
                        fallback_primary_model.as_deref(),
                        fallback_cheap_model.as_deref(),
                    ),
                }
            } else {
                ProviderModelsResponse {
                    slug: slug.to_string(),
                    display_name,
                    discovery_supported: true,
                    discovery_status: "discovered".to_string(),
                    error: result.error,
                    current_primary_model,
                    current_cheap_model,
                    suggested_primary_model,
                    suggested_cheap_model,
                    models: discovered_models,
                }
            }
        }
        Err(error) => {
            let suggested_primary_model = current_primary_model
                .clone()
                .or_else(|| Some(default_model.clone()));
            let suggested_cheap_model = current_cheap_model
                .clone()
                .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
            ProviderModelsResponse {
                slug: slug.to_string(),
                display_name,
                discovery_supported: true,
                discovery_status: "fallback".to_string(),
                error: Some(error),
                current_primary_model: current_primary_model.clone(),
                current_cheap_model: current_cheap_model.clone(),
                suggested_primary_model: suggested_primary_model.clone(),
                suggested_cheap_model: suggested_cheap_model.clone(),
                models: fallback_provider_model_options(
                    slug,
                    default_model.as_str(),
                    current_primary_model.as_deref(),
                    current_cheap_model.as_deref(),
                    suggested_primary_model.as_deref(),
                    suggested_cheap_model.as_deref(),
                ),
            }
        }
    }
}

fn provider_identity(slug: &str) -> (String, String) {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        return (
            endpoint.display_name.to_string(),
            endpoint.default_model.to_string(),
        );
    }

    match slug {
        "ollama" => ("Ollama".to_string(), "llama3".to_string()),
        "openai_compatible" => ("OpenAI-compatible".to_string(), "default".to_string()),
        "bedrock" => (
            "AWS Bedrock".to_string(),
            "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
        ),
        "llama_cpp" => ("llama.cpp".to_string(), "llama-local".to_string()),
        other => (other.to_string(), "default".to_string()),
    }
}

async fn discover_provider_models(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Result<crate::llm::discovery::DiscoveryResult, String> {
    let discovery = crate::llm::discovery::ModelDiscovery::new();

    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        let missing_credentials =
            || format!("{} credentials are not configured", endpoint.display_name);
        return match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::Anthropic => {
                let api_key = resolve_provider_secret(
                    user_id,
                    slug,
                    settings,
                    &endpoint.env_key_name,
                    &endpoint.secret_name,
                    secrets,
                )
                .await
                .ok_or_else(missing_credentials)?;
                Ok(discovery.discover_anthropic(&api_key).await)
            }
            crate::config::provider_catalog::ApiStyle::Ollama => {
                let base_url = settings
                    .ollama_base_url
                    .clone()
                    .or_else(|| {
                        crate::config::helpers::optional_env("OLLAMA_BASE_URL")
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_else(|| endpoint.base_url.to_string());
                Ok(discovery.discover_ollama(&base_url).await)
            }
            crate::config::provider_catalog::ApiStyle::OpenAi
            | crate::config::provider_catalog::ApiStyle::OpenAiCompatible => {
                let api_key = resolve_provider_secret(
                    user_id,
                    slug,
                    settings,
                    &endpoint.env_key_name,
                    &endpoint.secret_name,
                    secrets,
                )
                .await;
                if slug == "cohere" {
                    let api_key = api_key.ok_or_else(missing_credentials)?;
                    Ok(discovery.discover_cohere(&api_key).await)
                } else {
                    let auth = Some(format!(
                        "Bearer {}",
                        api_key.ok_or_else(missing_credentials)?
                    ));
                    Ok(discovery
                        .discover_openai_compatible(&endpoint.base_url, auth.as_deref())
                        .await)
                }
            }
        };
    }

    match slug {
        "ollama" => {
            let base_url = settings
                .ollama_base_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("OLLAMA_BASE_URL")
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            Ok(discovery.discover_ollama(&base_url).await)
        }
        "openai_compatible" => {
            let base_url = settings
                .openai_compatible_base_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("LLM_BASE_URL")
                        .ok()
                        .flatten()
                })
                .ok_or_else(|| "Set a compatible base URL before discovering models".to_string())?;
            let auth = resolve_provider_secret(
                user_id,
                slug,
                settings,
                "LLM_API_KEY",
                "llm_compatible_api_key",
                secrets,
            )
            .await
            .map(|key| format!("Bearer {key}"));
            Ok(discovery
                .discover_openai_compatible(&base_url, auth.as_deref())
                .await)
        }
        "bedrock" => {
            let (base_url, auth) =
                resolve_bedrock_discovery_target(user_id, settings, secrets).await?;
            Ok(discovery
                .discover_openai_compatible(&base_url, auth.as_deref())
                .await)
        }
        "llama_cpp" => {
            let base_url = settings
                .llama_cpp_server_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("LLAMA_SERVER_URL")
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| "http://localhost:8080".to_string());
            Ok(discovery.discover_openai_compatible(&base_url, None).await)
        }
        other => Err(format!("Model discovery is not supported for '{}'", other)),
    }
}

async fn resolve_provider_secret(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    env_key: &str,
    secret_name: &str,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Option<String> {
    if provider_auth_mode(&settings.providers, slug)
        == crate::settings::ProviderCredentialMode::ExternalOAuthSync
        && let Some(value) = crate::config::helpers::synced_oauth_env(env_key)
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    crate::config::resolve_provider_secret_value(user_id, env_key, secret_name, secrets).await
}

async fn resolve_bedrock_discovery_target(
    user_id: &str,
    settings: &crate::settings::Settings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Result<(String, Option<String>), String> {
    let region = settings
        .bedrock_region
        .clone()
        .or_else(|| {
            crate::config::helpers::optional_env("AWS_REGION")
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| "us-east-1".to_string());

    if let Some(api_key) = resolve_provider_secret(
        user_id,
        "bedrock",
        settings,
        "BEDROCK_API_KEY",
        "llm_bedrock_api_key",
        secrets,
    )
    .await
    {
        return Ok((
            crate::llm::discovery::bedrock_mantle_base_url(&region),
            Some(format!("Bearer {api_key}")),
        ));
    }

    if let Some(proxy_url) = settings.bedrock_proxy_url.clone().or_else(|| {
        crate::config::helpers::optional_env("BEDROCK_PROXY_URL")
            .ok()
            .flatten()
    }) {
        let auth = resolve_provider_secret(
            user_id,
            "bedrock",
            settings,
            "BEDROCK_PROXY_API_KEY",
            "llm_bedrock_proxy_api_key",
            secrets,
        )
        .await
        .map(|key| format!("Bearer {key}"));
        return Ok((proxy_url, auth));
    }

    Err(
        "Configure BEDROCK_API_KEY for native Bedrock access or set a legacy Bedrock proxy URL."
            .to_string(),
    )
}

pub(crate) fn provider_model_options_from_discovery(
    slug: &str,
    default_model: &str,
    discovered: Vec<crate::llm::discovery::DiscoveredModel>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
) -> (
    Vec<ProviderModelOption>,
    Option<String>,
    Option<String>,
    bool,
) {
    use std::collections::{BTreeMap, BTreeSet};

    let mut discovered_map = BTreeMap::new();
    for model in discovered.into_iter().filter(|model| {
        if slug == "openai" {
            crate::llm::discovery::is_openai_chat_model(&model.id)
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
    let suggested_provider_cheap = suggested_cheap_model_for_slug(slug, default_model)
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
        .or_else(|| suggested_provider_cheap.clone())
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
                suggested_cheap_model_for_slug(slug, default_model)
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
                "openai" => crate::llm::discovery::openai_model_priority(model),
                "minimax" => crate::llm::discovery::minimax_model_priority(model),
                "cohere" => crate::llm::discovery::cohere_model_priority(model),
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

fn fallback_provider_model_options(
    slug: &str,
    default_model: &str,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
) -> Vec<ProviderModelOption> {
    use std::collections::BTreeSet;

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

    for (static_id, label) in static_fallback_models(slug) {
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
            recommended_cheap: suggested_cheap_model_for_slug(slug, default_model).as_deref()
                == Some(default_model),
        });
    }

    models
}

fn static_fallback_models(slug: &str) -> Vec<(String, String)> {
    let dynamic: Vec<(String, String)> = crate::config::model_compat::models_by_provider(slug)
        .into_iter()
        .map(|model| {
            let label = if model.display_name.trim().is_empty() {
                model.model_id.clone()
            } else {
                model.display_name
            };
            (model.model_id, label)
        })
        .collect();
    if !dynamic.is_empty() {
        return dynamic;
    }

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

fn primary_model_rank(model: &str) -> i32 {
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

fn cheap_model_rank(model: &str) -> i32 {
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

fn model_display_rank(
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

pub(crate) async fn secret_exists(
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    user_id: &str,
    secret_name: &str,
) -> bool {
    if let Some(ss) = secrets {
        ss.exists(user_id, secret_name).await.unwrap_or(false)
    } else {
        false
    }
}

async fn provider_credential_metadata(
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    user_id: &str,
    secret_name: &str,
    env_key: &str,
) -> Option<ProviderCredentialMetadata> {
    if let Ok(Some(value)) = crate::config::helpers::optional_env(env_key)
        && !value.trim().is_empty()
    {
        return Some(ProviderCredentialMetadata {
            source: "env".to_string(),
            masked_preview: Some(mask_provider_key(&value)),
            fingerprint: Some(provider_key_fingerprint(&value)),
            created_at: None,
            updated_at: None,
            last_used_at: None,
            key_version: None,
            encryption_version: None,
        });
    }

    let store = secrets?;
    let secret = store.get(user_id, secret_name).await.ok()?;
    let value = store
        .get_for_injection(
            user_id,
            secret_name,
            crate::secrets::SecretAccessContext::new(
                "provider_vault.metadata",
                "credential_metadata",
            ),
        )
        .await
        .ok();
    Some(ProviderCredentialMetadata {
        source: "local_encrypted".to_string(),
        masked_preview: value
            .as_ref()
            .map(|secret| mask_provider_key(secret.expose())),
        fingerprint: value
            .as_ref()
            .map(|secret| provider_key_fingerprint(secret.expose())),
        created_at: Some(secret.created_at),
        updated_at: Some(secret.updated_at),
        last_used_at: secret.last_used_at,
        key_version: Some(secret.key_version),
        encryption_version: Some(secret.encryption_version),
    })
}

pub(crate) async fn providers_config_set_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(body): Json<ProvidersConfigWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let map = store
        .get_all_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to load settings before provider config write: {}",
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut settings = crate::settings::Settings::from_db_map(&map);

    settings.providers.smart_routing_enabled = body.routing_enabled;
    settings.providers.routing_mode = match body.routing_mode.as_str() {
        "cheap_split" => crate::settings::RoutingMode::CheapSplit,
        "advisor_executor" | "advisor" => crate::settings::RoutingMode::AdvisorExecutor,
        "policy" => crate::settings::RoutingMode::Policy,
        _ => crate::settings::RoutingMode::PrimaryOnly,
    };
    settings.providers.smart_routing_cascade = body.cascade_enabled;
    settings.providers.tool_phase_synthesis_enabled = body.tool_phase_synthesis_enabled;
    settings.providers.tool_phase_primary_thinking_enabled =
        body.tool_phase_primary_thinking_enabled;
    settings.openai_compatible_base_url = body
        .compatible_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.ollama_base_url = body
        .ollama_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.bedrock_region = body
        .bedrock_region
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.bedrock_proxy_url = body
        .bedrock_proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.llama_cpp_server_url = body
        .llama_cpp_server_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.primary = body.primary_provider.clone();
    settings.providers.primary_model = body
        .primary_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.preferred_cheap_provider = body.preferred_cheap_provider.clone();
    settings.providers.cheap_model = body
        .cheap_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.primary_pool_order = body.primary_pool_order.clone();
    settings.providers.cheap_pool_order = body.cheap_pool_order.clone();
    settings.providers.fallback_chain = body.fallback_chain.clone();
    settings.providers.policy_rules = body.policy_rules.clone();
    settings.providers.advisor_max_calls = body.advisor_max_calls;
    settings.providers.advisor_auto_escalation_mode = body.advisor_auto_escalation_mode;
    settings.providers.advisor_escalation_prompt = body
        .advisor_escalation_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
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
        let auth_mode = match provider.auth_mode.as_str() {
            "oauth_sync" => crate::settings::ProviderCredentialMode::ExternalOAuthSync,
            _ => crate::settings::ProviderCredentialMode::ApiKey,
        };
        if provider.oauth_supported
            && auth_mode == crate::settings::ProviderCredentialMode::ExternalOAuthSync
        {
            settings
                .providers
                .provider_credential_modes
                .insert(provider.slug.clone(), auth_mode);
        }

        let previous_slots = previous_provider_models.get(&provider.slug);
        let (primary_model, cheap_model, should_persist_slots) = resolve_saved_provider_models(
            provider,
            previous_slots,
            previous_allowed_models.get(&provider.slug),
        );

        if should_persist_slots {
            settings.providers.provider_models.insert(
                provider.slug.clone(),
                crate::settings::ProviderModelSlots {
                    primary: primary_model.clone(),
                    cheap: cheap_model.clone(),
                },
            );
        }

        if provider.primary {
            settings.providers.primary = Some(provider.slug.clone());
            settings.providers.primary_model = primary_model.clone();
        }
        if provider.preferred_cheap {
            settings.providers.preferred_cheap_provider = Some(provider.slug.clone());
        }
        if provider.enabled
            && let Some(model) = primary_model.as_deref()
        {
            settings
                .providers
                .allowed_models
                .insert(provider.slug.clone(), vec![model.to_string()]);
        }
    }

    let enabled_set: std::collections::HashSet<String> =
        settings.providers.enabled.iter().cloned().collect();
    settings.providers.primary = settings
        .providers
        .primary
        .filter(|slug| enabled_set.contains(slug));
    settings.providers.preferred_cheap_provider = settings
        .providers
        .preferred_cheap_provider
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
        .any(|mode| *mode == crate::settings::ProviderCredentialMode::ExternalOAuthSync);
    settings.providers.oauth_sync_enabled =
        explicit_provider_oauth || !settings.providers.oauth_sync_sources.is_empty();

    let diagnostics = crate::llm::validate_providers_settings(&settings.providers);
    for diagnostic in &diagnostics {
        tracing::warn!(
            "Provider config diagnostic while saving (auto_fix={}): {}",
            body.auto_fix,
            diagnostic
        );
    }

    if body.auto_fix {
        settings.providers = crate::llm::derive_runtime_defaults(&settings);
    }

    sync_legacy_llm_settings(&mut settings);
    let next_settings_map = settings.to_db_map();
    let stale_provider_keys = stale_provider_namespace_keys(&map, &next_settings_map);

    for key in stale_provider_keys {
        store
            .delete_setting(&request_identity.principal_id, &key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete stale provider setting '{}': {}", key, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    store
        .set_all_settings(&request_identity.principal_id, &next_settings_map)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save provider config: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    reload_llm_runtime(state.as_ref()).await.map_err(|e| {
        tracing::error!("Provider config reload failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}

fn trimmed_optional_model(value: Option<&String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub(crate) fn resolve_saved_provider_models(
    provider: &ProviderConfigEntry,
    previous_slots: Option<&crate::settings::ProviderModelSlots>,
    previous_allowed_models: Option<&Vec<String>>,
) -> (Option<String>, Option<String>, bool) {
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

    (primary_model, cheap_model, should_persist_slots)
}

pub(crate) fn stale_provider_namespace_keys(
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

fn unique_enabled_provider_order(
    entries: &[String],
    enabled: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut unique = Vec::new();
    for entry in entries {
        if enabled.contains(entry) && !unique.iter().any(|existing| existing == entry) {
            unique.push(entry.clone());
        }
    }
    unique
}

pub(crate) fn route_target_is_available_for_enabled_providers(
    target: &str,
    enabled: &std::collections::HashSet<String>,
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

pub(crate) async fn provider_models_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(slug): Path<String>,
) -> Result<Json<ProviderModelsResponse>, StatusCode> {
    let settings = if let Some(ref store) = state.store {
        let map = store
            .get_all_settings(&request_identity.principal_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to load provider settings for model discovery: {}",
                    e
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::load()
    };

    let providers_settings = crate::llm::normalize_providers_settings(&settings);
    let response = build_provider_models_response(
        &request_identity.principal_id,
        &slug,
        &settings,
        &providers_settings,
        state.secrets_store.as_ref(),
    )
    .await;

    Ok(Json(response))
}

pub(crate) async fn providers_route_simulate_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<RouteSimulateRequest>,
) -> Result<Json<RouteSimulateResponse>, StatusCode> {
    let runtime = state
        .llm_runtime
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let ctx = crate::llm::routing_policy::RoutingContext {
        estimated_input_tokens: (body.prompt.len() / 4) as u32,
        has_vision: body.has_vision,
        has_tools: body.has_tools,
        requires_streaming: body.requires_streaming,
        budget_usd: None,
    };
    let result = runtime.simulate_route_details(ctx, Some(body.prompt.as_str()));
    Ok(Json(RouteSimulateResponse {
        target: result.target,
        reason: result.reason,
        fallback_chain: result.fallback_chain,
        candidate_list: result.candidate_list,
        rejections: result.rejections,
        score_breakdown: result
            .score_breakdown
            .into_iter()
            .map(|score| RouteSimulateScore {
                target: score.target,
                telemetry_key: score.telemetry_key,
                quality: score.quality,
                cost: score.cost,
                latency: score.latency,
                health: score.health,
                policy_bias: score.policy_bias,
                composite: score.composite,
            })
            .collect(),
        diagnostics: result.diagnostics,
    }))
}

#[derive(serde::Deserialize)]
pub(crate) struct ProviderKeyRequest {
    #[serde(default)]
    api_key: Option<String>,
}

fn provider_key_write_limiter() -> &'static RateLimiter {
    static LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    LIMITER.get_or_init(|| RateLimiter::new(10, 60))
}

fn require_sensitive_route_auth(identity: &GatewayRequestIdentity) -> Result<(), StatusCode> {
    match identity.auth_source {
        GatewayAuthSource::BearerHeader | GatewayAuthSource::TrustedProxy => Ok(()),
        GatewayAuthSource::BearerQuery => Err(StatusCode::FORBIDDEN),
    }
}

fn validate_provider_api_key(raw: Option<&str>) -> Result<String, StatusCode> {
    let api_key = raw.unwrap_or("").trim().to_string();
    if api_key.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if api_key
        .chars()
        .any(|ch| ch.is_control() || ch == '\n' || ch == '\r')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(api_key)
}

fn mask_provider_key(value: &str) -> String {
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

fn provider_key_fingerprint(value: &str) -> String {
    let key = blake3::derive_key(
        "thinclaw.provider-vault.fingerprint.v1",
        b"local-display-only",
    );
    let hash = blake3::keyed_hash(&key, value.as_bytes());
    hex::encode(&hash.as_bytes()[..12])
}

pub(crate) async fn providers_save_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(slug): Path<String>,
    Json(body): Json<ProviderKeyRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    require_sensitive_route_auth(&request_identity)?;
    if !provider_key_write_limiter().check() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let spec = provider_credential_spec(&slug).ok_or(StatusCode::NOT_FOUND)?;

    match &spec {
        ProviderCredentialSpec::ApiKey { secret_name, .. } => {
            let api_key = validate_provider_api_key(body.api_key.as_deref())?;
            let masked = mask_provider_key(&api_key);
            let fingerprint = provider_key_fingerprint(&api_key);
            let params = crate::secrets::CreateSecretParams::new(*secret_name, api_key)
                .with_provider(slug.clone())
                .with_created_by(format!(
                    "provider_vault:{}",
                    request_identity.auth_source.as_str()
                ));
            secrets
                .create(&request_identity.principal_id, params)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to save API key for '{}': {}", slug, e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            tracing::info!(
                provider = %slug,
                fingerprint = %fingerprint,
                masked = %masked,
                "Provider Vault credential atomically upserted"
            );
        }
    }

    let count =
        crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    tracing::info!(
        provider = %slug,
        refreshed = count,
        "Provider Vault credentials saved and secrets refreshed"
    );

    if let Some(ref db) = state.store {
        auto_enable_provider(
            db.as_ref(),
            &request_identity.principal_id,
            &slug,
            spec.default_model(),
        )
        .await;
    }
    if let Err(e) = reload_llm_runtime(state.as_ref()).await {
        tracing::warn!("Provider Vault runtime reload failed after save: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "{} credentials were saved, but the live LLM runtime could not be reloaded: {}",
                    spec.display_name(), e
                ),
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("Credentials saved for {}", spec.display_name()),
            "credential": {
                "source": "local_encrypted",
                "provider": slug,
                "masked_preview": body.api_key.as_deref().map(mask_provider_key),
                "fingerprint": body.api_key.as_deref().map(provider_key_fingerprint),
            }
        })),
    ))
}

pub(crate) async fn providers_delete_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(slug): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    require_sensitive_route_auth(&request_identity)?;
    if !provider_key_write_limiter().check() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let spec = provider_credential_spec(&slug).ok_or(StatusCode::NOT_FOUND)?;

    match &spec {
        ProviderCredentialSpec::ApiKey { secret_name, .. } => {
            secrets
                .delete(&request_identity.principal_id, secret_name)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete API key for '{}': {}", slug, e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
        }
    }

    let count =
        crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    tracing::info!(
        provider = %slug,
        refreshed = count,
        "Provider Vault credentials removed and secrets refreshed"
    );

    if let Some(ref db) = state.store {
        auto_disable_provider(db.as_ref(), &request_identity.principal_id, &slug).await;
    }
    if let Err(e) = reload_llm_runtime(state.as_ref()).await {
        tracing::warn!("Provider Vault runtime reload failed after delete: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "{} credentials were removed, but the live LLM runtime could not be reloaded: {}",
                    spec.display_name(), e
                ),
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("Credentials removed for {}", spec.display_name()),
        })),
    ))
}

enum ProviderCredentialSpec {
    ApiKey {
        display_name: &'static str,
        secret_name: &'static str,
        default_model: &'static str,
    },
}

impl ProviderCredentialSpec {
    fn display_name(&self) -> &'static str {
        match self {
            Self::ApiKey { display_name, .. } => display_name,
        }
    }

    fn default_model(&self) -> &'static str {
        match self {
            Self::ApiKey { default_model, .. } => default_model,
        }
    }
}

fn provider_credential_spec(slug: &str) -> Option<ProviderCredentialSpec> {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        return Some(ProviderCredentialSpec::ApiKey {
            display_name: endpoint.display_name.as_str(),
            secret_name: endpoint.secret_name.as_str(),
            default_model: endpoint.default_model.as_str(),
        });
    }

    match slug {
        "openai_compatible" => Some(ProviderCredentialSpec::ApiKey {
            display_name: "OpenAI-compatible",
            secret_name: "llm_compatible_api_key",
            default_model: "default",
        }),
        "bedrock" => Some(ProviderCredentialSpec::ApiKey {
            display_name: "AWS Bedrock",
            secret_name: "llm_bedrock_api_key",
            default_model: "anthropic.claude-3-sonnet-20240229-v1:0",
        }),
        _ => None,
    }
}

async fn auto_enable_provider(
    db: &dyn crate::db::Database,
    user_id: &str,
    slug: &str,
    default_model: &str,
) {
    let enabled = db
        .get_setting(user_id, "providers.enabled")
        .await
        .ok()
        .flatten();
    let mut enabled_list: Vec<String> = enabled
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if !enabled_list.iter().any(|s| s == slug) {
        enabled_list.push(slug.to_string());
        if let Err(e) = db
            .set_setting(
                user_id,
                "providers.enabled",
                &serde_json::json!(enabled_list),
            )
            .await
        {
            tracing::warn!("Failed to auto-enable provider '{}': {}", slug, e);
        } else {
            tracing::info!(provider = %slug, "Provider auto-enabled in providers.enabled");
        }
    }

    let chain = db
        .get_setting(user_id, "providers.fallback_chain")
        .await
        .ok()
        .flatten();
    let mut chain_list: Vec<String> = chain
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let fallback_entry = format!("{}/{}", slug, default_model);
    if !chain_list
        .iter()
        .any(|s| s.starts_with(&format!("{}/", slug)))
    {
        chain_list.push(fallback_entry.clone());
        if let Err(e) = db
            .set_setting(
                user_id,
                "providers.fallback_chain",
                &serde_json::json!(chain_list),
            )
            .await
        {
            tracing::warn!(
                "Failed to add '{}' to fallback chain: {}",
                fallback_entry,
                e
            );
        } else {
            tracing::info!(entry = %fallback_entry, "Provider added to fallback chain");
        }
    }
}

async fn auto_disable_provider(db: &dyn crate::db::Database, user_id: &str, slug: &str) {
    let enabled = db
        .get_setting(user_id, "providers.enabled")
        .await
        .ok()
        .flatten();
    if let Some(mut enabled_list) =
        enabled.and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
    {
        let before = enabled_list.len();
        enabled_list.retain(|s| s != slug);
        if enabled_list.len() != before {
            let _ = db
                .set_setting(
                    user_id,
                    "providers.enabled",
                    &serde_json::json!(enabled_list),
                )
                .await;
            tracing::info!(provider = %slug, "Provider removed from providers.enabled");
        }
    }

    let chain = db
        .get_setting(user_id, "providers.fallback_chain")
        .await
        .ok()
        .flatten();
    if let Some(mut chain_list) = chain.and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
    {
        let prefix = format!("{}/", slug);
        let before = chain_list.len();
        chain_list.retain(|s| !s.starts_with(&prefix));
        if chain_list.len() != before {
            let _ = db
                .set_setting(
                    user_id,
                    "providers.fallback_chain",
                    &serde_json::json!(chain_list),
                )
                .await;
            tracing::info!(provider = %slug, "Provider entries removed from fallback chain");
        }
    }
}

pub(crate) async fn reload_llm_runtime(state: &GatewayState) -> Result<(), String> {
    if let Some(ref runtime) = state.llm_runtime {
        runtime.reload().await.map_err(|e| e.to_string())?;
        reconcile_advisor_tool_registration(state).await;
    }
    Ok(())
}

async fn reconcile_advisor_tool_registration(state: &GatewayState) {
    let Some(ref registry) = state.tool_registry else {
        return;
    };
    let Some(ref runtime) = state.llm_runtime else {
        return;
    };

    let status = runtime.status();
    registry
        .reconcile_advisor_tool_readiness(status.advisor_ready)
        .await;
}

pub(crate) fn sync_legacy_llm_settings(settings: &mut crate::settings::Settings) {
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

    if settings.providers.primary_model.is_some() {
        settings.selected_model = settings.providers.primary_model.clone();
    } else {
        settings.selected_model = None;
    }
}

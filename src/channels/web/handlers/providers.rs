use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::identity_helpers::{GatewayAuthSource, GatewayRequestIdentity};
use crate::channels::web::rate_limiter::RateLimiter;
use crate::channels::web::server::GatewayState;
use thinclaw_gateway::web::providers::{
    DiscoveredProviderModel, ProviderCredentialMetadata, ProviderCredentialSpec, ProviderIdentity,
    ProviderInfoInput, ProviderKeyMutationResponse, ProviderKeyRequest, ProviderModelOption,
    ProviderModelsResponse, ProviderModelsResponseInput, ProviderOauthUiSourceInput,
    ProvidersConfigResponse, ProvidersConfigWriteRequest, ProvidersListResponse,
    RouteSimulateRequest, RouteSimulateResponse, RouteSimulateResponseInput,
    RouteSimulateScoreInput, SyntheticProviderEntryInput, apply_providers_config_write,
    fallback_provider_credential_spec,
    fallback_provider_model_options as gateway_fallback_provider_model_options, mask_provider_key,
    provider_auth_mode, provider_auto_disable_setting_updates,
    provider_auto_enable_setting_updates, provider_cheap_model_for_slug,
    provider_credential_spec_not_found_status, provider_credentials_not_configured_message,
    provider_fallback_model_catalog as gateway_provider_fallback_model_catalog,
    provider_identity as gateway_provider_identity, provider_info,
    provider_key_delete_partial_failure_response, provider_key_deleted_response,
    provider_key_fingerprint, provider_key_save_partial_failure_response,
    provider_key_saved_response,
    provider_model_options_from_discovery as gateway_provider_model_options_from_discovery,
    provider_models_response, provider_oauth_ui_state as gateway_provider_oauth_ui_state,
    provider_primary_model_for_slug, provider_runtime_unavailable_status,
    provider_secrets_store_unavailable_status, provider_sensitive_route_forbidden_status,
    provider_store_unavailable_status,
    provider_supports_model_discovery as gateway_provider_supports_model_discovery,
    providers_list_response, route_simulate_response,
    suggested_cheap_model_from_catalog as gateway_suggested_cheap_model_from_catalog,
    synthetic_provider_entry as gateway_synthetic_provider_entry, validate_provider_api_key,
};
pub(crate) use thinclaw_gateway::web::providers::{
    PROVIDERS_ENABLED_SETTING_KEY, PROVIDERS_FALLBACK_CHAIN_SETTING_KEY, ProviderConfigEntry,
    stale_provider_namespace_keys, sync_legacy_llm_settings,
};

fn provider_oauth_ui_source(slug: &str) -> Option<ProviderOauthUiSourceInput> {
    let kind = crate::llm::credential_sync::provider_oauth_source_kind(slug)?;
    Some(ProviderOauthUiSourceInput {
        available: crate::llm::credential_sync::oauth_source_available(kind),
        source_label: crate::llm::credential_sync::oauth_source_label(kind).to_string(),
        source_location: crate::llm::credential_sync::oauth_source_location_hint(kind),
    })
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
        let oauth = gateway_provider_oauth_ui_state(provider_oauth_ui_source(slug));

        let api_style_str = match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::OpenAi => "openai",
            crate::config::provider_catalog::ApiStyle::Anthropic => "anthropic",
            crate::config::provider_catalog::ApiStyle::OpenAiCompatible => "openai_compatible",
            crate::config::provider_catalog::ApiStyle::Ollama => "ollama",
        };

        providers.push(provider_info(ProviderInfoInput {
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
        }));
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
    providers.push(provider_info(ProviderInfoInput {
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
    }));

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
    providers.push(provider_info(ProviderInfoInput {
        slug: "bedrock".to_string(),
        display_name: "AWS Bedrock".to_string(),
        api_style: "bedrock".to_string(),
        default_model: "anthropic.claude-opus-4-8".to_string(),
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
    }));

    Ok(Json(providers_list_response(providers)))
}

pub(crate) async fn providers_config_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<ProvidersConfigResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(provider_store_unavailable_status)?;
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
        let oauth = gateway_provider_oauth_ui_state(provider_oauth_ui_source(slug));
        let primary_model = provider_primary_model_for_slug(
            settings,
            providers_settings,
            slug,
            &endpoint.default_model,
        );
        let suggested_cheap_model = suggested_cheap_model_for_slug(slug, &endpoint.default_model);
        let cheap_model = provider_cheap_model_for_slug(
            settings,
            providers_settings,
            slug,
            &endpoint.default_model,
            suggested_cheap_model.as_deref(),
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
            suggested_cheap_model: cheap_model.or(suggested_cheap_model),
            setup_url: endpoint.setup_url.clone(),
            tier: endpoint.tier.clone(),
        });
    }

    providers.push(synthetic_provider_entry(
        "ollama",
        "ollama",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("ollama")),
        "OLLAMA_BASE_URL",
        providers_settings,
        settings,
        true,
        false,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "openai_compatible",
        "openai_compatible",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("openai_compatible")),
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
        "bedrock",
        None,
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
        "llama_cpp",
        None,
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
    api_style: &str,
    default_model_override: Option<&str>,
    env_key_name: &str,
    providers_settings: &crate::settings::ProvidersSettings,
    settings: &crate::settings::Settings,
    has_key: bool,
    auth_required: bool,
    oauth_supported: bool,
) -> ProviderConfigEntry {
    let identity = provider_identity(slug);
    let default_model = default_model_override
        .map(str::to_string)
        .unwrap_or_else(|| identity.default_model.clone());
    let suggested_cheap_model = suggested_cheap_model_for_slug(slug, &default_model);
    let suggested_primary_model = Some(default_model.clone());
    gateway_synthetic_provider_entry(
        SyntheticProviderEntryInput {
            slug: slug.to_string(),
            display_name: identity.display_name,
            api_style: api_style.to_string(),
            default_model,
            env_key_name: env_key_name.to_string(),
            has_key,
            auth_required,
            oauth_supported,
            discovery_supported: provider_supports_model_discovery(slug),
            suggested_primary_model,
            suggested_cheap_model,
            setup_url: None,
            tier: None,
        },
        providers_settings,
        settings,
    )
}

fn suggested_cheap_model_for_slug(slug: &str, default_model: &str) -> Option<String> {
    let catalog_suggested = crate::config::provider_catalog::endpoint_for(slug)
        .and_then(|endpoint| endpoint.suggested_cheap_model.as_deref());
    gateway_suggested_cheap_model_from_catalog(default_model, catalog_suggested)
}

fn provider_supports_model_discovery(slug: &str) -> bool {
    gateway_provider_supports_model_discovery(
        slug,
        crate::config::provider_catalog::endpoint_for(slug).is_some(),
    )
}

pub(crate) async fn build_provider_models_response(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> ProviderModelsResponse {
    let identity = provider_identity(slug);
    let display_name = identity.display_name;
    let default_model = identity.default_model;
    let catalog_suggested_cheap_model =
        suggested_cheap_model_for_slug(slug, default_model.as_str());
    let current_primary_model =
        provider_primary_model_for_slug(settings, providers_settings, slug, default_model.as_str());
    let current_cheap_model = provider_cheap_model_for_slug(
        settings,
        providers_settings,
        slug,
        default_model.as_str(),
        catalog_suggested_cheap_model.as_deref(),
    );
    let discovery_supported = provider_supports_model_discovery(slug);

    if !discovery_supported {
        let suggested_primary_model = current_primary_model
            .clone()
            .or_else(|| Some(default_model.clone()));
        let suggested_cheap_model = current_cheap_model
            .clone()
            .or_else(|| catalog_suggested_cheap_model.clone());
        return provider_models_response(ProviderModelsResponseInput {
            slug: slug.to_string(),
            display_name,
            discovery_supported: false,
            discovery_status: "unsupported".to_string(),
            error: None,
            current_primary_model: current_primary_model.clone(),
            current_cheap_model: current_cheap_model.clone(),
            suggested_primary_model: suggested_primary_model.clone(),
            suggested_cheap_model: suggested_cheap_model.clone(),
            models: gateway_fallback_provider_model_options(
                default_model.as_str(),
                current_primary_model.as_deref(),
                current_cheap_model.as_deref(),
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                fallback_provider_model_catalog(slug),
            ),
        });
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
                    .or_else(|| catalog_suggested_cheap_model.clone());
                provider_models_response(ProviderModelsResponseInput {
                    slug: slug.to_string(),
                    display_name,
                    discovery_supported: true,
                    discovery_status: "fallback".to_string(),
                    error: result.error,
                    current_primary_model: current_primary_model.clone(),
                    current_cheap_model: current_cheap_model.clone(),
                    suggested_primary_model: fallback_primary_model.clone(),
                    suggested_cheap_model: fallback_cheap_model.clone(),
                    models: gateway_fallback_provider_model_options(
                        default_model.as_str(),
                        current_primary_model.as_deref(),
                        current_cheap_model.as_deref(),
                        fallback_primary_model.as_deref(),
                        fallback_cheap_model.as_deref(),
                        fallback_provider_model_catalog(slug),
                    ),
                })
            } else {
                provider_models_response(ProviderModelsResponseInput {
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
                })
            }
        }
        Err(error) => {
            let suggested_primary_model = current_primary_model
                .clone()
                .or_else(|| Some(default_model.clone()));
            let suggested_cheap_model = current_cheap_model
                .clone()
                .or_else(|| catalog_suggested_cheap_model.clone());
            provider_models_response(ProviderModelsResponseInput {
                slug: slug.to_string(),
                display_name,
                discovery_supported: true,
                discovery_status: "fallback".to_string(),
                error: Some(error),
                current_primary_model: current_primary_model.clone(),
                current_cheap_model: current_cheap_model.clone(),
                suggested_primary_model: suggested_primary_model.clone(),
                suggested_cheap_model: suggested_cheap_model.clone(),
                models: gateway_fallback_provider_model_options(
                    default_model.as_str(),
                    current_primary_model.as_deref(),
                    current_cheap_model.as_deref(),
                    suggested_primary_model.as_deref(),
                    suggested_cheap_model.as_deref(),
                    fallback_provider_model_catalog(slug),
                ),
            })
        }
    }
}

fn provider_identity(slug: &str) -> ProviderIdentity {
    let catalog_identity = crate::config::provider_catalog::endpoint_for(slug).map(|endpoint| {
        ProviderIdentity::new(
            endpoint.display_name.as_str(),
            endpoint.default_model.as_str(),
        )
    });
    gateway_provider_identity(slug, catalog_identity)
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
            || provider_credentials_not_configured_message(&endpoint.display_name);
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
    let discovered = discovered
        .into_iter()
        .map(|id| DiscoveredProviderModel {
            id: id.id,
            name: id.name,
            is_chat: id.is_chat,
            context_length: id.context_length,
        })
        .collect();
    gateway_provider_model_options_from_discovery(
        slug,
        default_model,
        discovered,
        current_primary_model,
        current_cheap_model,
        suggested_cheap_model_for_slug(slug, default_model).as_deref(),
    )
}

fn fallback_provider_model_catalog(slug: &str) -> Vec<(String, String)> {
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
    gateway_provider_fallback_model_catalog(slug, dynamic)
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
        .ok_or_else(provider_store_unavailable_status)?;
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

    apply_providers_config_write(&mut settings, &body);

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
        .ok_or_else(provider_runtime_unavailable_status)?;
    let ctx = crate::llm::routing_policy::RoutingContext {
        estimated_input_tokens: (body.prompt.len() / 4) as u32,
        has_vision: body.has_vision,
        has_tools: body.has_tools,
        requires_streaming: body.requires_streaming,
        budget_usd: None,
    };
    let result = runtime.simulate_route_details(ctx, Some(body.prompt.as_str()));
    Ok(Json(route_simulate_response(RouteSimulateResponseInput {
        target: result.target,
        reason: result.reason,
        fallback_chain: result.fallback_chain,
        candidate_list: result.candidate_list,
        rejections: result.rejections,
        score_breakdown: result
            .score_breakdown
            .into_iter()
            .map(|score| RouteSimulateScoreInput {
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
    })))
}

fn provider_key_write_limiter() -> &'static RateLimiter {
    static LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    LIMITER.get_or_init(|| RateLimiter::new(10, 60))
}

fn require_sensitive_route_auth(identity: &GatewayRequestIdentity) -> Result<(), StatusCode> {
    match identity.auth_source {
        GatewayAuthSource::BearerHeader | GatewayAuthSource::TrustedProxy => Ok(()),
        // Provider Vault (API key management) is never grantable to device
        // tokens (docs/MOBILE_SECURITY.md D-T4: settings/secrets/providers
        // are excluded from all v1 device scopes), same as the query-param
        // bearer path.
        GatewayAuthSource::BearerQuery | GatewayAuthSource::DeviceToken => {
            Err(provider_sensitive_route_forbidden_status())
        }
    }
}

pub(crate) async fn providers_save_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(slug): Path<String>,
    Json(body): Json<ProviderKeyRequest>,
) -> Result<(StatusCode, Json<ProviderKeyMutationResponse>), StatusCode> {
    require_sensitive_route_auth(&request_identity)?;
    if !provider_key_write_limiter().check() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or_else(provider_secrets_store_unavailable_status)?;
    let spec =
        provider_credential_spec(&slug).ok_or_else(provider_credential_spec_not_found_status)?;

    let api_key =
        validate_provider_api_key(body.api_key.as_deref()).map_err(|error| error.status_code())?;
    let masked = mask_provider_key(&api_key);
    let fingerprint = provider_key_fingerprint(&api_key);
    let params = crate::secrets::CreateSecretParams::new(spec.secret_name.clone(), api_key)
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
            Json(provider_key_save_partial_failure_response(
                spec.display_name(),
                e,
            )),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(provider_key_saved_response(
            spec.display_name(),
            slug,
            Some(masked),
            Some(fingerprint),
        )),
    ))
}

pub(crate) async fn providers_delete_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(slug): Path<String>,
) -> Result<(StatusCode, Json<ProviderKeyMutationResponse>), StatusCode> {
    require_sensitive_route_auth(&request_identity)?;
    if !provider_key_write_limiter().check() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or_else(provider_secrets_store_unavailable_status)?;
    let spec =
        provider_credential_spec(&slug).ok_or_else(provider_credential_spec_not_found_status)?;

    secrets
        .delete(&request_identity.principal_id, &spec.secret_name)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete API key for '{}': {}", slug, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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
            Json(provider_key_delete_partial_failure_response(
                spec.display_name(),
                e,
            )),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(provider_key_deleted_response(spec.display_name())),
    ))
}

fn provider_credential_spec(slug: &str) -> Option<ProviderCredentialSpec> {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        return Some(ProviderCredentialSpec::api_key(
            endpoint.display_name.as_str(),
            endpoint.secret_name.as_str(),
            endpoint.default_model.as_str(),
        ));
    }

    fallback_provider_credential_spec(slug)
}

async fn auto_enable_provider(
    db: &dyn crate::db::Database,
    user_id: &str,
    slug: &str,
    default_model: &str,
) {
    let enabled = db
        .get_setting(user_id, PROVIDERS_ENABLED_SETTING_KEY)
        .await
        .ok()
        .flatten();
    let chain = db
        .get_setting(user_id, PROVIDERS_FALLBACK_CHAIN_SETTING_KEY)
        .await
        .ok()
        .flatten();

    for update in provider_auto_enable_setting_updates(enabled, chain, slug, default_model) {
        if let Err(e) = db
            .set_setting(user_id, update.key, &serde_json::json!(update.value))
            .await
        {
            match update.key {
                PROVIDERS_ENABLED_SETTING_KEY => {
                    tracing::warn!("Failed to auto-enable provider '{}': {}", slug, e);
                }
                PROVIDERS_FALLBACK_CHAIN_SETTING_KEY => {
                    tracing::warn!(
                        "Failed to add '{}/{}' to fallback chain: {}",
                        slug,
                        default_model,
                        e
                    );
                }
                _ => {}
            }
        } else {
            match update.key {
                PROVIDERS_ENABLED_SETTING_KEY => {
                    tracing::info!(provider = %slug, "Provider auto-enabled in providers.enabled");
                }
                PROVIDERS_FALLBACK_CHAIN_SETTING_KEY => {
                    let fallback_entry = format!("{slug}/{default_model}");
                    tracing::info!(entry = %fallback_entry, "Provider added to fallback chain");
                }
                _ => {}
            }
        }
    }
}

async fn auto_disable_provider(db: &dyn crate::db::Database, user_id: &str, slug: &str) {
    let enabled = db
        .get_setting(user_id, PROVIDERS_ENABLED_SETTING_KEY)
        .await
        .ok()
        .flatten();
    let chain = db
        .get_setting(user_id, PROVIDERS_FALLBACK_CHAIN_SETTING_KEY)
        .await
        .ok()
        .flatten();

    for update in provider_auto_disable_setting_updates(enabled, chain, slug) {
        let result = db
            .set_setting(user_id, update.key, &serde_json::json!(update.value))
            .await;
        if result.is_ok() {
            match update.key {
                PROVIDERS_ENABLED_SETTING_KEY => {
                    tracing::info!(provider = %slug, "Provider removed from providers.enabled");
                }
                PROVIDERS_FALLBACK_CHAIN_SETTING_KEY => {
                    tracing::info!(provider = %slug, "Provider entries removed from fallback chain");
                }
                _ => {}
            }
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

use super::*;
use axum::http::StatusCode;
use thinclaw_settings::{
    AdvisorAutoEscalationMode, ProviderCredentialMode, ProviderModelSlots, ProvidersSettings,
    RoutingMode, Settings,
};

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
        provider_primary_model_for_slug(&settings, &providers, "openrouter", "default").as_deref(),
        Some("configured-primary")
    );

    providers.primary_model = None;
    assert_eq!(
        provider_primary_model_for_slug(&settings, &providers, "openrouter", "default").as_deref(),
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

    let static_fallback = provider_fallback_model_catalog("openai", Vec::<(String, String)>::new());
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
    let enabled = std::collections::HashSet::from(["anthropic".to_string(), "openai".to_string()]);

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
    let enabled = std::collections::HashSet::from(["openai".to_string(), "anthropic".to_string()]);
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

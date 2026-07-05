use super::*;
#[cfg(feature = "libsql")]
use crate::db::ConversationStore;

#[test]
fn test_provider_model_options_from_discovery_returns_live_models_only() {
    let discovered = vec![
        crate::llm::discovery::DiscoveredModel {
            id: "gpt-4o".to_string(),
            name: "gpt-4o".to_string(),
            provider: "openai".to_string(),
            is_chat: true,
            context_length: None,
        },
        crate::llm::discovery::DiscoveredModel {
            id: "gpt-4o-mini".to_string(),
            name: "gpt-4o-mini".to_string(),
            provider: "openai".to_string(),
            is_chat: true,
            context_length: None,
        },
    ];

    let (models, suggested_primary, suggested_cheap, has_live_models) =
        provider_model_options_from_discovery(
            "openai",
            "gpt-4o",
            discovered,
            Some("gpt-legacy"),
            None,
        );

    let model_ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert!(has_live_models);
    assert_eq!(model_ids, vec!["gpt-4o", "gpt-4o-mini"]);
    assert_eq!(suggested_primary.as_deref(), Some("gpt-4o"));
    assert_eq!(suggested_cheap.as_deref(), Some("gpt-4o-mini"));
}

#[test]
fn auth_result_to_gateway_preserves_setup_metadata() {
    let status = auth_result_to_gateway(crate::extensions::AuthResult {
        name: "calendar".to_string(),
        kind: crate::extensions::ExtensionKind::WasmTool,
        auth_mode: "manual_token".to_string(),
        auth_status: "awaiting_token".to_string(),
        auth_url: None,
        callback_type: Some("web".to_string()),
        instructions: Some("Paste a token".to_string()),
        setup_url: Some("https://example.test/setup".to_string()),
        shared_auth_provider: Some("google".to_string()),
        missing_scopes: vec!["calendar.read".to_string()],
        awaiting_token: true,
        status: "awaiting_token".to_string(),
    });

    assert_eq!(status.extension_name, "calendar");
    assert_eq!(status.auth_mode, "manual_token");
    assert_eq!(status.auth_status, "awaiting_token");
    assert_eq!(status.missing_scopes, vec!["calendar.read"]);
    assert_eq!(status.metadata["kind"], "wasm_tool");
    assert_eq!(status.metadata["callback_type"], "web");
    assert_eq!(status.metadata["instructions"], "Paste a token");
    assert_eq!(status.metadata["setup_url"], "https://example.test/setup");
    assert_eq!(status.metadata["shared_auth_provider"], "google");
    assert_eq!(status.metadata["awaiting_token"], true);
}

#[test]
fn test_provider_model_options_from_discovery_prefers_catalog_default_primary() {
    let discovered = vec![
        crate::llm::discovery::DiscoveredModel {
            id: "claude-sonnet-4-6".to_string(),
            name: "claude-sonnet-4-6".to_string(),
            provider: "anthropic".to_string(),
            is_chat: true,
            context_length: None,
        },
        crate::llm::discovery::DiscoveredModel {
            id: "claude-opus-4-7".to_string(),
            name: "claude-opus-4-7".to_string(),
            provider: "anthropic".to_string(),
            is_chat: true,
            context_length: None,
        },
    ];

    let (_models, suggested_primary, suggested_cheap, has_live_models) =
        provider_model_options_from_discovery(
            "anthropic",
            "claude-opus-4-7",
            discovered,
            None,
            None,
        );

    assert!(has_live_models);
    assert_eq!(suggested_primary.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(suggested_cheap.as_deref(), Some("claude-sonnet-4-6"));
}

#[test]
fn test_provider_model_options_from_discovery_rejects_filtered_only_results() {
    let discovered = vec![crate::llm::discovery::DiscoveredModel {
        id: "text-embedding-3-small".to_string(),
        name: "text-embedding-3-small".to_string(),
        provider: "openai".to_string(),
        is_chat: false,
        context_length: None,
    }];

    let (models, suggested_primary, suggested_cheap, has_live_models) =
        provider_model_options_from_discovery(
            "openai",
            "gpt-4o",
            discovered,
            Some("gpt-legacy"),
            None,
        );

    assert!(!has_live_models);
    assert!(models.is_empty());
    assert_eq!(suggested_primary.as_deref(), Some("gpt-4o"));
    assert_eq!(suggested_cheap.as_deref(), Some("gpt-4o-mini"));
}

#[test]
fn test_provider_model_options_from_discovery_keeps_large_catalogs() {
    let discovered = (0..64)
        .map(|idx| crate::llm::discovery::DiscoveredModel {
            id: format!("anthropic/model-{idx:02}"),
            name: format!("Anthropic Model {idx:02}"),
            provider: "openai_compatible".to_string(),
            is_chat: true,
            context_length: Some(200_000),
        })
        .collect::<Vec<_>>();

    let (models, _suggested_primary, _suggested_cheap, has_live_models) =
        provider_model_options_from_discovery(
            "openrouter",
            "anthropic/model-00",
            discovered,
            None,
            None,
        );

    assert!(has_live_models);
    assert_eq!(models.len(), 64);
    assert!(
        models
            .iter()
            .all(|model| model.context_length == Some(200_000))
    );
    assert!(
        models
            .iter()
            .any(|model| model.label == "Anthropic Model 00" && model.id == "anthropic/model-00")
    );
}

#[test]
fn test_sync_legacy_llm_settings_clears_legacy_when_no_primary_provider() {
    let mut settings = crate::settings::Settings {
        llm_backend: Some("openai".to_string()),
        selected_model: Some("gpt-4o".to_string()),
        ..crate::settings::Settings::default()
    };

    settings.providers.primary = None;
    settings.providers.primary_model = None;

    sync_legacy_llm_settings(&mut settings);

    assert_eq!(settings.llm_backend, None);
    assert_eq!(settings.selected_model, None);
}

#[test]
fn test_sync_legacy_llm_settings_updates_legacy_for_primary_provider() {
    let mut settings = crate::settings::Settings::default();
    settings.providers.primary = Some("anthropic".to_string());
    settings.providers.primary_model = Some("claude-sonnet-4-6".to_string());

    sync_legacy_llm_settings(&mut settings);

    assert_eq!(settings.llm_backend.as_deref(), Some("anthropic"));
    assert_eq!(
        settings.selected_model.as_deref(),
        Some("claude-sonnet-4-6")
    );
}

#[test]
fn test_route_target_availability_respects_enabled_providers() {
    let enabled = std::collections::HashSet::from(["anthropic".to_string(), "openai".to_string()]);

    assert!(route_target_is_available_for_enabled_providers(
        "primary", &enabled
    ));
    assert!(route_target_is_available_for_enabled_providers(
        "cheap", &enabled
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
    assert!(!route_target_is_available_for_enabled_providers(
        "gemini/gemini-2.5-pro",
        &enabled
    ));
}

#[test]
fn test_stale_provider_namespace_keys_detect_removed_provider_entries() {
    let mut previous_settings = crate::settings::Settings::default();
    previous_settings.providers.enabled = vec!["openai".to_string()];
    previous_settings.providers.primary = Some("openai".to_string());
    previous_settings.providers.primary_model = Some("gpt-4o".to_string());
    previous_settings
        .providers
        .allowed_models
        .insert("openai".to_string(), vec!["gpt-4o".to_string()]);
    previous_settings.providers.provider_models.insert(
        "openai".to_string(),
        crate::settings::ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );

    let previous_map = previous_settings.to_db_map();
    let next_map = crate::settings::Settings::default().to_db_map();
    let stale = stale_provider_namespace_keys(&previous_map, &next_map);

    assert!(
        stale
            .iter()
            .any(|key| key == "providers.allowed_models.openai")
    );
    assert!(
        stale
            .iter()
            .any(|key| key == "providers.provider_models.openai.primary")
    );
    assert!(
        stale
            .iter()
            .any(|key| key == "providers.provider_models.openai.cheap")
    );
}

#[test]
fn test_stale_allowed_model_db_rows_can_reenable_provider_without_cleanup() {
    let mut previous_settings = crate::settings::Settings::default();
    previous_settings.providers.enabled = vec!["openai".to_string()];
    previous_settings
        .providers
        .allowed_models
        .insert("openai".to_string(), vec!["gpt-4o".to_string()]);

    let previous_map = previous_settings.to_db_map();
    let next_map = crate::settings::Settings::default().to_db_map();

    let mut merged_without_cleanup = previous_map.clone();
    merged_without_cleanup.extend(next_map.clone());

    let restored_without_cleanup = crate::settings::Settings::from_db_map(&merged_without_cleanup);
    let normalized_without_cleanup =
        crate::llm::normalize_providers_settings(&restored_without_cleanup);
    assert!(
        normalized_without_cleanup
            .enabled
            .iter()
            .any(|slug| slug == "openai")
    );

    let stale_keys = stale_provider_namespace_keys(&previous_map, &next_map);
    let mut merged_with_cleanup = merged_without_cleanup;
    for key in stale_keys {
        merged_with_cleanup.remove(&key);
    }

    let restored_with_cleanup = crate::settings::Settings::from_db_map(&merged_with_cleanup);
    let normalized_with_cleanup = crate::llm::normalize_providers_settings(&restored_with_cleanup);
    assert!(
        !normalized_with_cleanup
            .enabled
            .iter()
            .any(|slug| slug == "openai")
    );
}

#[test]
fn test_resolve_saved_provider_models_preserves_previous_slot_values() {
    let provider = ProviderConfigEntry {
        slug: "gemini".to_string(),
        display_name: "Google".to_string(),
        api_style: "openai".to_string(),
        default_model: "gemini-2.5-flash".to_string(),
        env_key_name: "GOOGLE_API_KEY".to_string(),
        has_key: true,
        credential_ready: true,
        auth_required: true,
        auth_mode: "api_key".to_string(),
        oauth_supported: false,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        enabled: true,
        primary: false,
        preferred_cheap: false,
        discovery_supported: true,
        primary_model: None,
        cheap_model: None,
        suggested_primary_model: Some("gemini-2.5-flash".to_string()),
        suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
        setup_url: None,
        tier: None,
    };
    let previous_slots = crate::settings::ProviderModelSlots {
        primary: Some("gemini-3.1-flash-live-preview".to_string()),
        cheap: Some("gemini-2.5-flash-lite-preview".to_string()),
    };

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
    let previous_slots = ProviderModelSlotsSnapshot {
        primary: previous_slots.primary.clone(),
        cheap: previous_slots.cheap.clone(),
    };
    let resolved = thinclaw_gateway::web::providers::resolve_saved_provider_models(
        &input,
        Some(&previous_slots),
        None,
    );
    let (primary_model, cheap_model, should_persist) = (
        resolved.primary_model,
        resolved.cheap_model,
        resolved.should_persist_slots,
    );

    assert_eq!(
        primary_model.as_deref(),
        Some("gemini-3.1-flash-live-preview")
    );
    assert_eq!(
        cheap_model.as_deref(),
        Some("gemini-2.5-flash-lite-preview")
    );
    assert!(should_persist);
}

#[test]
fn test_resolve_saved_provider_models_prefers_incoming_values() {
    let provider = ProviderConfigEntry {
        slug: "gemini".to_string(),
        display_name: "Google".to_string(),
        api_style: "openai".to_string(),
        default_model: "gemini-2.5-flash".to_string(),
        env_key_name: "GOOGLE_API_KEY".to_string(),
        has_key: true,
        credential_ready: true,
        auth_required: true,
        auth_mode: "api_key".to_string(),
        oauth_supported: false,
        oauth_available: false,
        oauth_source_label: None,
        oauth_source_location: None,
        enabled: true,
        primary: false,
        preferred_cheap: false,
        discovery_supported: true,
        primary_model: Some("gemini-3.1-flash-live-preview".to_string()),
        cheap_model: Some("gemini-2.5-flash-lite-preview".to_string()),
        suggested_primary_model: Some("gemini-2.5-flash".to_string()),
        suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
        setup_url: None,
        tier: None,
    };
    let previous_slots = crate::settings::ProviderModelSlots {
        primary: Some("gemini-1.5-pro".to_string()),
        cheap: Some("gemini-1.5-flash".to_string()),
    };

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
    let previous_slots = ProviderModelSlotsSnapshot {
        primary: previous_slots.primary.clone(),
        cheap: previous_slots.cheap.clone(),
    };
    let resolved = thinclaw_gateway::web::providers::resolve_saved_provider_models(
        &input,
        Some(&previous_slots),
        None,
    );
    let (primary_model, cheap_model, should_persist) = (
        resolved.primary_model,
        resolved.cheap_model,
        resolved.should_persist_slots,
    );

    assert_eq!(
        primary_model.as_deref(),
        Some("gemini-3.1-flash-live-preview")
    );
    assert_eq!(
        cheap_model.as_deref(),
        Some("gemini-2.5-flash-lite-preview")
    );
    assert!(should_persist);
}

#[tokio::test]
#[ignore = "live diagnostic for WebUI provider model discovery"]
async fn live_webui_provider_model_discovery_report() {
    let settings = crate::settings::Settings::default();
    let providers_settings = crate::settings::ProvidersSettings::default();
    let visible_providers =
        build_routing_provider_entries("test-user", &settings, &providers_settings, None)
            .await
            .into_iter()
            .filter(|provider| !matches!(provider.slug.as_str(), "llama_cpp" | "openai_compatible"))
            .collect::<Vec<_>>();

    assert!(
        !visible_providers.is_empty(),
        "expected at least one WebUI-visible provider"
    );

    let mut structural_failures = Vec::new();

    for provider in visible_providers {
        let response = build_provider_models_response(
            "test-user",
            &provider.slug,
            &settings,
            &providers_settings,
            None,
        )
        .await;

        let sample_models = response
            .models
            .iter()
            .take(5)
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        eprintln!(
            "provider={} auth_required={} has_key={} status={} models={} suggested_primary={:?} suggested_cheap={:?} error={} sample=[{}]",
            provider.slug,
            provider.auth_required,
            provider.has_key,
            response.discovery_status,
            response.models.len(),
            response.suggested_primary_model,
            response.suggested_cheap_model,
            response.error.as_deref().unwrap_or("-"),
            sample_models,
        );

        if provider.auth_required && !provider.has_key {
            assert_eq!(
                response.discovery_status, "fallback",
                "expected {} to fall back cleanly when credentials are missing",
                provider.slug
            );
            assert!(
                response
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("credentials are not configured"),
                "expected {} to report missing credentials, got {:?}",
                provider.slug,
                response.error
            );
        }

        if response.models.is_empty() {
            structural_failures.push(format!(
                "{} returned no models (status={}, error={:?})",
                provider.slug, response.discovery_status, response.error
            ));
        }

        if response.suggested_primary_model.is_none() {
            structural_failures.push(format!(
                "{} did not provide a suggested primary model",
                provider.slug
            ));
        }

        if response.suggested_cheap_model.is_none() {
            structural_failures.push(format!(
                "{} did not provide a suggested cheap model",
                provider.slug
            ));
        }
    }

    assert!(
        structural_failures.is_empty(),
        "provider model discovery structural issues:\n{}",
        structural_failures.join("\n")
    );
}

#[test]
fn test_build_turns_from_db_messages_complete() {
    let now = chrono::Utc::now();
    let messages = vec![
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "Hello".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now,
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "Hi there!".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(1),
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "How are you?".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(2),
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "Doing well!".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(3),
        },
    ];

    let turns = build_turns_from_db_messages(&messages);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].user_input, "Hello");
    assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
    assert_eq!(turns[0].state, "Completed");
    assert_eq!(turns[1].user_input, "How are you?");
    assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
}

#[test]
fn test_build_turns_from_db_messages_incomplete_last() {
    let now = chrono::Utc::now();
    let messages = vec![
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "Hello".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now,
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "Hi!".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(1),
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "Lost message".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(2),
        },
    ];

    let turns = build_turns_from_db_messages(&messages);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].user_input, "Lost message");
    assert!(turns[1].response.is_none());
    assert_eq!(turns[1].state, "Failed");
}

#[test]
fn test_build_turns_from_db_messages_empty() {
    let turns = build_turns_from_db_messages(&[]);
    assert!(turns.is_empty());
}

#[test]
fn test_build_turns_from_db_messages_hides_only_startup_user_prompt() {
    let now = chrono::Utc::now();
    let messages = vec![
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "boot prompt".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({"hide_from_webui_chat": true}),
            created_at: now,
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "boot reply".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
            created_at: now + chrono::TimeDelta::seconds(1),
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "real question".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(2),
        },
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "real answer".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: now + chrono::TimeDelta::seconds(3),
        },
    ];

    let turns = build_turns_from_db_messages(&messages);
    assert_eq!(turns.len(), 2);
    assert!(turns[0].hide_user_input);
    assert_eq!(turns[0].user_input, "");
    assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
    assert_eq!(turns[1].user_input, "real question");
    assert_eq!(turns[1].response.as_deref(), Some("real answer"));
}

#[test]
fn test_build_turns_from_db_messages_preserves_legacy_assistant_only_startup_reply() {
    let now = chrono::Utc::now();
    let messages = vec![crate::history::ConversationMessage {
        id: Uuid::new_v4(),
        role: "assistant".to_string(),
        content: "boot reply".to_string(),
        actor_id: None,
        actor_display_name: None,
        raw_sender_id: None,
        metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
        created_at: now,
    }];

    let turns = build_turns_from_db_messages(&messages);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].hide_user_input);
    assert_eq!(turns[0].user_input, "");
    assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
}

#[test]
fn test_conversation_visible_to_actor_treats_missing_actor_as_legacy_base_user() {
    assert!(conversation_visible_to_actor(
        None,
        "base-user",
        "base-user"
    ));
    assert!(!conversation_visible_to_actor(
        None,
        "base-user",
        "family-member"
    ));
    assert!(conversation_visible_to_actor(
        Some("family-member"),
        "base-user",
        "family-member"
    ));
}

fn test_gateway_state(
    user_id: &str,
    actor_id: &str,
    store: Option<Arc<dyn Database>>,
) -> GatewayState {
    GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store,
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        skill_remote_hub: None,
        skill_quarantine: None,
        chat_rate_limiter: RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        metrics_registry: None,
        response_cache: None,
        routine_engine: None,
        repo_project_supervisor: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        secrets_store: None,
        channel_manager: None,
        hooks: None,
    }
}

#[tokio::test]
async fn test_request_user_id_prefers_non_empty_request_value() {
    let state = test_gateway_state("gateway-default", "gateway-actor", None);

    assert_eq!(request_user_id(&state, Some("family-1")).await, "family-1");
    assert_eq!(
        request_user_id(&state, Some("   ")).await,
        "gateway-default"
    );
    assert_eq!(request_user_id(&state, None).await, "gateway-default");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_request_user_id_infers_primary_gateway_principal_from_history() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("gateway-history.db");
    let backend = Arc::new(
        crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .unwrap(),
    );
    backend.run_migrations().await.unwrap();

    backend
        .create_conversation_with_metadata(
            "gateway",
            "default",
            &serde_json::json!({"thread_type": "thread"}),
        )
        .await
        .unwrap();

    for _ in 0..3 {
        backend
            .create_conversation_with_metadata(
                "gateway",
                "legacy-base-user",
                &serde_json::json!({"thread_type": "thread"}),
            )
            .await
            .unwrap();
    }

    let state = test_gateway_state("default", "default", Some(backend));

    let user_id = request_user_id(&state, None).await;
    assert_eq!(user_id, "legacy-base-user");
    assert_eq!(request_actor_id(&state, None, &user_id), "legacy-base-user");
}

#[tokio::test]
async fn test_request_user_id_prefers_configured_non_default_principal() {
    let state = test_gateway_state("configured-user", "configured-user", None);

    assert_eq!(request_user_id(&state, None).await, "configured-user");
    assert_eq!(
        request_actor_id(&state, None, "configured-user"),
        "configured-user"
    );
}

#[test]
fn test_request_actor_id_preserves_explicit_family_member_default() {
    let state = GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: "gateway-default".to_string(),
        actor_id: "gateway-actor".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        skill_remote_hub: None,
        skill_quarantine: None,
        chat_rate_limiter: RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        metrics_registry: None,
        response_cache: None,
        routine_engine: None,
        repo_project_supervisor: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        secrets_store: None,
        channel_manager: None,
        hooks: None,
    };

    assert_eq!(
        request_actor_id(&state, Some("family-2"), "gateway-default"),
        "family-2"
    );
    assert_eq!(
        request_actor_id(&state, None, "gateway-default"),
        "gateway-actor"
    );
}

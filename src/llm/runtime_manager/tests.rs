use std::sync::Arc;

use crate::config::Config;
use crate::llm::provider::{CompletionRequest, ToolCompletionRequest};
use crate::llm::routing_policy::RoutingRule;
use crate::settings::{
    ProviderModelSlots, ProvidersSettings, RoutingMode, SecretsMasterKeySource, Settings,
};

use super::manager::LlmRuntimeManager;
use super::provider_slots::provider_slot_selectors;
use super::settings_defaults::{normalize_providers_settings, validate_providers_settings};
use super::types::{ProviderModelRole, RuntimeLlmProvider};

async fn advisor_executor_test_config() -> Config {
    let mut settings = Settings {
        llm_backend: Some("openai_compatible".to_string()),
        openai_compatible_base_url: Some("http://localhost:12345/v1".to_string()),
        selected_model: Some("gpt-5.4".to_string()),
        ..Settings::default()
    };
    settings.secrets.master_key_source = SecretsMasterKeySource::None;

    Config::from_test_settings(&settings)
        .await
        .expect("config should load without touching the OS keychain")
}

#[test]
fn resolved_completion_request_clears_model_override() {
    let request = CompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")])
        .with_model("openai/gpt-5.4-mini");

    let resolved = RuntimeLlmProvider::resolved_completion_request(
        request,
        "test|openai|gpt-5.4-mini",
        "test",
    );

    assert!(resolved.model.is_none());
}

#[test]
fn resolved_tool_completion_request_clears_model_override() {
    let request = ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")], Vec::new())
        .with_model("openai/gpt-5.4-mini");

    let resolved = RuntimeLlmProvider::resolved_tool_completion_request(
        request,
        "test|openai|gpt-5.4-mini",
        "test",
    );

    assert!(resolved.model.is_none());
}

#[test]
fn normalize_promotes_legacy_models_into_provider_slots() {
    let mut settings = Settings {
        llm_backend: Some("openai".to_string()),
        selected_model: Some("gpt-4o".to_string()),
        ..Settings::default()
    };
    settings.providers.cheap_model = Some("openai/gpt-4o-mini".to_string());

    let providers = normalize_providers_settings(&settings);
    let openai = providers
        .provider_models
        .get("openai")
        .expect("openai slots should exist");

    assert_eq!(providers.primary.as_deref(), Some("openai"));
    assert_eq!(providers.primary_model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        providers.preferred_cheap_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(openai.primary.as_deref(), Some("gpt-4o"));
    assert_eq!(openai.cheap.as_deref(), Some("gpt-4o-mini"));
}

#[test]
fn provider_slot_selectors_prioritize_primary_and_preferred_cheap() {
    let mut providers = ProvidersSettings {
        enabled: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "gemini".to_string(),
        ],
        primary: Some("anthropic".to_string()),
        preferred_cheap_provider: Some("gemini".to_string()),
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "anthropic".to_string(),
        ProviderModelSlots {
            primary: Some("claude-opus-4-7".to_string()),
            cheap: Some("claude-sonnet-4-6".to_string()),
        },
    );
    providers.provider_models.insert(
        "openai".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );
    providers.provider_models.insert(
        "gemini".to_string(),
        ProviderModelSlots {
            primary: Some("gemini-2.5-flash".to_string()),
            cheap: Some("gemini-2.5-flash-lite".to_string()),
        },
    );

    let primary_targets = provider_slot_selectors(&providers, ProviderModelRole::Primary);
    let cheap_targets = provider_slot_selectors(&providers, ProviderModelRole::Cheap);

    assert_eq!(
        primary_targets.first().map(String::as_str),
        Some("anthropic@primary")
    );
    assert_eq!(
        cheap_targets.first().map(String::as_str),
        Some("gemini@cheap")
    );
    assert!(cheap_targets.iter().any(|target| target == "openai@cheap"));
}

#[test]
fn provider_slot_selectors_respect_explicit_pool_order() {
    let mut providers = ProvidersSettings {
        enabled: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "gemini".to_string(),
        ],
        primary: Some("anthropic".to_string()),
        preferred_cheap_provider: Some("gemini".to_string()),
        primary_pool_order: vec![
            "openai".to_string(),
            "anthropic".to_string(),
            "gemini".to_string(),
        ],
        cheap_pool_order: vec![
            "openai".to_string(),
            "gemini".to_string(),
            "anthropic".to_string(),
        ],
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "anthropic".to_string(),
        ProviderModelSlots {
            primary: Some("claude-opus-4-7".to_string()),
            cheap: Some("claude-sonnet-4-6".to_string()),
        },
    );
    providers.provider_models.insert(
        "openai".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );
    providers.provider_models.insert(
        "gemini".to_string(),
        ProviderModelSlots {
            primary: Some("gemini-2.5-flash".to_string()),
            cheap: Some("gemini-2.5-flash-lite".to_string()),
        },
    );

    let primary_targets = provider_slot_selectors(&providers, ProviderModelRole::Primary);
    let cheap_targets = provider_slot_selectors(&providers, ProviderModelRole::Cheap);

    assert_eq!(
        primary_targets,
        vec![
            "openai@primary".to_string(),
            "anthropic@primary".to_string(),
            "gemini@primary".to_string(),
        ]
    );
    assert_eq!(
        cheap_targets,
        vec![
            "openai@cheap".to_string(),
            "gemini@cheap".to_string(),
            "anthropic@cheap".to_string(),
        ]
    );
}

#[test]
fn normalize_populates_pool_orders_from_role_preferences() {
    let mut settings = Settings::default();
    settings.providers.enabled = vec!["openai".to_string(), "anthropic".to_string()];
    settings.providers.primary = Some("anthropic".to_string());
    settings.providers.preferred_cheap_provider = Some("openai".to_string());
    settings.providers.provider_models.insert(
        "openai".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );
    settings.providers.provider_models.insert(
        "anthropic".to_string(),
        ProviderModelSlots {
            primary: Some("claude-opus-4-7".to_string()),
            cheap: Some("claude-sonnet-4-6".to_string()),
        },
    );

    let providers = normalize_providers_settings(&settings);

    assert_eq!(
        providers.primary_pool_order,
        vec!["anthropic".to_string(), "openai".to_string()]
    );
    assert_eq!(
        providers.cheap_pool_order,
        vec!["openai".to_string(), "anthropic".to_string()]
    );
}

#[test]
fn provider_models_do_not_auto_enable_disabled_providers() {
    let mut settings = Settings::default();
    settings.providers.provider_models.insert(
        "openai".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );

    let providers = normalize_providers_settings(&settings);

    assert!(providers.enabled.is_empty());
    assert_eq!(
        providers
            .provider_models
            .get("openai")
            .and_then(|slots| slots.primary.as_deref()),
        Some("gpt-4o")
    );
    assert_eq!(
        providers
            .provider_models
            .get("openai")
            .and_then(|slots| slots.cheap.as_deref()),
        Some("gpt-4o-mini")
    );
}

#[test]
fn validate_flags_unresolvable_policy_targets() {
    let mut providers = ProvidersSettings {
        enabled: vec!["openai".to_string()],
        routing_mode: RoutingMode::Policy,
        ..ProvidersSettings::default()
    };
    providers.policy_rules = vec![RoutingRule::VisionContent {
        provider: "anthropic@primary".to_string(),
    }];

    let diagnostics = validate_providers_settings(&providers);
    assert!(
        diagnostics
            .iter()
            .any(|entry| entry.contains("cannot be resolved")),
        "expected unresolved policy target diagnostic, got: {:?}",
        diagnostics
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn advisor_target_primary_resolves_to_primary_provider_in_advisor_executor() {
    let config = advisor_executor_test_config().await;

    let mut providers = ProvidersSettings {
        enabled: vec!["openai_compatible".to_string()],
        primary: Some("openai_compatible".to_string()),
        primary_model: Some("gpt-5.4".to_string()),
        cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
        smart_routing_enabled: true,
        routing_mode: RoutingMode::AdvisorExecutor,
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "openai_compatible".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-5.4".to_string()),
            cheap: Some("gpt-5.4-mini".to_string()),
        },
    );

    let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
        .expect("runtime manager should build");

    let provider = manager
        .provider_handle_for_target("primary")
        .expect("primary advisor target should resolve");

    assert_eq!(provider.active_model_name(), "gpt-5.4");
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn advisor_executor_status_reports_readiness_and_targets() {
    let config = advisor_executor_test_config().await;

    let mut providers = ProvidersSettings {
        enabled: vec!["openai_compatible".to_string()],
        primary: Some("openai_compatible".to_string()),
        primary_model: Some("gpt-5.4".to_string()),
        cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
        smart_routing_enabled: true,
        routing_mode: RoutingMode::AdvisorExecutor,
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "openai_compatible".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-5.4".to_string()),
            cheap: Some("gpt-5.4-mini".to_string()),
        },
    );

    let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
        .expect("runtime manager should build");
    let status = manager.status();

    assert!(status.advisor_ready);
    assert_eq!(
        status.executor_target.as_deref(),
        Some("openai_compatible/gpt-5.4-mini")
    );
    assert_eq!(
        status.advisor_target.as_deref(),
        Some("openai_compatible/gpt-5.4")
    );
    assert_eq!(
        status.advisor_auto_escalation_mode,
        crate::settings::AdvisorAutoEscalationMode::RiskAndComplexFinal
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn advisor_ready_callback_reports_current_readiness_immediately() {
    let config = advisor_executor_test_config().await;

    let mut providers = ProvidersSettings {
        enabled: vec!["openai_compatible".to_string()],
        primary: Some("openai_compatible".to_string()),
        primary_model: Some("gpt-5.4".to_string()),
        cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
        smart_routing_enabled: true,
        routing_mode: RoutingMode::AdvisorExecutor,
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "openai_compatible".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-5.4".to_string()),
            cheap: Some("gpt-5.4-mini".to_string()),
        },
    );

    let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
        .expect("runtime manager should build");
    let seen = Arc::new(std::sync::Mutex::new(None));
    let seen_for_callback = Arc::clone(&seen);

    manager.set_advisor_ready_callback(move |advisor_ready| {
        *seen_for_callback.lock().expect("callback lock") = Some(advisor_ready);
    });

    assert_eq!(*seen.lock().expect("callback lock"), Some(true));
}

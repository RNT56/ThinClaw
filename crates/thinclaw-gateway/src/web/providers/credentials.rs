//! Provider credentials, identity, auth mode, and settings mutation/apply.

use super::*;
use std::collections::HashSet;
use thinclaw_llm_core::RoutingRule;
use thinclaw_settings::{
    AdvisorAutoEscalationMode, ProviderCredentialMode, ProviderModelSlots, ProvidersSettings,
    RoutingMode, Settings,
};

pub const PROVIDERS_ENABLED_SETTING_KEY: &str = "providers.enabled";
pub const PROVIDERS_FALLBACK_CHAIN_SETTING_KEY: &str = "providers.fallback_chain";

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

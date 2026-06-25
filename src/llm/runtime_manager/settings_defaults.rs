//! Settings normalization: deriving the runtime `ProvidersSettings` shape from
//! raw user `Settings` (filling provider slots, pool orders, cheap/fallback
//! defaults) and validating provider configuration.

use std::collections::BTreeSet;

use crate::settings::{ProvidersSettings, RoutingMode, Settings};

use super::provider_slots::{
    default_model_for_runtime_slug, normalized_provider_pool_order, parse_provider_slot_selector,
    preferred_cheap_target, provider_role_target, provider_slot_selector, provider_slot_selectors,
    route_target_known_cost_per_m_usd, suggest_provider_cheap_model,
};
use crate::llm::route_planner::validate_providers_settings as validate_planner_settings;
use crate::llm::routing_policy::{RoutingRule, policy_rule_targets};

use super::types::ProviderModelRole;

pub fn validate_providers_settings(raw: &ProvidersSettings) -> Vec<String> {
    let mut diagnostics = validate_planner_settings(raw);

    if raw.smart_routing_enabled && raw.enabled.is_empty() {
        diagnostics.push(
            "smart_routing_enabled is true but no providers are enabled; runtime will fall back to legacy backend".to_string(),
        );
    }
    if raw.routing_mode == RoutingMode::CheapSplit
        && raw.cheap_model.is_none()
        && raw.preferred_cheap_provider.is_none()
    {
        diagnostics.push(
            "CheapSplit mode is enabled but cheap model is not explicitly configured; runtime defaults will infer one when possible".to_string(),
        );
    }
    if raw.routing_mode == RoutingMode::Policy {
        for target in policy_rule_targets(&raw.policy_rules) {
            if !route_target_resolves_in_settings(raw, &target) {
                diagnostics.push(format!(
                    "Policy target '{}' cannot be resolved with current provider configuration",
                    target
                ));
            }
        }
    }

    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

pub fn derive_runtime_defaults(settings: &Settings) -> ProvidersSettings {
    derive_runtime_defaults_from_parts(
        settings.providers.clone(),
        legacy_primary_slug(settings),
        settings.selected_model.clone(),
    )
}

/// Backward-compatible alias. New code should prefer `derive_runtime_defaults`.
pub fn normalize_providers_settings(settings: &Settings) -> ProvidersSettings {
    derive_runtime_defaults(settings)
}

pub(super) fn derive_runtime_defaults_from_parts(
    mut providers: ProvidersSettings,
    legacy_primary: Option<String>,
    legacy_model: Option<String>,
) -> ProvidersSettings {
    if providers.primary.is_none() {
        providers.primary = legacy_primary;
    }
    if providers.primary_model.is_none() {
        providers.primary_model = legacy_model;
    }
    if let Some(primary_slug) = providers.primary.clone() {
        let slots = providers.provider_models.entry(primary_slug).or_default();
        if slots.primary.is_none() {
            slots.primary = providers.primary_model.clone();
        }
    }
    if let Some(spec) = providers.cheap_model.clone()
        && let Some((slug, model)) = spec.split_once('/')
    {
        let slots = providers
            .provider_models
            .entry(slug.to_string())
            .or_default();
        if slots.cheap.is_none() {
            slots.cheap = Some(model.to_string());
        }
        if providers.preferred_cheap_provider.is_none() {
            providers.preferred_cheap_provider = Some(slug.to_string());
        }
    }
    for (slug, allowed) in &providers.allowed_models {
        if let Some(model) = allowed.first() {
            let slots = providers.provider_models.entry(slug.clone()).or_default();
            if slots.primary.is_none() {
                slots.primary = Some(model.clone());
            }
        }
    }

    let mut enabled = BTreeSet::new();
    for provider in &providers.enabled {
        enabled.insert(provider.clone());
    }
    if let Some(primary) = providers.primary.as_ref() {
        enabled.insert(primary.clone());
    }
    if let Some((slug, _)) = providers
        .cheap_model
        .as_deref()
        .and_then(|spec| spec.split_once('/'))
    {
        enabled.insert(slug.to_string());
    }
    if let Some(preferred_cheap_provider) = providers.preferred_cheap_provider.as_ref() {
        enabled.insert(preferred_cheap_provider.clone());
    }
    for entry in &providers.fallback_chain {
        if let Some((slug, _)) = entry.split_once('/') {
            enabled.insert(slug.to_string());
        } else if let Some((slug, _)) = parse_provider_slot_selector(entry) {
            enabled.insert(slug.to_string());
        }
    }
    for slug in providers.allowed_models.keys() {
        enabled.insert(slug.clone());
    }
    providers.enabled = enabled.into_iter().collect();

    if providers.primary.is_none() {
        providers.primary = providers
            .primary_pool_order
            .iter()
            .find(|slug| providers.enabled.iter().any(|enabled| enabled == *slug))
            .cloned()
            .or_else(|| providers.enabled.first().cloned());
    }

    let enabled_snapshot = providers.enabled.clone();
    for slug in enabled_snapshot {
        let slots = providers.provider_models.entry(slug.clone()).or_default();

        if slots.primary.is_none() {
            slots.primary = if providers.primary.as_deref() == Some(slug.as_str()) {
                providers.primary_model.clone()
            } else {
                providers
                    .allowed_models
                    .get(&slug)
                    .and_then(|models| models.first().cloned())
            }
            .or_else(|| default_model_for_runtime_slug(&slug).map(ToOwned::to_owned));
        }

        if slots.cheap.is_none() {
            slots.cheap = suggest_provider_cheap_model(&slug, slots.primary.as_deref())
                .or_else(|| slots.primary.clone());
        }
    }

    if let Some(primary_slug) = providers.primary.as_deref() {
        providers.primary_model = providers
            .provider_models
            .get(primary_slug)
            .and_then(|slots| slots.primary.clone())
            .or_else(|| providers.primary_model.clone());
    }

    if providers.preferred_cheap_provider.is_none() {
        providers.preferred_cheap_provider = providers
            .cheap_pool_order
            .iter()
            .find(|slug| {
                providers.enabled.iter().any(|enabled| enabled == *slug)
                    && provider_role_target(&providers, slug, ProviderModelRole::Cheap).is_some()
            })
            .cloned()
            .or_else(|| providers.primary.clone())
            .or_else(|| {
                provider_slot_selectors(&providers, ProviderModelRole::Cheap)
                    .into_iter()
                    .next()
                    .and_then(|selector| {
                        parse_provider_slot_selector(&selector).map(|(slug, _)| slug.to_string())
                    })
            });
    }

    providers.primary_pool_order =
        normalized_provider_pool_order(&providers, ProviderModelRole::Primary);
    providers.cheap_pool_order =
        normalized_provider_pool_order(&providers, ProviderModelRole::Cheap);

    if providers.smart_routing_enabled
        && providers.routing_mode == RoutingMode::PrimaryOnly
        && (!provider_slot_selectors(&providers, ProviderModelRole::Cheap).is_empty()
            || providers.enabled.len() > 1)
    {
        providers.routing_mode = RoutingMode::CheapSplit;
    }

    providers.cheap_model = preferred_cheap_target(&providers).or_else(|| {
        suggest_cheap_model(
            providers.primary.as_deref(),
            providers.primary_model.as_deref(),
            &providers.enabled,
        )
    });

    if providers.fallback_chain.is_empty() {
        providers.fallback_chain = providers
            .enabled
            .iter()
            .filter(|slug| providers.primary.as_deref() != Some(slug.as_str()))
            .map(|slug| provider_slot_selector(slug, ProviderModelRole::Primary))
            .collect();
    }

    if providers.routing_mode == RoutingMode::Policy && providers.policy_rules.is_empty() {
        providers.policy_rules = vec![
            RoutingRule::VisionContent {
                provider: "primary".to_string(),
            },
            RoutingRule::LargeContext {
                threshold: 120_000,
                provider: "primary".to_string(),
            },
        ];
    }

    providers
}

fn legacy_primary_slug(settings: &Settings) -> Option<String> {
    match settings.llm_backend.as_deref() {
        Some("openai") => Some("openai".to_string()),
        Some("anthropic") => Some("anthropic".to_string()),
        Some("ollama") => Some("ollama".to_string()),
        Some("openai_compatible") => settings
            .openai_compatible_base_url
            .as_deref()
            .and_then(|url| {
                if url.contains("openrouter.ai") {
                    Some("openrouter".to_string())
                } else {
                    None
                }
            })
            .or_else(|| Some("openai_compatible".to_string())),
        Some("tinfoil") => Some("tinfoil".to_string()),
        Some("gemini") => Some("gemini".to_string()),
        Some("bedrock") => Some("bedrock".to_string()),
        Some("llama_cpp") => Some("llama_cpp".to_string()),
        _ => None,
    }
}

fn suggest_cheap_model(
    primary: Option<&str>,
    primary_model: Option<&str>,
    enabled: &[String],
) -> Option<String> {
    if let Some(primary_slug) = primary
        && let Some(candidate_model) = suggest_provider_cheap_model(primary_slug, primary_model)
        && primary_model != Some(candidate_model.as_str())
    {
        let candidate = format!("{primary_slug}/{candidate_model}");
        return Some(candidate);
    }

    enabled
        .iter()
        .filter(|slug| Some(slug.as_str()) != primary)
        .filter_map(|slug| {
            suggest_provider_cheap_model(slug, None)
                .or_else(|| default_model_for_runtime_slug(slug).map(ToOwned::to_owned))
                .map(|model| format!("{slug}/{model}"))
        })
        .min_by(|a, b| {
            route_target_known_cost_per_m_usd(a)
                .partial_cmp(&route_target_known_cost_per_m_usd(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn route_target_resolves_in_settings(settings: &ProvidersSettings, target: &str) -> bool {
    match target {
        "primary" => settings.primary.is_some() || !settings.enabled.is_empty(),
        "cheap" => {
            settings.cheap_model.is_some()
                || settings.preferred_cheap_provider.is_some()
                || settings.primary.is_some()
                || !settings.enabled.is_empty()
        }
        other if parse_provider_slot_selector(other).is_some() => {
            let (slug, role) = parse_provider_slot_selector(other)
                .expect("slot selector checked above for route_target_resolves_in_settings");
            provider_declared_for_routing(settings, slug)
                && provider_role_target(settings, slug, role).is_some()
        }
        other => {
            if let Some((slug, _)) = other.split_once('/') {
                provider_declared_for_routing(settings, slug)
            } else {
                false
            }
        }
    }
}

fn provider_declared_for_routing(settings: &ProvidersSettings, slug: &str) -> bool {
    if settings.enabled.iter().any(|entry| entry == slug)
        || settings.primary.as_deref() == Some(slug)
        || settings.preferred_cheap_provider.as_deref() == Some(slug)
        || settings.allowed_models.contains_key(slug)
    {
        return true;
    }

    if settings
        .cheap_model
        .as_deref()
        .and_then(|spec| spec.split_once('/'))
        .is_some_and(|(cheap_slug, _)| cheap_slug == slug)
    {
        return true;
    }

    settings.fallback_chain.iter().any(|target| {
        target
            .split_once('/')
            .map(|(fallback_slug, _)| fallback_slug == slug)
            .or_else(|| {
                parse_provider_slot_selector(target).map(|(fallback_slug, _)| fallback_slug == slug)
            })
            .unwrap_or(false)
    })
}

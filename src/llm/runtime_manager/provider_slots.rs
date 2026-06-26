//! Provider-slot algebra: resolving provider/role selectors to concrete model
//! specs, ordering provider pools by cost, and cost lookups.
//!
//! These free functions are shared across the manager, routing, and
//! settings-defaults submodules. They stay `pub(super)` and are re-exported at
//! the façade root for sibling `use super::*` access.

use rust_decimal::prelude::ToPrimitive;

use crate::settings::ProvidersSettings;

use super::types::ProviderModelRole;

pub(super) fn default_model_for_runtime_slug(slug: &str) -> Option<&'static str> {
    crate::config::provider_catalog::endpoint_for(slug)
        .map(|endpoint| endpoint.default_model.as_str())
        .or(match slug {
            "ollama" => Some("llama3"),
            "openai_compatible" => Some("default"),
            "bedrock" => Some("anthropic.claude-3-sonnet-20240229-v1:0"),
            "llama_cpp" => Some("llama-local"),
            _ => None,
        })
}

pub(super) fn provider_slot_selector(slug: &str, role: ProviderModelRole) -> String {
    format!("{slug}@{}", role.as_str())
}

pub(super) fn parse_provider_slot_selector(selector: &str) -> Option<(&str, ProviderModelRole)> {
    if let Some(slug) = selector.strip_suffix("@primary") {
        return Some((slug, ProviderModelRole::Primary));
    }
    if let Some(slug) = selector.strip_suffix("@cheap") {
        return Some((slug, ProviderModelRole::Cheap));
    }
    None
}

pub(super) fn suggest_provider_cheap_model(
    slug: &str,
    primary_model: Option<&str>,
) -> Option<String> {
    let mapped = match slug {
        "openai" => Some("gpt-4o-mini"),
        "anthropic" => Some("claude-sonnet-4-6"),
        "gemini" => Some("gemini-2.5-flash-lite"),
        "openrouter" => Some("openai/gpt-4o-mini"),
        "tinfoil" => Some("kimi-k2-5"),
        _ => None,
    };

    if let Some(candidate) = mapped
        && primary_model != Some(candidate)
    {
        return Some(candidate.to_string());
    }

    default_model_for_runtime_slug(slug)
        .map(ToOwned::to_owned)
        .filter(|model| primary_model != Some(model.as_str()))
}

pub(super) fn provider_role_target(
    settings: &ProvidersSettings,
    slug: &str,
    role: ProviderModelRole,
) -> Option<String> {
    let slots = settings.provider_models.get(slug);
    let model = match role {
        ProviderModelRole::Primary => slots
            .and_then(|entry| entry.primary.clone())
            .or_else(|| {
                if settings.primary.as_deref() == Some(slug) {
                    settings.primary_model.clone()
                } else {
                    settings
                        .allowed_models
                        .get(slug)
                        .and_then(|models| models.first().cloned())
                }
            })
            .or_else(|| default_model_for_runtime_slug(slug).map(ToOwned::to_owned)),
        ProviderModelRole::Cheap => slots
            .and_then(|entry| entry.cheap.clone())
            .or_else(|| {
                settings
                    .cheap_model
                    .as_deref()
                    .and_then(|spec| spec.split_once('/'))
                    .and_then(|(cheap_slug, model)| {
                        if cheap_slug == slug {
                            Some(model.to_string())
                        } else {
                            None
                        }
                    })
            })
            .or_else(|| {
                provider_role_target(settings, slug, ProviderModelRole::Primary)
                    .and_then(|target| target.split_once('/').map(|(_, model)| model.to_string()))
            })
            .or_else(|| {
                suggest_provider_cheap_model(
                    slug,
                    provider_role_target(settings, slug, ProviderModelRole::Primary)
                        .as_deref()
                        .and_then(|target| target.split_once('/').map(|(_, model)| model)),
                )
            }),
    }?;

    Some(format!("{slug}/{model}"))
}

pub(super) fn provider_slot_selectors(
    settings: &ProvidersSettings,
    role: ProviderModelRole,
) -> Vec<String> {
    let mut selectors = Vec::new();
    let push = |slug: &str, selectors: &mut Vec<String>| {
        if provider_role_target(settings, slug, role).is_some() {
            let selector = provider_slot_selector(slug, role);
            if !selectors.contains(&selector) {
                selectors.push(selector);
            }
        }
    };

    match role {
        ProviderModelRole::Primary => {
            for slug in &settings.primary_pool_order {
                push(slug, &mut selectors);
            }
            for slug in &settings.enabled {
                if !settings
                    .primary_pool_order
                    .iter()
                    .any(|ordered| ordered == slug)
                {
                    push(slug, &mut selectors);
                }
            }
        }
        ProviderModelRole::Cheap => {
            for slug in &settings.cheap_pool_order {
                push(slug, &mut selectors);
            }
            let mut remaining: Vec<(String, String)> = settings
                .enabled
                .iter()
                .filter(|slug| {
                    !settings
                        .cheap_pool_order
                        .iter()
                        .any(|ordered| ordered == *slug)
                })
                .filter_map(|slug| {
                    provider_role_target(settings, slug, role).map(|target| (slug.clone(), target))
                })
                .collect();
            remaining.sort_by(|a, b| {
                route_target_known_cost_per_m_usd(&a.1)
                    .partial_cmp(&route_target_known_cost_per_m_usd(&b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (slug, _) in remaining {
                push(&slug, &mut selectors);
            }
        }
    }

    selectors
}

pub(super) fn normalized_provider_pool_order(
    settings: &ProvidersSettings,
    role: ProviderModelRole,
) -> Vec<String> {
    let mut order = Vec::new();
    let push = |slug: &str, order: &mut Vec<String>| {
        if settings.enabled.iter().any(|enabled| enabled == slug)
            && provider_role_target(settings, slug, role).is_some()
            && !order.iter().any(|existing| existing == slug)
        {
            order.push(slug.to_string());
        }
    };

    match role {
        ProviderModelRole::Primary => {
            if let Some(primary_slug) = settings.primary.as_deref() {
                push(primary_slug, &mut order);
            }
            for slug in &settings.primary_pool_order {
                push(slug, &mut order);
            }
            for slug in &settings.enabled {
                push(slug, &mut order);
            }
        }
        ProviderModelRole::Cheap => {
            if let Some(preferred_slug) = settings.preferred_cheap_provider.as_deref() {
                push(preferred_slug, &mut order);
            }
            for slug in &settings.cheap_pool_order {
                push(slug, &mut order);
            }
            let mut remaining: Vec<(String, String)> = settings
                .enabled
                .iter()
                .filter(|slug| !order.iter().any(|existing| existing == *slug))
                .filter_map(|slug| {
                    provider_role_target(settings, slug, role).map(|target| (slug.clone(), target))
                })
                .collect();
            remaining.sort_by(|a, b| {
                route_target_known_cost_per_m_usd(&a.1)
                    .partial_cmp(&route_target_known_cost_per_m_usd(&b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (slug, _) in remaining {
                push(&slug, &mut order);
            }
        }
    }

    order
}

pub(super) fn preferred_cheap_target(settings: &ProvidersSettings) -> Option<String> {
    if let Some(slug) = settings.preferred_cheap_provider.as_deref() {
        return provider_role_target(settings, slug, ProviderModelRole::Cheap);
    }
    provider_slot_selectors(settings, ProviderModelRole::Cheap)
        .into_iter()
        .next()
        .and_then(|selector| {
            parse_provider_slot_selector(&selector).map(|(slug, role)| (slug.to_string(), role))
        })
        .and_then(|(slug, role)| provider_role_target(settings, &slug, role))
}

pub(super) fn route_target_known_cost_per_m_usd(target: &str) -> f64 {
    if matches!(target, "primary" | "cheap") {
        let (input, output) = crate::llm::costs::default_cost();
        return ((input + output) * rust_decimal::Decimal::from(1_000_000u64))
            .to_f64()
            .unwrap_or(f64::MAX);
    }

    if let Some((slug, role)) = parse_provider_slot_selector(target)
        && let Some(concrete_target) = provider_role_target(
            &ProvidersSettings {
                enabled: vec![slug.to_string()],
                primary: Some(slug.to_string()),
                preferred_cheap_provider: Some(slug.to_string()),
                ..ProvidersSettings::default()
            },
            slug,
            role,
        )
    {
        return route_target_known_cost_per_m_usd(&concrete_target);
    }

    let (input, output) = crate::llm::costs::model_cost(target).unwrap_or_else(|| {
        if target.starts_with("ollama/") || target.starts_with("llama_cpp/") {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        } else {
            crate::llm::costs::default_cost()
        }
    });
    ((input + output) * rust_decimal::Decimal::from(1_000_000u64))
        .to_f64()
        .unwrap_or(f64::MAX)
}

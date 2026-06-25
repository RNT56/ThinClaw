//! Runtime credential hydration: pulling provider API keys out of the
//! encrypted secrets store into the in-memory `Config`/`ProvidersSettings`
//! before provider chains are built.

use std::collections::BTreeSet;
use std::sync::Arc;

use secrecy::SecretString;

use crate::config::{Config, LlmBackend};
use crate::llm::routing_policy::policy_rule_targets;
use crate::secrets::SecretsStore;
use crate::settings::ProvidersSettings;

use super::provider_slots::parse_provider_slot_selector;

pub async fn hydrate_runtime_credentials_from_secrets(
    config: &mut Config,
    providers: &mut ProvidersSettings,
    secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
    user_id: &str,
) -> usize {
    let mut loaded = hydrate_llm_config_api_keys_from_secrets(config, secrets_store, user_id).await;
    loaded += hydrate_provider_api_keys_from_secrets(providers, secrets_store, user_id).await;
    if loaded > 0 {
        tracing::info!(
            loaded,
            "Hydrated scoped LLM/provider credentials from encrypted secrets store"
        );
    }
    loaded
}

async fn runtime_secret_from_store(
    secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
    user_id: &str,
    names: &[&str],
    caller: &str,
    purpose: &str,
    target_host: &str,
) -> Option<SecretString> {
    let store = secrets_store?;
    for name in names {
        let secret_name = name.trim();
        if secret_name.is_empty() {
            continue;
        }
        match store
            .get_for_injection(
                user_id,
                secret_name,
                crate::secrets::SecretAccessContext::new(caller, purpose)
                    .target(target_host, secret_name),
            )
            .await
        {
            Ok(secret) => {
                let value = secret.expose().trim().to_string();
                if !value.is_empty() {
                    return Some(SecretString::from(value));
                }
            }
            Err(crate::secrets::SecretError::NotFound(_)) => {}
            Err(crate::secrets::SecretError::LegacySecret(reason)) => {
                tracing::warn!(
                    secret = %secret_name,
                    reason = %reason,
                    "Legacy secret cannot be used for runtime credential hydration"
                );
            }
            Err(err) => {
                tracing::warn!(
                    secret = %secret_name,
                    error = %err,
                    "Failed to hydrate runtime credential from encrypted store"
                );
            }
        }
    }
    None
}

fn replace_secret_keys(
    primary: &mut Option<SecretString>,
    all: &mut Vec<SecretString>,
    value: SecretString,
) {
    *primary = Some(value.clone());
    *all = vec![value];
}

async fn hydrate_llm_config_api_keys_from_secrets(
    config: &mut Config,
    secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
    user_id: &str,
) -> usize {
    let Some(_) = secrets_store else {
        return 0;
    };

    match config.llm.backend {
        LlmBackend::OpenAi => {
            let Some(openai) = config.llm.openai.as_mut() else {
                return 0;
            };
            let Some(endpoint) = crate::config::provider_catalog::endpoint_for("openai") else {
                return 0;
            };
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &[endpoint.secret_name.as_str(), "openai"],
                "llm.runtime_manager",
                "direct_provider_credential",
                "openai",
            )
            .await
            {
                replace_secret_keys(&mut openai.api_key, &mut openai.api_keys, value);
                1
            } else {
                0
            }
        }
        LlmBackend::Anthropic => {
            let Some(anthropic) = config.llm.anthropic.as_mut() else {
                return 0;
            };
            let Some(endpoint) = crate::config::provider_catalog::endpoint_for("anthropic") else {
                return 0;
            };
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &[endpoint.secret_name.as_str(), "anthropic"],
                "llm.runtime_manager",
                "direct_provider_credential",
                "anthropic",
            )
            .await
            {
                replace_secret_keys(&mut anthropic.api_key, &mut anthropic.api_keys, value);
                1
            } else {
                0
            }
        }
        LlmBackend::OpenAiCompatible => {
            let Some(compat) = config.llm.openai_compatible.as_mut() else {
                return 0;
            };
            let (target_host, names): (&str, Vec<&str>) =
                if compat.base_url.contains("openrouter.ai") {
                    if let Some(endpoint) =
                        crate::config::provider_catalog::endpoint_for("openrouter")
                    {
                        (
                            "openrouter",
                            vec![endpoint.secret_name.as_str(), "openrouter"],
                        )
                    } else {
                        ("openrouter", vec!["openrouter"])
                    }
                } else {
                    (
                        "openai_compatible",
                        vec!["llm_compatible_api_key", "openai_compatible"],
                    )
                };
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &names,
                "llm.runtime_manager",
                "direct_provider_credential",
                target_host,
            )
            .await
            {
                replace_secret_keys(&mut compat.api_key, &mut compat.api_keys, value);
                1
            } else {
                0
            }
        }
        LlmBackend::Tinfoil => {
            let Some(tinfoil) = config.llm.tinfoil.as_mut() else {
                return 0;
            };
            let names = crate::config::provider_catalog::endpoint_for("tinfoil")
                .map(|endpoint| vec![endpoint.secret_name.as_str(), "tinfoil"])
                .unwrap_or_else(|| vec!["tinfoil"]);
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &names,
                "llm.runtime_manager",
                "direct_provider_credential",
                "tinfoil",
            )
            .await
            {
                replace_secret_keys(&mut tinfoil.api_key, &mut tinfoil.api_keys, value);
                1
            } else {
                0
            }
        }
        LlmBackend::Gemini => {
            let Some(gemini) = config.llm.gemini.as_mut() else {
                return 0;
            };
            let names = crate::config::provider_catalog::endpoint_for("gemini")
                .map(|endpoint| vec![endpoint.secret_name.as_str(), "gemini"])
                .unwrap_or_else(|| vec!["gemini"]);
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &names,
                "llm.runtime_manager",
                "direct_provider_credential",
                "gemini",
            )
            .await
            {
                replace_secret_keys(&mut gemini.api_key, &mut gemini.api_keys, value);
                1
            } else {
                0
            }
        }
        LlmBackend::Bedrock => {
            let Some(bedrock) = config.llm.bedrock.as_mut() else {
                return 0;
            };
            let mut loaded = 0;
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &["llm_bedrock_api_key", "bedrock"],
                "llm.runtime_manager",
                "direct_provider_credential",
                "bedrock",
            )
            .await
            {
                replace_secret_keys(&mut bedrock.api_key, &mut bedrock.api_keys, value);
                loaded += 1;
            }
            if let Some(value) = runtime_secret_from_store(
                secrets_store,
                user_id,
                &["llm_bedrock_proxy_api_key", "bedrock_proxy"],
                "llm.runtime_manager",
                "direct_provider_proxy_credential",
                "bedrock",
            )
            .await
            {
                bedrock.proxy_api_key = Some(value);
                loaded += 1;
            }
            loaded
        }
        LlmBackend::Ollama | LlmBackend::LlamaCpp => 0,
    }
}

async fn hydrate_provider_api_keys_from_secrets(
    providers: &mut ProvidersSettings,
    secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
    user_id: &str,
) -> usize {
    providers.resolved_provider_api_keys.clear();
    let Some(_) = secrets_store else {
        return 0;
    };

    let slugs = provider_slugs_for_secret_hydration(providers);
    let mut loaded = 0;
    for slug in slugs {
        let Some(endpoint) = crate::config::provider_catalog::endpoint_for(&slug) else {
            continue;
        };
        if let Some(value) = runtime_secret_from_store(
            secrets_store,
            user_id,
            &[endpoint.secret_name.as_str(), slug.as_str()],
            "llm.runtime_manager",
            "catalog_provider_credential",
            &slug,
        )
        .await
        {
            providers
                .resolved_provider_api_keys
                .insert(slug.clone(), vec![value]);
            loaded += 1;
        }
    }
    loaded
}

fn provider_slugs_for_secret_hydration(providers: &ProvidersSettings) -> BTreeSet<String> {
    let mut slugs = BTreeSet::new();

    if let Some(primary) = providers.primary.as_deref() {
        push_provider_slug(&mut slugs, primary);
    }
    if let Some(preferred) = providers.preferred_cheap_provider.as_deref() {
        push_provider_slug(&mut slugs, preferred);
    }
    for slug in &providers.enabled {
        push_provider_slug(&mut slugs, slug);
    }
    for slug in &providers.primary_pool_order {
        push_provider_slug(&mut slugs, slug);
    }
    for slug in &providers.cheap_pool_order {
        push_provider_slug(&mut slugs, slug);
    }
    for slug in providers.provider_models.keys() {
        push_provider_slug(&mut slugs, slug);
    }
    for slug in providers.allowed_models.keys() {
        push_provider_slug(&mut slugs, slug);
    }
    for slug in providers.provider_credential_modes.keys() {
        push_provider_slug(&mut slugs, slug);
    }
    if let Some(spec) = providers.cheap_model.as_deref() {
        push_provider_slug_from_target(&mut slugs, spec);
    }
    for target in &providers.fallback_chain {
        push_provider_slug_from_target(&mut slugs, target);
    }
    for target in policy_rule_targets(&providers.policy_rules) {
        push_provider_slug_from_target(&mut slugs, &target);
    }

    slugs
}

fn push_provider_slug_from_target(slugs: &mut BTreeSet<String>, target: &str) {
    let target = target.trim();
    if matches!(target, "" | "primary" | "cheap") {
        return;
    }
    if let Some((slug, _)) = target.split_once('/') {
        push_provider_slug(slugs, slug);
        return;
    }
    if let Some((slug, _)) = parse_provider_slot_selector(target) {
        push_provider_slug(slugs, slug);
        return;
    }
    push_provider_slug(slugs, target);
}

fn push_provider_slug(slugs: &mut BTreeSet<String>, slug: &str) {
    let slug = slug.trim();
    if !slug.is_empty() && crate::config::provider_catalog::endpoint_for(slug).is_some() {
        slugs.insert(slug.to_string());
    }
}

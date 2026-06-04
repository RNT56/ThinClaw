use std::collections::HashMap;

use serde_json::{Map, Value};

fn clean_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn normalize_provider_slug(provider: &str) -> String {
    match provider.trim() {
        "amazon-bedrock" => "bedrock".to_string(),
        "custom" | "openai-compatible" => "openai_compatible".to_string(),
        "google" => "gemini".to_string(),
        other => other.to_string(),
    }
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn set_string_or_null(target: &mut Map<String, Value>, key: &str, value: Option<String>) {
    target.insert(
        key.to_string(),
        value.map(Value::String).unwrap_or(Value::Null),
    );
}

fn set_array(target: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    target.insert(
        key.to_string(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
}

fn unique_push(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn enabled_provider_order(enabled_providers: &[String], custom_enabled: bool) -> Vec<String> {
    let mut enabled = Vec::new();
    for provider in enabled_providers {
        unique_push(&mut enabled, normalize_provider_slug(provider));
    }
    if custom_enabled {
        unique_push(&mut enabled, "openai_compatible".to_string());
    }
    enabled
}

fn first_enabled_model(
    enabled_models: &HashMap<String, Vec<String>>,
    provider_slug: &str,
) -> Option<String> {
    let keys: Vec<&str> = match provider_slug {
        "bedrock" => vec!["bedrock", "amazon-bedrock"],
        "gemini" => vec!["gemini", "google"],
        "openai_compatible" => vec!["openai_compatible", "openai-compatible", "custom"],
        other => vec![other],
    };

    keys.iter().find_map(|key| {
        enabled_models.get(*key).and_then(|models| {
            models
                .iter()
                .map(String::as_str)
                .find_map(|model| clean_string(Some(model)))
        })
    })
}

fn provider_model_from_entry(provider: &Value) -> Option<String> {
    value_string(provider, "primary_model")
        .or_else(|| value_string(provider, "suggested_primary_model"))
        .or_else(|| value_string(provider, "default_model"))
}

fn provider_enabled(provider: &Value) -> bool {
    provider
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn current_primary_provider(config: &Value) -> Option<String> {
    value_string(config, "primary_provider").map(|slug| normalize_provider_slug(&slug))
}

fn current_preferred_cheap_provider(config: &Value) -> Option<String> {
    value_string(config, "preferred_cheap_provider").map(|slug| normalize_provider_slug(&slug))
}

fn primary_model_for_provider(
    config: &Value,
    provider_slug: &str,
    enabled_models: &HashMap<String, Vec<String>>,
) -> Option<String> {
    first_enabled_model(enabled_models, provider_slug).or_else(|| {
        config
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|providers| {
                providers.iter().find_map(|provider| {
                    let slug = value_string(provider, "slug")?;
                    (normalize_provider_slug(&slug) == provider_slug)
                        .then(|| provider_model_from_entry(provider))
                        .flatten()
                })
            })
    })
}

fn filtered_provider_order(current: Option<&Value>, enabled: &[String]) -> Vec<String> {
    let mut order = Vec::new();
    if let Some(values) = current.and_then(Value::as_array) {
        for value in values {
            if let Some(slug) = value.as_str().map(normalize_provider_slug) {
                if enabled.iter().any(|enabled_slug| enabled_slug == &slug) {
                    unique_push(&mut order, slug);
                }
            }
        }
    }
    for slug in enabled {
        unique_push(&mut order, slug.clone());
    }
    order
}

fn choose_primary(config: &Value, enabled: &[String]) -> Option<String> {
    current_primary_provider(config)
        .filter(|slug| enabled.iter().any(|enabled_slug| enabled_slug == slug))
        .or_else(|| enabled.first().cloned())
}

fn choose_preferred_cheap(config: &Value, enabled: &[String]) -> Option<String> {
    current_preferred_cheap_provider(config)
        .filter(|slug| enabled.iter().any(|enabled_slug| enabled_slug == slug))
}

pub(super) fn apply_remote_cloud_config(
    config: &mut Value,
    enabled_providers: &[String],
    enabled_models: &HashMap<String, Vec<String>>,
    custom_enabled: bool,
    custom_url: Option<&str>,
    custom_model: Option<&str>,
) {
    let enabled = enabled_provider_order(enabled_providers, custom_enabled);
    let primary = choose_primary(config, &enabled);
    let preferred_cheap = choose_preferred_cheap(config, &enabled);
    let custom_model = clean_string(custom_model);

    let mut selected_models: HashMap<String, String> = HashMap::new();

    if let Some(providers) = config.get_mut("providers").and_then(Value::as_array_mut) {
        for provider in providers {
            let Some(slug) =
                value_string(provider, "slug").map(|slug| normalize_provider_slug(&slug))
            else {
                continue;
            };
            let is_enabled = enabled.iter().any(|enabled_slug| enabled_slug == &slug);
            let is_primary = primary.as_deref() == Some(slug.as_str());
            let is_preferred_cheap = preferred_cheap.as_deref() == Some(slug.as_str());
            let selected_model = first_enabled_model(enabled_models, &slug)
                .or_else(|| {
                    (slug == "openai_compatible" && custom_enabled)
                        .then(|| custom_model.clone())
                        .flatten()
                })
                .or_else(|| provider_model_from_entry(provider));

            if let Some(obj) = provider.as_object_mut() {
                obj.insert("enabled".to_string(), Value::Bool(is_enabled));
                obj.insert("primary".to_string(), Value::Bool(is_primary));
                obj.insert(
                    "preferred_cheap".to_string(),
                    Value::Bool(is_preferred_cheap),
                );
                if is_enabled {
                    if let Some(model) = selected_model.clone() {
                        obj.insert("primary_model".to_string(), Value::String(model.clone()));
                        selected_models.insert(slug.clone(), model);
                    }
                }
            }
        }
    }

    let top_primary_model = primary
        .as_deref()
        .and_then(|slug| selected_models.get(slug).cloned())
        .or_else(|| {
            primary.as_deref().and_then(|slug| {
                if slug == "openai_compatible" && custom_enabled {
                    custom_model.clone()
                } else {
                    primary_model_for_provider(config, slug, enabled_models)
                }
            })
        });

    let primary_order = filtered_provider_order(config.get("primary_pool_order"), &enabled);
    let cheap_order = filtered_provider_order(config.get("cheap_pool_order"), &enabled);

    if let Some(obj) = config.as_object_mut() {
        if custom_enabled {
            if let Some(url) = clean_string(custom_url) {
                obj.insert("compatible_base_url".to_string(), Value::String(url));
            }
        }
        set_string_or_null(obj, "primary_provider", primary);
        set_string_or_null(obj, "primary_model", top_primary_model);
        set_string_or_null(obj, "preferred_cheap_provider", preferred_cheap);
        set_array(obj, "primary_pool_order", primary_order);
        set_array(obj, "cheap_pool_order", cheap_order);
    }
}

pub(super) fn apply_remote_selected_brain(config: &mut Value, brain: Option<&str>) {
    let selected = clean_string(brain).map(|brain| normalize_provider_slug(&brain));
    let mut selected_model = selected
        .as_deref()
        .and_then(|slug| primary_model_for_provider(config, slug, &HashMap::new()));
    let mut enabled_slugs = Vec::new();

    if let Some(providers) = config.get_mut("providers").and_then(Value::as_array_mut) {
        for provider in providers {
            let Some(slug) =
                value_string(provider, "slug").map(|slug| normalize_provider_slug(&slug))
            else {
                continue;
            };
            let is_selected = selected.as_deref() == Some(slug.as_str());
            let will_be_enabled = is_selected || provider_enabled(provider);
            if will_be_enabled {
                unique_push(&mut enabled_slugs, slug.clone());
            }
            let fallback_model = provider_model_from_entry(provider);
            if let Some(obj) = provider.as_object_mut() {
                obj.insert("primary".to_string(), Value::Bool(is_selected));
                if is_selected {
                    obj.insert("enabled".to_string(), Value::Bool(true));
                    selected_model = selected_model.or(fallback_model);
                    if let Some(model) = selected_model.clone() {
                        obj.insert("primary_model".to_string(), Value::String(model));
                    }
                }
            }
        }
    }

    let enabled = filtered_provider_order(config.get("primary_pool_order"), &enabled_slugs);

    if let Some(obj) = config.as_object_mut() {
        set_string_or_null(obj, "primary_provider", selected);
        set_string_or_null(obj, "primary_model", selected_model);
        set_array(obj, "primary_pool_order", enabled);
    }
}

pub(super) fn apply_remote_selected_cloud_model(config: &mut Value, model: Option<&str>) {
    let selected_model = clean_string(model);
    let primary = current_primary_provider(config).or_else(|| {
        config
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|providers| {
                providers.iter().find_map(|provider| {
                    let slug = value_string(provider, "slug")?;
                    provider_enabled(provider).then(|| normalize_provider_slug(&slug))
                })
            })
    });

    if let Some(primary_slug) = primary.as_deref() {
        if let Some(providers) = config.get_mut("providers").and_then(Value::as_array_mut) {
            for provider in providers {
                let Some(slug) =
                    value_string(provider, "slug").map(|slug| normalize_provider_slug(&slug))
                else {
                    continue;
                };
                if slug == primary_slug {
                    if let Some(obj) = provider.as_object_mut() {
                        set_string_or_null(obj, "primary_model", selected_model.clone());
                    }
                    break;
                }
            }
        }
    }

    if let Some(obj) = config.as_object_mut() {
        set_string_or_null(obj, "primary_provider", primary);
        set_string_or_null(obj, "primary_model", selected_model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_desktop_provider_aliases() {
        assert_eq!(normalize_provider_slug("custom"), "openai_compatible");
        assert_eq!(normalize_provider_slug("amazon-bedrock"), "bedrock");
        assert_eq!(normalize_provider_slug("google"), "gemini");
    }

    #[test]
    fn cloud_config_maps_enabled_models_to_gateway_provider_config() {
        let mut config = json!({
            "primary_provider": "anthropic",
            "providers": [
                {
                    "slug": "anthropic",
                    "enabled": false,
                    "primary": false,
                    "preferred_cheap": false,
                    "primary_model": null,
                    "suggested_primary_model": "claude-sonnet-4-6",
                    "default_model": "claude-sonnet-4-6"
                },
                {
                    "slug": "openai_compatible",
                    "enabled": false,
                    "primary": false,
                    "preferred_cheap": false,
                    "primary_model": null,
                    "suggested_primary_model": "llama",
                    "default_model": "llama"
                }
            ],
            "primary_pool_order": [],
            "cheap_pool_order": []
        });
        let mut enabled_models = HashMap::new();
        enabled_models.insert("anthropic".to_string(), vec!["claude-opus-4-7".to_string()]);

        apply_remote_cloud_config(
            &mut config,
            &["anthropic".to_string()],
            &enabled_models,
            false,
            None,
            None,
        );

        assert_eq!(config["primary_provider"], "anthropic");
        assert_eq!(config["primary_model"], "claude-opus-4-7");
        assert_eq!(config["primary_pool_order"], json!(["anthropic"]));
        assert_eq!(config["providers"][0]["enabled"], true);
        assert_eq!(config["providers"][0]["primary"], true);
        assert_eq!(config["providers"][0]["primary_model"], "claude-opus-4-7");
    }

    #[test]
    fn cloud_config_maps_custom_provider_to_openai_compatible() {
        let mut config = json!({
            "primary_provider": null,
            "compatible_base_url": null,
            "providers": [
                {
                    "slug": "openai_compatible",
                    "enabled": false,
                    "primary": false,
                    "preferred_cheap": false,
                    "primary_model": null,
                    "suggested_primary_model": "default",
                    "default_model": "default"
                }
            ],
            "primary_pool_order": [],
            "cheap_pool_order": []
        });

        apply_remote_cloud_config(
            &mut config,
            &["custom".to_string()],
            &HashMap::new(),
            true,
            Some(" https://models.example/v1 "),
            Some(" llama-3.3 "),
        );

        assert_eq!(config["compatible_base_url"], "https://models.example/v1");
        assert_eq!(config["primary_provider"], "openai_compatible");
        assert_eq!(config["primary_model"], "llama-3.3");
        assert_eq!(config["providers"][0]["enabled"], true);
        assert_eq!(config["providers"][0]["primary_model"], "llama-3.3");
    }

    #[test]
    fn selected_brain_enables_and_marks_primary_provider() {
        let mut config = json!({
            "primary_provider": null,
            "primary_model": null,
            "providers": [
                {
                    "slug": "bedrock",
                    "enabled": false,
                    "primary": false,
                    "primary_model": null,
                    "suggested_primary_model": "anthropic.claude-3-sonnet",
                    "default_model": "anthropic.claude-3-sonnet"
                }
            ],
            "primary_pool_order": ["anthropic"]
        });

        apply_remote_selected_brain(&mut config, Some("amazon-bedrock"));

        assert_eq!(config["primary_provider"], "bedrock");
        assert_eq!(config["primary_model"], "anthropic.claude-3-sonnet");
        assert_eq!(config["providers"][0]["enabled"], true);
        assert_eq!(config["providers"][0]["primary"], true);
        assert_eq!(config["primary_pool_order"], json!(["bedrock"]));
    }

    #[test]
    fn selected_cloud_model_updates_current_primary_provider() {
        let mut config = json!({
            "primary_provider": "gemini",
            "primary_model": "gemini-2.5-flash",
            "providers": [
                {
                    "slug": "gemini",
                    "enabled": true,
                    "primary": true,
                    "primary_model": "gemini-2.5-flash"
                }
            ]
        });

        apply_remote_selected_cloud_model(&mut config, Some("gemini-2.5-pro"));

        assert_eq!(config["primary_provider"], "gemini");
        assert_eq!(config["primary_model"], "gemini-2.5-pro");
        assert_eq!(config["providers"][0]["primary_model"], "gemini-2.5-pro");
    }
}

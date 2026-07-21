//! Model discovery, selection, and the AI-stack summary surface.
//!
//! Owns provider model fetching, interactive model choice, routed-secondary
//! configuration, and Bedrock fetch-target resolution. Shared entry points use
//! `pub(in crate::setup::wizard)` so the `steps` and `providers` submodules can
//! reach them.

use secrecy::ExposeSecret;
use thinclaw_app::{
    provider_default_model, provider_display_name, suggested_cheap_model_for_provider,
};

use crate::setup::prompts::{
    confirm, input, print_blank_line, print_error, print_info, print_success, select_one,
};

use super::super::helpers::{
    capitalize_first, fetch_anthropic_models, fetch_ollama_models, fetch_openai_compatible_models,
    fetch_openai_models,
};
use super::super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(in crate::setup::wizard) fn choose_model_from_list(
        &mut self,
        models: &[(String, String)],
        prompt: &str,
    ) -> Result<String, SetupError> {
        print_info("Available models:");
        print_blank_line();

        let mut options: Vec<&str> = models.iter().map(|(_, desc)| desc.as_str()).collect();
        options.push("Custom model ID");

        let choice = select_one(prompt, &options).map_err(SetupError::Io)?;

        let selected = if choice == options.len() - 1 {
            loop {
                let raw = input("Enter model ID").map_err(SetupError::Io)?;
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    print_error("Model ID cannot be empty.");
                    continue;
                }
                break trimmed;
            }
        } else {
            models[choice].0.clone()
        };

        Ok(selected)
    }

    pub(in crate::setup::wizard) async fn configure_routed_secondary_model(
        &mut self,
        routing_mode: crate::settings::RoutingMode,
        slot_label: &str,
        slot_prompt: &str,
    ) -> Result<(), SetupError> {
        let current = self
            .settings
            .providers
            .cheap_model
            .clone()
            .unwrap_or_default();
        if !current.is_empty() {
            let keep = confirm(
                &format!("Keep current {slot_label} model ({})?", current),
                true,
            )
            .map_err(SetupError::Io)?;
            if keep {
                self.settings.providers.smart_routing_enabled = true;
                self.settings.providers.routing_mode = routing_mode;
                if let Some((slug, model)) = current.split_once('/') {
                    self.set_preferred_cheap_slot_model(slug, model.to_string());
                }
                self.remove_followup("routing-policy");
                print_success(&format!(
                    "{} enabled. {slot_label} model: {}",
                    routing_mode.as_str(),
                    current
                ));
                return Ok(());
            }
        }

        let mut provider_slugs = std::collections::BTreeSet::new();
        if let Some(primary_slug) = self.primary_provider_slug() {
            provider_slugs.insert(primary_slug.to_string());
        }
        for slug in &self.settings.providers.enabled {
            provider_slugs.insert(slug.clone());
        }
        for slug in crate::config::provider_catalog::catalog().keys() {
            provider_slugs.insert((*slug).to_string());
        }
        if self.settings.openai_compatible_base_url.is_some() {
            provider_slugs.insert("openai_compatible".to_string());
        }
        if self.settings.bedrock_proxy_url.is_some()
            || self.primary_provider_slug() == Some("bedrock")
        {
            provider_slugs.insert("bedrock".to_string());
        }
        if self.settings.llama_cpp_server_url.is_some()
            || self.primary_provider_slug() == Some("llama_cpp")
        {
            provider_slugs.insert("llama_cpp".to_string());
        }
        provider_slugs.insert("ollama".to_string());

        let mut provider_choices: Vec<String> = provider_slugs.into_iter().collect();
        let preferred_provider = self
            .settings
            .providers
            .preferred_cheap_provider
            .clone()
            .or_else(|| self.primary_provider_slug().map(str::to_string));
        provider_choices.sort_by(|a, b| {
            let a_score = usize::from(preferred_provider.as_deref() != Some(a.as_str()));
            let b_score = usize::from(preferred_provider.as_deref() != Some(b.as_str()));
            a_score
                .cmp(&b_score)
                .then_with(|| provider_display_name(a).cmp(&provider_display_name(b)))
        });

        let provider_option_labels: Vec<String> = provider_choices
            .iter()
            .map(|slug| {
                let mut label = provider_display_name(slug);
                if self.primary_provider_slug() == Some(slug.as_str()) {
                    label.push_str(" (current primary)");
                }
                if self.settings.providers.preferred_cheap_provider.as_deref()
                    == Some(slug.as_str())
                {
                    label.push_str(" (current fast)");
                }
                label
            })
            .collect();
        let provider_option_refs: Vec<&str> =
            provider_option_labels.iter().map(String::as_str).collect();
        let provider_prompt = format!("{} provider:", capitalize_first(slot_prompt));
        let provider_choice =
            select_one(&provider_prompt, &provider_option_refs).map_err(SetupError::Io)?;
        let cheap_provider_slug =
            provider_choices
                .get(provider_choice)
                .cloned()
                .ok_or_else(|| {
                    SetupError::Config("Invalid secondary provider selection".to_string())
                })?;

        let display_name = provider_display_name(&cheap_provider_slug);

        let mut model_options = self.fetch_models_for_provider(&cheap_provider_slug).await;
        if model_options.is_empty()
            && let Some(default_model) = provider_default_model(&cheap_provider_slug)
        {
            model_options.push((default_model.clone(), default_model));
        }
        let suggested_cheap = suggested_cheap_model_for_provider(
            &cheap_provider_slug,
            self.settings
                .providers
                .provider_models
                .get(&cheap_provider_slug)
                .and_then(|slots| slots.primary.as_deref()),
        );
        if let Some(suggested) = suggested_cheap.clone() {
            if let Some(index) = model_options.iter().position(|(id, _)| id == &suggested) {
                let entry = model_options.remove(index);
                model_options.insert(
                    0,
                    (
                        entry.0.clone(),
                        format!("{} (recommended fast default)", entry.1),
                    ),
                );
            } else {
                model_options.insert(
                    0,
                    (
                        suggested.clone(),
                        format!("{} (recommended fast default)", suggested),
                    ),
                );
            }
        }

        let model_prompt = format!("Select the {slot_prompt} model:");
        let cheap_model_id = self.choose_model_from_list(&model_options, &model_prompt)?;
        self.settings.providers.smart_routing_enabled = true;
        self.settings.providers.routing_mode = routing_mode;
        self.set_preferred_cheap_slot_model(&cheap_provider_slug, cheap_model_id.clone());
        self.remove_followup("routing-policy");
        print_success(&format!(
            "{} enabled — {slot_label} model: {}/{} ({})",
            routing_mode.as_str(),
            cheap_provider_slug,
            cheap_model_id,
            display_name
        ));

        if let Some(cheap_provider_slug) = self
            .settings
            .providers
            .cheap_model
            .as_deref()
            .and_then(|spec| spec.split('/').next())
            .map(str::to_string)
        {
            let primary_slug = self.primary_provider_slug().unwrap_or("");
            if !cheap_provider_slug.is_empty()
                && cheap_provider_slug != primary_slug
                && !matches!(
                    cheap_provider_slug.as_str(),
                    "ollama" | "llama_cpp" | "openai_compatible" | "bedrock"
                )
            {
                if let Some(endpoint) =
                    crate::config::provider_catalog::endpoint_for(&cheap_provider_slug)
                {
                    let has_provider_key = self
                        .has_provider_secret(&endpoint.env_key_name, &endpoint.secret_name)
                        .await;

                    if std::env::var(&endpoint.env_key_name).is_ok() {
                        crate::setup::prompts::print_blank_line();
                        print_success(&format!(
                            "✓ {} API key found in environment ({}).",
                            endpoint.display_name, endpoint.env_key_name
                        ));
                    } else if has_provider_key {
                        crate::setup::prompts::print_blank_line();
                        print_success(&format!(
                            "✓ {} credentials already stored.",
                            endpoint.display_name
                        ));
                    } else {
                        crate::setup::prompts::print_blank_line();
                        print_info(&format!(
                            "The {slot_label} model uses a different provider than your primary."
                        ));
                        print_info(&format!(
                            "An API key for {} is required.",
                            endpoint.display_name
                        ));

                        self.setup_additional_api_key_provider(
                            &cheap_provider_slug,
                            &endpoint.env_key_name,
                            &endpoint.secret_name,
                            &format!("{} API key", endpoint.display_name),
                            &format!("https://console.{}", cheap_provider_slug),
                            &endpoint.display_name,
                        )
                        .await?;
                    }
                } else {
                    crate::setup::prompts::print_blank_line();
                    print_info(&format!(
                        "Provider '{}' is not in the built-in catalog.",
                        cheap_provider_slug
                    ));
                    print_info(
                        "Make sure the API key is set via the matching environment variable.",
                    );
                }
            }
        }

        Ok(())
    }

    pub(in crate::setup::wizard) fn print_ai_stack_summary(&self) {
        let provider = self
            .primary_provider_slug()
            .map(provider_display_name)
            .unwrap_or_else(|| "unconfigured".to_string());
        let primary_model = self
            .settings
            .providers
            .primary_model
            .clone()
            .or_else(|| self.settings.selected_model.clone())
            .unwrap_or_else(|| "unselected".to_string());
        let routing = self.settings.providers.routing_mode.as_str();
        let aux_model = self
            .settings
            .providers
            .cheap_model
            .clone()
            .unwrap_or_else(|| "none".to_string());
        let fallback_count = self.settings.providers.fallback_chain.len();
        let embeddings = if self.settings.embeddings.enabled {
            format!(
                "{} / {}",
                self.settings.embeddings.provider, self.settings.embeddings.model
            )
        } else {
            "disabled".to_string()
        };

        print_info(&format!(
            "AI stack: provider={}, primary={}, routing={}, aux={}, fallbacks={}, embeddings={}",
            provider, primary_model, routing, aux_model, fallback_count, embeddings
        ));
    }

    fn bedrock_region(&self) -> String {
        std::env::var("AWS_REGION")
            .ok()
            .or_else(|| self.settings.bedrock_region.clone())
            .unwrap_or_else(|| "us-east-1".to_string())
    }

    async fn resolve_bedrock_model_fetch_target(&mut self) -> (String, Option<String>, bool) {
        if let Some(api_key) = self
            .resolve_provider_secret_value("BEDROCK_API_KEY", "llm_bedrock_api_key")
            .await
        {
            match crate::llm::discovery::bedrock_mantle_base_url(&self.bedrock_region()) {
                Ok(base_url) => return (base_url, Some(format!("Bearer {api_key}")), true),
                Err(error) => {
                    tracing::warn!(%error, "Ignoring malformed Bedrock region during model discovery");
                    return (String::new(), None, true);
                }
            }
        }

        let base_url = self
            .settings
            .bedrock_proxy_url
            .clone()
            .or_else(|| std::env::var("BEDROCK_PROXY_URL").ok())
            .unwrap_or_default();
        let auth = self
            .resolve_provider_secret_value("BEDROCK_PROXY_API_KEY", "llm_bedrock_proxy_api_key")
            .await
            .map(|key| format!("Bearer {key}"));
        (base_url, auth, false)
    }

    pub(in crate::setup::wizard) async fn fetch_models_for_provider(
        &mut self,
        provider_slug: &str,
    ) -> Vec<(String, String)> {
        if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(provider_slug) {
            return match endpoint.api_style {
                crate::config::provider_catalog::ApiStyle::Anthropic => {
                    let api_key = self
                        .resolve_provider_secret_value(
                            &endpoint.env_key_name,
                            &endpoint.secret_name,
                        )
                        .await;
                    fetch_anthropic_models(api_key.as_deref()).await
                }
                crate::config::provider_catalog::ApiStyle::OpenAi => {
                    let api_key = self
                        .resolve_provider_secret_value(
                            &endpoint.env_key_name,
                            &endpoint.secret_name,
                        )
                        .await;
                    fetch_openai_models(api_key.as_deref()).await
                }
                crate::config::provider_catalog::ApiStyle::Ollama => {
                    let base_url = self
                        .settings
                        .ollama_base_url
                        .clone()
                        .unwrap_or_else(|| endpoint.base_url.to_string());
                    fetch_ollama_models(&base_url).await
                }
                crate::config::provider_catalog::ApiStyle::OpenAiCompatible => {
                    let api_key = self
                        .resolve_provider_secret_value(
                            &endpoint.env_key_name,
                            &endpoint.secret_name,
                        )
                        .await
                        .or_else(|| {
                            if provider_slug == "openrouter" {
                                self.llm_api_key
                                    .as_ref()
                                    .map(|key| key.expose_secret().to_string())
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            if provider_slug == "openrouter" {
                                std::env::var("LLM_API_KEY").ok()
                            } else {
                                None
                            }
                        });
                    if provider_slug == "cohere" {
                        let static_defaults = vec![(
                            endpoint.default_model.to_string(),
                            endpoint.default_model.to_string(),
                        )];
                        let Some(api_key) = api_key else {
                            return static_defaults;
                        };
                        let result = crate::llm::discovery::ModelDiscovery::new()
                            .discover_cohere(&api_key)
                            .await;
                        let mut models: Vec<(String, String)> = result
                            .models
                            .into_iter()
                            .filter(|model| model.is_chat)
                            .map(|model| (model.id.clone(), model.id))
                            .collect();
                        if models.is_empty() {
                            return static_defaults;
                        }
                        models.sort_by(|a, b| {
                            crate::llm::discovery::cohere_model_priority(&a.0)
                                .cmp(&crate::llm::discovery::cohere_model_priority(&b.0))
                                .then_with(|| a.0.cmp(&b.0))
                        });
                        models
                    } else {
                        let auth_header = api_key.as_ref().map(|key| format!("Bearer {key}"));
                        let mut models = fetch_openai_compatible_models(
                            &endpoint.base_url,
                            auth_header.as_deref(),
                            true,
                            vec![(
                                endpoint.default_model.to_string(),
                                endpoint.default_model.to_string(),
                            )],
                        )
                        .await;
                        if provider_slug == "minimax" {
                            models.sort_by(|a, b| {
                                crate::llm::discovery::minimax_model_priority(&a.0)
                                    .cmp(&crate::llm::discovery::minimax_model_priority(&b.0))
                                    .then_with(|| a.0.cmp(&b.0))
                            });
                        }
                        models
                    }
                }
            };
        }

        match provider_slug {
            "openai_compatible" => {
                let base_url = self
                    .settings
                    .openai_compatible_base_url
                    .clone()
                    .or_else(|| std::env::var("LLM_BASE_URL").ok())
                    .unwrap_or_else(|| "http://localhost:8000/v1".to_string());
                let api_key = self
                    .resolve_provider_secret_value("LLM_API_KEY", "llm_compatible_api_key")
                    .await
                    .or_else(|| {
                        self.llm_api_key
                            .as_ref()
                            .map(|key| key.expose_secret().to_string())
                    });
                let auth_header = api_key.as_ref().map(|key| format!("Bearer {key}"));
                fetch_openai_compatible_models(
                    &base_url,
                    auth_header.as_deref(),
                    false,
                    vec![("default".to_string(), "default".to_string())],
                )
                .await
            }
            "bedrock" => {
                let (base_url, auth_header, public_only) =
                    self.resolve_bedrock_model_fetch_target().await;
                fetch_openai_compatible_models(
                    &base_url,
                    auth_header.as_deref(),
                    public_only,
                    vec![
                        (
                            "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
                            "Claude Sonnet (Bedrock)".to_string(),
                        ),
                        (
                            "anthropic.claude-3-haiku-20240307-v1:0".to_string(),
                            "Claude Haiku (Bedrock)".to_string(),
                        ),
                    ],
                )
                .await
            }
            "llama_cpp" => {
                let base_url = self
                    .settings
                    .llama_cpp_server_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080".to_string());
                fetch_openai_compatible_models(
                    &base_url,
                    None,
                    false,
                    vec![("llama-local".to_string(), "llama-local".to_string())],
                )
                .await
            }
            "ollama" => {
                let base_url = self
                    .settings
                    .ollama_base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                fetch_ollama_models(&base_url).await
            }
            _ => provider_default_model(provider_slug)
                .map(|model| vec![(model.clone(), model)])
                .unwrap_or_default(),
        }
    }
}

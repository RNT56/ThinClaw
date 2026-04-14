//! LLM wizard steps: inference provider, model selection, smart routing, fallback, embeddings.

use secrecy::{ExposeSecret, SecretString};

use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, print_warning,
    secret_input, select_one,
};

use super::helpers::{
    capitalize_first, fetch_anthropic_models, fetch_ollama_models, fetch_openai_compatible_models,
    fetch_openai_models, mask_api_key,
};
use super::{SetupError, SetupWizard};

impl SetupWizard {
    fn set_provider_credential_mode(
        &mut self,
        provider_slug: &str,
        mode: crate::settings::ProviderCredentialMode,
    ) {
        if mode == crate::settings::ProviderCredentialMode::ApiKey {
            self.settings
                .providers
                .provider_credential_modes
                .remove(provider_slug);
        } else {
            self.settings
                .providers
                .provider_credential_modes
                .insert(provider_slug.to_string(), mode);
        }

        self.settings.providers.oauth_sync_enabled = self
            .settings
            .providers
            .provider_credential_modes
            .values()
            .any(|entry| *entry == crate::settings::ProviderCredentialMode::ExternalOAuthSync)
            || !self.settings.providers.oauth_sync_sources.is_empty();
    }

    fn sync_primary_provider_settings(&mut self, provider_slug: &str) {
        self.settings.providers.primary = Some(provider_slug.to_string());
        if !self
            .settings
            .providers
            .enabled
            .iter()
            .any(|slug| slug == provider_slug)
        {
            self.settings
                .providers
                .enabled
                .push(provider_slug.to_string());
        }
    }

    pub(super) fn ensure_provider_enabled(&mut self, provider_slug: &str) {
        if !self
            .settings
            .providers
            .enabled
            .iter()
            .any(|slug| slug == provider_slug)
        {
            self.settings
                .providers
                .enabled
                .push(provider_slug.to_string());
        }
    }

    fn provider_default_model(provider_slug: &str) -> Option<String> {
        crate::config::provider_catalog::endpoint_for(provider_slug)
            .map(|endpoint| endpoint.default_model.to_string())
            .or_else(|| match provider_slug {
                "ollama" => Some("llama3".to_string()),
                "openai_compatible" => Some("default".to_string()),
                "bedrock" => Some("anthropic.claude-3-sonnet-20240229-v1:0".to_string()),
                "llama_cpp" => Some("llama-local".to_string()),
                _ => None,
            })
    }

    fn provider_display_name(provider_slug: &str) -> String {
        crate::config::provider_catalog::endpoint_for(provider_slug)
            .map(|endpoint| endpoint.display_name.to_string())
            .unwrap_or_else(|| match provider_slug {
                "ollama" => "Ollama".to_string(),
                "openai_compatible" => "OpenAI-compatible".to_string(),
                "bedrock" => "AWS Bedrock".to_string(),
                "llama_cpp" => "llama.cpp".to_string(),
                other => other.to_string(),
            })
    }

    fn offer_external_auth_sync(
        &mut self,
        provider_slug: &str,
        display_name: &str,
    ) -> Result<bool, SetupError> {
        let Some(kind) = crate::llm::credential_sync::provider_oauth_source_kind(provider_slug)
        else {
            return Ok(false);
        };
        if !crate::llm::credential_sync::oauth_source_available(kind) {
            return Ok(false);
        }

        let source_label = crate::llm::credential_sync::oauth_source_label(kind);
        let source_location = crate::llm::credential_sync::oauth_source_location_hint(kind);
        print_info(&format!(
            "Detected {} for {} ({})",
            source_label, display_name, source_location
        ));

        if confirm(
            &format!("Use detected {} instead of an API key?", source_label),
            false,
        )
        .map_err(SetupError::Io)?
        {
            self.set_provider_credential_mode(
                provider_slug,
                crate::settings::ProviderCredentialMode::ExternalOAuthSync,
            );
            print_success(&format!(
                "{} configured to use external auth sync",
                display_name
            ));
            return Ok(true);
        }

        Ok(false)
    }

    fn suggested_cheap_model_for_provider(
        provider_slug: &str,
        primary_model: Option<&str>,
    ) -> Option<String> {
        let mapped = match provider_slug {
            "openai" => Some("gpt-4o-mini"),
            "anthropic" => Some("claude-3-5-haiku-latest"),
            "gemini" => Some("gemini-2.5-flash-lite"),
            "minimax" => Some("MiniMax-M2.5-highspeed"),
            "cohere" => Some("command-r7b-12-2024"),
            "openrouter" => Some("openai/gpt-4o-mini"),
            "tinfoil" => Some("kimi-k2-5"),
            _ => None,
        };

        if let Some(candidate) = mapped
            && primary_model != Some(candidate)
        {
            return Some(candidate.to_string());
        }

        Self::provider_default_model(provider_slug)
            .filter(|model| primary_model != Some(model.as_str()))
    }

    pub(super) fn ensure_provider_slot_defaults(&mut self, provider_slug: &str) {
        let current_primary = if self.primary_provider_slug() == Some(provider_slug) {
            self.settings
                .providers
                .primary_model
                .clone()
                .or_else(|| self.settings.selected_model.clone())
        } else {
            None
        };
        let default_primary =
            current_primary.or_else(|| Self::provider_default_model(provider_slug));
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();

        if slots.primary.is_none() {
            slots.primary = default_primary;
        }
        if slots.cheap.is_none() {
            slots.cheap =
                Self::suggested_cheap_model_for_provider(provider_slug, slots.primary.as_deref())
                    .or_else(|| slots.primary.clone());
        }
    }

    fn set_primary_slot_model(&mut self, provider_slug: &str, model: String) {
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();
        slots.primary = Some(model.clone());
        if slots.cheap.is_none() {
            slots.cheap = Self::suggested_cheap_model_for_provider(provider_slug, Some(&model))
                .or_else(|| Some(model.clone()));
        }
        if self.primary_provider_slug() == Some(provider_slug)
            || self.settings.providers.primary.as_deref() == Some(provider_slug)
        {
            self.settings.selected_model = Some(model.clone());
            self.settings.providers.primary_model = Some(model);
        }
        self.ensure_provider_enabled(provider_slug);
    }

    fn set_preferred_cheap_slot_model(&mut self, provider_slug: &str, model: String) {
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();
        if slots.primary.is_none() {
            slots.primary = Self::provider_default_model(provider_slug);
        }
        slots.cheap = Some(model.clone());
        self.settings.providers.preferred_cheap_provider = Some(provider_slug.to_string());
        self.settings.providers.cheap_model = Some(format!("{provider_slug}/{model}"));
        self.ensure_provider_enabled(provider_slug);
    }

    fn choose_model_from_list(
        &mut self,
        models: &[(String, String)],
        prompt: &str,
    ) -> Result<String, SetupError> {
        println!("Available models:");
        println!();

        let mut options: Vec<&str> = models.iter().map(|(_, desc)| desc.as_str()).collect();
        options.push("Custom model ID");

        let choice = select_one(prompt, &options).map_err(SetupError::Io)?;

        let selected = if choice == options.len() - 1 {
            loop {
                let raw = input("Enter model ID").map_err(SetupError::Io)?;
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    println!("Model ID cannot be empty.");
                    continue;
                }
                break trimmed;
            }
        } else {
            models[choice].0.clone()
        };

        Ok(selected)
    }

    fn print_ai_stack_summary(&self) {
        let provider = self
            .primary_provider_slug()
            .map(Self::provider_display_name)
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

    pub(super) async fn step_provider_review_skip_auth(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_warning(
            "Skip-auth mode keeps provider review credential-free. This step will not ask for API keys.",
        );
        println!();

        let options = &[
            "Keep current provider",
            "Pick a provider without asking for credentials",
        ];
        let choice = select_one("Review mode", options).map_err(SetupError::Io)?;
        if choice == 0
            && let Some(current) = self
                .primary_provider_slug()
                .map(str::to_string)
                .or_else(|| self.settings.llm_backend.clone())
        {
            self.ensure_provider_enabled(&current);
            self.ensure_provider_slot_defaults(&current);
            print_success(&format!(
                "Keeping provider review on {} without changing credentials.",
                Self::provider_display_name(&current)
            ));
            return Ok(());
        }

        print_info("Choose the provider ThinClaw should be ready for after onboarding.");
        println!();
        let options = &[
            "Anthropic        - Claude models (add credentials later)",
            "OpenAI           - GPT models (add credentials later)",
            "Gemini           - Google Gemini via AI Studio (add credentials later)",
            "Tinfoil          - Private inference API (add credentials later)",
            "Ollama           - local models, no API key needed",
            "AWS Bedrock      - add credentials later",
            "llama.cpp        - local llama.cpp OpenAI-compatible server",
            "OpenRouter       - OpenAI-compatible endpoint with a later API key",
            "OpenAI-compatible - custom endpoint (base URL set now, auth optional later)",
        ];
        let choice = select_one("Provider", options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                self.settings.llm_backend = Some("anthropic".to_string());
                self.settings.providers.primary = Some("anthropic".to_string());
                self.ensure_provider_enabled("anthropic");
                self.ensure_provider_slot_defaults("anthropic");
            }
            1 => {
                self.settings.llm_backend = Some("openai".to_string());
                self.settings.providers.primary = Some("openai".to_string());
                self.ensure_provider_enabled("openai");
                self.ensure_provider_slot_defaults("openai");
            }
            2 => {
                self.settings.llm_backend = Some("gemini".to_string());
                self.settings.providers.primary = Some("gemini".to_string());
                self.ensure_provider_enabled("gemini");
                self.ensure_provider_slot_defaults("gemini");
            }
            3 => {
                self.settings.llm_backend = Some("tinfoil".to_string());
                self.settings.providers.primary = Some("tinfoil".to_string());
                self.ensure_provider_enabled("tinfoil");
                self.ensure_provider_slot_defaults("tinfoil");
            }
            4 => {
                self.setup_ollama()?;
            }
            5 => {
                self.settings.llm_backend = Some("bedrock".to_string());
                self.settings.providers.primary = Some("bedrock".to_string());
                self.ensure_provider_enabled("bedrock");
                self.ensure_provider_slot_defaults("bedrock");
            }
            6 => {
                self.setup_llama_cpp()?;
            }
            7 => {
                self.settings.llm_backend = Some("openai_compatible".to_string());
                self.settings.openai_compatible_base_url =
                    Some("https://openrouter.ai/api/v1".to_string());
                self.settings.providers.primary = Some("openrouter".to_string());
                self.ensure_provider_enabled("openrouter");
                self.ensure_provider_slot_defaults("openrouter");
            }
            8 => {
                self.settings.llm_backend = Some("openai_compatible".to_string());
                let url = optional_input(
                    "Base URL",
                    self.settings
                        .openai_compatible_base_url
                        .as_deref()
                        .or(Some("e.g. http://localhost:8000/v1")),
                )
                .map_err(SetupError::Io)?
                .ok_or_else(|| {
                    SetupError::Config(
                        "Base URL is required when selecting an OpenAI-compatible endpoint."
                            .to_string(),
                    )
                })?;
                self.settings.openai_compatible_base_url = Some(url);
                self.settings.providers.primary = Some("openai_compatible".to_string());
                self.ensure_provider_enabled("openai_compatible");
                self.ensure_provider_slot_defaults("openai_compatible");
            }
            _ => unreachable!(),
        }

        print_success("Provider review updated. You can add credentials later.");
        Ok(())
    }

    pub(super) async fn step_inference_provider(&mut self) -> Result<(), SetupError> {
        // Show current provider if already configured
        if let Some(ref current) = self.settings.llm_backend {
            let is_openrouter = current == "openai_compatible"
                && self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .is_some_and(|u| u.contains("openrouter.ai"));

            let display = if is_openrouter {
                "OpenRouter"
            } else {
                match current.as_str() {
                    "anthropic" => "Anthropic (Claude)",
                    "openai" => "OpenAI",
                    "gemini" => "Gemini",
                    "tinfoil" => "Tinfoil",
                    "ollama" => "Ollama (local)",
                    "bedrock" => "AWS Bedrock",
                    "llama_cpp" => "llama.cpp server",
                    "openai_compatible" => "OpenAI-compatible endpoint",
                    other => other,
                }
            };
            print_info(&format!("Current provider: {}", display));
            println!();

            let is_known = matches!(
                current.as_str(),
                "anthropic"
                    | "openai"
                    | "gemini"
                    | "tinfoil"
                    | "ollama"
                    | "bedrock"
                    | "llama_cpp"
                    | "openai_compatible"
            );

            if is_known && confirm("Keep this provider?", true).map_err(SetupError::Io)? {
                if is_openrouter {
                    return self.setup_openrouter().await;
                }
                match current.as_str() {
                    "anthropic" => return self.setup_anthropic().await,
                    "openai" => return self.setup_openai().await,
                    "gemini" => return self.setup_gemini().await,
                    "tinfoil" => return self.setup_tinfoil().await,
                    "ollama" => return self.setup_ollama(),
                    "bedrock" => return self.setup_bedrock().await,
                    "llama_cpp" => return self.setup_llama_cpp(),
                    "openai_compatible" => return self.setup_openai_compatible().await,
                    _ => {
                        return Err(SetupError::Config(format!(
                            "Unhandled provider: {}",
                            current
                        )));
                    }
                }
            }

            if !is_known {
                print_info(&format!(
                    "Unknown provider '{}', please select a supported provider.",
                    current
                ));
            }
        }

        print_info("Choose your inference provider:");
        println!();

        let options = &[
            "Anthropic        - Claude models (direct API key)",
            "OpenAI           - GPT models (direct API key)",
            "Gemini           - Google Gemini via AI Studio API key",
            "Tinfoil          - Private inference API key",
            "Ollama           - local models, no API key needed",
            "AWS Bedrock      - AWS-hosted models via native API key",
            "llama.cpp        - local llama.cpp OpenAI-compatible server",
            "OpenRouter       - 200+ models with one API key",
            "OpenAI-compatible - custom endpoint (vLLM, LiteLLM, etc.)",
        ];

        let choice = select_one("Provider:", options).map_err(SetupError::Io)?;

        match choice {
            0 => self.setup_anthropic().await?,
            1 => self.setup_openai().await?,
            2 => self.setup_gemini().await?,
            3 => self.setup_tinfoil().await?,
            4 => self.setup_ollama()?,
            5 => self.setup_bedrock().await?,
            6 => self.setup_llama_cpp()?,
            7 => self.setup_openrouter().await?,
            8 => self.setup_openai_compatible().await?,
            _ => return Err(SetupError::Config("Invalid provider selection".to_string())),
        }

        Ok(())
    }

    fn primary_provider_slug(&self) -> Option<&str> {
        self.settings.providers.primary.as_deref().or_else(|| {
            match self.settings.llm_backend.as_deref() {
                Some("openai_compatible")
                    if self
                        .settings
                        .openai_compatible_base_url
                        .as_deref()
                        .is_some_and(|url| url.contains("openrouter.ai")) =>
                {
                    Some("openrouter")
                }
                other => other,
            }
        })
    }

    pub(super) async fn has_saved_secret(&mut self, secret_name: &str) -> bool {
        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.secret_exists(secret_name).await
        } else {
            false
        }
    }

    async fn has_provider_secret(&mut self, env_var: &str, secret_name: &str) -> bool {
        std::env::var(env_var).is_ok()
            || crate::secrets::keychain::get_api_key(secret_name)
                .await
                .is_some()
            || self.has_saved_secret(secret_name).await
    }

    async fn resolve_provider_secret_value(
        &mut self,
        env_var: &str,
        secret_name: &str,
    ) -> Option<String> {
        if let Ok(value) = std::env::var(env_var)
            && !value.trim().is_empty()
        {
            return Some(value);
        }

        if let Some(value) = crate::secrets::keychain::get_api_key(secret_name).await
            && !value.trim().is_empty()
        {
            return Some(value);
        }

        if let Ok(ctx) = self.init_secrets_context().await
            && let Ok(secret) = ctx.get_secret(secret_name).await
        {
            let value = secret.expose_secret().trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }

        match env_var {
            "OPENROUTER_API_KEY" => {
                if let Ok(value) = std::env::var("LLM_API_KEY")
                    && !value.trim().is_empty()
                {
                    return Some(value);
                }
            }
            "BEDROCK_API_KEY" => {
                if let Ok(value) = std::env::var("AWS_BEARER_TOKEN_BEDROCK")
                    && !value.trim().is_empty()
                {
                    return Some(value);
                }
            }
            _ => {}
        }

        None
    }

    fn bedrock_region(&self) -> String {
        std::env::var("AWS_REGION")
            .ok()
            .or_else(|| self.settings.bedrock_region.clone())
            .unwrap_or_else(|| "us-east-1".to_string())
    }

    async fn resolve_bedrock_model_fetch_target(&mut self) -> (String, Option<String>) {
        if let Some(api_key) = self
            .resolve_provider_secret_value("BEDROCK_API_KEY", "llm_bedrock_api_key")
            .await
        {
            let base_url = crate::llm::discovery::bedrock_mantle_base_url(&self.bedrock_region());
            return (base_url, Some(format!("Bearer {api_key}")));
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
        (base_url, auth)
    }

    async fn fetch_models_for_provider(&mut self, provider_slug: &str) -> Vec<(String, String)> {
        if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(provider_slug) {
            return match endpoint.api_style {
                crate::config::provider_catalog::ApiStyle::Anthropic => {
                    let api_key = self
                        .resolve_provider_secret_value(endpoint.env_key_name, endpoint.secret_name)
                        .await;
                    fetch_anthropic_models(api_key.as_deref()).await
                }
                crate::config::provider_catalog::ApiStyle::OpenAi => {
                    let api_key = self
                        .resolve_provider_secret_value(endpoint.env_key_name, endpoint.secret_name)
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
                        .resolve_provider_secret_value(endpoint.env_key_name, endpoint.secret_name)
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
                            endpoint.base_url,
                            auth_header.as_deref(),
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
                    vec![("default".to_string(), "default".to_string())],
                )
                .await
            }
            "bedrock" => {
                let (base_url, auth_header) = self.resolve_bedrock_model_fetch_target().await;
                fetch_openai_compatible_models(
                    &base_url,
                    auth_header.as_deref(),
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
            _ => Self::provider_default_model(provider_slug)
                .map(|model| vec![(model.clone(), model)])
                .unwrap_or_default(),
        }
    }

    async fn has_any_remote_provider_credentials(&mut self) -> bool {
        for (slug, env_var, secret_name) in [
            ("anthropic", "ANTHROPIC_API_KEY", "llm_anthropic_api_key"),
            ("openai", "OPENAI_API_KEY", "llm_openai_api_key"),
            ("gemini", "GEMINI_API_KEY", "gemini"),
            ("tinfoil", "TINFOIL_API_KEY", "llm_tinfoil_api_key"),
            ("openrouter", "LLM_API_KEY", "llm_compatible_api_key"),
            ("openai_compatible", "LLM_API_KEY", "llm_compatible_api_key"),
        ] {
            if !self
                .settings
                .providers
                .enabled
                .iter()
                .any(|enabled| enabled == slug)
                && self.primary_provider_slug() != Some(slug)
            {
                continue;
            }

            if self.has_provider_secret(env_var, secret_name).await {
                return true;
            }
        }

        if (self.primary_provider_slug() == Some("bedrock")
            || self
                .settings
                .providers
                .enabled
                .iter()
                .any(|slug| slug == "bedrock"))
            && (self
                .has_provider_secret("BEDROCK_API_KEY", "llm_bedrock_api_key")
                .await
                || self
                    .has_provider_secret("BEDROCK_PROXY_API_KEY", "llm_bedrock_proxy_api_key")
                    .await)
        {
            return true;
        }

        false
    }

    pub(super) async fn ensure_onboarding_provider_api_key(&mut self) -> Result<(), SetupError> {
        if self.has_any_remote_provider_credentials().await {
            return Ok(());
        }

        println!();
        print_info(
            "ThinClaw onboarding requires at least one authenticated remote provider so routing and failover have a real remote backend available.",
        );
        print_info(
            "You can still keep Ollama, llama.cpp, or a no-auth compatible endpoint as primary, but please add one authenticated remote provider now.",
        );
        println!();

        let options = &[
            "OpenRouter  - single key, broad model coverage",
            "OpenAI      - GPT models",
            "Anthropic   - Claude models",
            "Gemini      - Google Gemini models",
            "Tinfoil     - Private inference",
        ];

        let choice = select_one("Provider to add:", options).map_err(SetupError::Io)?;
        match choice {
            0 => {
                self.setup_additional_api_key_provider(
                    "openrouter",
                    "LLM_API_KEY",
                    "llm_compatible_api_key",
                    "OpenRouter API key",
                    "https://openrouter.ai/settings/keys",
                    "OpenRouter",
                )
                .await?
            }
            1 => {
                self.setup_additional_api_key_provider(
                    "openai",
                    "OPENAI_API_KEY",
                    "llm_openai_api_key",
                    "OpenAI API key",
                    "https://platform.openai.com/api-keys",
                    "OpenAI",
                )
                .await?
            }
            2 => {
                self.setup_additional_api_key_provider(
                    "anthropic",
                    "ANTHROPIC_API_KEY",
                    "llm_anthropic_api_key",
                    "Anthropic API key",
                    "https://console.anthropic.com/settings/keys",
                    "Anthropic",
                )
                .await?
            }
            3 => {
                self.setup_additional_api_key_provider(
                    "gemini",
                    "GEMINI_API_KEY",
                    "gemini",
                    "Gemini API key",
                    "https://aistudio.google.com/app/apikey",
                    "Gemini",
                )
                .await?
            }
            4 => {
                self.setup_additional_api_key_provider(
                    "tinfoil",
                    "TINFOIL_API_KEY",
                    "llm_tinfoil_api_key",
                    "Tinfoil API key",
                    "https://inference.tinfoil.sh",
                    "Tinfoil",
                )
                .await?
            }
            _ => {
                return Err(SetupError::Config(
                    "Invalid provider key selection".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Anthropic provider setup: collect API key and store in secrets.
    pub(super) async fn setup_anthropic(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "anthropic",
            "ANTHROPIC_API_KEY",
            "llm_anthropic_api_key",
            "Anthropic API key",
            "https://console.anthropic.com/settings/keys",
            None,
        )
        .await
    }

    /// OpenAI provider setup: collect API key and store in secrets.
    pub(super) async fn setup_openai(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "openai",
            "OPENAI_API_KEY",
            "llm_openai_api_key",
            "OpenAI API key",
            "https://platform.openai.com/api-keys",
            None,
        )
        .await
    }

    pub(super) async fn setup_gemini(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "gemini",
            "GEMINI_API_KEY",
            "gemini",
            "Gemini API key",
            "https://aistudio.google.com/app/apikey",
            Some("Gemini"),
        )
        .await
    }

    pub(super) async fn setup_tinfoil(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "tinfoil",
            "TINFOIL_API_KEY",
            "llm_tinfoil_api_key",
            "Tinfoil API key",
            "https://inference.tinfoil.sh",
            Some("Tinfoil"),
        )
        .await
    }

    /// Shared setup flow for API-key-based providers (Anthropic, OpenAI, OpenRouter).
    pub(super) async fn setup_api_key_provider(
        &mut self,
        backend: &str,
        env_var: &str,
        secret_name: &str,
        prompt_label: &str,
        hint_url: &str,
        override_display_name: Option<&str>,
    ) -> Result<(), SetupError> {
        let display_name = override_display_name.unwrap_or(match backend {
            "anthropic" => "Anthropic",
            "openai" => "OpenAI",
            "gemini" => "Gemini",
            "tinfoil" => "Tinfoil",
            other => other,
        });

        self.settings.llm_backend = Some(backend.to_string());
        self.sync_primary_provider_settings(match backend {
            "openai_compatible" => {
                if self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .is_some_and(|url| url.contains("openrouter.ai"))
                {
                    "openrouter"
                } else {
                    "openai_compatible"
                }
            }
            other => other,
        });
        if let Some(primary_slug) = self.primary_provider_slug().map(str::to_string) {
            self.ensure_provider_slot_defaults(&primary_slug);
        }
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        // Check env var first
        if let Ok(existing) = std::env::var(env_var) {
            print_info(&format!("{env_var} found: {}", mask_api_key(&existing)));
            if confirm("Use this key?", true).map_err(SetupError::Io)? {
                // Persist env-provided key to secrets store for future runs
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret(secret_name, &key).await {
                        tracing::warn!("Failed to persist env key to secrets: {}", e);
                    }
                }
                self.set_provider_credential_mode(
                    backend,
                    crate::settings::ProviderCredentialMode::ApiKey,
                );
                self.llm_api_key = Some(SecretString::from(existing));
                if let Some(primary_slug) = self.primary_provider_slug().map(str::to_string) {
                    self.ensure_provider_slot_defaults(&primary_slug);
                }
                print_success(&format!("{display_name} configured (from env)"));
                return Ok(());
            }
        }

        if self.offer_external_auth_sync(backend, display_name)? {
            return Ok(());
        }

        println!();
        print_info(&format!("Get your API key here: {hint_url}"));
        println!();

        let key = secret_input(prompt_label).map_err(SetupError::Io)?;
        let key_str = key.expose_secret();

        if key_str.is_empty() {
            return Err(SetupError::Config("API key cannot be empty".to_string()));
        }

        // Store in secrets if available
        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.save_secret(secret_name, &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save API key: {e}")))?;
            print_success("API key encrypted and saved");
        } else {
            print_info(&format!(
                "Secrets aren't available. Set {env_var} in your environment."
            ));
        }

        // Cache key in memory for model fetching later in the wizard
        self.set_provider_credential_mode(backend, crate::settings::ProviderCredentialMode::ApiKey);
        self.llm_api_key = Some(SecretString::from(key_str.to_string()));
        if let Some(primary_slug) = self.primary_provider_slug().map(str::to_string) {
            self.ensure_provider_slot_defaults(&primary_slug);
        }

        print_success(&format!("{display_name} configured"));
        Ok(())
    }

    pub(super) async fn setup_additional_api_key_provider(
        &mut self,
        provider_slug: &str,
        env_var: &str,
        secret_name: &str,
        prompt_label: &str,
        hint_url: &str,
        display_name: &str,
    ) -> Result<(), SetupError> {
        if let Ok(existing) = std::env::var(env_var) {
            print_info(&format!("{env_var} found: {}", mask_api_key(&existing)));
            if confirm("Use this key?", true).map_err(SetupError::Io)? {
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret(secret_name, &key).await {
                        tracing::warn!("Failed to persist env key to secrets: {}", e);
                    }
                }
                self.set_provider_credential_mode(
                    provider_slug,
                    crate::settings::ProviderCredentialMode::ApiKey,
                );
                if !self
                    .settings
                    .providers
                    .enabled
                    .iter()
                    .any(|slug| slug == provider_slug)
                {
                    self.settings
                        .providers
                        .enabled
                        .push(provider_slug.to_string());
                }
                self.ensure_provider_slot_defaults(provider_slug);
                print_success(&format!("{display_name} configured"));
                return Ok(());
            }
        }

        if self.offer_external_auth_sync(provider_slug, display_name)? {
            if !self
                .settings
                .providers
                .enabled
                .iter()
                .any(|slug| slug == provider_slug)
            {
                self.settings
                    .providers
                    .enabled
                    .push(provider_slug.to_string());
            }
            self.ensure_provider_slot_defaults(provider_slug);
            return Ok(());
        }

        println!();
        print_info(&format!("Get your API key here: {hint_url}"));
        println!();

        let key = secret_input(prompt_label).map_err(SetupError::Io)?;
        if key.expose_secret().is_empty() {
            return Ok(());
        }

        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.save_secret(secret_name, &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save API key: {e}")))?;
            print_success("API key encrypted and saved");
        } else {
            print_info(&format!(
                "Secrets aren't available. Set {env_var} in your environment."
            ));
        }
        self.set_provider_credential_mode(
            provider_slug,
            crate::settings::ProviderCredentialMode::ApiKey,
        );

        if !self
            .settings
            .providers
            .enabled
            .iter()
            .any(|slug| slug == provider_slug)
        {
            self.settings
                .providers
                .enabled
                .push(provider_slug.to_string());
        }
        self.ensure_provider_slot_defaults(provider_slug);
        print_success(&format!("{display_name} configured"));
        Ok(())
    }

    /// Ollama provider setup: just needs a base URL, no API key.
    pub(super) fn setup_ollama(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("ollama".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let default_url = self
            .settings
            .ollama_base_url
            .as_deref()
            .unwrap_or("http://localhost:11434");

        let url_input = optional_input(
            "Ollama base URL",
            Some(&format!("default: {}", default_url)),
        )
        .map_err(SetupError::Io)?;

        let url = url_input.unwrap_or_else(|| default_url.to_string());
        self.settings.ollama_base_url = Some(url.clone());
        self.sync_primary_provider_settings("ollama");
        self.ensure_provider_slot_defaults("ollama");

        print_success(&format!("Ollama configured ({})", url));
        Ok(())
    }

    /// AWS Bedrock setup: region + native API key, with optional legacy proxy fallback.
    pub(super) async fn setup_bedrock(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("bedrock".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let default_region = self
            .settings
            .bedrock_region
            .as_deref()
            .unwrap_or("us-east-1");
        let region = optional_input("AWS region", Some(&format!("default: {}", default_region)))
            .map_err(SetupError::Io)?
            .unwrap_or_else(|| default_region.to_string());
        self.settings.bedrock_region = Some(region.clone());
        self.sync_primary_provider_settings("bedrock");
        self.ensure_provider_slot_defaults("bedrock");

        print_info("ThinClaw now prefers Bedrock's native OpenAI-compatible Mantle endpoint.");
        print_info("Use a Bedrock API key for native access. A legacy proxy URL is optional.");

        let mut native_key_ready = false;
        if let Ok(existing) = std::env::var("BEDROCK_API_KEY")
            && !existing.trim().is_empty()
        {
            print_info(&format!(
                "BEDROCK_API_KEY found: {}",
                mask_api_key(&existing)
            ));
            if confirm("Use this Bedrock API key?", true).map_err(SetupError::Io)? {
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret("llm_bedrock_api_key", &key).await {
                        tracing::warn!("Failed to persist Bedrock API key to secrets: {}", e);
                    }
                }
                native_key_ready = true;
            }
        }
        if !native_key_ready
            && self
                .has_provider_secret("BEDROCK_API_KEY", "llm_bedrock_api_key")
                .await
        {
            print_info("Bedrock API key found in environment or secrets.");
            native_key_ready = true;
        }
        if !native_key_ready {
            println!();
            let api_key = secret_input("Bedrock API key").map_err(SetupError::Io)?;
            if api_key.expose_secret().is_empty() {
                return Err(SetupError::Config(
                    "Bedrock API key cannot be empty.".to_string(),
                ));
            }

            if let Ok(ctx) = self.init_secrets_context().await {
                ctx.save_secret("llm_bedrock_api_key", &api_key)
                    .await
                    .map_err(|e| {
                        SetupError::Config(format!("Failed to save Bedrock API key: {e}"))
                    })?;
                print_success("Bedrock API key encrypted and saved");
            } else {
                print_info("Secrets aren't available. Set BEDROCK_API_KEY in your environment.");
            }
        }

        let current_proxy = std::env::var("BEDROCK_PROXY_URL")
            .ok()
            .or_else(|| self.settings.bedrock_proxy_url.clone());
        if let Some(proxy) = current_proxy {
            if confirm("Keep the legacy Bedrock proxy fallback configured?", true)
                .map_err(SetupError::Io)?
            {
                let proxy_url = optional_input("Legacy Bedrock proxy URL", Some(&proxy))
                    .map_err(SetupError::Io)?
                    .unwrap_or(proxy);
                self.settings.bedrock_proxy_url = Some(proxy_url);
            } else {
                self.settings.bedrock_proxy_url = None;
            }
        } else if confirm("Configure a legacy Bedrock proxy fallback?", false)
            .map_err(SetupError::Io)?
        {
            let proxy_url = input("Legacy Bedrock proxy URL").map_err(SetupError::Io)?;
            if proxy_url.trim().is_empty() {
                self.settings.bedrock_proxy_url = None;
            } else {
                self.settings.bedrock_proxy_url = Some(proxy_url.clone());
                if confirm("Does your legacy Bedrock proxy require an API key?", false)
                    .map_err(SetupError::Io)?
                {
                    let proxy_api_key =
                        secret_input("Legacy Bedrock proxy API key").map_err(SetupError::Io)?;
                    if proxy_api_key.expose_secret().is_empty() {
                        return Err(SetupError::Config(
                            "Legacy Bedrock proxy API key cannot be empty.".to_string(),
                        ));
                    }

                    if let Ok(ctx) = self.init_secrets_context().await {
                        ctx.save_secret("llm_bedrock_proxy_api_key", &proxy_api_key)
                            .await
                            .map_err(|e| {
                                SetupError::Config(format!(
                                    "Failed to save legacy Bedrock proxy API key: {e}"
                                ))
                            })?;
                        print_success("Legacy Bedrock proxy API key encrypted and saved");
                    } else {
                        print_info(
                            "Secrets aren't available. Set BEDROCK_PROXY_API_KEY in your environment.",
                        );
                    }
                }
            }
        } else {
            self.settings.bedrock_proxy_url = None;
        }

        print_success(&format!("AWS Bedrock configured ({})", region));
        Ok(())
    }

    /// llama.cpp setup: local server URL, no credentials required.
    pub(super) fn setup_llama_cpp(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("llama_cpp".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let default_url = self
            .settings
            .llama_cpp_server_url
            .as_deref()
            .unwrap_or("http://localhost:8080");
        let server_url = optional_input(
            "llama.cpp server URL",
            Some(&format!("default: {}", default_url)),
        )
        .map_err(SetupError::Io)?
        .unwrap_or_else(|| default_url.to_string());

        self.settings.llama_cpp_server_url = Some(server_url.clone());
        self.sync_primary_provider_settings("llama_cpp");
        self.ensure_provider_slot_defaults("llama_cpp");

        print_success(&format!("llama.cpp configured ({})", server_url));
        Ok(())
    }

    /// OpenRouter provider setup: pre-configured OpenAI-compatible endpoint.
    ///
    /// Sets the base URL to `https://openrouter.ai/api/v1` and delegates
    /// API key collection to `setup_api_key_provider` with a display name
    /// override so messages say "OpenRouter" instead of "openai_compatible".
    pub(super) async fn setup_openrouter(&mut self) -> Result<(), SetupError> {
        self.settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        self.setup_api_key_provider(
            "openai_compatible",
            "LLM_API_KEY",
            "llm_compatible_api_key",
            "OpenRouter API key",
            "https://openrouter.ai/settings/keys",
            Some("OpenRouter"),
        )
        .await
    }

    /// OpenAI-compatible provider setup: base URL + optional API key.
    pub(super) async fn setup_openai_compatible(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai_compatible".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let existing_url = self
            .settings
            .openai_compatible_base_url
            .clone()
            .or_else(|| std::env::var("LLM_BASE_URL").ok());

        let url = if let Some(ref u) = existing_url {
            let url_input = optional_input("Base URL", Some(&format!("current: {}", u)))
                .map_err(SetupError::Io)?;
            url_input.unwrap_or_else(|| u.clone())
        } else {
            input("Base URL (e.g., http://localhost:8000/v1)").map_err(SetupError::Io)?
        };

        if url.is_empty() {
            return Err(SetupError::Config(
                "Base URL is required for OpenAI-compatible provider".to_string(),
            ));
        }

        self.settings.openai_compatible_base_url = Some(url.clone());
        self.sync_primary_provider_settings("openai_compatible");
        self.ensure_provider_slot_defaults("openai_compatible");

        // Optional API key
        if confirm("Does this endpoint require an API key?", false).map_err(SetupError::Io)? {
            let key = secret_input("API key").map_err(SetupError::Io)?;
            let key_str = key.expose_secret();

            if !key_str.is_empty() {
                if let Ok(ctx) = self.init_secrets_context().await {
                    ctx.save_secret("llm_compatible_api_key", &key)
                        .await
                        .map_err(|e| {
                            SetupError::Config(format!("Failed to save API key: {}", e))
                        })?;
                    print_success("API key encrypted and saved");
                } else {
                    print_info("Secrets aren't available. Set LLM_API_KEY in your environment.");
                }
                self.llm_api_key = Some(SecretString::from(key_str.to_string()));
            }
        }

        print_success(&format!("OpenAI-compatible configured ({})", url));
        Ok(())
    }

    /// Step 4: Model selection.
    ///
    /// Branches on the selected LLM backend and fetches models from the
    /// appropriate provider API, with static defaults as fallback.
    pub(super) async fn step_model_selection(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();

        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            println!();

            let options = ["Keep current model", "Change model"];
            let choice =
                select_one("What would you like to do?", &options).map_err(SetupError::Io)?;

            if choice == 0 {
                let current_model = current.clone();
                let provider_slug = self
                    .primary_provider_slug()
                    .map(str::to_string)
                    .or_else(|| self.settings.llm_backend.clone())
                    .unwrap_or_else(|| "openai_compatible".to_string());
                self.set_primary_slot_model(&provider_slug, current_model.clone());
                print_success(&format!("Keeping {}", current_model));
                return Ok(());
            }
        }

        let provider_slug = self
            .primary_provider_slug()
            .map(str::to_string)
            .or_else(|| self.settings.llm_backend.clone())
            .unwrap_or_else(|| "openai_compatible".to_string());

        let mut models = self.fetch_models_for_provider(&provider_slug).await;
        if models.is_empty()
            && let Some(default_model) = Self::provider_default_model(&provider_slug)
        {
            models.push((default_model.clone(), default_model));
        }
        if provider_slug == "ollama" && models.is_empty() {
            print_info("No models found. Pull one first with: ollama pull llama3");
        }

        let selected = self.choose_model_from_list(&models, "Choose a model:")?;
        self.set_primary_slot_model(&provider_slug, selected.clone());
        print_success(&format!("Selected {}", selected));

        Ok(())
    }

    /// Step 6: Embeddings configuration.
    pub(super) fn step_embeddings(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_info("Embeddings turn on semantic search in workspace memory.");
        println!();

        if !confirm("Enable semantic search?", true).map_err(SetupError::Io)? {
            self.settings.embeddings.enabled = false;
            self.remove_followup("embeddings");
            print_info("Embeddings disabled. Workspace will fall back to keyword search.");
            return Ok(());
        }

        let openai_is_primary = self.settings.llm_backend.as_deref() == Some("openai")
            || self.settings.providers.primary.as_deref() == Some("openai");
        let has_openai_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
            || (openai_is_primary && self.llm_api_key.is_some());
        let has_local_ollama = self.settings.llm_backend.as_deref() == Some("ollama")
            || self.settings.providers.primary.as_deref() == Some("ollama")
            || self
                .settings
                .providers
                .enabled
                .iter()
                .any(|slug| slug == "ollama")
            || self.settings.ollama_base_url.is_some()
            || std::env::var("OLLAMA_BASE_URL")
                .ok()
                .is_some_and(|value| !value.trim().is_empty());

        if has_openai_key && !has_local_ollama {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "openai".to_string();
            self.settings.embeddings.model = "text-embedding-3-small".to_string();
            self.remove_followup("embeddings");
            print_success("Embeddings enabled via OpenAI because a remote key was detected.");
            return Ok(());
        }

        if has_local_ollama && !has_openai_key {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "ollama".to_string();
            self.settings.embeddings.model = "nomic-embed-text".to_string();
            self.remove_followup("embeddings");
            print_success("Embeddings enabled via Ollama because a local provider was detected.");
            return Ok(());
        }

        if has_openai_key && has_local_ollama {
            let choose_openai = match self.selected_profile {
                super::OnboardingProfile::LocalAndPrivate => {
                    let options = [
                        "Ollama (recommended)       - local/private embeddings",
                        "OpenAI (requires API key)  - higher-quality remote embeddings",
                    ];
                    let choice = select_one(
                        "Select embeddings provider (Enter uses recommended):",
                        &options,
                    )
                    .map_err(SetupError::Io)?;
                    choice == 1
                }
                super::OnboardingProfile::CustomAdvanced => {
                    let options = [
                        "OpenAI                    - higher-quality remote embeddings",
                        "Ollama                    - local/private embeddings",
                    ];
                    let choice = select_one("Select embeddings provider:", &options)
                        .map_err(SetupError::Io)?;
                    choice == 0
                }
                _ => {
                    let options = [
                        "OpenAI (recommended)       - higher-quality remote embeddings",
                        "Ollama (local, no API key) - local/private embeddings",
                    ];
                    let choice = select_one(
                        "Select embeddings provider (Enter uses recommended):",
                        &options,
                    )
                    .map_err(SetupError::Io)?;
                    choice == 0
                }
            };

            if choose_openai {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "openai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                self.remove_followup("embeddings");
                print_success("Embeddings configured for OpenAI.");
            } else {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "ollama".to_string();
                self.settings.embeddings.model = "nomic-embed-text".to_string();
                self.remove_followup("embeddings");
                print_success("Embeddings configured for Ollama.");
            }
            return Ok(());
        }

        print_warning("No ready embeddings backend was detected.");
        print_info(
            "You can continue now and keep semantic search disabled until credentials or a local embeddings service is ready.",
        );
        self.settings.embeddings.enabled = false;
        self.add_followup(super::FollowupDraft {
            id: "embeddings".to_string(),
            title: "Enable semantic search".to_string(),
            category: crate::settings::OnboardingFollowupCategory::Authentication,
            status: crate::settings::OnboardingFollowupStatus::Optional,
            instructions: "Embeddings were requested, but neither a usable OpenAI key nor a local Ollama embeddings path was detected during onboarding.".to_string(),
            action_hint: Some("Set OPENAI_API_KEY or configure Ollama, then rerun `thinclaw onboard` to enable semantic search.".to_string()),
        });
        print_info("A follow-up was saved so onboarding can continue now.");

        Ok(())
    }

    pub(super) async fn step_smart_routing(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_info("Choose how ThinClaw should split work across models.");
        print_info("You can stay on one model, split cheaper work to a faster auxiliary model,");
        print_info(
            "or use advisor/executor mode so a lighter advisor can review work before heavy execution.",
        );
        println!();

        let recommended_mode = match self.selected_profile {
            super::OnboardingProfile::LocalAndPrivate => {
                Some(crate::settings::RoutingMode::PrimaryOnly)
            }
            super::OnboardingProfile::BuilderAndCoding => {
                Some(crate::settings::RoutingMode::AdvisorExecutor)
            }
            super::OnboardingProfile::Balanced | super::OnboardingProfile::ChannelFirst => {
                Some(crate::settings::RoutingMode::CheapSplit)
            }
            super::OnboardingProfile::CustomAdvanced => None,
        };
        if let Some(recommended_mode) = recommended_mode {
            print_success(&format!(
                "Recommended for this profile: {}",
                recommended_mode.as_str()
            ));
        } else {
            print_info(
                "Custom / Advanced does not force a routing recommendation. Choose the mode that matches your own cost, control, and determinism goals.",
            );
        }

        let mode_options = &[
            "Primary only      - one model handles everything",
            "Cheap split       - send lighter work to a faster or cheaper auxiliary model",
            "Advisor executor  - use an auxiliary advisor model before heavier execution",
            "Policy            - keep or use advanced ordered routing rules",
        ];
        let mode_choice = select_one("Routing mode", mode_options).map_err(SetupError::Io)?;

        match mode_choice {
            0 => {
                self.settings.providers.smart_routing_enabled = false;
                self.settings.providers.routing_mode = crate::settings::RoutingMode::PrimaryOnly;
                self.remove_followup("routing-policy");
                print_success(
                    "Primary-only routing enabled. ThinClaw will use the primary model for every request.",
                );
                return Ok(());
            }
            3 => {
                self.settings.providers.smart_routing_enabled = true;
                self.settings.providers.routing_mode = crate::settings::RoutingMode::Policy;
                if self.settings.providers.policy_rules.is_empty() {
                    print_warning(
                        "No custom policy rules are configured yet. ThinClaw will keep your existing defaults until you add rules later.",
                    );
                    self.add_followup(super::FollowupDraft {
                        id: "routing-policy".to_string(),
                        title: "Add policy routing rules".to_string(),
                        category: crate::settings::OnboardingFollowupCategory::Provider,
                        status: crate::settings::OnboardingFollowupStatus::Optional,
                        instructions: "Policy mode was selected, but no ordered policy rules are configured yet.".to_string(),
                        action_hint: Some("Set `providers.policy_rules` later if you want deterministic rule-based routing.".to_string()),
                    });
                } else {
                    self.remove_followup("routing-policy");
                }
                print_success("Policy routing enabled.");
                return Ok(());
            }
            _ => {}
        }

        let routing_mode = if mode_choice == 2 {
            crate::settings::RoutingMode::AdvisorExecutor
        } else {
            crate::settings::RoutingMode::CheapSplit
        };
        let aux_label = if routing_mode == crate::settings::RoutingMode::AdvisorExecutor {
            "advisor"
        } else {
            "cheap"
        };

        let current = self
            .settings
            .providers
            .cheap_model
            .clone()
            .unwrap_or_default();
        if !current.is_empty() {
            let keep = confirm(
                &format!("Keep current {aux_label} model ({})?", current),
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
                    "{} routing enabled. {aux_label} model: {}",
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
                .then_with(|| Self::provider_display_name(a).cmp(&Self::provider_display_name(b)))
        });

        let provider_option_labels: Vec<String> = provider_choices
            .iter()
            .map(|slug| {
                let mut label = Self::provider_display_name(slug);
                if self.primary_provider_slug() == Some(slug.as_str()) {
                    label.push_str(" (current primary)");
                }
                if self.settings.providers.preferred_cheap_provider.as_deref()
                    == Some(slug.as_str())
                {
                    label.push_str(" (current cheap)");
                }
                label
            })
            .collect();
        let provider_option_refs: Vec<&str> =
            provider_option_labels.iter().map(String::as_str).collect();
        let provider_prompt = format!("{} model provider:", capitalize_first(aux_label));
        let provider_choice =
            select_one(&provider_prompt, &provider_option_refs).map_err(SetupError::Io)?;
        let cheap_provider_slug = provider_choices
            .get(provider_choice)
            .cloned()
            .ok_or_else(|| SetupError::Config("Invalid cheap provider selection".to_string()))?;

        let display_name = Self::provider_display_name(&cheap_provider_slug);

        let mut model_options = self.fetch_models_for_provider(&cheap_provider_slug).await;
        if model_options.is_empty()
            && let Some(default_model) = Self::provider_default_model(&cheap_provider_slug)
        {
            model_options.push((default_model.clone(), default_model));
        }
        let suggested_cheap = Self::suggested_cheap_model_for_provider(
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
                        format!("{} (recommended cheap default)", entry.1),
                    ),
                );
            } else {
                model_options.insert(
                    0,
                    (
                        suggested.clone(),
                        format!("{} (recommended cheap default)", suggested),
                    ),
                );
            }
        }

        let model_prompt = format!("Select the {aux_label} model:");
        let cheap_model_id = self.choose_model_from_list(&model_options, &model_prompt)?;
        self.settings.providers.smart_routing_enabled = true;
        self.settings.providers.routing_mode = routing_mode;
        self.set_preferred_cheap_slot_model(&cheap_provider_slug, cheap_model_id.clone());
        self.remove_followup("routing-policy");
        print_success(&format!(
            "{} routing enabled — {aux_label} model: {}/{} ({})",
            routing_mode.as_str(),
            cheap_provider_slug,
            cheap_model_id,
            display_name
        ));

        // ── Check if the cheap model's provider needs a separate API key ──
        // Parse provider slug from "provider/model" format.
        if let Some(cheap_provider_slug) = self
            .settings
            .providers
            .cheap_model
            .as_deref()
            .and_then(|spec| spec.split('/').next())
            .map(str::to_string)
        {
            // Determine the primary provider slug for comparison.
            let primary_slug = self.primary_provider_slug().unwrap_or("");

            // Only prompt for a key if the cheap provider differs from the primary.
            if !cheap_provider_slug.is_empty()
                && cheap_provider_slug != primary_slug
                && !matches!(
                    cheap_provider_slug.as_str(),
                    "ollama" | "llama_cpp" | "openai_compatible" | "bedrock"
                )
            {
                // Look up the cheap provider in the catalog.
                if let Some(endpoint) =
                    crate::config::provider_catalog::endpoint_for(&cheap_provider_slug)
                {
                    // Check if the API key is already available (env var, keychain, or secrets).
                    let has_provider_key = self
                        .has_provider_secret(endpoint.env_key_name, endpoint.secret_name)
                        .await;

                    if std::env::var(endpoint.env_key_name).is_ok() {
                        println!();
                        print_success(&format!(
                            "✓ {} API key found in environment ({}).",
                            endpoint.display_name, endpoint.env_key_name
                        ));
                    } else if has_provider_key {
                        println!();
                        print_success(&format!(
                            "✓ {} credentials already stored.",
                            endpoint.display_name
                        ));
                    } else {
                        // API key is missing — prompt the user.
                        println!();
                        print_info(&format!(
                            "The {} model uses a different provider than your primary.",
                            aux_label
                        ));
                        print_info(&format!(
                            "An API key for {} is required.",
                            endpoint.display_name
                        ));

                        self.setup_additional_api_key_provider(
                            &cheap_provider_slug,
                            endpoint.env_key_name,
                            endpoint.secret_name,
                            &format!("{} API key", endpoint.display_name),
                            &format!("https://console.{}", cheap_provider_slug),
                            endpoint.display_name,
                        )
                        .await?;
                    }
                } else {
                    // Provider not in catalog — warn but continue.
                    println!();
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

    /// Step 6: Fallback Providers (optional secondary providers for failover).
    ///
    /// Allows the user to add API keys for additional LLM providers so that
    /// the failover chain and agent-initiated model switching actually work.
    pub(super) async fn step_fallback_providers(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_info("ThinClaw can use multiple LLM providers for failover and cost control.");
        print_info("If your primary provider is down, it will automatically try fallbacks.");
        println!();

        if !confirm("Add a fallback provider?", false).map_err(SetupError::Io)? {
            print_info("No fallback providers configured. Primary-only mode is active.");
            return Ok(());
        }

        let primary_slug = self.primary_provider_slug().unwrap_or("").to_string();

        let catalog = crate::config::provider_catalog::catalog();
        // Build a list of providers excluding the primary and ollama (local).
        let mut available: Vec<(&str, &crate::config::provider_catalog::ProviderEndpoint)> =
            catalog
                .iter()
                .filter(|(slug, ep)| {
                    **slug != primary_slug
                        && !matches!(
                            ep.api_style,
                            crate::config::provider_catalog::ApiStyle::Ollama
                        )
                })
                .map(|(slug, ep)| (*slug, ep))
                .collect();
        available.sort_by_key(|(slug, _)| *slug);

        let mut fallback_slugs: Vec<String> = Vec::new();

        loop {
            println!();
            print_info("Available fallback providers:");
            for (i, (slug, ep)) in available.iter().enumerate() {
                // Check if key already exists
                let has_env = std::env::var(ep.env_key_name).is_ok();
                let has_saved = self
                    .has_provider_secret(ep.env_key_name, ep.secret_name)
                    .await;
                let status = if has_env || has_saved { " ✅" } else { "" };
                println!("  {}. {} ({}){status}", i + 1, ep.display_name, slug);
            }

            let choice =
                input("Select a provider by number, or type 'done'").map_err(SetupError::Io)?;
            if choice.trim().eq_ignore_ascii_case("done") || choice.trim().is_empty() {
                break;
            }

            let idx: usize = match choice.trim().parse::<usize>() {
                Ok(n) if n >= 1 && n <= available.len() => n - 1,
                _ => {
                    print_error("Invalid selection. Enter a number or 'done'.");
                    continue;
                }
            };

            let (slug, endpoint) = available[idx];

            // Check if key is already available
            let has_env = std::env::var(endpoint.env_key_name).is_ok();
            let has_saved = self
                .has_provider_secret(endpoint.env_key_name, endpoint.secret_name)
                .await;

            if has_env {
                print_success(&format!(
                    "✓ {} API key found in environment ({})",
                    endpoint.display_name, endpoint.env_key_name
                ));
            } else if has_saved {
                print_success(&format!(
                    "✓ {} credentials already stored",
                    endpoint.display_name
                ));
            } else {
                // Prompt for API key
                println!();
                print_info(&format!("Enter the {} API key:", endpoint.display_name));

                self.setup_additional_api_key_provider(
                    slug,
                    endpoint.env_key_name,
                    endpoint.secret_name,
                    &format!("{} API key", endpoint.display_name),
                    &format!("https://console.{slug}"),
                    endpoint.display_name,
                )
                .await?;
            }

            self.ensure_provider_enabled(slug);
            let discovered_primary = self
                .fetch_models_for_provider(slug)
                .await
                .into_iter()
                .map(|(id, _)| id)
                .next()
                .or_else(|| Self::provider_default_model(slug))
                .unwrap_or_else(|| endpoint.default_model.to_string());
            let cheap_default =
                Self::suggested_cheap_model_for_provider(slug, Some(discovered_primary.as_str()))
                    .unwrap_or_else(|| discovered_primary.clone());
            let slots = self
                .settings
                .providers
                .provider_models
                .entry(slug.to_string())
                .or_default();
            if slots.primary.is_none() {
                slots.primary = Some(discovered_primary.clone());
            }
            if slots.cheap.is_none() {
                slots.cheap = Some(cheap_default);
            }
            fallback_slugs.push(format!("{}/{}", slug, discovered_primary));
            print_success(&format!(
                "Added {} to the fallback chain.",
                endpoint.display_name
            ));

            if !confirm("Add another fallback provider?", false).map_err(SetupError::Io)? {
                break;
            }
        }

        if !fallback_slugs.is_empty() {
            let chain = fallback_slugs.join(",");
            self.settings.providers.fallback_chain = fallback_slugs;
            if self.settings.providers.smart_routing_enabled
                && self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
            {
                self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
            }
            println!();
            print_success(&format!("Fallback chain: {}", chain));
        } else {
            print_info("No fallback providers were added.");
        }

        Ok(())
    }
}

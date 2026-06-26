//! Top-level LLM onboarding steps invoked by the wizard flow dispatcher:
//! provider selection/review, model selection, embeddings, smart routing, and
//! fallback providers. These mirror the step entry points consumed from
//! `flow::execute_step`, so they are scoped to `pub(in crate::setup::wizard)`.

use thinclaw_app::{
    provider_default_model, provider_display_name, setup_quick_embeddings_defaults,
    suggested_cheap_model_for_provider,
};

use crate::setup::prompts::{
    confirm, input, optional_input, print_blank_line, print_error, print_info, print_success,
    print_warning, select_one,
};

use super::super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(in crate::setup::wizard) fn apply_quick_embeddings_defaults(&mut self) {
        let plan = setup_quick_embeddings_defaults(self.primary_provider_slug());
        self.settings.embeddings.enabled = plan.enabled;
        self.settings.embeddings.provider = plan.provider;
        self.settings.embeddings.model = plan.model;
        self.remove_followup("embeddings");
    }
    pub(in crate::setup::wizard) async fn step_provider_review_skip_auth(
        &mut self,
    ) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_warning(
            "Skip-auth mode keeps provider review credential-free. This step will not ask for API keys.",
        );
        crate::setup::prompts::print_blank_line();

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
                provider_display_name(&current)
            ));
            return Ok(());
        }

        print_info("Choose the provider ThinClaw should be ready for after onboarding.");
        crate::setup::prompts::print_blank_line();
        let options = &[
            "OpenRouter       - 200+ models, one API key (recommended easiest setup)",
            "Anthropic        - Claude models (add credentials later)",
            "OpenAI           - GPT models (add credentials later)",
            "Gemini           - Google Gemini via AI Studio (add credentials later)",
            "Tinfoil          - Private inference API (add credentials later)",
            "Ollama           - local models, no API key needed",
            "AWS Bedrock      - add credentials later",
            "llama.cpp        - local llama.cpp OpenAI-compatible server",
            "OpenAI-compatible - custom endpoint (base URL set now, auth optional later)",
        ];
        let choice = select_one("Provider", options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                self.settings.llm_backend = Some("openai_compatible".to_string());
                self.settings.openai_compatible_base_url =
                    Some("https://openrouter.ai/api/v1".to_string());
                self.settings.providers.primary = Some("openrouter".to_string());
                self.ensure_provider_enabled("openrouter");
                self.ensure_provider_slot_defaults("openrouter");
            }
            1 => {
                self.settings.llm_backend = Some("anthropic".to_string());
                self.settings.providers.primary = Some("anthropic".to_string());
                self.ensure_provider_enabled("anthropic");
                self.ensure_provider_slot_defaults("anthropic");
            }
            2 => {
                self.settings.llm_backend = Some("openai".to_string());
                self.settings.providers.primary = Some("openai".to_string());
                self.ensure_provider_enabled("openai");
                self.ensure_provider_slot_defaults("openai");
            }
            3 => {
                self.settings.llm_backend = Some("gemini".to_string());
                self.settings.providers.primary = Some("gemini".to_string());
                self.ensure_provider_enabled("gemini");
                self.ensure_provider_slot_defaults("gemini");
            }
            4 => {
                self.settings.llm_backend = Some("tinfoil".to_string());
                self.settings.providers.primary = Some("tinfoil".to_string());
                self.ensure_provider_enabled("tinfoil");
                self.ensure_provider_slot_defaults("tinfoil");
            }
            5 => {
                self.setup_ollama()?;
            }
            6 => {
                self.settings.llm_backend = Some("bedrock".to_string());
                self.settings.providers.primary = Some("bedrock".to_string());
                self.ensure_provider_enabled("bedrock");
                self.ensure_provider_slot_defaults("bedrock");
            }
            7 => {
                self.setup_llama_cpp()?;
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

    pub(in crate::setup::wizard) async fn step_inference_provider(
        &mut self,
    ) -> Result<(), SetupError> {
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
            crate::setup::prompts::print_blank_line();

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
        crate::setup::prompts::print_blank_line();

        let options = &[
            "OpenRouter       - 200+ models, one API key (recommended easiest setup)",
            "Anthropic        - Claude models (direct API key)",
            "OpenAI           - GPT models (direct API key)",
            "Gemini           - Google Gemini via AI Studio API key",
            "Tinfoil          - Private inference API key",
            "Ollama           - local models, no API key needed",
            "AWS Bedrock      - AWS-hosted models via native API key",
            "llama.cpp        - local llama.cpp OpenAI-compatible server",
            "OpenAI-compatible - custom endpoint (vLLM, LiteLLM, etc.)",
        ];

        let choice = select_one("Provider:", options).map_err(SetupError::Io)?;

        match choice {
            0 => self.setup_openrouter().await?,
            1 => self.setup_anthropic().await?,
            2 => self.setup_openai().await?,
            3 => self.setup_gemini().await?,
            4 => self.setup_tinfoil().await?,
            5 => self.setup_ollama()?,
            6 => self.setup_bedrock().await?,
            7 => self.setup_llama_cpp()?,
            8 => self.setup_openai_compatible().await?,
            _ => return Err(SetupError::Config("Invalid provider selection".to_string())),
        }

        Ok(())
    }

    /// Step 4: Model selection.
    ///
    /// Branches on the selected LLM backend and fetches models from the
    /// appropriate provider API, with static defaults as fallback.
    pub(in crate::setup::wizard) async fn step_model_selection(
        &mut self,
    ) -> Result<(), SetupError> {
        self.print_ai_stack_summary();

        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            crate::setup::prompts::print_blank_line();

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
            && let Some(default_model) = provider_default_model(&provider_slug)
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
    pub(in crate::setup::wizard) fn step_embeddings(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_info("Embeddings turn on semantic search in workspace memory.");
        crate::setup::prompts::print_blank_line();

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
                super::super::OnboardingProfile::LocalAndPrivate => {
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
                super::super::OnboardingProfile::CustomAdvanced => {
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
        self.add_followup(super::super::FollowupDraft {
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

    pub(in crate::setup::wizard) async fn step_smart_routing(&mut self) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        if self.is_quick_setup() {
            self.settings.providers.smart_routing_enabled = true;
            self.settings.providers.routing_mode = crate::settings::RoutingMode::AdvisorExecutor;
            print_info(
                "Quick setup uses advisor/executor routing by default: the primary model acts as the advisor and the fast model acts as the executor.",
            );
            print_info(
                "Choose the executor model ThinClaw should use for regular execution before escalating to the advisor.",
            );
            crate::setup::prompts::print_blank_line();
            return self
                .configure_routed_secondary_model(
                    crate::settings::RoutingMode::AdvisorExecutor,
                    "executor",
                    "executor",
                )
                .await;
        }

        print_info("Choose how ThinClaw should split work across models.");
        print_info("You can stay on one model, split cheaper work to a faster auxiliary model,");
        print_info(
            "or use advisor/executor mode so a fast executor can consult the primary advisor when needed.",
        );
        crate::setup::prompts::print_blank_line();

        let recommended_mode = match self.selected_profile {
            super::super::OnboardingProfile::LocalAndPrivate => {
                Some(crate::settings::RoutingMode::PrimaryOnly)
            }
            super::super::OnboardingProfile::BuilderAndCoding => {
                Some(crate::settings::RoutingMode::AdvisorExecutor)
            }
            super::super::OnboardingProfile::Balanced
            | super::super::OnboardingProfile::ChannelFirst
            | super::super::OnboardingProfile::RemoteServer
            | super::super::OnboardingProfile::PiOsLite64 => {
                Some(crate::settings::RoutingMode::CheapSplit)
            }
            super::super::OnboardingProfile::CustomAdvanced => None,
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
                    self.add_followup(super::super::FollowupDraft {
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
        let (slot_label, slot_prompt) =
            if routing_mode == crate::settings::RoutingMode::AdvisorExecutor {
                ("executor", "executor")
            } else {
                ("fast", "fast")
            };
        self.configure_routed_secondary_model(routing_mode, slot_label, slot_prompt)
            .await
    }

    /// Step 6: Fallback Providers (optional secondary providers for failover).
    ///
    /// Allows the user to add API keys for additional LLM providers so that
    /// the failover chain and agent-initiated model switching actually work.
    pub(in crate::setup::wizard) async fn step_fallback_providers(
        &mut self,
    ) -> Result<(), SetupError> {
        self.print_ai_stack_summary();
        print_info("ThinClaw can use multiple LLM providers for failover and cost control.");
        print_info("If your primary provider is down, it will automatically try fallbacks.");
        crate::setup::prompts::print_blank_line();

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
                .map(|(slug, ep)| (slug.as_str(), ep))
                .collect();
        available.sort_by_key(|(slug, _): &(&str, _)| *slug);

        let mut fallback_slugs: Vec<String> = Vec::new();

        loop {
            print_blank_line();
            print_info("Available fallback providers:");
            for (i, (slug, ep)) in available.iter().enumerate() {
                // Check if key already exists
                let has_env = std::env::var(&ep.env_key_name).is_ok();
                let has_saved = self
                    .has_provider_secret(&ep.env_key_name, &ep.secret_name)
                    .await;
                let status = if has_env || has_saved { " ✅" } else { "" };
                print_info(&format!(
                    "{}. {} ({}){status}",
                    i + 1,
                    ep.display_name,
                    slug
                ));
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
            let has_env = std::env::var(&endpoint.env_key_name).is_ok();
            let has_saved = self
                .has_provider_secret(&endpoint.env_key_name, &endpoint.secret_name)
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
                crate::setup::prompts::print_blank_line();
                print_info(&format!("Enter the {} API key:", endpoint.display_name));

                self.setup_additional_api_key_provider(
                    slug,
                    &endpoint.env_key_name,
                    &endpoint.secret_name,
                    &format!("{} API key", endpoint.display_name),
                    &format!("https://console.{slug}"),
                    &endpoint.display_name,
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
                .or_else(|| provider_default_model(slug))
                .unwrap_or_else(|| endpoint.default_model.to_string());
            let cheap_default =
                suggested_cheap_model_for_provider(slug, Some(discovered_primary.as_str()))
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
            crate::setup::prompts::print_blank_line();
            print_success(&format!("Fallback chain: {}", chain));
        } else {
            print_info("No fallback providers were added.");
        }

        Ok(())
    }
}

//! LLM wizard steps: inference provider, model selection, smart routing, fallback, embeddings.

use secrecy::{ExposeSecret, SecretString};

use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, secret_input,
    select_one,
};

use super::{SetupError, SetupWizard};
use super::helpers::{
    fetch_anthropic_models, fetch_ollama_models, fetch_openai_models, mask_api_key,
};

impl SetupWizard {
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
                    "ollama" => "Ollama (local)",
                    "openai_compatible" => "OpenAI-compatible endpoint",
                    other => other,
                }
            };
            print_info(&format!("Current provider: {}", display));
            println!();

            let is_known = matches!(
                current.as_str(),
                "anthropic" | "openai" | "ollama" | "openai_compatible"
            );

            if is_known && confirm("Keep current provider?", true).map_err(SetupError::Io)? {
                // Still run the auth sub-flow in case they need to update keys
                if is_openrouter {
                    return self.setup_openrouter().await;
                }
                match current.as_str() {
                    "anthropic" => return self.setup_anthropic().await,
                    "openai" => return self.setup_openai().await,
                    "ollama" => return self.setup_ollama(),
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

        print_info("Select your inference provider:");
        println!();

        let options = &[
            "Anthropic        - Claude models (direct API key)",
            "OpenAI           - GPT models (direct API key)",
            "Ollama           - local models, no API key needed",
            "OpenRouter       - 200+ models via single API key",
            "OpenAI-compatible - custom endpoint (vLLM, LiteLLM, etc.)",
        ];

        let choice = select_one("Provider:", options).map_err(SetupError::Io)?;

        match choice {
            0 => self.setup_anthropic().await?,
            1 => self.setup_openai().await?,
            2 => self.setup_ollama()?,
            3 => self.setup_openrouter().await?,
            4 => self.setup_openai_compatible().await?,
            _ => return Err(SetupError::Config("Invalid provider selection".to_string())),
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
            other => other,
        });

        self.settings.llm_backend = Some(backend.to_string());
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
                self.llm_api_key = Some(SecretString::from(existing));
                print_success(&format!("{display_name} configured (from env)"));
                return Ok(());
            }
        }

        println!();
        print_info(&format!("Get your API key from: {hint_url}"));
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
                "Secrets not available. Set {env_var} in your environment."
            ));
        }

        // Cache key in memory for model fetching later in the wizard
        self.llm_api_key = Some(SecretString::from(key_str.to_string()));

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

        print_success(&format!("Ollama configured ({})", url));
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
                    print_info("Secrets not available. Set LLM_API_KEY in your environment.");
                }
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
        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            println!();

            let options = ["Keep current model", "Change model"];
            let choice =
                select_one("What would you like to do?", &options).map_err(SetupError::Io)?;

            if choice == 0 {
                print_success(&format!("Keeping {}", current));
                return Ok(());
            }
        }

        let backend = self
            .settings
            .llm_backend
            .as_deref()
            .unwrap_or("openai_compatible");

        match backend {
            "anthropic" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_anthropic_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "openai" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_openai_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "ollama" => {
                let base_url = self
                    .settings
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                let models = fetch_ollama_models(base_url).await;
                if models.is_empty() {
                    print_info("No models found. Pull one first: ollama pull llama3");
                }
                self.select_from_model_list(&models)?;
            }
            "openai_compatible" => {
                // No standard API for listing models on arbitrary endpoints
                let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                    .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model name is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
            _ => {
                // Generic fallback: ask for model name manually
                let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                    .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model name is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
        }

        Ok(())
    }

    /// Present a model list to the user, with a "Custom model ID" escape hatch.
    ///
    /// Each entry is `(model_id, display_label)`.
    pub(super) fn select_from_model_list(&mut self, models: &[(String, String)]) -> Result<(), SetupError> {
        println!("Available models:");
        println!();

        let mut options: Vec<&str> = models.iter().map(|(_, desc)| desc.as_str()).collect();
        options.push("Custom model ID");

        let choice = select_one("Select a model:", &options).map_err(SetupError::Io)?;

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

        self.settings.selected_model = Some(selected.clone());
        print_success(&format!("Selected {}", selected));
        Ok(())
    }

    /// Step 6: Embeddings configuration.
    pub(super) fn step_embeddings(&mut self) -> Result<(), SetupError> {
        print_info("Embeddings enable semantic search in your workspace memory.");
        println!();

        if !confirm("Enable semantic search?", true).map_err(SetupError::Io)? {
            self.settings.embeddings.enabled = false;
            print_info("Embeddings disabled. Workspace will use keyword search only.");
            return Ok(());
        }

        let backend = self
            .settings
            .llm_backend
            .as_deref()
            .unwrap_or("openai_compatible");
        let has_openai_key = std::env::var("OPENAI_API_KEY").is_ok()
            || (backend == "openai" && self.llm_api_key.is_some());

        // If the LLM backend is OpenAI and we already have a key, default to OpenAI embeddings
        if backend == "openai" && has_openai_key {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "openai".to_string();
            self.settings.embeddings.model = "text-embedding-3-small".to_string();
            print_success("Embeddings enabled via OpenAI (using existing API key)");
            return Ok(());
        }

        if !has_openai_key {
            print_info("No OPENAI_API_KEY found for embeddings.");
            print_info("Set OPENAI_API_KEY in your environment to enable embeddings.");
            self.settings.embeddings.enabled = false;
            return Ok(());
        }

        let options = &["OpenAI (requires API key)", "Ollama (local, no API key)"];

        let choice = select_one("Select embeddings provider:", options).map_err(SetupError::Io)?;

        match choice {
            1 => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "ollama".to_string();
                self.settings.embeddings.model = "nomic-embed-text".to_string();
                print_success("Embeddings enabled via Ollama");
            }
            _ => {
                if !has_openai_key {
                    print_info("OPENAI_API_KEY not set in environment.");
                    print_info("Add it to your .env file or environment to enable embeddings.");
                }
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "openai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings configured for OpenAI");
            }
        }

        Ok(())
    }

    pub(super) async fn step_smart_routing(&mut self) -> Result<(), SetupError> {
        print_info("Smart Routing can use a cheaper/faster model for lightweight tasks");
        print_info("(e.g., routing decisions, heartbeat checks, prompt evaluation).");
        print_info("The primary model is still used for complex conversations.");
        println!();

        if !confirm("Configure a cheap model for smart routing?", false).map_err(SetupError::Io)? {
            print_info("Smart routing disabled — all tasks use the primary model.");
            return Ok(());
        }

        println!();
        print_info("Format: provider/model (e.g., \"groq/llama-3.1-8b-instant\",");
        print_info("\"openai/gpt-4o-mini\", \"anthropic/claude-3-5-haiku-20241022\")");

        let current = self.settings.providers.cheap_model.as_deref().unwrap_or("");
        let cheap_model = if current.is_empty() {
            input("Cheap model").map_err(SetupError::Io)?
        } else {
            let keep = confirm(&format!("Keep current cheap model ({})?", current), true)
                .map_err(SetupError::Io)?;
            if keep {
                current.to_string()
            } else {
                input("Cheap model").map_err(SetupError::Io)?
            }
        };

        if cheap_model.is_empty() {
            print_info("No cheap model set — smart routing disabled.");
            return Ok(());
        }

        self.settings.providers.cheap_model = Some(cheap_model.clone());
        print_success(&format!(
            "Smart routing enabled — cheap model: {}",
            cheap_model
        ));

        // ── Check if the cheap model's provider needs a separate API key ──
        // Parse provider slug from "provider/model" format.
        if let Some(cheap_provider_slug) = cheap_model.split('/').next() {
            // Determine the primary provider slug for comparison.
            let primary_slug = self
                .settings
                .llm_backend
                .as_deref()
                .unwrap_or("");

            // Only prompt for a key if the cheap provider differs from the primary.
            if !cheap_provider_slug.is_empty()
                && cheap_provider_slug != primary_slug
                && cheap_provider_slug != "ollama"
            {
                // Look up the cheap provider in the catalog.
                if let Some(endpoint) =
                    crate::config::provider_catalog::endpoint_for(cheap_provider_slug)
                {
                    // Check if the API key is already available (env var, keychain, or secrets).
                    let has_env_key = std::env::var(endpoint.env_key_name).is_ok();
                    let has_keychain_key = crate::secrets::keychain::get_api_key(
                        endpoint.secret_name,
                    )
                    .await
                    .is_some();

                    if has_env_key {
                        println!();
                        print_success(&format!(
                            "✓ {} API key found in environment ({}).",
                            endpoint.display_name, endpoint.env_key_name
                        ));
                    } else if has_keychain_key {
                        println!();
                        print_success(&format!(
                            "✓ {} API key found in OS keychain.",
                            endpoint.display_name
                        ));
                    } else {
                        // API key is missing — prompt the user.
                        println!();
                        print_info(&format!(
                            "The cheap model uses {} — a different provider than your primary.",
                            endpoint.display_name
                        ));
                        print_info(&format!(
                            "An API key for {} is required.",
                            endpoint.display_name
                        ));

                        self.setup_api_key_provider(
                            cheap_provider_slug,
                            endpoint.env_key_name,
                            endpoint.secret_name,
                            &format!("{} API key", endpoint.display_name),
                            &format!(
                                "https://console.{}",
                                cheap_provider_slug
                            ),
                            Some(endpoint.display_name),
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
                    print_info(&format!(
                        "Make sure the API key is set via the appropriate environment variable."
                    ));
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
        print_info("ThinClaw can use multiple LLM providers for failover and cost optimization.");
        print_info("If your primary provider is down, it will automatically try fallbacks.");
        println!();

        if !confirm("Add a fallback provider?", false).map_err(SetupError::Io)? {
            print_info("No fallback providers configured — primary-only mode.");
            return Ok(());
        }

        let primary_slug = self
            .settings
            .llm_backend
            .as_deref()
            .unwrap_or("")
            .to_string();

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
            print_info("Available providers:");
            for (i, (slug, ep)) in available.iter().enumerate() {
                // Check if key already exists
                let has_env =
                    std::env::var(ep.env_key_name).is_ok();
                let has_keychain =
                    crate::secrets::keychain::get_api_key(ep.secret_name)
                        .await
                        .is_some();
                let status = if has_env || has_keychain {
                    " ✅"
                } else {
                    ""
                };
                println!(
                    "  {}. {} ({}){status}",
                    i + 1,
                    ep.display_name,
                    slug
                );
            }

            let choice = input("Select provider (number, or 'done')").map_err(SetupError::Io)?;
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
            let has_keychain = crate::secrets::keychain::get_api_key(endpoint.secret_name)
                .await
                .is_some();

            if has_env {
                print_success(&format!(
                    "✓ {} API key found in environment ({})",
                    endpoint.display_name, endpoint.env_key_name
                ));
                fallback_slugs.push(format!("{}/{}", slug, endpoint.default_model));
            } else if has_keychain {
                print_success(&format!(
                    "✓ {} API key found in OS keychain",
                    endpoint.display_name
                ));
                fallback_slugs.push(format!("{}/{}", slug, endpoint.default_model));
            } else {
                // Prompt for API key
                println!();
                print_info(&format!(
                    "Enter your {} API key:",
                    endpoint.display_name
                ));

                let key = secret_input(&format!("{} API key", endpoint.display_name))
                    .map_err(SetupError::Io)?;

                if key.expose_secret().is_empty() {
                    print_info("Skipped — no key entered.");
                    continue;
                }

                // Save to secrets store
                if let Ok(ctx) = self.init_secrets_context().await {
                    match ctx.save_secret(endpoint.secret_name, &key).await {
                        Ok(()) => {
                            print_success(&format!(
                                "{} key encrypted and saved",
                                endpoint.display_name
                            ));
                        }
                        Err(e) => {
                            print_error(&format!("Failed to save key: {e}"));
                            continue;
                        }
                    }
                } else {
                    // No secrets store — fall back to env var hint
                    print_info(&format!(
                        "Set {} in your environment to use {}.",
                        endpoint.env_key_name, endpoint.display_name
                    ));
                    continue;
                }

                fallback_slugs.push(format!("{}/{}", slug, endpoint.default_model));
            }

            print_success(&format!("Added {} to fallback chain.", endpoint.display_name));

            if !confirm("Add another fallback provider?", false).map_err(SetupError::Io)? {
                break;
            }
        }

        if !fallback_slugs.is_empty() {
            let chain = fallback_slugs.join(",");
            self.settings.providers.fallback_chain = fallback_slugs;
            println!();
            print_success(&format!("Fallback chain: {}", chain));
        } else {
            print_info("No fallback providers added.");
        }

        Ok(())
    }
}

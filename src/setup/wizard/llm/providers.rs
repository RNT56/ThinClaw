//! Provider credential, slot, and per-provider setup wizard logic.
//!
//! Owns provider enablement/slot bookkeeping, external-auth-sync offers,
//! secret resolution, and the per-provider credential setup flows
//! (`setup_anthropic`, `setup_openai`, ...). These methods are shared across
//! the `llm` submodules (and a few are reached from sibling wizard steps), so
//! their cross-module surface is scoped to `pub(in crate::setup::wizard)`.

use secrecy::{ExposeSecret, SecretString};
use thinclaw_app::{
    SetupProviderSlotDefaultsInput, provider_default_model, setup_provider_slot_defaults,
    suggested_cheap_model_for_provider,
};

use crate::setup::prompts::{
    confirm, input, optional_input, print_info, print_success, secret_input, select_one,
};

use super::super::helpers::mask_api_key;
use super::super::{SetupError, SetupWizard};

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

    pub(in crate::setup::wizard) fn ensure_provider_enabled(&mut self, provider_slug: &str) {
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

    pub(in crate::setup::wizard) fn ensure_provider_slot_defaults(&mut self, provider_slug: &str) {
        let current_primary = if self.primary_provider_slug() == Some(provider_slug) {
            self.settings
                .providers
                .primary_model
                .clone()
                .or_else(|| self.settings.selected_model.clone())
        } else {
            None
        };
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();
        let plan = setup_provider_slot_defaults(&SetupProviderSlotDefaultsInput {
            provider_slug: provider_slug.to_string(),
            current_primary_model: current_primary,
            existing_primary: slots.primary.clone(),
            existing_cheap: slots.cheap.clone(),
        });

        if slots.primary.is_none() {
            slots.primary = plan.primary;
        }
        if slots.cheap.is_none() {
            slots.cheap = plan.cheap;
        }
    }

    pub(in crate::setup::wizard) fn set_primary_slot_model(
        &mut self,
        provider_slug: &str,
        model: String,
    ) {
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();
        slots.primary = Some(model.clone());
        if slots.cheap.is_none() {
            slots.cheap = suggested_cheap_model_for_provider(provider_slug, Some(&model))
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

    pub(in crate::setup::wizard) fn set_preferred_cheap_slot_model(
        &mut self,
        provider_slug: &str,
        model: String,
    ) {
        let slots = self
            .settings
            .providers
            .provider_models
            .entry(provider_slug.to_string())
            .or_default();
        if slots.primary.is_none() {
            slots.primary = provider_default_model(provider_slug);
        }
        slots.cheap = Some(model.clone());
        self.settings.providers.preferred_cheap_provider = Some(provider_slug.to_string());
        self.settings.providers.cheap_model = Some(format!("{provider_slug}/{model}"));
        self.ensure_provider_enabled(provider_slug);
    }

    pub(in crate::setup::wizard) fn primary_provider_slug(&self) -> Option<&str> {
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

    pub(in crate::setup::wizard) async fn has_saved_secret(&mut self, secret_name: &str) -> bool {
        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.secret_exists(secret_name).await
        } else {
            false
        }
    }

    pub(in crate::setup::wizard) async fn has_provider_secret(
        &mut self,
        env_var: &str,
        secret_name: &str,
    ) -> bool {
        if std::env::var(env_var).is_ok() {
            return true;
        }

        #[cfg(target_os = "macos")]
        if self.has_saved_secret(secret_name).await {
            return true;
        }

        #[cfg(not(target_os = "macos"))]
        if crate::platform::secure_store::get_api_key(secret_name)
            .await
            .is_some()
        {
            return true;
        }

        #[cfg(not(target_os = "macos"))]
        if self.has_saved_secret(secret_name).await {
            return true;
        }

        false
    }

    pub(in crate::setup::wizard) async fn resolve_provider_secret_value(
        &mut self,
        env_var: &str,
        secret_name: &str,
    ) -> Option<String> {
        if let Ok(value) = std::env::var(env_var)
            && !value.trim().is_empty()
        {
            return Some(value);
        }

        #[cfg(target_os = "macos")]
        if let Ok(ctx) = self.init_secrets_context().await
            && let Ok(secret) = ctx.get_secret(secret_name).await
        {
            let value = secret.expose_secret().trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }

        #[cfg(not(target_os = "macos"))]
        if let Some(value) = crate::platform::secure_store::get_api_key(secret_name).await
            && !value.trim().is_empty()
        {
            return Some(value);
        }

        #[cfg(not(target_os = "macos"))]
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

    async fn has_any_remote_provider_credentials(&mut self) -> bool {
        for (slug, env_var, secret_name) in [
            ("anthropic", "ANTHROPIC_API_KEY", "llm_anthropic_api_key"),
            ("openai", "OPENAI_API_KEY", "llm_openai_api_key"),
            ("gemini", "GEMINI_API_KEY", "gemini"),
            ("tinfoil", "TINFOIL_API_KEY", "llm_tinfoil_api_key"),
            ("openrouter", "OPENROUTER_API_KEY", "llm_compatible_api_key"),
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

    pub(in crate::setup::wizard) async fn ensure_onboarding_provider_api_key(
        &mut self,
    ) -> Result<(), SetupError> {
        if self.has_any_remote_provider_credentials().await {
            return Ok(());
        }

        crate::setup::prompts::print_blank_line();
        print_info(
            "ThinClaw onboarding requires at least one authenticated remote provider so routing and failover have a real remote backend available.",
        );
        print_info(
            "You can still keep Ollama, llama.cpp, or a no-auth compatible endpoint as primary, but please add one authenticated remote provider now.",
        );
        crate::setup::prompts::print_blank_line();

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
    pub(in crate::setup::wizard) async fn setup_anthropic(&mut self) -> Result<(), SetupError> {
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
    pub(in crate::setup::wizard) async fn setup_openai(&mut self) -> Result<(), SetupError> {
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

    pub(in crate::setup::wizard) async fn setup_gemini(&mut self) -> Result<(), SetupError> {
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

    pub(in crate::setup::wizard) async fn setup_tinfoil(&mut self) -> Result<(), SetupError> {
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
    pub(in crate::setup::wizard) async fn setup_api_key_provider(
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

        crate::setup::prompts::print_blank_line();
        print_info(&format!("Get your API key here: {hint_url}"));
        crate::setup::prompts::print_blank_line();

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

    pub(in crate::setup::wizard) async fn setup_additional_api_key_provider(
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

        crate::setup::prompts::print_blank_line();
        print_info(&format!("Get your API key here: {hint_url}"));
        crate::setup::prompts::print_blank_line();

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
    pub(in crate::setup::wizard) fn setup_ollama(&mut self) -> Result<(), SetupError> {
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
    pub(in crate::setup::wizard) async fn setup_bedrock(&mut self) -> Result<(), SetupError> {
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
            crate::setup::prompts::print_blank_line();
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
    pub(in crate::setup::wizard) fn setup_llama_cpp(&mut self) -> Result<(), SetupError> {
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
    /// Sets the base URL to `https://openrouter.ai/api/v1` and routes
    /// through the catalog-native `OPENROUTER_API_KEY` env var. The primary
    /// provider is set to `"openrouter"` (not `"openai_compatible"`) so the
    /// routing system can track it distinctly.
    pub(in crate::setup::wizard) async fn setup_openrouter(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai_compatible".to_string());
        self.settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        self.sync_primary_provider_settings("openrouter");
        self.ensure_provider_slot_defaults("openrouter");

        self.setup_api_key_provider(
            "openrouter",
            "OPENROUTER_API_KEY",
            "llm_compatible_api_key",
            "OpenRouter API key",
            "https://openrouter.ai/settings/keys",
            Some("OpenRouter"),
        )
        .await
    }

    /// OpenAI-compatible provider setup: base URL + optional API key.
    pub(in crate::setup::wizard) async fn setup_openai_compatible(
        &mut self,
    ) -> Result<(), SetupError> {
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
}

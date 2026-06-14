//! Runtime configuration support shared by ThinClaw crates.

use std::fmt::Display;

use thinclaw_types::error::ConfigError;

pub mod agent;
pub mod builder;
pub mod channel_config;
pub mod channels;
pub mod comfyui;
pub mod database;
pub mod desktop_autonomy;
pub mod embeddings;
pub mod experiments;
pub mod formats;
pub mod heartbeat;
pub mod helpers;
pub mod hygiene;
pub mod llm;
pub mod mdns_discovery;
pub mod model_compat;
pub mod network_modes;
pub mod provider_catalog;
pub mod repo_projects;
pub mod routines;
pub mod safety;
pub mod sandbox;
pub mod secrets;
pub mod skills;
pub mod tunnel;
pub mod wasm;
pub mod watcher;
pub mod webchat;

pub use llm::{
    AnthropicDirectConfig, BedrockDirectConfig, GeminiDirectConfig, LlamaCppConfig, LlmBackend,
    LlmConfig, OllamaConfig, OpenAiCompatibleConfig, OpenAiDirectConfig, ReliabilityConfig,
    TinfoilConfig,
};

pub fn setting_not_found_message(key: impl Display) -> String {
    format!("Setting '{key}' not found")
}

pub fn provider_secret_legacy_env_aliases(env_key: &str) -> &'static [&'static str] {
    match env_key {
        "OPENROUTER_API_KEY" => &["LLM_API_KEY"],
        "BEDROCK_API_KEY" => &["AWS_BEARER_TOKEN_BEDROCK"],
        _ => &[],
    }
}

pub fn resolve_provider_secret_legacy_env_alias(
    env_key: &str,
) -> Result<Option<String>, ConfigError> {
    for alias in provider_secret_legacy_env_aliases(env_key) {
        if let Some(value) = helpers::optional_env(alias)?
            && !value.trim().is_empty()
        {
            return Ok(Some(value));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        provider_secret_legacy_env_aliases, resolve_provider_secret_legacy_env_alias,
        setting_not_found_message,
    };
    use crate::helpers::{
        clear_bridge_vars, clear_injected_vars_for_tests, inject_bridge_vars, lock_env,
    };

    #[test]
    fn setting_not_found_message_preserves_api_text() {
        assert_eq!(
            setting_not_found_message("theme"),
            "Setting 'theme' not found"
        );
    }

    #[test]
    fn provider_secret_legacy_env_aliases_cover_catalog_compat_keys() {
        assert_eq!(
            provider_secret_legacy_env_aliases("OPENROUTER_API_KEY"),
            &["LLM_API_KEY"]
        );
        assert_eq!(
            provider_secret_legacy_env_aliases("BEDROCK_API_KEY"),
            &["AWS_BEARER_TOKEN_BEDROCK"]
        );
        assert!(provider_secret_legacy_env_aliases("OPENAI_API_KEY").is_empty());
    }

    #[test]
    fn resolve_provider_secret_legacy_env_alias_reads_overlay() {
        let _guard = lock_env();
        clear_bridge_vars();
        clear_injected_vars_for_tests();
        inject_bridge_vars(HashMap::from([(
            "LLM_API_KEY".to_string(),
            "legacy-openrouter-key".to_string(),
        )]));

        let value = resolve_provider_secret_legacy_env_alias("OPENROUTER_API_KEY")
            .expect("alias lookup should parse");
        assert_eq!(value.as_deref(), Some("legacy-openrouter-key"));

        clear_bridge_vars();
        clear_injected_vars_for_tests();
    }

    #[test]
    fn resolve_provider_secret_legacy_env_alias_ignores_blank_values() {
        let _guard = lock_env();
        clear_bridge_vars();
        clear_injected_vars_for_tests();
        inject_bridge_vars(HashMap::from([(
            "AWS_BEARER_TOKEN_BEDROCK".to_string(),
            "   ".to_string(),
        )]));

        let value = resolve_provider_secret_legacy_env_alias("BEDROCK_API_KEY")
            .expect("alias lookup should parse");
        assert!(value.is_none());

        clear_bridge_vars();
        clear_injected_vars_for_tests();
    }
}

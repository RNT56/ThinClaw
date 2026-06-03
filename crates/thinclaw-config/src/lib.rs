//! Runtime configuration support shared by ThinClaw crates.

use std::fmt::Display;

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

#[cfg(test)]
mod tests {
    use super::setting_not_found_message;

    #[test]
    fn setting_not_found_message_preserves_api_text() {
        assert_eq!(
            setting_not_found_message("theme"),
            "Setting 'theme' not found"
        );
    }
}

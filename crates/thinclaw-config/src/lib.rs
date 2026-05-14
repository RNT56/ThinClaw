//! Runtime configuration support shared by ThinClaw crates.

pub mod formats;
pub mod helpers;
pub mod llm;
pub mod mdns_discovery;
pub mod model_compat;
pub mod network_modes;
pub mod provider_catalog;
pub mod watcher;

pub use llm::{
    AnthropicDirectConfig, BedrockDirectConfig, GeminiDirectConfig, LlamaCppConfig, LlmBackend,
    LlmConfig, OllamaConfig, OpenAiCompatibleConfig, OpenAiDirectConfig, ReliabilityConfig,
    TinfoilConfig,
};

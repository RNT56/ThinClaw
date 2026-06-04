use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ModelCategory {
    Chat,
    Embedding,
    Tts,
    Stt,
    Diffusion,
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelPricing {
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub per_image: Option<f64>,
    pub per_minute: Option<f64>,
    pub per_1k_chars: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelCapabilitySet {
    pub streaming: bool,
    pub tools: bool,
    pub vision: bool,
    pub thinking: bool,
    pub json_mode: bool,
    pub system_prompt: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub provider_name: String,
    pub category: ModelCategory,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_streaming: bool,
    #[serde(default)]
    pub capabilities: ModelCapabilitySet,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    #[serde(default)]
    pub embedding_dimensions: Option<u32>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderDiscoveryResult {
    pub provider: String,
    pub models: Vec<ModelDescriptor>,
    pub from_cache: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelDiscoveryResult {
    pub providers: Vec<ProviderDiscoveryResult>,
    pub total_models: u32,
    #[serde(default)]
    pub errors: Vec<String>,
}

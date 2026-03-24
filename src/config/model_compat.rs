//! Full model compatibility fields.
//!
//! Exposes model-specific compatibility metadata in the config schema,
//! including context window, feature support, and pricing info.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Full model compatibility descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCompat {
    /// Model identifier (e.g., "gpt-4o", "claude-3.5-sonnet").
    pub model_id: String,
    /// Display name.
    pub display_name: String,
    /// Provider.
    pub provider: String,
    /// Context window (tokens).
    pub context_window: u32,
    /// Max output tokens.
    pub max_output_tokens: u32,
    /// Whether the model supports vision (images).
    pub supports_vision: bool,
    /// Whether the model supports tool use.
    pub supports_tools: bool,
    /// Whether the model supports streaming.
    pub supports_streaming: bool,
    /// Whether the model supports extended thinking.
    pub supports_thinking: bool,
    /// Whether the model supports JSON mode.
    pub supports_json_mode: bool,
    /// Whether the model supports system prompts.
    pub supports_system_prompt: bool,
    /// Input price per million tokens (USD).
    pub input_price_per_m: Option<f64>,
    /// Output price per million tokens (USD).
    pub output_price_per_m: Option<f64>,
    /// Additional capabilities.
    pub capabilities: HashMap<String, String>,
}

impl ModelCompat {
    /// Whether this model is a frontier (top-tier) model.
    pub fn is_frontier(&self) -> bool {
        self.context_window >= 100_000 && self.supports_tools && self.supports_streaming
    }

    /// Whether this model is "cheap" (under $1/M input).
    pub fn is_budget(&self) -> bool {
        self.input_price_per_m.map(|p| p < 1.0).unwrap_or(true)
    }

    /// Total cost estimate for a conversation (input + output tokens).
    pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> Option<f64> {
        let input_price = self.input_price_per_m?;
        let output_price = self.output_price_per_m?;

        Some(
            (input_tokens as f64 / 1_000_000.0) * input_price
                + (output_tokens as f64 / 1_000_000.0) * output_price,
        )
    }
}

/// Build the known model compatibility database.
pub fn known_models() -> Vec<ModelCompat> {
    vec![
        ModelCompat {
            model_id: "gpt-4o".into(),
            display_name: "GPT-4o".into(),
            provider: "openai".into(),
            context_window: 128_000,
            max_output_tokens: 16_384,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: false,
            supports_json_mode: true,
            supports_system_prompt: true,
            input_price_per_m: Some(2.50),
            output_price_per_m: Some(10.00),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "claude-3-5-sonnet-20241022".into(),
            display_name: "Claude 3.5 Sonnet".into(),
            provider: "anthropic".into(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: true,
            supports_json_mode: false,
            supports_system_prompt: true,
            input_price_per_m: Some(3.00),
            output_price_per_m: Some(15.00),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "gemini-2.0-flash".into(),
            display_name: "Gemini 2.0 Flash".into(),
            provider: "google".into(),
            context_window: 1_000_000,
            max_output_tokens: 8_192,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: true,
            supports_json_mode: true,
            supports_system_prompt: true,
            input_price_per_m: Some(0.10),
            output_price_per_m: Some(0.40),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "pi-ai".into(),
            display_name: "Pi AI".into(),
            provider: "inflection".into(),
            context_window: 8_000,
            max_output_tokens: 2_048,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: true,
            supports_thinking: false,
            supports_json_mode: false,
            supports_system_prompt: false,
            input_price_per_m: None,
            output_price_per_m: None,
            capabilities: HashMap::new(),
        },
    ]
}

/// Look up a model by ID.
pub fn find_model(model_id: &str) -> Option<ModelCompat> {
    known_models()
        .into_iter()
        .find(|m| m.model_id == model_id || m.display_name.eq_ignore_ascii_case(model_id))
}

/// List models by provider.
pub fn models_by_provider(provider: &str) -> Vec<ModelCompat> {
    known_models()
        .into_iter()
        .filter(|m| m.provider == provider)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_models_not_empty() {
        assert!(!known_models().is_empty());
    }

    #[test]
    fn test_find_model() {
        let model = find_model("gpt-4o");
        assert!(model.is_some());
        assert_eq!(model.unwrap().context_window, 128_000);
    }

    #[test]
    fn test_find_model_case_insensitive() {
        let model = find_model("GPT-4o");
        assert!(model.is_some());
    }

    #[test]
    fn test_find_model_not_found() {
        assert!(find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_is_frontier() {
        let model = find_model("gpt-4o").unwrap();
        assert!(model.is_frontier());
    }

    #[test]
    fn test_is_budget() {
        let model = find_model("gemini-2.0-flash").unwrap();
        assert!(model.is_budget());
    }

    #[test]
    fn test_estimate_cost() {
        let model = find_model("gpt-4o").unwrap();
        let cost = model.estimate_cost(1000, 500).unwrap();
        assert!(cost > 0.0);
    }

    #[test]
    fn test_pi_ai_no_cost() {
        let model = find_model("pi-ai").unwrap();
        assert!(model.estimate_cost(1000, 500).is_none());
    }

    #[test]
    fn test_models_by_provider() {
        let openai = models_by_provider("openai");
        assert!(!openai.is_empty());
        assert!(openai.iter().all(|m| m.provider == "openai"));
    }

    #[test]
    fn test_pi_ai_compat() {
        let model = find_model("pi-ai").unwrap();
        assert!(!model.supports_vision);
        assert!(!model.supports_tools);
        assert!(!model.supports_system_prompt);
    }
}

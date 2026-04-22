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

    /// Routing quality score derived from structured compatibility metadata.
    pub fn routing_quality_score(&self) -> f64 {
        estimate_routing_quality(
            Some(self.supports_streaming),
            Some(self.supports_tools),
            Some(self.supports_vision),
            Some(self.supports_thinking),
            Some(self.supports_json_mode),
            Some(self.supports_system_prompt),
            Some(self.context_window),
            Some(self.max_output_tokens),
            total_price_per_m(self.input_price_per_m, self.output_price_per_m),
        )
    }
}

/// Estimate a routing quality score from structured model metadata.
///
/// This returns a concrete score for every input shape so routing never falls
/// back to an "unknown/generic" bucket.
pub fn estimate_routing_quality(
    supports_streaming: Option<bool>,
    supports_tools: Option<bool>,
    supports_vision: Option<bool>,
    supports_thinking: Option<bool>,
    supports_json_mode: Option<bool>,
    supports_system_prompt: Option<bool>,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    total_price_per_m: Option<f64>,
) -> f64 {
    let mut score = 0.16;
    score += bool_signal(supports_streaming, 0.04, 0.02);
    score += bool_signal(supports_tools, 0.12, 0.03);
    score += bool_signal(supports_vision, 0.06, 0.01);
    score += bool_signal(supports_thinking, 0.14, 0.03);
    score += bool_signal(supports_json_mode, 0.05, 0.02);
    score += bool_signal(supports_system_prompt, 0.03, 0.02);
    score += normalized_log_range(
        context_window.map(|value| value as f64),
        8_000.0,
        1_000_000.0,
    ) * 0.18;
    score += normalized_log_range(
        max_output_tokens.map(|value| value as f64),
        2_048.0,
        128_000.0,
    ) * 0.10;
    score += normalized_log_range(total_price_per_m, 0.10, 40.0) * 0.10;
    score.clamp(0.05, 0.99)
}

fn bool_signal(value: Option<bool>, yes_weight: f64, unknown_weight: f64) -> f64 {
    match value {
        Some(true) => yes_weight,
        Some(false) => 0.0,
        None => unknown_weight,
    }
}

fn normalized_log_range(value: Option<f64>, min: f64, max: f64) -> f64 {
    let Some(value) = value.filter(|value| *value > 0.0) else {
        return 0.5;
    };
    let clamped = value.clamp(min, max);
    let min_ln = min.ln();
    let max_ln = max.ln();
    if (max_ln - min_ln).abs() < f64::EPSILON {
        return 1.0;
    }
    ((clamped.ln() - min_ln) / (max_ln - min_ln)).clamp(0.0, 1.0)
}

fn total_price_per_m(
    input_price_per_m: Option<f64>,
    output_price_per_m: Option<f64>,
) -> Option<f64> {
    match (input_price_per_m, output_price_per_m) {
        (Some(input), Some(output)) => Some(input + output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
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
            model_id: "gpt-5.4".into(),
            display_name: "GPT-5.4".into(),
            provider: "openai".into(),
            context_window: 400_000,
            max_output_tokens: 128_000,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: true,
            supports_json_mode: true,
            supports_system_prompt: true,
            input_price_per_m: Some(2.50),
            output_price_per_m: Some(15.00),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "gpt-5.4-mini".into(),
            display_name: "GPT-5.4 Mini".into(),
            provider: "openai".into(),
            context_window: 400_000,
            max_output_tokens: 128_000,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: true,
            supports_json_mode: true,
            supports_system_prompt: true,
            input_price_per_m: Some(0.75),
            output_price_per_m: Some(4.50),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "claude-opus-4-7".into(),
            display_name: "Claude Opus 4.7".into(),
            provider: "anthropic".into(),
            context_window: 200_000,
            max_output_tokens: 64_000,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
            supports_thinking: true,
            supports_json_mode: false,
            supports_system_prompt: true,
            input_price_per_m: Some(5.00),
            output_price_per_m: Some(25.00),
            capabilities: HashMap::new(),
        },
        ModelCompat {
            model_id: "claude-sonnet-4-6".into(),
            display_name: "Claude Sonnet 4.6".into(),
            provider: "anthropic".into(),
            context_window: 200_000,
            max_output_tokens: 64_000,
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

    #[test]
    fn test_routing_quality_score_orders_known_models() {
        let opus = find_model("claude-opus-4-7")
            .unwrap()
            .routing_quality_score();
        let gpt_54 = find_model("gpt-5.4").unwrap().routing_quality_score();
        let gpt_54_mini = find_model("gpt-5.4-mini").unwrap().routing_quality_score();
        let pi_ai = find_model("pi-ai").unwrap().routing_quality_score();

        assert!(gpt_54 > gpt_54_mini);
        assert!(opus > pi_ai);
        assert!(gpt_54_mini > pi_ai);
        assert!((0.0..=1.0).contains(&opus));
        assert!((0.0..=1.0).contains(&gpt_54));
    }

    #[test]
    fn test_estimate_routing_quality_never_returns_unknown_bucket() {
        let score = estimate_routing_quality(None, None, None, None, None, None, None, None, None);
        assert!((0.0..=1.0).contains(&score));
        assert_ne!(
            score, 0.5,
            "fallback quality should not collapse to a generic unknown score"
        );
    }
}

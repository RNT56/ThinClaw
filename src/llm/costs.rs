//! Per-model cost lookup table for multi-provider LLM support.
//!
//! Returns (input_cost_per_token, output_cost_per_token) as Decimal pairs.
//! Ollama and other local models return zero cost.
//!
//! These rates are the standard interactive text-token prices for each model's
//! base tier. The table intentionally does not try to encode provider-specific
//! variants such as cached-input discounts, batch/flex/priority pricing, or
//! long-context surcharges.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Dynamic pricing overlay populated by [`crate::llm::pricing_sync`].
///
/// Checked **before** the static `model_cost` match table so that
/// OpenRouter-sourced prices take precedence over hardcoded values.
static DYNAMIC_PRICING: OnceLock<RwLock<HashMap<String, (Decimal, Decimal)>>> = OnceLock::new();

fn dynamic_pricing() -> &'static RwLock<HashMap<String, (Decimal, Decimal)>> {
    DYNAMIC_PRICING.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Replace the entire dynamic pricing overlay.
///
/// Called by [`crate::llm::pricing_sync`] after fetching from OpenRouter.
pub fn set_dynamic_pricing(pricing: HashMap<String, (Decimal, Decimal)>) {
    let lock = dynamic_pricing();
    if let Ok(mut guard) = lock.write() {
        *guard = pricing;
    }
}

/// Look up a model in the dynamic pricing overlay.
///
/// The overlay stores OpenRouter-style IDs (`provider/model-name`), so we
/// check both the raw `model_id` and the normalized (provider-stripped) form.
fn dynamic_cost(model_id: &str) -> Option<(Decimal, Decimal)> {
    let lock = dynamic_pricing();
    let guard = lock.read().ok()?;
    // Try exact match first (e.g. "openai/gpt-5.4-mini")
    if let Some(cost) = guard.get(model_id) {
        return Some(*cost);
    }
    // Try with provider prefix prepended to normalized id
    // OpenRouter uses "provider/model" but our static table uses bare names
    None
}

fn normalize_model_id(model_id: &str) -> String {
    let mut id = model_id
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(model_id)
        .to_string();

    // AWS Bedrock model IDs use `anthropic.<model>-v1:0`, and some endpoints
    // may prepend a region segment such as `us.` or `eu.`.
    if let Some(idx) = id.find("claude-") {
        id = id[idx..].to_string();
    }
    if let Some((base, suffix)) = id.rsplit_once("-v")
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ':' || ch == '.')
    {
        id = base.to_string();
    }

    // Vertex AI Claude aliases use `claude-...@YYYYMMDD`.
    if let Some((base, snapshot)) = id.split_once('@')
        && base.starts_with("claude-")
        && snapshot.chars().all(|ch| ch.is_ascii_digit())
    {
        id = format!("{base}-{snapshot}");
    }

    // OpenAI snapshot aliases commonly suffix `-YYYY-MM-DD`.
    if let Some((base, ymd)) = id.rsplit_once('-')
        && ymd.len() == 2
        && ymd.chars().all(|ch| ch.is_ascii_digit())
    {
        if let Some((base, month)) = base.rsplit_once('-')
            && month.len() == 2
            && month.chars().all(|ch| ch.is_ascii_digit())
        {
            if let Some((base, year)) = base.rsplit_once('-')
                && year.len() == 4
                && year.chars().all(|ch| ch.is_ascii_digit())
            {
                id = base.to_string();
            }
        }
    }

    id
}

/// Look up known per-token costs for a model by its identifier.
///
/// Returns `Some((input_cost, output_cost))` for known models, `None` otherwise.
pub fn model_cost(model_id: &str) -> Option<(Decimal, Decimal)> {
    // 1. Check dynamic pricing overlay first (OpenRouter-sourced)
    if let Some(cost) = dynamic_cost(model_id) {
        return Some(cost);
    }

    // 2. Fall back to static pricing table
    let id = normalize_model_id(model_id);
    static_model_cost(&id)
}

/// Static pricing table — hardcoded per-model rates.
fn static_model_cost(id: &str) -> Option<(Decimal, Decimal)> {
    match id {
        // OpenAI — GPT-5.x / Codex
        "gpt-5.4" => Some((dec!(0.0000025), dec!(0.000015))),
        // `gpt-5.4-mini` / `gpt-5.4-nano` / `gpt-5.4-pro` share the same
        // family naming scheme as other GPT-5 models but have distinct prices.
        "gpt-5.4-mini" => Some((dec!(0.00000075), dec!(0.0000045))),
        "gpt-5.4-nano" => Some((dec!(0.0000002), dec!(0.00000125))),
        "gpt-5.4-pro" => Some((dec!(0.00003), dec!(0.00018))),
        // `gpt-5.3-codex-spark` is treated as the same tier as `gpt-5.3-codex`
        // because OpenAI only publishes the base `gpt-5.3-codex` rate.
        "gpt-5.3-chat-latest" | "gpt-5.3-codex" | "gpt-5.3-codex-spark" => {
            Some((dec!(0.00000175), dec!(0.000014)))
        }
        "gpt-5.2-codex" | "gpt-5.2" => Some((dec!(0.00000175), dec!(0.000014))),
        "gpt-5.2-pro" => Some((dec!(0.000021), dec!(0.000168))),
        "gpt-5.1-codex" | "gpt-5.1-codex-max" | "gpt-5.1" => {
            Some((dec!(0.00000125), dec!(0.00001)))
        }
        "gpt-5.1-codex-mini" => Some((dec!(0.00000025), dec!(0.000002))),
        "gpt-5-codex" | "gpt-5" => Some((dec!(0.00000125), dec!(0.00001))),
        "gpt-5-pro" => Some((dec!(0.000015), dec!(0.00012))),
        "gpt-5-mini" => Some((dec!(0.00000025), dec!(0.000002))),
        "gpt-5-nano" => Some((dec!(0.00000005), dec!(0.0000004))),
        // OpenAI — GPT-4.x
        "gpt-4.1" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-4.1-mini" => Some((dec!(0.0000004), dec!(0.0000016))),
        "gpt-4.1-nano" => Some((dec!(0.0000001), dec!(0.0000004))),
        "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" => {
            Some((dec!(0.0000025), dec!(0.00001)))
        }
        "chatgpt-4o-latest" => Some((dec!(0.000005), dec!(0.000015))),
        "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => Some((dec!(0.00000015), dec!(0.0000006))),
        "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => Some((dec!(0.00001), dec!(0.00003))),
        "gpt-4" | "gpt-4-0613" => Some((dec!(0.00003), dec!(0.00006))),
        "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" => Some((dec!(0.0000005), dec!(0.0000015))),
        "codex-mini-latest" => Some((dec!(0.0000015), dec!(0.000006))),
        // OpenAI — reasoning
        "o3" => Some((dec!(0.000002), dec!(0.000008))),
        "o3-mini" | "o3-mini-2025-01-31" => Some((dec!(0.0000011), dec!(0.0000044))),
        "o4-mini" => Some((dec!(0.0000011), dec!(0.0000044))),
        "o1" | "o1-2024-12-17" => Some((dec!(0.000015), dec!(0.00006))),
        "o1-mini" | "o1-mini-2024-09-12" => Some((dec!(0.0000011), dec!(0.0000044))),

        // Anthropic
        "claude-opus-4-6" | "claude-opus-4-5" | "claude-opus-4-5-20251101" => {
            Some((dec!(0.000005), dec!(0.000025)))
        }
        "claude-opus-4"
        | "claude-opus-4-1"
        | "claude-opus-4-1-20250805"
        | "claude-opus-4-0"
        | "claude-opus-4-20250514" => Some((dec!(0.000015), dec!(0.000075))),
        "claude-3-opus" | "claude-3-opus-20240229" | "claude-3-opus-latest" => {
            Some((dec!(0.000015), dec!(0.000075)))
        }
        "claude-sonnet-4"
        | "claude-sonnet-4-6"
        | "claude-sonnet-4-5"
        | "claude-sonnet-4-5-20250929"
        | "claude-sonnet-4-0"
        | "claude-sonnet-4-20250514"
        | "claude-3-7-sonnet-20250219"
        | "claude-3-7-sonnet-latest"
        | "claude-3-sonnet"
        | "claude-3-sonnet-20240229"
        | "claude-3-5-sonnet-20241022"
        | "claude-3-5-sonnet-latest" => Some((dec!(0.000003), dec!(0.000015))),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" | "claude-haiku-4.5" => {
            Some((dec!(0.000001), dec!(0.000005)))
        }
        "claude-3-5-haiku-20241022" | "claude-3-5-haiku-latest" => {
            Some((dec!(0.0000008), dec!(0.000004)))
        }
        "claude-3-haiku" | "claude-3-haiku-20240307" => Some((dec!(0.00000025), dec!(0.00000125))),

        // Google Gemini
        // `gemini-3.1-flash` is not currently listed on Google's published
        // pricing page, so we conservatively bucket it with the closest
        // source-backed Flash family rate.
        "gemini-3.1-flash" | "gemini-2.5-flash" => Some((dec!(0.0000003), dec!(0.0000025))),
        "gemini-2.0-flash" | "gemini-2.0-flash-exp" => Some((dec!(0.0000001), dec!(0.0000004))),
        "gemini-2.5-flash-lite" | "gemini-2.5-flash-lite-preview-09-2025" => {
            Some((dec!(0.0000001), dec!(0.0000004)))
        }
        "gemini-2.0-flash-lite" => Some((dec!(0.000000075), dec!(0.0000003))),
        "gemini-1.5-flash" | "gemini-1.5-flash-latest" => {
            Some((dec!(0.000000075), dec!(0.0000003)))
        }
        "gemini-1.5-flash-8b" => Some((dec!(0.0000000375), dec!(0.00000015))),
        // `gemini-2.0-pro` / `gemini-2.0-pro-exp` do not have a currently
        // published paid price in Google's pricing docs, so we keep them on
        // the same tier as the closest generally-available Pro family.
        "gemini-2.5-pro" | "gemini-2.0-pro" | "gemini-2.0-pro-exp" => {
            Some((dec!(0.00000125), dec!(0.00001)))
        }
        "gemini-1.5-pro" | "gemini-1.5-pro-latest" => Some((dec!(0.00000125), dec!(0.000005))),
        "gemini-1.0-pro" => Some((dec!(0.0000005), dec!(0.0000015))),

        // Ollama / local models -- free
        _ if is_local_model(&id) => Some((Decimal::ZERO, Decimal::ZERO)),

        _ => None,
    }
}

/// Default cost for unknown models.
pub fn default_cost() -> (Decimal, Decimal) {
    // Conservative estimate: roughly GPT-4o pricing
    (dec!(0.0000025), dec!(0.00001))
}

/// Heuristic to detect local/self-hosted models (Ollama, llama.cpp, etc.).
fn is_local_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    lower.starts_with("llama")
        || lower.starts_with("mistral")
        || lower.starts_with("mixtral")
        || lower.starts_with("phi")
        || lower.starts_with("gemma")
        || lower.starts_with("qwen")
        || lower.starts_with("codellama")
        || lower.starts_with("deepseek")
        || lower.starts_with("starcoder")
        || lower.starts_with("vicuna")
        || lower.starts_with("yi")
        || lower.contains(":latest")
        || lower.contains(":instruct")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_model_costs() {
        let (input, output) = model_cost("gpt-4o").unwrap();
        assert!(input > Decimal::ZERO);
        assert!(output > input);
    }

    #[test]
    fn test_claude_costs() {
        let (input, output) = model_cost("claude-3-5-sonnet-20241022").unwrap();
        assert!(input > Decimal::ZERO);
        assert!(output > input);
    }

    #[test]
    fn test_gpt_5_4_mini_costs() {
        let (input, output) = model_cost("gpt-5.4-mini").unwrap();
        assert_eq!(input, dec!(0.00000075));
        assert_eq!(output, dec!(0.0000045));
    }

    #[test]
    fn test_gpt_5_2_pro_costs() {
        let (input, output) = model_cost("gpt-5.2-pro").unwrap();
        assert_eq!(input, dec!(0.000021));
        assert_eq!(output, dec!(0.000168));
    }

    #[test]
    fn test_o1_mini_costs() {
        let (input, output) = model_cost("o1-mini").unwrap();
        assert_eq!(input, dec!(0.0000011));
        assert_eq!(output, dec!(0.0000044));
    }

    #[test]
    fn test_gemini_2_5_flash_costs() {
        let (input, output) = model_cost("gemini-2.5-flash").unwrap();
        assert_eq!(input, dec!(0.0000003));
        assert_eq!(output, dec!(0.0000025));
    }

    #[test]
    fn test_gemini_1_5_flash_8b_costs() {
        let (input, output) = model_cost("gemini-1.5-flash-8b").unwrap();
        assert_eq!(input, dec!(0.0000000375));
        assert_eq!(output, dec!(0.00000015));
    }

    #[test]
    fn test_claude_opus_4_5_costs() {
        let (input, output) = model_cost("claude-opus-4-5").unwrap();
        assert_eq!(input, dec!(0.000005));
        assert_eq!(output, dec!(0.000025));
    }

    #[test]
    fn test_claude_haiku_4_5_costs() {
        let (input, output) = model_cost("claude-haiku-4-5").unwrap();
        assert_eq!(input, dec!(0.000001));
        assert_eq!(output, dec!(0.000005));
    }

    #[test]
    fn test_gemini_2_5_flash_lite_costs() {
        let (input, output) = model_cost("gemini-2.5-flash-lite").unwrap();
        assert_eq!(input, dec!(0.0000001));
        assert_eq!(output, dec!(0.0000004));
    }

    #[test]
    fn test_local_model_free() {
        let (input, output) = model_cost("llama3").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_ollama_tagged_model_free() {
        let (input, output) = model_cost("mistral:latest").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_unknown_model_returns_none() {
        assert!(model_cost("some-totally-unknown-model-xyz").is_none());
    }

    #[test]
    fn test_default_cost_nonzero() {
        let (input, output) = default_cost();
        assert!(input > Decimal::ZERO);
        assert!(output > Decimal::ZERO);
    }

    #[test]
    fn test_provider_prefix_stripped() {
        // "openai/gpt-4o" should resolve to same as "gpt-4o"
        assert_eq!(model_cost("openai/gpt-4o"), model_cost("gpt-4o"));
    }

    #[test]
    fn test_chatgpt_4o_latest_costs() {
        let (input, output) = model_cost("chatgpt-4o-latest").unwrap();
        assert_eq!(input, dec!(0.000005));
        assert_eq!(output, dec!(0.000015));
    }

    #[test]
    fn test_bedrock_claude_model_normalized() {
        assert_eq!(
            model_cost("anthropic.claude-3-sonnet-20240229-v1:0"),
            model_cost("claude-3-sonnet-20240229")
        );
    }

    #[test]
    fn test_vertex_claude_model_normalized() {
        assert_eq!(
            model_cost("claude-sonnet-4-5@20250929"),
            model_cost("claude-sonnet-4-5-20250929")
        );
    }
}

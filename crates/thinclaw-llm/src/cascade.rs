//! Shared cascade-escalation heuristics for the unified route planner.
//!
//! When `RoutePlanner` selects the cheap lane for a Moderate-complexity turn
//! and cascade is enabled, the runtime runs the cheap completion, inspects it
//! for uncertainty, and re-issues against the primary chain when the cheap
//! response looks incomplete or refused. This module owns the inspection
//! heuristic so the planner-driven runtime path is the single home for cascade
//! behavior.
//!
//! The heuristic mirrors the conservative approach documented in the legacy
//! `SmartRoutingProvider` decorator (Bug 7 fix): avoid escalating confident but
//! brief or contextual answers, escalating only on empty/very-short responses
//! or explicit inability signals.

use thinclaw_llm_core::provider::CompletionResponse;

/// Explicit inability signals that warrant escalation. Deliberately excludes
/// clarification requests ("could you clarify"), which are valid, confident
/// responses.
const HARD_REFUSAL_PATTERNS: &[&str] = &[
    "i'm not able to",
    "i am not able to",
    "i cannot complete",
    "i can't complete",
    "beyond my capabilities",
    "i don't have access",
    "i do not have access",
];

/// Check if a cheap-lane response shows uncertainty, warranting escalation to
/// the primary chain.
///
/// Conservative rules:
///   1. Empty responses are always uncertain.
///   2. Very short responses (< 10 chars) are likely incomplete/truncated.
///   3. Explicit refusal patterns only (not clarification requests).
pub fn response_is_uncertain(response: &CompletionResponse) -> bool {
    let content = response.content.trim();

    // Empty response is always uncertain.
    if content.is_empty() {
        return true;
    }

    // Very short response from the cheap model likely means incomplete/truncated
    // output. Legitimate short answers like "Yes." or "42" are only 3–4 chars;
    // the 10-char cutoff avoids escalating those while still catching single-word
    // fragments or error stubs.
    if content.len() < 10 {
        return true;
    }

    let lower = content.to_lowercase();
    HARD_REFUSAL_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_llm_core::provider::FinishReason;

    fn response_with(content: &str) -> CompletionResponse {
        CompletionResponse {
            content: content.to_string(),
            provider_model: None,
            cost_usd: None,
            thinking_content: None,
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        }
    }

    #[test]
    fn empty_response_is_uncertain() {
        assert!(response_is_uncertain(&response_with("")));
        assert!(response_is_uncertain(&response_with("   ")));
    }

    #[test]
    fn very_short_response_is_uncertain() {
        assert!(response_is_uncertain(&response_with("hmm")));
    }

    #[test]
    fn explicit_refusal_is_uncertain() {
        assert!(response_is_uncertain(&response_with(
            "I'm not able to complete this request without more tooling."
        )));
    }

    #[test]
    fn confident_answer_is_not_uncertain() {
        assert!(!response_is_uncertain(&response_with(
            "The capital of France is Paris, a city on the Seine."
        )));
    }

    #[test]
    fn clarification_request_is_not_uncertain() {
        // Clarification requests are confident, valid responses and must not
        // trigger escalation.
        assert!(!response_is_uncertain(&response_with(
            "Could you clarify which database backend you mean?"
        )));
    }
}

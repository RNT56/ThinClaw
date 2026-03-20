//! LLM token sanitizer — strips leaked ChatML / Jinja template tokens
//!
//! Local models (Qwen, Mistral, Llama, etc.) sometimes emit control tokens
//! in their output. This module provides a function to strip them before
//! the text reaches the UI.
//!
//! IronClaw emits raw LLM output — Scrappy applies this sanitizer before
//! rendering in the frontend.

use regex::Regex;
use std::sync::LazyLock;

/// Compiled regexes for stripping LLM control tokens from output text.
/// These patterns catch ChatML (Qwen, Mistral, etc.), Llama, and common
/// template artifacts that local models sometimes emit.
static LLM_TOKEN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // ChatML block markers: <|im_start|> optionally followed by a role word (assistant, user, etc.)
        Regex::new(r"<\|im_start\|>\w*").unwrap(),
        Regex::new(r"<\|im_end\|>").unwrap(),
        // Generic special tokens
        Regex::new(r"<\|end\|>").unwrap(),
        Regex::new(r"<\|endoftext\|>").unwrap(),
        Regex::new(r"<\|eot_id\|>").unwrap(),
        // Llama header blocks: <|start_header_id|>role<|end_header_id|> as a single unit
        Regex::new(r"<\|start_header_id\|>\w*<\|end_header_id\|>").unwrap(),
        // Fallback: catch orphaned header tokens that appear without the other half
        Regex::new(r"<\|start_header_id\|>").unwrap(),
        Regex::new(r"<\|end_header_id\|>").unwrap(),
        // Thinking blocks: <think>...</think>
        Regex::new(r"(?s)<think>.*?</think>").unwrap(),
        // Bare role markers that sometimes leak mid-text
        Regex::new(r"(?m)^(user|assistant|system|tool)>\s*$").unwrap(),
        // Mistral-style raw tool call tokens: [TOOL_CALLS]name[ARGS]{...}
        // Small models sometimes emit these as text instead of using the function calling API.
        Regex::new(r"\[TOOL_CALLS\]\w+\[ARGS\]\{[^}]*\}").unwrap(),
    ]
});

/// Collapse runs of 3+ newlines into 2 — compiled once, used on every call.
static NEWLINE_COLLAPSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// Strip leaked LLM template tokens from text before it reaches the UI.
///
/// Applied to all assistant text (deltas, snapshots, finals) before rendering.
/// IronClaw emits raw LLM output; this function cleans it for display.
pub fn strip_llm_tokens(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in LLM_TOKEN_PATTERNS.iter() {
        result = pattern.replace_all(&result, "").to_string();
    }
    // Collapse runs of 3+ newlines into 2
    result = NEWLINE_COLLAPSE.replace_all(&result, "\n\n").to_string();
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_chatml_tokens() {
        let input = "Hello<|im_end|>\n<|im_start|>assistant\nI'm fine<|im_end|>";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Hello\n\nI'm fine");
    }

    #[test]
    fn test_strip_thinking_blocks() {
        let input =
            "Let me help. <think>I should check the weather first...</think>Here's the plan:";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Let me help. Here's the plan:");
    }

    #[test]
    fn test_strip_llama_tokens() {
        let input = "Hello<|eot_id|><|start_header_id|>assistant<|end_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_orphaned_header_tokens() {
        let input = "Hello<|start_header_id|>World";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn test_strip_preserves_normal_text() {
        let input = "This is a normal response with **markdown** and `code`.";
        let result = strip_llm_tokens(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_collapses_newlines() {
        let input = "Part 1\n\n\n\n\nPart 2";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Part 1\n\nPart 2");
    }

    #[test]
    fn test_strip_mistral_tool_calls() {
        let input = "Loading identity...[TOOL_CALLS]memory_read[ARGS]{\"path\": \"SOUL.md\"}[TOOL_CALLS]memory_read[ARGS]{\"path\": \"USER.md\"}";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Loading identity...");
    }

    #[test]
    fn test_strip_tool_calls_with_surrounding_text() {
        let input = "Let me check.\n[TOOL_CALLS]memory_search[ARGS]{\"query\": \"user preferences\"}\nHere's what I found:";
        let result = strip_llm_tokens(input);
        assert_eq!(result, "Let me check.\n\nHere's what I found:");
    }
}

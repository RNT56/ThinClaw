//! Model-family-specific prompt guidance helpers.

/// Recognized model families for prompt guidance selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Gpt,
    Gemini,
    Claude,
    DeepSeek,
    Llama,
    Qwen,
    Other,
}

/// Guidance block for GPT-family models.
pub const GPT_EXECUTION_GUIDANCE: &str = r#"GPT-family models:
- Be persistent when the task requires tools or verification.
- Use tools when they are available instead of guessing.
- Check prerequisites before acting and verify the result after changes.
- Prefer concrete execution over speculative advice.
- If a safe action can be taken directly, do it instead of asking the user to repeat instructions."#;

/// Guidance block for Gemini-family models.
pub const GEMINI_OPERATIONAL_GUIDANCE: &str = r#"Gemini-family models:
- Use absolute paths whenever you reference files.
- Verify before you modify anything important.
- Prefer parallel tool calls when the tasks are independent.
- Keep operational steps explicit and grounded in the current workspace state.
- When multiple checks are needed, run them before reporting success."#;

/// Detect the model family from a model name.
pub fn detect_family(model_name: &str) -> ModelFamily {
    let model_name = model_name.to_lowercase();

    if model_name.contains("gpt") || model_name.contains("codex") {
        ModelFamily::Gpt
    } else if model_name.contains("gemini") || model_name.contains("gemma") {
        ModelFamily::Gemini
    } else if model_name.contains("claude") {
        ModelFamily::Claude
    } else if model_name.contains("deepseek") {
        ModelFamily::DeepSeek
    } else if model_name.contains("llama") {
        ModelFamily::Llama
    } else if model_name.contains("qwen") {
        ModelFamily::Qwen
    } else {
        ModelFamily::Other
    }
}

/// Return the guidance block for a given model family, if any.
pub fn guidance_block(family: ModelFamily) -> Option<&'static str> {
    match family {
        ModelFamily::Gpt => Some(GPT_EXECUTION_GUIDANCE),
        ModelFamily::Gemini => Some(GEMINI_OPERATIONAL_GUIDANCE),
        ModelFamily::Claude
        | ModelFamily::DeepSeek
        | ModelFamily::Llama
        | ModelFamily::Qwen
        | ModelFamily::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gpt_family() {
        assert_eq!(detect_family("gpt-4o-mini"), ModelFamily::Gpt);
        assert_eq!(detect_family("codex-5"), ModelFamily::Gpt);
    }

    #[test]
    fn detects_other_families() {
        assert_eq!(detect_family("gemini-2.5-flash"), ModelFamily::Gemini);
        assert_eq!(detect_family("claude-sonnet-4"), ModelFamily::Claude);
        assert_eq!(detect_family("deepseek-r1"), ModelFamily::DeepSeek);
        assert_eq!(detect_family("llama-3.1-70b"), ModelFamily::Llama);
        assert_eq!(detect_family("qwen2.5-coder"), ModelFamily::Qwen);
        assert_eq!(detect_family("unknown-model"), ModelFamily::Other);
    }

    #[test]
    fn guidance_blocks_match_family() {
        assert_eq!(
            guidance_block(ModelFamily::Gpt),
            Some(GPT_EXECUTION_GUIDANCE)
        );
        assert_eq!(
            guidance_block(ModelFamily::Gemini),
            Some(GEMINI_OPERATIONAL_GUIDANCE)
        );
        assert_eq!(guidance_block(ModelFamily::Claude), None);
        assert_eq!(guidance_block(ModelFamily::Other), None);
    }
}

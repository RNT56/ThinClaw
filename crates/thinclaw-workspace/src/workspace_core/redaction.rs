//! Prompt-content redaction and linked-conversation recall formatting.
//!
//! Owns the [`PromptRedaction`] policy (channel-aware PII redaction toggles),
//! suspicious-content sanitization at prompt-assembly time, memory-content
//! summarization, and linked conversation recall rendering.

use thinclaw_safety::sanitize_prompt_bound_content;

pub(super) fn sanitize_prompt_context(
    file_name: &str,
    content: &str,
    redaction: PromptRedaction<'_>,
) -> String {
    let sanitized = sanitize_prompt_bound_content(content, redaction.channel, redaction.enabled);
    for warning in sanitized.warnings {
        tracing::warn!(
            file = file_name,
            pattern = %warning.pattern,
            "Suspicious context content detected during prompt assembly"
        );
    }
    sanitized.content
}

#[allow(dead_code)] // Retained for shared/global memory summarisation
pub(super) fn summarize_memory_content(content: &str) -> String {
    let entry_count = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .count();

    if entry_count == 0 {
        String::new()
    } else {
        format!("MEMORY.md: {} entries (long-term notes)", entry_count)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PromptRedaction<'a> {
    pub(super) channel: Option<&'a str>,
    pub(super) enabled: bool,
}

impl<'a> PromptRedaction<'a> {
    pub(super) fn new(channel: Option<&'a str>, enabled: bool) -> Self {
        Self { channel, enabled }
    }
}

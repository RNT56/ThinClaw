//! Prompt-content redaction and linked-conversation recall formatting.
//!
//! Owns the [`PromptRedaction`] policy (channel-aware PII redaction toggles),
//! suspicious-content sanitization at prompt-assembly time, memory-content
//! summarization, and linked conversation recall rendering.

use thinclaw_identity::LinkedConversationRecall;
use thinclaw_safety::{pii_redactor, sanitize_prompt_bound_content};

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

pub(super) fn summarize_actor_memory_content(content: &str) -> String {
    let entry_count = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .count();

    if entry_count == 0 {
        String::new()
    } else {
        format!("MEMORY.md: {} entries (actor-private notes)", entry_count)
    }
}

pub(super) fn linked_recall_is_empty(recall: &LinkedConversationRecall) -> bool {
    recall.source_channel.is_empty()
        && recall.source_conversation_key.is_empty()
        && recall
            .handoff_summary
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        && recall.summary.as_deref().unwrap_or("").trim().is_empty()
        && recall
            .last_user_goal
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
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

    pub(super) fn should_redact(self) -> bool {
        self.enabled && self.channel.is_some_and(pii_redactor::should_redact)
    }

    pub(super) fn actor_label(self, actor_id: &str) -> String {
        match self.channel {
            Some(channel) if self.enabled => pii_redactor::redact_for_prompt(actor_id, channel),
            _ => actor_id.to_string(),
        }
    }

    pub(super) fn sensitive_label(self, value: &str) -> String {
        if self.should_redact() {
            pii_redactor::hash_user_id(value)
        } else {
            value.to_string()
        }
    }
}

pub(super) fn format_linked_recall(
    recall: &LinkedConversationRecall,
    redaction: PromptRedaction<'_>,
) -> String {
    let mut lines = vec!["## Linked Conversation Recall".to_string()];
    if !recall.actor_id.is_empty() {
        lines.push(format!(
            "- Actor: {}",
            redaction.actor_label(&recall.actor_id)
        ));
    }
    if !recall.source_channel.is_empty() {
        lines.push(format!("- Source channel: {}", recall.source_channel));
    }
    if !recall.source_conversation_key.is_empty() {
        lines.push(format!(
            "- Source conversation: {}",
            redaction.sensitive_label(&recall.source_conversation_key)
        ));
    }
    if !recall
        .handoff_summary
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        lines.push(format!(
            "- Handoff: {}",
            recall.handoff_summary.as_deref().unwrap_or("").trim()
        ));
    }
    if !recall.summary.as_deref().unwrap_or("").trim().is_empty() {
        lines.push(format!(
            "- Summary: {}",
            recall.summary.as_deref().unwrap_or("").trim()
        ));
    }
    if !recall
        .last_user_goal
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        lines.push(format!(
            "- Last goal: {}",
            recall.last_user_goal.as_deref().unwrap_or("").trim()
        ));
    }
    lines.join("\n")
}

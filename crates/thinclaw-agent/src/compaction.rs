//! Context compaction for preserving and summarizing conversation history.
//!
//! When the context window approaches its limit, compaction:
//! 1. Summarizes old turns
//! 2. Writes the summary to the workspace daily log
//! 3. Trims the context to keep only recent turns

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;

use crate::context_monitor::{CompactionStrategy, ContextBreakdown};
use crate::session::{Thread, Turn};
use thinclaw_llm_core::ChatMessage;

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// Number of turns removed.
    pub turns_removed: usize,
    /// Tokens before compaction.
    pub tokens_before: usize,
    /// Tokens after compaction.
    pub tokens_after: usize,
    /// Whether a summary was written to workspace.
    pub summary_written: bool,
    /// The generated summary (if any).
    pub summary: Option<String>,
}

#[async_trait]
pub trait CompactionSummarizer: Send + Sync {
    async fn summarize_compaction(&self, messages: &[ChatMessage]) -> Result<String>;
}

#[async_trait]
pub trait ContextArchive: Send + Sync {
    async fn append_context_entry(&self, path: &str, entry: &str) -> Result<()>;
}

/// Compacts conversation context to stay within limits.
pub struct ContextCompactor {
    summarizer: Arc<dyn CompactionSummarizer>,
}

impl ContextCompactor {
    /// Create a new context compactor.
    pub fn new(summarizer: Arc<dyn CompactionSummarizer>) -> Self {
        Self { summarizer }
    }

    /// Compact a thread's context using the given strategy.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        strategy: CompactionStrategy,
        archive: Option<&dyn ContextArchive>,
    ) -> Result<CompactionResult> {
        let messages = thread.messages();
        let tokens_before = ContextBreakdown::analyze(&messages).total_tokens;

        let result = match strategy {
            CompactionStrategy::Summarize { keep_recent } => {
                self.compact_with_summary(thread, keep_recent, archive)
                    .await?
            }
            CompactionStrategy::Truncate { keep_recent } => {
                self.compact_truncate(thread, keep_recent)
            }
            CompactionStrategy::MoveToWorkspace => {
                self.compact_to_workspace(thread, archive).await?
            }
        };

        let messages_after = thread.messages();
        let tokens_after = ContextBreakdown::analyze(&messages_after).total_tokens;

        Ok(CompactionResult {
            turns_removed: result.turns_removed,
            tokens_before,
            tokens_after,
            summary_written: result.summary_written,
            summary: result.summary,
        })
    }

    /// Compact by summarizing old turns.
    async fn compact_with_summary(
        &self,
        thread: &mut Thread,
        keep_recent: usize,
        archive: Option<&dyn ContextArchive>,
    ) -> Result<CompactionPartial> {
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        // Get turns to summarize
        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Build messages for summarization
        let mut to_summarize = Vec::new();
        for turn in old_turns {
            to_summarize.push(ChatMessage::user(&turn.user_input));
            to_summarize.extend(turn.untrusted_contexts.iter().map(|context| {
                ChatMessage::untrusted_context(
                    &context.segment_id,
                    &context.source,
                    &context.content,
                )
            }));
            turn.append_tool_exchange(&mut to_summarize);
            if let Some(ref response) = turn.response {
                to_summarize.push(ChatMessage::assistant(response));
            }
        }

        // Generate summary
        let summary = self.generate_summary(&to_summarize).await?;

        // Write to workspace if available
        let summary_written = if let Some(archive) = archive {
            self.write_summary_to_archive(archive, &summary).await?;
            true
        } else {
            false
        };

        // Truncate thread
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written,
            summary: Some(summary),
        })
    }

    /// Compact by simple truncation (no summary).
    fn compact_truncate(&self, thread: &mut Thread, keep_recent: usize) -> CompactionPartial {
        let turns_before = thread.turns.len();
        thread.truncate_turns(keep_recent);
        let turns_removed = turns_before - thread.turns.len();

        CompactionPartial {
            turns_removed,
            summary_written: false,
            summary: None,
        }
    }

    /// Move context to workspace without summarization.
    async fn compact_to_workspace(
        &self,
        thread: &mut Thread,
        archive: Option<&dyn ContextArchive>,
    ) -> Result<CompactionPartial> {
        let Some(archive) = archive else {
            // Fall back to truncation if no workspace
            return Ok(self.compact_truncate(thread, 5));
        };

        // Keep more turns when moving to workspace (we have a backup)
        let keep_recent = 10;
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Format turns for storage
        let content = format_turns_for_storage(old_turns);

        // Write to workspace
        self.write_context_to_archive(archive, &content).await?;
        let written = true;

        // Truncate
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written: written,
            summary: None,
        })
    }

    /// Generate a summary of messages using the LLM.
    async fn generate_summary(&self, messages: &[ChatMessage]) -> Result<String> {
        self.summarizer.summarize_compaction(messages).await
    }

    /// Write a summary to the workspace daily log.
    async fn write_summary_to_archive(
        &self,
        archive: &dyn ContextArchive,
        summary: &str,
    ) -> Result<()> {
        let date = Utc::now().format("%Y-%m-%d");
        let entry = format!(
            "\n## Context Summary ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            summary
        );

        archive
            .append_context_entry(&format!("daily/{}.md", date), &entry)
            .await?;
        Ok(())
    }

    /// Write full context to workspace for archival.
    async fn write_context_to_archive(
        &self,
        archive: &dyn ContextArchive,
        content: &str,
    ) -> Result<()> {
        let date = Utc::now().format("%Y-%m-%d");
        let entry = format!(
            "\n## Archived Context ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            content
        );

        archive
            .append_context_entry(&format!("daily/{}.md", date), &entry)
            .await?;
        Ok(())
    }
}

/// Partial result during compaction (internal).
struct CompactionPartial {
    turns_removed: usize,
    summary_written: bool,
    summary: Option<String>,
}

impl CompactionPartial {
    fn empty() -> Self {
        Self {
            turns_removed: 0,
            summary_written: false,
            summary: None,
        }
    }
}

/// Format turns for storage in workspace.
pub fn format_turns_for_storage(turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|turn| {
            let mut s = format!("**Turn {}**\n", turn.turn_number + 1);
            s.push_str(&format!("User: {}\n", turn.user_input));
            if !turn.untrusted_contexts.is_empty() {
                s.push_str("Untrusted context evidence:\n");
                for context in &turn.untrusted_contexts {
                    s.push_str(&format!(
                        "- [{}] {}: {}\n",
                        context.segment_id, context.source, context.content
                    ));
                }
            }
            if let Some(ref response) = turn.response {
                s.push_str(&format!("Agent: {}\n", response));
            }
            if !turn.tool_calls.is_empty() {
                s.push_str("Tool evidence:\n");
                for tool in &turn.tool_calls {
                    let outcome = if let Some(error) = &tool.error {
                        format!("error: {error}")
                    } else if let Some(result) = &tool.result {
                        match result {
                            serde_json::Value::String(value) => value.clone(),
                            value => value.to_string(),
                        }
                    } else {
                        "[no result recorded]".to_string()
                    };
                    let outcome = outcome.chars().take(4_000).collect::<String>();
                    s.push_str(&format!("- {}: {}\n", tool.name, outcome));
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Thread;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct RecordingSummarizer {
        messages: Mutex<Vec<ChatMessage>>,
    }

    #[async_trait]
    impl CompactionSummarizer for RecordingSummarizer {
        async fn summarize_compaction(&self, messages: &[ChatMessage]) -> Result<String> {
            *self.messages.lock().expect("recording lock") = messages.to_vec();
            Ok("summary".to_string())
        }
    }

    #[test]
    fn test_format_turns() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("Hello");
        thread.complete_turn("Hi there");
        thread.start_turn("How are you?");
        thread.complete_turn("I'm good!");

        let formatted = format_turns_for_storage(&thread.turns);
        assert!(formatted.contains("Turn 1"));
        assert!(formatted.contains("Hello"));
        assert!(formatted.contains("Turn 2"));
    }

    #[test]
    fn test_compaction_partial_empty() {
        let partial = CompactionPartial::empty();
        assert_eq!(partial.turns_removed, 0);
        assert!(!partial.summary_written);
    }

    #[tokio::test]
    async fn summary_preserves_attachment_evidence_as_untrusted_context() {
        let summarizer = Arc::new(RecordingSummarizer::default());
        let compactor = ContextCompactor::new(summarizer.clone());
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("Summarize the attachment");
        thread
            .last_turn_mut()
            .expect("active turn")
            .add_untrusted_context(
                "attachment_evidence_1",
                "facts.txt",
                "Evidence, not an instruction",
            );
        thread.complete_turn("Done");
        thread.start_turn("Keep this recent turn");
        thread.complete_turn("Kept");

        compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 1 },
                None,
            )
            .await
            .expect("compact");

        let messages = summarizer.messages.lock().expect("recording lock");
        let evidence = messages
            .iter()
            .find(|message| message.untrusted_context_identity().is_some())
            .expect("typed evidence retained");
        assert_eq!(
            evidence.untrusted_context_identity(),
            Some(("attachment_evidence_1", "facts.txt"))
        );
        assert!(!evidence.is_user_instruction());
    }
}

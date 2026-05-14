//! Context compaction compatibility facade.
//!
//! The compaction algorithm lives in `thinclaw-agent`. Root keeps concrete
//! adapters for LLM reasoning, cost tracking, safety, and workspace archival.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::agent::context_monitor::CompactionStrategy;
use crate::agent::session::Thread;
use crate::error::{Error, OrchestratorError};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning, Role};
use crate::safety::SafetyLayer;
use crate::workspace::Workspace;

pub use thinclaw_agent::compaction::{
    CompactionResult, CompactionSummarizer, ContextArchive, format_turns_for_storage,
};

/// Compacts conversation context to stay within limits.
pub struct ContextCompactor {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    cost_tracker: Option<Arc<Mutex<crate::llm::cost_tracker::CostTracker>>>,
}

impl ContextCompactor {
    /// Create a new context compactor.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self {
            llm,
            safety,
            cost_tracker: None,
        }
    }

    /// Attach a shared cost tracker so compaction LLM calls are recorded.
    pub fn with_cost_tracker(
        mut self,
        tracker: Arc<Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// Compact a thread's context using the given strategy.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        strategy: CompactionStrategy,
        workspace: Option<&Workspace>,
    ) -> Result<CompactionResult, Error> {
        let summarizer = Arc::new(RootCompactionSummarizer {
            llm: Arc::clone(&self.llm),
            safety: Arc::clone(&self.safety),
            cost_tracker: self.cost_tracker.clone(),
        });
        let compactor = thinclaw_agent::compaction::ContextCompactor::new(summarizer);
        let archive = workspace.map(|workspace| RootContextArchive { workspace });
        compactor
            .compact(
                thread,
                strategy,
                archive
                    .as_ref()
                    .map(|archive| archive as &dyn ContextArchive),
            )
            .await
            .map_err(|error| {
                Error::Orchestrator(OrchestratorError::ApiError {
                    reason: error.to_string(),
                })
            })
    }
}

struct RootCompactionSummarizer {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    cost_tracker: Option<Arc<Mutex<crate::llm::cost_tracker::CostTracker>>>,
}

#[async_trait]
impl CompactionSummarizer for RootCompactionSummarizer {
    async fn summarize_compaction(&self, messages: &[ChatMessage]) -> anyhow::Result<String> {
        let prompt = ChatMessage::system(
            r#"Summarize the following conversation concisely. Focus on:
- Key decisions made
- Important information exchanged
- Actions taken
- Outcomes achieved

Be brief but capture all important details. Use bullet points."#,
        );

        let formatted = messages
            .iter()
            .map(|message| {
                let role_str = match message.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    Role::System => "System",
                    Role::Tool => {
                        return format!(
                            "Tool {}: {}",
                            message.name.as_deref().unwrap_or("unknown"),
                            message.content
                        );
                    }
                };
                format!("{}: {}", role_str, message.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let request = CompletionRequest::new(vec![
            prompt,
            ChatMessage::user(format!(
                "Please summarize this conversation:\n\n{}",
                formatted
            )),
        ])
        .with_max_tokens(1024)
        .with_temperature(0.3);

        let mut reasoning = Reasoning::new(Arc::clone(&self.llm), Arc::clone(&self.safety));
        if let Some(ref tracker) = self.cost_tracker {
            reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
        }
        let (text, usage) = reasoning.complete(request).await?;

        tracing::info!(
            "[compaction] Summary LLM call: input_tokens={}, output_tokens={}",
            usage.input_tokens,
            usage.output_tokens,
        );

        Ok(text)
    }
}

struct RootContextArchive<'a> {
    workspace: &'a Workspace,
}

#[async_trait]
impl ContextArchive for RootContextArchive<'_> {
    async fn append_context_entry(&self, path: &str, entry: &str) -> anyhow::Result<()> {
        self.workspace.append(path, entry).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
}

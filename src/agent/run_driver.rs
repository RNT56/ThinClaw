use std::sync::Arc;

use anyhow::Context;
use uuid::Uuid;

use crate::agent::run_artifact::{AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus};
use crate::tools::execution_backend::interactive_chat_runtime_descriptor;

#[derive(Debug, Clone, Default)]
pub struct AgentRunDriver {
    logger: AgentRunArtifactLogger,
}

impl AgentRunDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_logger(logger: AgentRunArtifactLogger) -> Self {
        Self { logger }
    }

    pub fn logger(&self) -> &AgentRunArtifactLogger {
        &self.logger
    }

    pub async fn append_artifact(&self, artifact: &AgentRunArtifact) -> anyhow::Result<()> {
        self.logger.append_artifact(artifact).await?;
        Ok(())
    }

    pub async fn record_chat_turn(
        &self,
        llm_provider: &str,
        llm_model: &str,
        store: Option<Arc<dyn crate::db::Database>>,
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> anyhow::Result<AgentRunArtifact> {
        let mut artifact = AgentRunArtifact::new(
            "chat",
            match turn.state {
                crate::agent::session::TurnState::Completed => AgentRunStatus::Completed,
                crate::agent::session::TurnState::Failed => AgentRunStatus::Failed,
                crate::agent::session::TurnState::Interrupted => AgentRunStatus::Interrupted,
                crate::agent::session::TurnState::Processing => AgentRunStatus::Interrupted,
            },
            turn.started_at,
            turn.completed_at,
        )
        .with_chat_turn_snapshot(session, thread_id, incoming, turn)
        .with_failure_reason(turn.error.clone())
        .with_runtime_descriptor(Some(&interactive_chat_runtime_descriptor()))
        .with_metadata(serde_json::json!({
            "turn_status": match turn.state {
                crate::agent::session::TurnState::Completed => "completed",
                crate::agent::session::TurnState::Failed => "failed",
                crate::agent::session::TurnState::Interrupted => "interrupted",
                crate::agent::session::TurnState::Processing => "processing",
            },
            "llm_provider": llm_provider,
            "llm_model": llm_model,
        }));

        if let Some(store) = store
            && let Some(thread_id) = artifact.thread_id
            && let Ok(Some(runtime)) = crate::agent::load_thread_runtime(&store, thread_id).await
        {
            artifact = artifact
                .with_prompt_hashes(runtime.prompt_snapshot_hash, runtime.ephemeral_overlay_hash)
                .with_provider_context_refs(runtime.provider_context_refs);
        }

        self.append_artifact(&artifact)
            .await
            .context("failed to append canonical run artifact")?;
        Ok(artifact)
    }
}

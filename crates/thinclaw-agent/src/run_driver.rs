use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use thinclaw_channels_core::IncomingMessage;
use uuid::Uuid;

use crate::ports::ThreadRuntimeSnapshot;
use crate::run_artifact::{
    AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus, RunRuntimeDescriptor,
};
use crate::session::{Session, Turn, TurnState};

#[async_trait]
pub trait RunThreadRuntimeLookup: Send + Sync {
    async fn load_thread_runtime(
        &self,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<ThreadRuntimeSnapshot>>;
}

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

    #[allow(clippy::too_many_arguments)]
    pub async fn record_chat_turn(
        &self,
        llm_provider: &str,
        llm_model: &str,
        runtime_lookup: Option<Arc<dyn RunThreadRuntimeLookup>>,
        session: &Session,
        thread_id: Uuid,
        incoming: &IncomingMessage,
        turn: &Turn,
    ) -> anyhow::Result<AgentRunArtifact> {
        let mut artifact = AgentRunArtifact::new(
            "chat",
            match turn.state {
                TurnState::Completed => AgentRunStatus::Completed,
                TurnState::Failed => AgentRunStatus::Failed,
                TurnState::Interrupted => AgentRunStatus::Interrupted,
                TurnState::Processing => AgentRunStatus::Interrupted,
            },
            turn.started_at,
            turn.completed_at,
        )
        .with_chat_turn_snapshot(session, thread_id, incoming, turn)
        .with_failure_reason(turn.error.clone())
        .with_runtime_descriptor(Some(&interactive_chat_run_runtime_descriptor()))
        .with_metadata(serde_json::json!({
            "turn_status": match turn.state {
                TurnState::Completed => "completed",
                TurnState::Failed => "failed",
                TurnState::Interrupted => "interrupted",
                TurnState::Processing => "processing",
            },
            "llm_provider": llm_provider,
            "llm_model": llm_model,
        }));

        if let Some(runtime_lookup) = runtime_lookup
            && let Some(thread_id) = artifact.thread_id
            && let Ok(Some(runtime)) = runtime_lookup.load_thread_runtime(thread_id).await
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

fn interactive_chat_run_runtime_descriptor() -> RunRuntimeDescriptor {
    RunRuntimeDescriptor::new(
        "interactive_chat",
        "agent_surface",
        "interactive_chat",
        vec![
            "conversation_state".to_string(),
            "llm_turn".to_string(),
            "thread_history".to_string(),
        ],
        Some("none".to_string()),
    )
}

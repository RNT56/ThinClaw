//! Root-compatible workspace prompt assembly adapter for extracted agent ports.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{
    SkillContext, WorkspacePromptAssembly, WorkspacePromptAssemblyPort, WorkspacePromptMaterials,
    WorkspacePromptRequest,
};
use thinclaw_agent::prompt_assembly::assemble_workspace_prompt_materials;

use crate::error::WorkspaceError;

pub struct RootWorkspacePromptAssemblyPort;

impl RootWorkspacePromptAssemblyPort {
    pub fn shared() -> Arc<dyn WorkspacePromptAssemblyPort> {
        Arc::new(Self)
    }
}

#[async_trait]
impl WorkspacePromptAssemblyPort for RootWorkspacePromptAssemblyPort {
    async fn load_prompt_materials(
        &self,
        request: &WorkspacePromptRequest,
    ) -> Result<WorkspacePromptMaterials, WorkspaceError> {
        Ok(WorkspacePromptMaterials {
            post_compaction_context: request
                .existing_runtime
                .as_ref()
                .and_then(|runtime| runtime.post_compaction_context.clone()),
            ..WorkspacePromptMaterials::default()
        })
    }

    async fn assemble_workspace_prompt(
        &self,
        _request: WorkspacePromptRequest,
        materials: WorkspacePromptMaterials,
        skills: SkillContext,
    ) -> Result<WorkspacePromptAssembly, WorkspaceError> {
        let assembly = assemble_workspace_prompt_materials(&materials, &skills);

        Ok(WorkspacePromptAssembly {
            materials,
            skill_context: skills,
            assembly,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workspace_prompt_assembly_keeps_stable_and_ephemeral_segments_separate() {
        let port = RootWorkspacePromptAssemblyPort;
        let result = port
            .assemble_workspace_prompt(
                WorkspacePromptRequest {
                    scope: thinclaw_agent::ports::AgentScope::new("user-1", "actor-1"),
                    user_input: "hello".to_string(),
                    channel: "web".to_string(),
                    routed_workspace_id: None,
                    agent_system_prompt: None,
                    session_freeze_enabled: false,
                    existing_runtime: None,
                    metadata: serde_json::Value::Null,
                },
                WorkspacePromptMaterials {
                    workspace_prompt: Some("You are ThinClaw.".to_string()),
                    provider_recall_block: Some("memory".to_string()),
                    provider_context_refs: vec!["ctx-1".to_string()],
                    ..WorkspacePromptMaterials::default()
                },
                SkillContext::default(),
            )
            .await
            .expect("assembly");

        assert!(
            result
                .assembly
                .stable_snapshot
                .contains("You are ThinClaw.")
        );
        assert_eq!(result.assembly.ephemeral_documents, vec!["memory"]);
        assert_eq!(result.assembly.provider_context_refs, vec!["ctx-1"]);
    }
}

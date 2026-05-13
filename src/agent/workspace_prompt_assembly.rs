//! Root-compatible workspace prompt assembly adapter for extracted agent ports.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{
    SkillContext, WorkspacePromptAssembly, WorkspacePromptAssemblyPort, WorkspacePromptMaterials,
    WorkspacePromptRequest,
};

use crate::agent::prompt_assembly::PromptAssemblyV2;
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
        let skill_index = skills
            .available_index_block
            .as_ref()
            .map(|block| format!("## Skills\n{block}"))
            .unwrap_or_default();
        let active_skills = skills
            .active_skill_block
            .as_ref()
            .map(|block| format!("## Skill Expansion\n{block}"))
            .unwrap_or_default();

        let assembly = PromptAssemblyV2::new()
            .push_stable(
                "workspace_prompt",
                materials.workspace_prompt.clone().unwrap_or_default(),
            )
            .push_stable(
                "provider_system_prompt",
                materials.provider_system_prompt.clone().unwrap_or_default(),
            )
            .push_stable("skills_index", skill_index)
            .push_ephemeral(
                "provider_recall",
                materials.provider_recall_block.clone().unwrap_or_default(),
            )
            .push_ephemeral(
                "linked_recall",
                materials.linked_recall_block.clone().unwrap_or_default(),
            )
            .push_ephemeral(
                "channel_formatting_hints",
                materials
                    .channel_formatting_hints
                    .clone()
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "runtime_capabilities",
                materials
                    .runtime_capability_hint
                    .clone()
                    .unwrap_or_default(),
            )
            .push_ephemeral("active_skills", active_skills)
            .push_ephemeral(
                "post_compaction_fragment",
                materials
                    .post_compaction_context
                    .clone()
                    .unwrap_or_default(),
            )
            .with_provider_context_refs(materials.provider_context_refs.clone())
            .build();

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

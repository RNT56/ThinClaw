//! Compatibility adapters for extracted agent-management tools.

use async_trait::async_trait;
pub use thinclaw_tools::builtin::agent_management::{
    AgentManagementPort, AgentToolRecord, AgentToolWorkspace,
};

use crate::agent::agent_registry::AgentRegistry;
use crate::agent::agent_router::AgentWorkspace;
use crate::db::AgentWorkspaceRecord;
use crate::tools::ToolProfile;

pub use thinclaw_tools::builtin::agent_management::{
    CreateAgentTool, ListAgentsTool, MessageAgentTool, RemoveAgentTool, UpdateAgentTool,
};

fn record_to_tool_record(record: AgentWorkspaceRecord) -> AgentToolRecord {
    AgentToolRecord {
        id: record.id,
        agent_id: record.agent_id,
        display_name: record.display_name,
        system_prompt: record.system_prompt,
        model: record.model,
        allowed_tools: record.allowed_tools,
        allowed_skills: record.allowed_skills,
        tool_profile: record.tool_profile,
        is_default: record.is_default,
    }
}

fn workspace_to_tool_workspace(workspace: AgentWorkspace) -> AgentToolWorkspace {
    AgentToolWorkspace {
        agent_id: workspace.agent_id,
        display_name: workspace.display_name,
        system_prompt: workspace.system_prompt,
        model: workspace.model,
        bound_channels: workspace.bound_channels,
        trigger_keywords: workspace.trigger_keywords,
        allowed_tools: workspace.allowed_tools,
        allowed_skills: workspace.allowed_skills,
        tool_profile: workspace.tool_profile,
        is_default: workspace.is_default,
    }
}

fn tool_record_to_db_record(record: &AgentToolRecord) -> AgentWorkspaceRecord {
    let now = chrono::Utc::now();
    AgentWorkspaceRecord {
        id: record.id,
        agent_id: record.agent_id.clone(),
        display_name: record.display_name.clone(),
        system_prompt: record.system_prompt.clone(),
        model: record.model.clone(),
        bound_channels: Vec::new(),
        trigger_keywords: Vec::new(),
        allowed_tools: record.allowed_tools.clone(),
        allowed_skills: record.allowed_skills.clone(),
        tool_profile: record.tool_profile,
        is_default: record.is_default,
        created_at: now,
        updated_at: now,
    }
}

#[async_trait]
impl AgentManagementPort for AgentRegistry {
    async fn create_agent(
        &self,
        agent_id: &str,
        display_name: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        bound_channels: Vec<String>,
        trigger_keywords: Vec<String>,
        is_default: bool,
        allowed_tools: Option<Vec<String>>,
        allowed_skills: Option<Vec<String>>,
        tool_profile: Option<ToolProfile>,
    ) -> Result<AgentToolRecord, String> {
        AgentRegistry::create_agent(
            self,
            agent_id,
            display_name,
            system_prompt,
            model,
            bound_channels,
            trigger_keywords,
            is_default,
            allowed_tools,
            allowed_skills,
            tool_profile,
        )
        .await
        .map(record_to_tool_record)
        .map_err(|error| error.to_string())
    }

    async fn list_agents(&self) -> Vec<AgentToolWorkspace> {
        AgentRegistry::list_agents(self)
            .await
            .into_iter()
            .map(workspace_to_tool_workspace)
            .collect()
    }

    async fn update_agent(
        &self,
        agent_id: &str,
        display_name: Option<&str>,
        system_prompt: Option<Option<&str>>,
        model: Option<Option<&str>>,
        bound_channels: Option<Vec<String>>,
        trigger_keywords: Option<Vec<String>>,
        is_default: Option<bool>,
        allowed_tools: Option<Option<Vec<String>>>,
        allowed_skills: Option<Option<Vec<String>>>,
        tool_profile: Option<Option<ToolProfile>>,
    ) -> Result<AgentToolRecord, String> {
        AgentRegistry::update_agent(
            self,
            agent_id,
            display_name,
            system_prompt,
            model,
            bound_channels,
            trigger_keywords,
            is_default,
            allowed_tools,
            allowed_skills,
            tool_profile,
        )
        .await
        .map(record_to_tool_record)
        .map_err(|error| error.to_string())
    }

    async fn remove_agent(&self, agent_id: &str, force: bool) -> Result<bool, String> {
        AgentRegistry::remove_agent(self, agent_id, force)
            .await
            .map_err(|error| error.to_string())
    }

    async fn get_agent_record(&self, agent_id: &str) -> Result<Option<AgentToolRecord>, String> {
        AgentRegistry::get_agent_record(self, agent_id)
            .await
            .map(|record| record.map(record_to_tool_record))
            .map_err(|error| error.to_string())
    }

    async fn agent_context(&self, record: &AgentToolRecord, user_id: &str) -> String {
        let db_record = tool_record_to_db_record(record);
        let target_workspace = self.build_workspace_for_agent(&db_record, user_id);
        let system_prompt = if let Some(ref prompt) = record.system_prompt {
            prompt.clone()
        } else if let Some(ref ws) = target_workspace {
            match ws.read("IDENTITY.md").await {
                Ok(doc) => doc.content,
                Err(_) => format!("You are {}.", record.display_name),
            }
        } else {
            format!("You are {}.", record.display_name)
        };

        let mut context_parts = vec![system_prompt];
        if let Some(ref ws) = target_workspace
            && let Ok(memory) = ws.read("MEMORY.md").await
        {
            let content = memory.content.trim();
            if !content.is_empty() {
                context_parts.push(format!("\n--- Your Memory (MEMORY.md) ---\n{}", content));
            }
        }

        context_parts.join("\n")
    }
}

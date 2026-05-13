use std::path::PathBuf;

use crate::RuntimeWorkspaceMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainToolProfilePlan {
    Acp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpRuntimeConfigInput {
    pub workspace: Option<PathBuf>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpRuntimeConfigPlan {
    pub acp_channel_enabled: bool,
    pub main_tool_profile: MainToolProfilePlan,
    pub workspace_mode: Option<RuntimeWorkspaceMode>,
    pub workspace_root: Option<PathBuf>,
    pub model_override: Option<String>,
}

impl AcpRuntimeConfigPlan {
    pub fn from_input(input: AcpRuntimeConfigInput) -> Self {
        Self {
            acp_channel_enabled: true,
            main_tool_profile: MainToolProfilePlan::Acp,
            workspace_mode: input
                .workspace
                .as_ref()
                .map(|_| RuntimeWorkspaceMode::Project),
            workspace_root: input.workspace,
            model_override: input.model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_runtime_plan_enables_protocol_profile() {
        let plan = AcpRuntimeConfigPlan::from_input(AcpRuntimeConfigInput {
            workspace: None,
            model: None,
        });

        assert!(plan.acp_channel_enabled);
        assert_eq!(plan.main_tool_profile, MainToolProfilePlan::Acp);
        assert_eq!(plan.workspace_mode, None);
        assert_eq!(plan.workspace_root, None);
        assert_eq!(plan.model_override, None);
    }

    #[test]
    fn acp_runtime_plan_promotes_workspace_to_project_mode() {
        let workspace = PathBuf::from("/workspace/project");
        let plan = AcpRuntimeConfigPlan::from_input(AcpRuntimeConfigInput {
            workspace: Some(workspace.clone()),
            model: Some("gpt-test".to_string()),
        });

        assert_eq!(plan.workspace_mode, Some(RuntimeWorkspaceMode::Project));
        assert_eq!(plan.workspace_root, Some(workspace));
        assert_eq!(plan.model_override.as_deref(), Some("gpt-test"));
    }
}

//! Root adapter bundle for extracted agent-runtime ports.

use std::sync::Arc;

use thinclaw_agent::ports::{
    ChannelStatusPort, HookDispatchPort, LearningOutcomesPort, ModelOverridePort,
    RoutineExecutionPort, RoutineStorePort, SettingsPort, SkillContextPort, ThreadStorePort,
    ToolExecutionPort, WorkspacePromptAssemblyPort,
};
use tokio::sync::RwLock;

use crate::agent::{
    RootChannelStatusPort, RootHookDispatchPort, RootLearningOutcomesPort, RootModelOverridePort,
    RootRoutineExecutionPort, RootRoutineStorePort, RootSettingsPort, RootSkillContextPort,
    RootThreadStorePort, RootToolExecutionPort, RootWorkspacePromptAssemblyPort,
};
use crate::channels::ChannelManager;
use crate::config::SkillsConfig;
use crate::db::Database;
use crate::hooks::HookRegistry;
use crate::safety::SafetyLayer;
use crate::skills::SkillRegistry;
use crate::tools::{ToolRegistry, builtin::SharedModelOverride};

/// Concrete root-backed implementations of the agent crate's runtime ports.
///
/// This keeps compatibility wiring in one place while extracted agent code
/// depends only on `thinclaw_agent::ports`.
#[derive(Clone)]
pub struct RootAgentRuntimePorts {
    pub channel_status: Arc<dyn ChannelStatusPort>,
    pub hooks: Arc<dyn HookDispatchPort>,
    pub tools: Arc<dyn ToolExecutionPort>,
    pub model_overrides: Option<Arc<dyn ModelOverridePort>>,
    pub learning_outcomes: Option<Arc<dyn LearningOutcomesPort>>,
    pub skills: Option<Arc<dyn SkillContextPort>>,
    pub prompt_assembly: Arc<dyn WorkspacePromptAssemblyPort>,
    pub settings: Option<Arc<dyn SettingsPort>>,
    pub threads: Option<Arc<dyn ThreadStorePort>>,
    pub routines: Option<Arc<dyn RoutineStorePort>>,
    pub routine_execution: Option<Arc<dyn RoutineExecutionPort>>,
}

impl RootAgentRuntimePorts {
    pub fn new(
        channels: Arc<ChannelManager>,
        hooks: Arc<HookRegistry>,
        tools: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        store: Option<Arc<dyn Database>>,
        model_override: Option<SharedModelOverride>,
        skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
        skills_config: SkillsConfig,
        routine_engine: Option<Arc<crate::agent::RoutineEngine>>,
    ) -> Self {
        Self {
            channel_status: RootChannelStatusPort::shared(channels),
            hooks: RootHookDispatchPort::shared(hooks),
            tools: RootToolExecutionPort::shared(tools, safety),
            model_overrides: model_override.map(RootModelOverridePort::shared),
            learning_outcomes: store
                .as_ref()
                .map(|store| RootLearningOutcomesPort::shared(Arc::clone(store))),
            skills: skill_registry
                .map(|registry| RootSkillContextPort::shared(registry, skills_config)),
            prompt_assembly: RootWorkspacePromptAssemblyPort::shared(),
            settings: store
                .as_ref()
                .map(|store| RootSettingsPort::shared(Arc::clone(store))),
            threads: store
                .as_ref()
                .map(|store| RootThreadStorePort::shared(Arc::clone(store))),
            routines: store.map(RootRoutineStorePort::shared),
            routine_execution: routine_engine.map(RootRoutineExecutionPort::shared),
        }
    }

    pub fn has_persistence(&self) -> bool {
        self.settings.is_some()
            && self.threads.is_some()
            && self.routines.is_some()
            && self.learning_outcomes.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_flag_requires_all_database_ports() {
        let ports = RootAgentRuntimePorts {
            channel_status: Arc::new(NoopChannelStatus),
            hooks: Arc::new(NoopHooks),
            tools: Arc::new(NoopTools),
            model_overrides: None,
            learning_outcomes: None,
            skills: None,
            prompt_assembly: RootWorkspacePromptAssemblyPort::shared(),
            settings: None,
            threads: None,
            routines: None,
            routine_execution: None,
        };

        assert!(!ports.has_persistence());
    }

    struct NoopChannelStatus;

    #[async_trait::async_trait]
    impl ChannelStatusPort for NoopChannelStatus {
        async fn respond(
            &self,
            _original: &crate::channels::IncomingMessage,
            _response: crate::channels::OutgoingResponse,
        ) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }

        async fn send_status(
            &self,
            _target: &thinclaw_agent::ports::ChannelTarget,
            _status: crate::channels::StatusUpdate,
        ) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }

        async fn broadcast(
            &self,
            _target: &thinclaw_agent::ports::ChannelTarget,
            _response: crate::channels::OutgoingResponse,
        ) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }
    }

    struct NoopHooks;

    #[async_trait::async_trait]
    impl HookDispatchPort for NoopHooks {
        async fn dispatch_hook(
            &self,
            _event: thinclaw_agent::ports::AgentHookEvent,
            _context: thinclaw_agent::ports::AgentHookContext,
        ) -> Result<thinclaw_agent::ports::AgentHookOutcome, thinclaw_agent::ports::HookPortError>
        {
            Ok(thinclaw_agent::ports::AgentHookOutcome::Continue { modified: None })
        }
    }

    struct NoopTools;

    #[async_trait::async_trait]
    impl ToolExecutionPort for NoopTools {
        async fn list_tools(
            &self,
        ) -> Result<Vec<thinclaw_tools_core::ToolDescriptor>, crate::error::ToolError> {
            Ok(Vec::new())
        }

        async fn get_tool(
            &self,
            _name: &str,
        ) -> Result<Option<thinclaw_tools_core::ToolDescriptor>, crate::error::ToolError> {
            Ok(None)
        }

        async fn prepare_tool(
            &self,
            _request: thinclaw_agent::ports::ToolExecutionRequest,
        ) -> Result<thinclaw_agent::ports::ToolPreparation, crate::error::ToolError> {
            Err(crate::error::ToolError::NotFound {
                name: "noop".to_string(),
            })
        }

        async fn execute_tool(
            &self,
            _request: thinclaw_agent::ports::ToolExecutionRequest,
        ) -> Result<thinclaw_agent::ports::ToolExecutionResult, crate::error::ToolError> {
            Err(crate::error::ToolError::NotFound {
                name: "noop".to_string(),
            })
        }
    }
}

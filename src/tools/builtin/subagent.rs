//! Compatibility adapters for extracted sub-agent tools.

use async_trait::async_trait;
use uuid::Uuid;

use crate::agent::subagent_executor::SubagentExecutor;

pub use thinclaw_tools::builtin::subagent::{
    CancelSubagentTool, ListSubagentsTool, SpawnSubagentTool, SubagentSpawnRequest,
    SubagentToolPort,
};

#[async_trait]
impl SubagentToolPort for SubagentExecutor {
    async fn list_subagents(&self) -> Vec<serde_json::Value> {
        self.list()
            .await
            .into_iter()
            .filter_map(|info| serde_json::to_value(info).ok())
            .collect()
    }

    async fn cancel_subagent(&self, agent_id: Uuid) -> bool {
        self.cancel(agent_id).await
    }
}

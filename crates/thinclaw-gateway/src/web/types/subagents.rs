//! Stable authenticated gateway contract for durable sub-agent runs.

use serde::{Deserialize, Serialize};
use thinclaw_types::subagent::SubagentRunRecord;
use thinclaw_types::{
    SubagentMemoryMode, SubagentSkillMode, SubagentTaskPacket, SubagentToolMode, ToolProfile,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnApiRequest {
    pub name: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_packet: Option<SubagentTaskPacket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mode: Option<SubagentMemoryMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_mode: Option<SubagentToolMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_mode: Option<SubagentSkillMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_profile: Option<ToolProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub wait: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentExecutionResult {
    pub agent_id: Uuid,
    pub name: String,
    pub response: String,
    pub iterations: usize,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnApiResponse {
    pub run: SubagentRunRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SubagentExecutionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRunsApiResponse {
    pub runs: Vec<SubagentRunRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRunApiResponse {
    pub run: SubagentRunRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentCancelApiResponse {
    pub agent_id: Uuid,
    /// Mirrors `cancel_subagent`: false means not running/already terminal.
    pub cancelled: bool,
    pub status: String,
    pub run: SubagentRunRecord,
}

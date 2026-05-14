//! Shared sub-agent task assignment types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentMemoryMode {
    #[default]
    ProvidedContextOnly,
    GrantedToolsOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentToolMode {
    #[default]
    ExplicitOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentSkillMode {
    #[default]
    ExplicitOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentProvidedContext {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubagentTaskPacket {
    pub objective: String,
    #[serde(default)]
    pub todos: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub provided_context: Vec<SubagentProvidedContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_summary: Option<String>,
}

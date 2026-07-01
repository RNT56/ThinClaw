//! Pending MCP server-initiated interaction DTOs (sampling/elicitation). Moved
//! from `thinclaw-tools` so light consumers (e.g. `thinclaw-gateway`) can
//! reference them without depending on the heavyweight tool runtime.
//! `thinclaw_tools::mcp::client` re-exports them for path stability.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpInteractionKind {
    Sampling,
    Elicitation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPendingInteraction {
    pub id: String,
    pub server_name: String,
    pub method: String,
    pub kind: McpInteractionKind,
    pub title: String,
    pub description: String,
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    pub created_at: String,
}

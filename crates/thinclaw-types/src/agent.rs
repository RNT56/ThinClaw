use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ToolProfile;

/// Persistent record for an agent workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkspaceRecord {
    /// Primary key (UUID).
    pub id: Uuid,
    /// Unique human-readable identifier (slug). Validated: `[a-z0-9_-]{2,32}`.
    pub agent_id: String,
    /// Display name for the agent.
    pub display_name: String,
    /// System prompt override for this agent.
    pub system_prompt: Option<String>,
    /// Model override (e.g. "openai/gpt-4o").
    pub model: Option<String>,
    /// Channels this agent is bound to (empty = all channels).
    pub bound_channels: Vec<String>,
    /// Keywords/mentions that trigger routing to this agent.
    pub trigger_keywords: Vec<String>,
    /// Optional per-agent tool allowlist.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Optional per-agent skill allowlist.
    #[serde(default)]
    pub allowed_skills: Option<Vec<String>>,
    /// Optional execution profile override for this agent workspace.
    #[serde(default)]
    pub tool_profile: Option<ToolProfile>,
    /// Whether this is the default agent (receives unrouted messages).
    pub is_default: bool,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
    /// When the record was last updated.
    pub updated_at: DateTime<Utc>,
}

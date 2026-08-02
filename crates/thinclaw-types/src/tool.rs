//! Shared tool-related domain types.

use serde::{Deserialize, Serialize};

/// Stable identity shared by every event emitted for one tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct ToolInvocationId(String);

impl ToolInvocationId {
    pub fn from_provider(value: &str) -> Self {
        let value = value.trim();
        if !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control) {
            Self(format!("provider:{value}"))
        } else {
            Self(format!("generated:{}", uuid::Uuid::new_v4()))
        }
    }

    pub fn generated() -> Self {
        Self(format!("generated:{}", uuid::Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolInvocationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Execution profile for a tool-capable agent lane.
///
/// Profiles determine which tools are implicitly available before explicit
/// grants (for example `allowed_tools`) are considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolProfile {
    /// All lane-eligible tools are implicitly available.
    #[default]
    Standard,
    /// Only safe read-only orchestrator tools are implicitly available.
    Restricted,
    /// Only coordination tools are implicitly available.
    ExplicitOnly,
    /// Editor-native Agent Client Protocol profile.
    ///
    /// ACP exposes local code, memory, skills, browser, and sub-agent tools while
    /// intentionally excluding async messaging, cron/routines, and broad channel
    /// management tools that do not belong inside an editor client.
    Acp,
}

impl ToolProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Restricted => "restricted",
            Self::ExplicitOnly => "explicit_only",
            Self::Acp => "acp",
        }
    }
}

impl std::str::FromStr for ToolProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "restricted" => Ok(Self::Restricted),
            "explicit_only" => Ok(Self::ExplicitOnly),
            "acp" => Ok(Self::Acp),
            other => Err(format!("Invalid tool_profile '{other}'")),
        }
    }
}

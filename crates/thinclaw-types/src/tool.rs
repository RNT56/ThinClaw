//! Shared tool-related domain types.

use serde::{Deserialize, Serialize};

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

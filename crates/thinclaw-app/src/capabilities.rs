//! Neutral, serializable capability and health vocabulary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadinessProfile {
    Server,
    Remote,
    Desktop,
    PiOsLite64,
    AllFeatures,
}

impl ReadinessProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Remote => "remote",
            Self::Desktop => "desktop",
            Self::PiOsLite64 => "pi-os-lite-64",
            Self::AllFeatures => "all-features",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactState {
    Yes,
    No,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Active,
    Inactive,
    Degraded,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Ready,
    NotReady,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyState {
    Available,
    Missing,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Unhealthy,
    Unknown,
    NotProbed,
    NotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutcome {
    pub health: HealthState,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFact {
    pub id: String,
    pub label: String,
    pub compiled: FactState,
    pub configured: FactState,
    pub available: FactState,
    pub active: ActivityState,
    pub ready: ReadinessState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub schema_version: u8,
    pub revision: String,
    pub profile: String,
    pub runtime_active: FactState,
    pub healthy: bool,
    pub facts: Vec<CapabilityFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapabilityFact {
    pub name: String,
    pub source_id: String,
    pub label: String,
    pub origin: String,
    pub compiled: FactState,
    pub configured: FactState,
    pub registered: FactState,
    pub dependency: DependencyState,
    pub exposed: FactState,
    pub approval: String,
    pub health: HealthState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapabilitySnapshot {
    pub schema_version: u8,
    pub revision: String,
    pub readiness_profile: ReadinessProfile,
    pub live: bool,
    pub tools: Vec<ToolCapabilityFact>,
}

impl CapabilitySnapshot {
    pub fn sort_facts(&mut self) {
        self.facts.sort_by(|left, right| left.id.cmp(&right.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_are_not_collapsed_into_one_ordered_state() {
        let fact = CapabilityFact {
            id: "database".to_string(),
            label: "Database".to_string(),
            compiled: FactState::Yes,
            configured: FactState::Yes,
            available: FactState::Unknown,
            active: ActivityState::Inactive,
            ready: ReadinessState::Unknown,
            reasons: Vec::new(),
            remediation: Vec::new(),
        };
        assert_ne!(fact.configured, fact.available);
    }
}

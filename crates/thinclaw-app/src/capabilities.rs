//! Neutral, serializable capability and health vocabulary.

use serde::{Deserialize, Serialize};

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

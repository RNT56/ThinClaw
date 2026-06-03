//! Root-independent learning domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Broad class of improvement a learning event or candidate belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementClass {
    Memory,
    Skill,
    Prompt,
    Routine,
    Code,
    #[default]
    Unknown,
}

impl ImprovementClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Skill => "skill",
            Self::Prompt => "prompt",
            Self::Routine => "routine",
            Self::Code => "code",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "memory" => Self::Memory,
            "skill" => Self::Skill,
            "prompt" => Self::Prompt,
            "routine" => Self::Routine,
            "code" => Self::Code,
            _ => Self::Unknown,
        }
    }

    pub fn from_str(value: &str) -> Self {
        Self::parse(value)
    }
}

impl std::str::FromStr for ImprovementClass {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse(value))
    }
}

/// Risk tier for a potential improvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl RiskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Medium,
        }
    }

    pub fn from_str(value: &str) -> Self {
        Self::parse(value)
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

impl std::str::FromStr for RiskTier {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse(value))
    }
}

/// How the learning loop should treat a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningDecision {
    Ignore,
    RecordOnly,
    AutoApply,
    Propose,
}

/// An improvement distilled from one or more learning events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementCandidate {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<Uuid>,
    pub class: ImprovementClass,
    pub risk_tier: RiskTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub reason: String,
    pub confidence: f32,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl Default for ImprovementCandidate {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            source_event_id: None,
            class: ImprovementClass::Unknown,
            risk_tier: RiskTier::Low,
            target: None,
            reason: String::new(),
            confidence: 0.0,
            metadata: serde_json::Value::Null,
            created_at: Utc::now(),
        }
    }
}

/// Metadata captured around a learned artifact revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub id: Uuid,
    pub artifact_name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// User/operator feedback on a learned artifact or candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningFeedback {
    pub id: Uuid,
    pub target: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Lifecycle state for approval-gated proposals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    #[default]
    Draft,
    PendingApproval,
    Approved,
    Applied,
    Rejected,
    RolledBack,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_class_and_risk_parse_compatibly() {
        assert_eq!(ImprovementClass::parse("skill"), ImprovementClass::Skill);
        assert_eq!(
            ImprovementClass::parse("unknown-value"),
            ImprovementClass::Unknown
        );
        assert_eq!(RiskTier::parse("critical"), RiskTier::Critical);
        assert_eq!(RiskTier::parse("unknown-value"), RiskTier::Medium);
        assert!(RiskTier::Critical.rank() > RiskTier::High.rank());
    }

    #[test]
    fn improvement_candidate_default_matches_legacy_values() {
        let candidate = ImprovementCandidate::default();

        assert_eq!(candidate.class, ImprovementClass::Unknown);
        assert_eq!(candidate.risk_tier, RiskTier::Low);
        assert_eq!(candidate.confidence, 0.0);
        assert_eq!(candidate.metadata, serde_json::Value::Null);
    }
}

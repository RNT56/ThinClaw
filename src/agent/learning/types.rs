use super::*;

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

    pub fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "memory" => Self::Memory,
            "skill" => Self::Skill,
            "prompt" => Self::Prompt,
            "routine" => Self::Routine,
            "code" => Self::Code,
            _ => Self::Unknown,
        }
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

    pub fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Medium,
        }
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

/// How the learning loop should treat a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningDecision {
    Ignore,
    RecordOnly,
    AutoApply,
    Propose,
}

/// A durable record of a learning-relevant event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvent {
    pub id: Uuid,
    pub source: String,
    pub class: ImprovementClass,
    pub risk_tier: RiskTier,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl LearningEvent {
    pub fn new(
        source: impl Into<String>,
        class: ImprovementClass,
        risk_tier: RiskTier,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: source.into(),
            class,
            risk_tier,
            summary: summary.into(),
            target: None,
            confidence: None,
            metadata: serde_json::Value::Null,
            created_at: Utc::now(),
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence.clamp(0.0, 1.0));
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Convert into the DB-backed learning event shape.
    pub fn into_persisted(
        self,
        user_id: String,
        actor_id: Option<String>,
        channel: Option<String>,
        thread_id: Option<String>,
        conversation_id: Option<Uuid>,
        message_id: Option<Uuid>,
        job_id: Option<Uuid>,
    ) -> DbLearningEvent {
        let mut payload = self.metadata;
        if !payload.is_object() {
            payload = serde_json::json!({});
        }
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("class".to_string(), serde_json::json!(self.class.as_str()));
            obj.insert(
                "risk_tier".to_string(),
                serde_json::json!(self.risk_tier.as_str()),
            );
            obj.insert(
                "summary".to_string(),
                serde_json::json!(self.summary.clone()),
            );
            if let Some(target) = self.target.clone() {
                obj.insert("target".to_string(), serde_json::json!(target));
            }
            if let Some(confidence) = self.confidence {
                obj.insert("confidence".to_string(), serde_json::json!(confidence));
            }
        }

        DbLearningEvent {
            id: self.id,
            user_id,
            actor_id,
            channel,
            thread_id,
            conversation_id,
            message_id,
            job_id,
            event_type: self.class.as_str().to_string(),
            source: self.source,
            payload,
            metadata: Some(serde_json::json!({
                "risk_tier": self.risk_tier.as_str(),
                "summary": self.summary,
                "target": self.target,
                "confidence": self.confidence,
            })),
            created_at: self.created_at,
        }
    }
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

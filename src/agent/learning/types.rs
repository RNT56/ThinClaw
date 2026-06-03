use super::*;

pub use thinclaw_agent::learning_types::{
    ArtifactVersion, ImprovementCandidate, ImprovementClass, LearningDecision, LearningFeedback,
    ProposalState, RiskTier,
};

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

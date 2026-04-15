//! Learning scaffolding and orchestration for ThinClaw's closed-loop improvement system.
//!
//! This module provides:
//! - Core learning-domain types (candidate/risk/decision/proposal state)
//! - Optional external memory providers (Honcho + Zep)
//! - A local-first `LearningOrchestrator` that records evaluations,
//!   creates candidates, applies low-risk mutations, and tracks code proposals.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use crate::db::Database;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion, LearningCandidate as DbLearningCandidate,
    LearningCodeProposal as DbLearningCodeProposal, LearningEvaluation as DbLearningEvaluation,
    LearningEvent as DbLearningEvent, LearningFeedbackRecord as DbLearningFeedbackRecord,
};
use crate::settings::LearningSettings;
use crate::skills::registry::SkillRegistry;
use crate::workspace::{Workspace, paths};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMemoryHit {
    pub provider: String,
    pub summary: String,
    pub score: Option<f64>,
    pub provenance: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthStatus {
    pub provider: String,
    pub enabled: bool,
    pub healthy: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus;
    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String>;
    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String>;
}

#[derive(Default)]
pub struct HonchoProvider;

#[derive(Default)]
pub struct ZepProvider;

fn provider_base_url(config: &std::collections::HashMap<String, String>) -> Option<String> {
    config
        .get("base_url")
        .or_else(|| config.get("url"))
        .cloned()
        .filter(|v| !v.trim().is_empty())
}

fn provider_token(config: &std::collections::HashMap<String, String>) -> Option<String> {
    if let Some(token) = config.get("api_key").cloned().filter(|v| !v.is_empty()) {
        return Some(token);
    }
    if let Some(env_name) = config
        .get("api_key_env")
        .cloned()
        .filter(|v| !v.trim().is_empty())
    {
        return std::env::var(env_name)
            .ok()
            .filter(|v| !v.trim().is_empty());
    }
    None
}

async fn provider_health_request(
    provider_name: &str,
    enabled: bool,
    base_url: Option<String>,
    token: Option<String>,
) -> ProviderHealthStatus {
    if !enabled {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            enabled,
            healthy: false,
            latency_ms: None,
            error: None,
            metadata: serde_json::json!({"state": "disabled"}),
        };
    }

    let Some(base_url) = base_url else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            enabled,
            healthy: false,
            latency_ms: None,
            error: Some("missing base_url".to_string()),
            metadata: serde_json::json!({}),
        };
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            enabled,
            healthy: false,
            latency_ms: None,
            error: Some("failed to initialize HTTP client".to_string()),
            metadata: serde_json::json!({}),
        };
    };

    let started = std::time::Instant::now();
    let mut req = client.get(format!("{}/health", base_url.trim_end_matches('/')));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }

    match req.send().await {
        Ok(response) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            enabled,
            healthy: response.status().is_success(),
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: if response.status().is_success() {
                None
            } else {
                Some(format!("HTTP {}", response.status()))
            },
            metadata: serde_json::json!({"status": response.status().as_u16()}),
        },
        Err(err) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            enabled,
            healthy: false,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: Some(err.to_string()),
            metadata: serde_json::json!({}),
        },
    }
}

#[async_trait]
impl MemoryProvider for HonchoProvider {
    fn name(&self) -> &'static str {
        "honcho"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        provider_health_request(
            self.name(),
            settings.providers.honcho.enabled,
            provider_base_url(&settings.providers.honcho.config),
            provider_token(&settings.providers.honcho.config),
        )
        .await
    }

    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        if !settings.providers.honcho.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url(&settings.providers.honcho.config)
            .ok_or_else(|| "Honcho base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.honcho.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/v1/search", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "query": query,
                "limit": limit,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }

        let response = req.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Honcho search failed: HTTP {}", response.status()));
        }
        let json = response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        let hits = json
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|item| ProviderMemoryHit {
                provider: self.name().to_string(),
                summary: item
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("text").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string(),
                score: item.get("score").and_then(|v| v.as_f64()),
                provenance: item,
            })
            .collect();
        Ok(hits)
    }

    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        if !settings.providers.honcho.enabled {
            return Ok(());
        }
        let base_url = provider_base_url(&settings.providers.honcho.config)
            .ok_or_else(|| "Honcho base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.honcho.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/v1/ingest", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "payload": payload,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let response = req.send().await.map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Honcho ingest failed: HTTP {}", response.status()))
        }
    }
}

#[async_trait]
impl MemoryProvider for ZepProvider {
    fn name(&self) -> &'static str {
        "zep"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        provider_health_request(
            self.name(),
            settings.providers.zep.enabled,
            provider_base_url(&settings.providers.zep.config),
            provider_token(&settings.providers.zep.config),
        )
        .await
    }

    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        if !settings.providers.zep.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url(&settings.providers.zep.config)
            .ok_or_else(|| "Zep base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.zep.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/api/v1/search", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "query": query,
                "limit": limit,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }

        let response = req.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Zep search failed: HTTP {}", response.status()));
        }
        let json = response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        let hits = json
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|item| ProviderMemoryHit {
                provider: self.name().to_string(),
                summary: item
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("content").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string(),
                score: item.get("score").and_then(|v| v.as_f64()),
                provenance: item,
            })
            .collect();
        Ok(hits)
    }

    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        if !settings.providers.zep.enabled {
            return Ok(());
        }
        let base_url = provider_base_url(&settings.providers.zep.config)
            .ok_or_else(|| "Zep base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.zep.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/api/v1/events", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "payload": payload,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let response = req.send().await.map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Zep export failed: HTTP {}", response.status()))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningOutcome {
    pub trigger: String,
    pub event_id: Uuid,
    pub evaluation_id: Option<Uuid>,
    pub candidate_id: Option<Uuid>,
    pub auto_applied: bool,
    pub code_proposal_id: Option<Uuid>,
    pub notes: Vec<String>,
}

pub struct LearningOrchestrator {
    store: Arc<dyn Database>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    providers: Vec<Arc<dyn MemoryProvider>>,
}

const PROPOSAL_SUPPRESSION_WINDOW_HOURS: i64 = 24 * 7;

impl LearningOrchestrator {
    pub fn new(
        store: Arc<dyn Database>,
        workspace: Option<Arc<Workspace>>,
        skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    ) -> Self {
        let providers: Vec<Arc<dyn MemoryProvider>> =
            vec![Arc::new(HonchoProvider), Arc::new(ZepProvider)];
        Self {
            store,
            workspace,
            skill_registry,
            providers,
        }
    }

    pub async fn load_settings_for_user(&self, user_id: &str) -> LearningSettings {
        match self.store.get_all_settings(user_id).await {
            Ok(map) => crate::settings::Settings::from_db_map(&map).learning,
            Err(_) => LearningSettings::default(),
        }
    }

    pub async fn provider_health(&self, user_id: &str) -> Vec<ProviderHealthStatus> {
        let settings = self.load_settings_for_user(user_id).await;
        let mut statuses = Vec::new();
        for provider in &self.providers {
            statuses.push(provider.health(&settings).await);
        }
        statuses
    }

    pub async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderMemoryHit> {
        let settings = self.load_settings_for_user(user_id).await;
        let mut all_hits = Vec::new();
        for provider in &self.providers {
            match provider.recall(&settings, user_id, query, limit).await {
                Ok(mut hits) => all_hits.append(&mut hits),
                Err(err) => {
                    tracing::debug!(
                        provider = provider.name(),
                        error = %err,
                        "learning provider recall skipped"
                    );
                }
            }
        }
        all_hits
    }

    pub async fn export_turn_to_providers(&self, user_id: &str, payload: &serde_json::Value) {
        let settings = self.load_settings_for_user(user_id).await;
        if !settings.exports.enabled {
            return;
        }
        for provider in &self.providers {
            if let Err(err) = provider.export_turn(&settings, user_id, payload).await {
                tracing::debug!(
                    provider = provider.name(),
                    error = %err,
                    "learning provider export skipped"
                );
            }
        }
    }

    pub async fn submit_feedback(
        &self,
        user_id: &str,
        target_type: &str,
        target_id: &str,
        verdict: &str,
        note: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, String> {
        let record = DbLearningFeedbackRecord {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
            verdict: verdict.to_string(),
            note: note.map(str::to_string),
            metadata: metadata.cloned().unwrap_or_else(|| serde_json::json!({})),
            created_at: Utc::now(),
        };
        let id = self
            .store
            .insert_learning_feedback(&record)
            .await
            .map_err(|e| e.to_string())?;

        let feedback_event = LearningEvent::new(
            "learning::explicit_feedback",
            ImprovementClass::Unknown,
            RiskTier::Medium,
            "Explicit user learning feedback received",
        )
        .with_target(format!("{target_type}:{target_id}"))
        .with_metadata(serde_json::json!({
            "target_type": target_type,
            "target_id": target_id,
            "verdict": verdict,
            "note": note,
            "feedback_id": id,
            "source": "learning_feedback_tool",
        }))
        .into_persisted(user_id.to_string(), None, None, None, None, None, None);
        if self
            .store
            .insert_learning_event(&feedback_event)
            .await
            .is_ok()
        {
            let _ = self
                .handle_event("explicit_user_feedback", &feedback_event)
                .await;
        }

        Ok(id)
    }

    pub async fn handle_event(
        &self,
        trigger: &str,
        event: &DbLearningEvent,
    ) -> Result<LearningOutcome, String> {
        let settings = self.load_settings_for_user(&event.user_id).await;
        let mut outcome = LearningOutcome {
            trigger: trigger.to_string(),
            event_id: event.id,
            evaluation_id: None,
            candidate_id: None,
            auto_applied: false,
            code_proposal_id: None,
            notes: Vec::new(),
        };

        if event.source == "learning::explicit_feedback" {
            outcome
                .notes
                .push("explicit feedback event recorded".to_string());
            return Ok(outcome);
        }

        if !settings.enabled {
            outcome
                .notes
                .push("learning disabled; event persisted only".to_string());
            return Ok(outcome);
        }

        if self.is_duplicate_or_cooldown(event).await {
            outcome
                .notes
                .push("duplicate/cooldown hit; skipped candidate generation".to_string());
            return Ok(outcome);
        }

        let (quality_score, evaluator_status, class, risk, confidence) =
            self.evaluate_event(event).await;

        let evaluation = DbLearningEvaluation {
            id: Uuid::new_v4(),
            learning_event_id: event.id,
            user_id: event.user_id.clone(),
            evaluator: "learning_orchestrator_v1".to_string(),
            status: evaluator_status,
            score: Some(quality_score as f64),
            details: serde_json::json!({
                "quality_score": quality_score,
                "class": class.as_str(),
                "risk_tier": risk.as_str(),
                "confidence": confidence,
            }),
            created_at: Utc::now(),
        };
        match self.store.insert_learning_evaluation(&evaluation).await {
            Ok(id) => outcome.evaluation_id = Some(id),
            Err(err) => {
                outcome
                    .notes
                    .push(format!("failed to persist evaluation: {err}"));
            }
        }

        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            candidate_type: class.as_str().to_string(),
            risk_tier: risk.as_str().to_string(),
            confidence: Some(confidence as f64),
            target_type: event
                .payload
                .get("target_type")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            target_name: event
                .payload
                .get("target")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            summary: Some(
                event
                    .payload
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Auto-distilled learning candidate")
                    .to_string(),
            ),
            proposal: event.payload.clone(),
            created_at: Utc::now(),
        };

        let candidate_id = self
            .store
            .insert_learning_candidate(&candidate)
            .await
            .map_err(|e| e.to_string())?;
        outcome.candidate_id = Some(candidate_id);

        if self.safe_mode_tripped(&settings, &event.user_id).await {
            outcome
                .notes
                .push("safe mode is active; candidate held for review".to_string());
            return Ok(outcome);
        }

        if risk.rank() >= RiskTier::High.rank() || class == ImprovementClass::Code {
            match self.create_code_proposal(event, &candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    outcome.notes.push(
                        "high-risk candidate routed to approval-gated code proposal".to_string(),
                    );
                }
                Err(err) => {
                    outcome
                        .notes
                        .push(format!("high-risk proposal suppressed: {err}"));
                }
            }
            return Ok(outcome);
        }

        let auto_apply_allowed = settings
            .auto_apply_classes
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(class.as_str()));
        if auto_apply_allowed
            && self
                .auto_apply_candidate(&settings, class, &candidate)
                .await
                .unwrap_or(false)
        {
            outcome.auto_applied = true;
            outcome
                .notes
                .push("candidate auto-applied in Tier A".to_string());
        } else {
            outcome
                .notes
                .push("candidate queued for manual review".to_string());
        }

        Ok(outcome)
    }

    async fn is_duplicate_or_cooldown(&self, event: &DbLearningEvent) -> bool {
        let Ok(recent) = self
            .store
            .list_learning_events(
                &event.user_id,
                event.actor_id.as_deref(),
                event.channel.as_deref(),
                event.thread_id.as_deref(),
                30,
            )
            .await
        else {
            return false;
        };

        let event_hash = stable_json_hash(&event.payload);
        for prior in recent {
            if prior.id == event.id {
                continue;
            }
            if prior.event_type != event.event_type || prior.source != event.source {
                continue;
            }
            if stable_json_hash(&prior.payload) != event_hash {
                continue;
            }
            let age_secs = (event.created_at - prior.created_at).num_seconds().abs();
            if age_secs <= 900 {
                return true;
            }
        }
        false
    }

    async fn evaluate_event(
        &self,
        event: &DbLearningEvent,
    ) -> (u32, String, ImprovementClass, RiskTier, f32) {
        let success = event
            .payload
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let wasted_tool_calls = event
            .payload
            .get("wasted_tool_calls")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let repeated_failures = event
            .payload
            .get("repeated_failures")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let correction_count = event
            .payload
            .get("correction_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let safety_incident = event
            .payload
            .get("safety_incident")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let class = classify_event(event);
        let mut risk = match class {
            ImprovementClass::Code => RiskTier::Critical,
            ImprovementClass::Prompt => RiskTier::Medium,
            ImprovementClass::Routine => RiskTier::Medium,
            ImprovementClass::Skill => RiskTier::Low,
            ImprovementClass::Memory => RiskTier::Low,
            ImprovementClass::Unknown => RiskTier::Medium,
        };
        if safety_incident {
            risk = RiskTier::Critical;
        }

        let mut score: i32 = if success { 82 } else { 45 };
        score -= (wasted_tool_calls as i32) * 4;
        score -= (repeated_failures as i32) * 7;
        score -= (correction_count as i32) * 5;
        if safety_incident {
            score -= 35;
        }
        score = score.clamp(0, 100);

        let confidence = ((score as f32 / 100.0)
            + if correction_count > 0 { 0.15 } else { 0.0 }
            + if repeated_failures > 0 { 0.1 } else { 0.0 })
        .clamp(0.0, 1.0);

        let status = if score >= 70 {
            "accepted"
        } else if score >= 45 {
            "review"
        } else {
            "poor"
        }
        .to_string();

        (score as u32, status, class, risk, confidence)
    }

    async fn safe_mode_tripped(&self, settings: &LearningSettings, user_id: &str) -> bool {
        if !settings.safe_mode.enabled {
            return false;
        }

        let feedback = match self
            .store
            .list_learning_feedback(user_id, None, None, 100)
            .await
        {
            Ok(feedback) => feedback,
            Err(_) => return false,
        };

        let rollbacks = self
            .store
            .list_learning_rollbacks(user_id, None, None, 100)
            .await
            .unwrap_or_default();

        let sample = feedback.len().max(rollbacks.len()) as u32;
        if sample < settings.safe_mode.thresholds.min_samples {
            return false;
        }

        let negative_feedback = feedback
            .iter()
            .filter(|entry| {
                matches!(
                    entry.verdict.to_ascii_lowercase().as_str(),
                    "harmful" | "revert" | "dont_learn" | "reject"
                )
            })
            .count() as f64;

        let feedback_ratio = negative_feedback / sample as f64;
        let rollback_ratio = rollbacks.len() as f64 / sample as f64;

        feedback_ratio >= settings.safe_mode.thresholds.negative_feedback_ratio
            || rollback_ratio >= settings.safe_mode.thresholds.rollback_ratio
    }

    async fn auto_apply_candidate(
        &self,
        settings: &LearningSettings,
        class: ImprovementClass,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        match class {
            ImprovementClass::Memory => self.auto_apply_memory(candidate).await,
            ImprovementClass::Prompt => {
                if !settings.prompt_mutation.enabled {
                    return Ok(false);
                }
                self.auto_apply_prompt(candidate).await
            }
            ImprovementClass::Skill => self.auto_apply_skill(candidate).await,
            _ => Ok(false),
        }
    }

    async fn auto_apply_memory(&self, candidate: &DbLearningCandidate) -> Result<bool, String> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Ok(false);
        };

        let entry = candidate
            .proposal
            .get("memory_entry")
            .and_then(|v| v.as_str())
            .or(candidate.summary.as_deref())
            .unwrap_or("New learning captured from recent interaction.");

        let before = workspace
            .read(paths::MEMORY)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();
        workspace
            .append_memory(entry)
            .await
            .map_err(|e| e.to_string())?;
        let after = workspace
            .read(paths::MEMORY)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "memory".to_string(),
            artifact_name: paths::MEMORY.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-appended memory entry".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({"auto_apply": true, "class": "memory"}),
            created_at: Utc::now(),
        };
        let _ = self.store.insert_learning_artifact_version(&version).await;

        Ok(true)
    }

    async fn auto_apply_prompt(&self, candidate: &DbLearningCandidate) -> Result<bool, String> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Ok(false);
        };

        let target = candidate
            .target_name
            .clone()
            .or_else(|| {
                candidate
                    .proposal
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| paths::USER.to_string());

        if !matches!(target.as_str(), paths::SOUL | paths::AGENTS | paths::USER) {
            return Ok(false);
        }

        let content = candidate
            .proposal
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "prompt candidate missing content".to_string())?;

        validate_prompt_content(content)?;

        let before = workspace
            .read(&target)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();
        workspace
            .write(&target, content)
            .await
            .map_err(|e| e.to_string())?;
        let after = workspace
            .read(&target)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "prompt".to_string(),
            artifact_name: target,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-applied prompt file update".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({"auto_apply": true, "class": "prompt"}),
            created_at: Utc::now(),
        };
        let _ = self.store.insert_learning_artifact_version(&version).await;

        Ok(true)
    }

    async fn auto_apply_skill(&self, candidate: &DbLearningCandidate) -> Result<bool, String> {
        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(false);
        };

        let Some(skill_content) = candidate
            .proposal
            .get("skill_content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return Ok(false);
        };

        let parsed = crate::skills::parser::parse_skill_md(&crate::skills::normalize_line_endings(
            &skill_content,
        ))
        .map_err(|e| e.to_string())?;
        let skill_name = parsed.manifest.name.clone();

        let mut guard = registry.write().await;
        let before_content = guard
            .find_by_name(&skill_name)
            .map(|s| s.prompt_content.clone());
        if guard.has(&skill_name) {
            let _ = guard.remove_skill(&skill_name).await;
        }
        guard
            .install_skill(&skill_content)
            .await
            .map_err(|e| e.to_string())?;

        let after_content = guard
            .find_by_name(&skill_name)
            .map(|s| s.prompt_content.clone());

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "skill".to_string(),
            artifact_name: skill_name,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-applied skill revision".to_string()),
            before_content,
            after_content,
            provenance: serde_json::json!({"auto_apply": true, "class": "skill"}),
            created_at: Utc::now(),
        };
        let _ = self.store.insert_learning_artifact_version(&version).await;

        Ok(true)
    }

    async fn create_code_proposal(
        &self,
        event: &DbLearningEvent,
        candidate: &DbLearningCandidate,
    ) -> Result<Uuid, String> {
        let title = event
            .payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Learning-driven code proposal")
            .to_string();
        let rationale = event
            .payload
            .get("rationale")
            .and_then(|v| v.as_str())
            .or(candidate.summary.as_deref())
            .unwrap_or("Distilled from repeated failures/corrections")
            .to_string();
        let target_files = event
            .payload
            .get("target_files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let diff = event
            .payload
            .get("diff")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let fingerprint = proposal_fingerprint(&title, &rationale, &target_files, &diff);

        if let Ok(rejected) = self
            .store
            .list_learning_code_proposals(&event.user_id, Some("rejected"), 64)
            .await
        {
            for prior in rejected {
                let prior_fp = prior
                    .metadata
                    .get("fingerprint")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        proposal_fingerprint(
                            &prior.title,
                            &prior.rationale,
                            &prior.target_files,
                            &prior.diff,
                        )
                    });
                if prior_fp != fingerprint {
                    continue;
                }
                let age_hours = (Utc::now() - prior.updated_at).num_hours().abs();
                if age_hours <= PROPOSAL_SUPPRESSION_WINDOW_HOURS {
                    return Err(format!(
                        "similar proposal was rejected {}h ago (fingerprint={}); cooldown active",
                        age_hours, fingerprint
                    ));
                }
            }
        }

        let evidence = event
            .payload
            .get("evidence")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "event_payload": event.payload }));

        let proposal = DbLearningCodeProposal {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            status: "proposed".to_string(),
            title: title.clone(),
            rationale: rationale.clone(),
            target_files: target_files.clone(),
            diff: diff.clone(),
            validation_results: event
                .payload
                .get("validation_results")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"status": "not_run"})),
            rollback_note: event
                .payload
                .get("rollback_note")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            confidence: candidate.confidence,
            branch_name: None,
            pr_url: None,
            metadata: serde_json::json!({
                "candidate_id": candidate.id,
                "source": event.source,
                "fingerprint": fingerprint,
                "package": {
                    "problem_statement": title,
                    "evidence": evidence,
                    "candidate_rationale": rationale,
                    "target_files": target_files,
                    "unified_diff": diff,
                    "validation_results": event.payload.get("validation_results").cloned().unwrap_or_else(|| serde_json::json!({"status": "not_run"})),
                    "rollback_note": event.payload.get("rollback_note").cloned().unwrap_or(serde_json::Value::Null),
                    "confidence": candidate.confidence,
                },
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.store
            .insert_learning_code_proposal(&proposal)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn review_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
        decision: &str,
        note: Option<&str>,
    ) -> Result<Option<DbLearningCodeProposal>, String> {
        let Some(existing) = self
            .store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };

        let decision_lower = decision.to_ascii_lowercase();
        if decision_lower == "reject" {
            let mut metadata = existing.metadata.clone();
            if !metadata.is_object() {
                metadata = serde_json::json!({});
            }
            if let Some(obj) = metadata.as_object_mut() {
                obj.insert(
                    "review".to_string(),
                    serde_json::json!({
                        "decision": "reject",
                        "at": Utc::now().to_rfc3339(),
                        "note": note,
                    }),
                );
                if let Some(fingerprint) = obj.get("fingerprint").cloned() {
                    obj.insert(
                        "anti_learning".to_string(),
                        serde_json::json!({
                            "fingerprint": fingerprint,
                            "suppressed_until": (Utc::now() + chrono::Duration::hours(PROPOSAL_SUPPRESSION_WINDOW_HOURS)).to_rfc3339(),
                        }),
                    );
                }
            }
            self.store
                .update_learning_code_proposal(proposal_id, "rejected", None, None, Some(&metadata))
                .await
                .map_err(|e| e.to_string())?;
            let _ = self
                .submit_feedback(
                    user_id,
                    "code_proposal",
                    &proposal_id.to_string(),
                    "dont_learn",
                    note,
                    Some(&serde_json::json!({"source": "proposal_review"})),
                )
                .await;
        } else {
            let settings = self.load_settings_for_user(user_id).await;
            let mut metadata = existing.metadata.clone();
            if !metadata.is_object() {
                metadata = serde_json::json!({});
            }
            if let Some(obj) = metadata.as_object_mut() {
                obj.insert(
                    "review".to_string(),
                    serde_json::json!({
                        "decision": "approve",
                        "at": Utc::now().to_rfc3339(),
                        "note": note,
                    }),
                );
            }

            match self.write_proposal_bundle(&existing).await {
                Ok(bundle_dir) => {
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert(
                            "bundle".to_string(),
                            serde_json::json!({
                                "status": "written",
                                "path": bundle_dir.to_string_lossy(),
                            }),
                        );
                    }
                }
                Err(err) => {
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert(
                            "bundle".to_string(),
                            serde_json::json!({
                                "status": "failed",
                                "error": err,
                            }),
                        );
                    }
                }
            }

            let mut final_status = "approved".to_string();
            let mut branch_name: Option<String> = None;
            let mut pr_url: Option<String> = None;

            if settings.code_proposals.enabled {
                match self
                    .publish_proposal_in_scratch(&existing, &settings.code_proposals.publish_mode)
                    .await
                {
                    Ok((branch, pr, publish_meta)) => {
                        branch_name = branch;
                        pr_url = pr;
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert("publish".to_string(), publish_meta);
                        }
                        final_status = "applied".to_string();
                    }
                    Err(err) => {
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert(
                                "publish".to_string(),
                                serde_json::json!({"status": "failed", "error": err}),
                            );
                        }
                    }
                }
            }

            self.store
                .update_learning_code_proposal(
                    proposal_id,
                    &final_status,
                    branch_name.as_deref(),
                    pr_url.as_deref(),
                    Some(&metadata),
                )
                .await
                .map_err(|e| e.to_string())?;
        }

        self.store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn write_proposal_bundle(
        &self,
        proposal: &DbLearningCodeProposal,
    ) -> Result<PathBuf, String> {
        let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
        let bundle_dir = repo_root
            .join(".thinclaw")
            .join("learning-proposals")
            .join(proposal.id.to_string());
        tokio::fs::create_dir_all(&bundle_dir)
            .await
            .map_err(|e| e.to_string())?;

        let package = serde_json::json!({
            "proposal_id": proposal.id,
            "problem_statement": proposal.title,
            "evidence": proposal.metadata.get("package").and_then(|v| v.get("evidence")).cloned().unwrap_or(serde_json::json!({})),
            "candidate_rationale": proposal.rationale,
            "target_files": proposal.target_files,
            "unified_diff": proposal.diff,
            "validation_results": proposal.validation_results,
            "rollback_note": proposal.rollback_note,
            "confidence": proposal.confidence,
            "status": proposal.status,
            "created_at": proposal.created_at,
            "updated_at": proposal.updated_at,
        });

        let package_path = bundle_dir.join("proposal.json");
        let diff_path = bundle_dir.join("proposal.diff");
        let summary_path = bundle_dir.join("README.md");

        let package_text = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
        tokio::fs::write(&package_path, package_text)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&diff_path, &proposal.diff)
            .await
            .map_err(|e| e.to_string())?;

        let summary = format!(
            "# Learning Proposal {}\n\n- Status: {}\n- Title: {}\n- Confidence: {}\n- Files: {}\n",
            proposal.id,
            proposal.status,
            proposal.title,
            proposal
                .confidence
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "-".to_string()),
            if proposal.target_files.is_empty() {
                "-".to_string()
            } else {
                proposal.target_files.join(", ")
            }
        );
        tokio::fs::write(summary_path, summary)
            .await
            .map_err(|e| e.to_string())?;

        Ok(bundle_dir)
    }

    async fn publish_proposal_in_scratch(
        &self,
        proposal: &DbLearningCodeProposal,
        publish_mode: &str,
    ) -> Result<(Option<String>, Option<String>, serde_json::Value), String> {
        if proposal.diff.trim().is_empty() {
            return Err("proposal diff is empty".to_string());
        }

        let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
        let scratch_dir = std::env::temp_dir().join(format!(
            "thinclaw-learning-{}",
            proposal.id.to_string().replace('-', "")
        ));
        if scratch_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&scratch_dir).await;
        }

        run_cmd(
            Command::new("git")
                .arg("clone")
                .arg("--no-hardlinks")
                .arg(repo_root.as_os_str())
                .arg(scratch_dir.as_os_str()),
        )
        .await?;

        let base_branch = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("rev-parse")
                .arg("--abbrev-ref")
                .arg("HEAD"),
        )
        .await?
        .trim()
        .to_string();

        let patch_path = scratch_dir.join("learning_proposal.patch");
        tokio::fs::write(&patch_path, &proposal.diff)
            .await
            .map_err(|e| e.to_string())?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg("--check")
                .arg(patch_path.as_os_str()),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg(patch_path.as_os_str()),
        )
        .await?;

        let branch_name = format!("codex/learning-proposal-{}", &proposal.id.to_string()[..8]);
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("checkout")
                .arg("-B")
                .arg(&branch_name),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("add")
                .arg("-A"),
        )
        .await?;

        let commit_message = format!(
            "feat(learning): apply proposal {}",
            &proposal.id.to_string()[..8]
        );
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("commit")
                .arg("-m")
                .arg(commit_message),
        )
        .await?;

        let mode = publish_mode.to_ascii_lowercase();
        let mut pr_url: Option<String> = None;

        if mode != "bundle_only" {
            run_cmd(
                Command::new("git")
                    .arg("-C")
                    .arg(scratch_dir.as_os_str())
                    .arg("push")
                    .arg("-u")
                    .arg("origin")
                    .arg(&branch_name),
            )
            .await?;
        }

        if mode == "branch_pr_draft" {
            let pr_body = format!(
                "Problem:\n{}\n\nRationale:\n{}\n\nGenerated by ThinClaw learning proposal {}.",
                proposal.title, proposal.rationale, proposal.id
            );
            let pr_title = format!("[learning] {}", proposal.title);
            let pr_output = run_cmd(
                Command::new("gh")
                    .arg("pr")
                    .arg("create")
                    .arg("--draft")
                    .arg("--base")
                    .arg(&base_branch)
                    .arg("--head")
                    .arg(&branch_name)
                    .arg("--title")
                    .arg(pr_title)
                    .arg("--body")
                    .arg(pr_body)
                    .current_dir(&scratch_dir),
            )
            .await;
            if let Ok(url) = pr_output {
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    pr_url = Some(trimmed.to_string());
                }
            }
        }

        Ok((
            Some(branch_name),
            pr_url,
            serde_json::json!({
                "status": "published",
                "mode": publish_mode,
                "scratch_dir": scratch_dir,
                "base_branch": base_branch,
            }),
        ))
    }
}

fn stable_json_hash(value: &serde_json::Value) -> u64 {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    hasher.finish()
}

fn proposal_fingerprint(
    title: &str,
    rationale: &str,
    target_files: &[String],
    diff: &str,
) -> String {
    let canonical = serde_json::json!({
        "title": title.trim(),
        "rationale": rationale.trim(),
        "target_files": target_files,
        "diff": diff.trim(),
    });
    format!("{:016x}", stable_json_hash(&canonical))
}

fn classify_event(event: &DbLearningEvent) -> ImprovementClass {
    let et = event.event_type.to_ascii_lowercase();
    if et.contains("code") || event.payload.get("diff").is_some() {
        return ImprovementClass::Code;
    }
    if et.contains("prompt") {
        return ImprovementClass::Prompt;
    }
    if et.contains("skill") {
        return ImprovementClass::Skill;
    }
    if et.contains("routine") {
        return ImprovementClass::Routine;
    }
    if let Some(target) = event.payload.get("target").and_then(|v| v.as_str())
        && matches!(target, "SOUL.md" | "AGENTS.md" | "USER.md")
    {
        return ImprovementClass::Prompt;
    }
    if event.payload.get("skill_content").is_some() {
        return ImprovementClass::Skill;
    }
    ImprovementClass::Memory
}

fn validate_prompt_content(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("prompt content cannot be empty".to_string());
    }
    if !trimmed.contains('#') {
        return Err("prompt content must include markdown headings".to_string());
    }
    let lowered = trimmed.to_ascii_lowercase();
    let suspicious_markers = ["role: user", "role: assistant", "tool_result", "<tool_call"];
    if suspicious_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err("prompt content appears to include transcript/tool residue".to_string());
    }
    Ok(())
}

async fn run_cmd(cmd: &mut Command) -> Result<String, String> {
    let output = cmd.output().await.map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(format!("command failed: {}", detail));
    }
    Ok(stdout)
}

/// Outcome classification for a completed turn trajectory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Success,
    Failure,
    Neutral,
}

/// Optional user feedback attached to a turn record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryFeedback {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// Assessment metadata used to classify a turn for training exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryAssessment {
    pub outcome: TrajectoryOutcome,
    pub score: f64,
    pub source: String,
    pub reasoning: String,
}

/// Structured turn record written to the trajectory JSONL archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryTurnRecord {
    pub session_id: Uuid,
    pub thread_id: Uuid,
    pub user_id: String,
    pub actor_id: String,
    pub channel: String,
    pub conversation_scope_id: Uuid,
    pub conversation_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    pub turn_number: usize,
    pub user_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_response: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<crate::agent::session::TurnToolCall>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<TrajectoryFeedback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<TrajectoryAssessment>,
}

impl TrajectoryTurnRecord {
    /// Build a trajectory record from a completed thread turn.
    pub fn from_turn(
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        _thread: &crate::agent::session::Thread,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Self {
        Self {
            session_id: session.id,
            thread_id,
            user_id: incoming.user_id.clone(),
            actor_id: session.actor_id.clone(),
            channel: incoming.channel.clone(),
            conversation_scope_id: session.conversation_scope_id,
            conversation_kind: session.conversation_kind.as_str().to_string(),
            external_thread_id: incoming.thread_id.clone(),
            turn_number: turn.turn_number,
            user_message: turn.user_input.clone(),
            assistant_response: turn.response.clone(),
            tool_calls: turn.tool_calls.clone(),
            started_at: turn.started_at,
            completed_at: turn.completed_at,
            outcome: Self::classify_turn(turn),
            user_feedback: None,
            assessment: Some(Self::heuristic_assessment(turn)),
        }
    }

    /// Stable target identifier for this turn, used by learning feedback and
    /// dataset exports.
    pub fn target_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.session_id, self.thread_id, self.turn_number
        )
    }

    /// Classify the recorded turn using the local thread state.
    pub fn classify_turn(turn: &crate::agent::session::Turn) -> TrajectoryOutcome {
        match turn.state {
            crate::agent::session::TurnState::Completed => TrajectoryOutcome::Success,
            crate::agent::session::TurnState::Failed => TrajectoryOutcome::Failure,
            crate::agent::session::TurnState::Interrupted => TrajectoryOutcome::Neutral,
            crate::agent::session::TurnState::Processing => TrajectoryOutcome::Neutral,
        }
    }

    /// Heuristic fallback assessment used when no explicit learning feedback
    /// exists for the turn.
    pub fn heuristic_assessment(turn: &crate::agent::session::Turn) -> TrajectoryAssessment {
        let has_response = turn
            .response
            .as_deref()
            .map(str::trim)
            .is_some_and(|response| !response.is_empty());
        let has_error = turn
            .error
            .as_deref()
            .map(str::trim)
            .is_some_and(|error| !error.is_empty());
        let tool_count = turn.tool_calls.len() as f64;

        match turn.state {
            crate::agent::session::TurnState::Failed => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Failure,
                score: 0.05,
                source: "turn_state".to_string(),
                reasoning: "Turn failed before producing a complete response.".to_string(),
            },
            crate::agent::session::TurnState::Interrupted => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.35,
                source: "turn_state".to_string(),
                reasoning: "Turn was interrupted before it could be evaluated.".to_string(),
            },
            crate::agent::session::TurnState::Processing => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.4,
                source: "turn_state".to_string(),
                reasoning: "Turn was still processing when archived.".to_string(),
            },
            crate::agent::session::TurnState::Completed => {
                if !has_response && has_error {
                    TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Failure,
                        score: 0.15,
                        source: "turn_state".to_string(),
                        reasoning: "Turn completed with an error and no assistant response."
                            .to_string(),
                    }
                } else if !has_response {
                    TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Neutral,
                        score: 0.45,
                        source: "turn_state".to_string(),
                        reasoning: "Turn completed without a durable assistant response."
                            .to_string(),
                    }
                } else {
                    let mut score = 0.72;
                    score += (tool_count.min(3.0)) * 0.05;
                    if has_error {
                        score -= 0.35;
                    }
                    let score = score.clamp(0.1, 0.95);
                    let outcome = if score >= 0.6 {
                        TrajectoryOutcome::Success
                    } else if score <= 0.25 {
                        TrajectoryOutcome::Failure
                    } else {
                        TrajectoryOutcome::Neutral
                    };
                    TrajectoryAssessment {
                        outcome,
                        score,
                        source: "heuristic_turn_eval_v1".to_string(),
                        reasoning: if has_error {
                            "Turn produced a response, but errors reduced confidence in its quality."
                                .to_string()
                        } else {
                            "Turn completed with a usable assistant response.".to_string()
                        },
                    }
                }
            }
        }
    }

    pub fn effective_assessment(&self) -> TrajectoryAssessment {
        self.assessment
            .clone()
            .unwrap_or_else(|| TrajectoryAssessment {
                outcome: self.outcome,
                score: match self.outcome {
                    TrajectoryOutcome::Success => 0.75,
                    TrajectoryOutcome::Failure => 0.1,
                    TrajectoryOutcome::Neutral => 0.45,
                },
                source: "legacy_archive".to_string(),
                reasoning: "Archive record predates structured trajectory assessment.".to_string(),
            })
    }

    pub fn preference_score(&self) -> f64 {
        self.effective_assessment().score
    }
}

fn feedback_outcome(verdict: &str) -> Option<TrajectoryOutcome> {
    match verdict.trim().to_ascii_lowercase().as_str() {
        "helpful" | "approve" | "approved" | "accept" | "accepted" | "useful" | "good"
        | "positive" | "success" | "like" => Some(TrajectoryOutcome::Success),
        "harmful" | "reject" | "rejected" | "dont_learn" | "bad" | "negative" | "failure"
        | "dislike" => Some(TrajectoryOutcome::Failure),
        "neutral" | "mixed" | "needs_review" | "unclear" => Some(TrajectoryOutcome::Neutral),
        _ => None,
    }
}

fn feedback_score(outcome: TrajectoryOutcome, fallback_score: f64) -> f64 {
    match outcome {
        TrajectoryOutcome::Success => fallback_score.max(0.95),
        TrajectoryOutcome::Failure => fallback_score.min(0.05),
        TrajectoryOutcome::Neutral => 0.5,
    }
}

fn feedback_matches_turn(
    feedback: &DbLearningFeedbackRecord,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    if feedback.target_id == target_id {
        return true;
    }

    let metadata = &feedback.metadata;
    metadata
        .get("trajectory_target_id")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == target_id)
        || metadata
            .get("thread_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.thread_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
        || metadata
            .get("session_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.session_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
}

fn metadata_matches_turn(
    metadata: &serde_json::Value,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    metadata
        .get("trajectory_target_id")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == target_id)
        || metadata
            .get("target_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == target_id)
        || metadata
            .get("thread_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.thread_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
        || metadata
            .get("session_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.session_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
}

fn event_matches_turn(
    event: &DbLearningEvent,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    metadata_matches_turn(&event.payload, record, target_id)
        || event
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata_matches_turn(metadata, record, target_id))
        || event
            .payload
            .get("target")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == format!("trajectory_turn:{target_id}"))
        || event
            .payload
            .get("target")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == format!("thread_turn:{target_id}"))
}

fn evaluation_outcome(
    evaluation: &DbLearningEvaluation,
    base_assessment: &TrajectoryAssessment,
) -> TrajectoryAssessment {
    let status = evaluation.status.trim().to_ascii_lowercase();
    let raw_score = evaluation
        .score
        .or_else(|| {
            evaluation
                .details
                .get("quality_score")
                .and_then(|value| value.as_f64())
        })
        .unwrap_or(base_assessment.score);
    let normalized_score = if raw_score > 1.0 {
        (raw_score / 100.0).clamp(0.0, 1.0)
    } else {
        raw_score.clamp(0.0, 1.0)
    };

    let outcome = match status.as_str() {
        "accepted" | "approve" | "approved" | "good" | "pass" | "passed" => {
            TrajectoryOutcome::Success
        }
        "poor" | "reject" | "rejected" | "bad" | "fail" | "failed" => TrajectoryOutcome::Failure,
        "review" | "needs_review" | "mixed" | "neutral" => TrajectoryOutcome::Neutral,
        _ if normalized_score >= 0.7 => TrajectoryOutcome::Success,
        _ if normalized_score <= 0.3 => TrajectoryOutcome::Failure,
        _ => TrajectoryOutcome::Neutral,
    };

    TrajectoryAssessment {
        outcome,
        score: normalized_score,
        source: format!("learning_evaluation:{}", evaluation.evaluator),
        reasoning: format!(
            "Turn label derived from learning evaluation status '{}' with score {:.2}.",
            evaluation.status, normalized_score
        ),
    }
}

pub async fn hydrate_trajectory_record(
    record: &mut TrajectoryTurnRecord,
    store: Option<&Arc<dyn Database>>,
) {
    let Some(store) = store else {
        let assessment = record
            .assessment
            .clone()
            .unwrap_or_else(|| TrajectoryAssessment {
                outcome: record.outcome,
                score: record.preference_score(),
                source: "legacy_archive".to_string(),
                reasoning: "Archive record was logged without store-backed feedback.".to_string(),
            });
        record.outcome = assessment.outcome;
        record.assessment = Some(assessment);
        return;
    };

    let target_id = record.target_id();
    let mut matched_feedback: Option<DbLearningFeedbackRecord> = None;
    let mut matched_evaluation: Option<DbLearningEvaluation> = None;

    for target_type in ["trajectory_turn", "thread_turn"] {
        match store
            .list_learning_feedback(&record.user_id, Some(target_type), Some(&target_id), 10)
            .await
        {
            Ok(entries) => {
                if let Some(entry) = entries.into_iter().next() {
                    matched_feedback = Some(entry);
                    break;
                }
            }
            Err(err) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    target_type,
                    error = %err,
                    "Failed to load targeted trajectory feedback"
                );
            }
        }
    }

    if matched_feedback.is_none() {
        match store
            .list_learning_feedback(&record.user_id, None, None, 100)
            .await
        {
            Ok(entries) => {
                matched_feedback = entries
                    .into_iter()
                    .find(|feedback| feedback_matches_turn(feedback, record, &target_id));
            }
            Err(err) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory feedback"
                );
            }
        }
    }

    if matched_feedback.is_none() {
        match (
            store
                .list_learning_events(&record.user_id, None, None, None, 200)
                .await,
            store.list_learning_evaluations(&record.user_id, 200).await,
        ) {
            (Ok(events), Ok(evaluations)) => {
                let matched_event_ids: std::collections::HashSet<_> = events
                    .iter()
                    .filter(|event| event_matches_turn(event, record, &target_id))
                    .map(|event| event.id)
                    .collect();
                matched_evaluation = evaluations
                    .into_iter()
                    .find(|evaluation| matched_event_ids.contains(&evaluation.learning_event_id));
            }
            (Err(err), _) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory learning events"
                );
            }
            (_, Err(err)) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory learning evaluations"
                );
            }
        }
    }

    let base_assessment = record
        .assessment
        .clone()
        .unwrap_or_else(|| TrajectoryAssessment {
            outcome: record.outcome,
            score: record.preference_score(),
            source: "legacy_archive".to_string(),
            reasoning: "Archive record predates structured trajectory assessment.".to_string(),
        });

    if let Some(feedback) = matched_feedback {
        let verdict_outcome =
            feedback_outcome(&feedback.verdict).unwrap_or(base_assessment.outcome);
        let score = feedback_score(verdict_outcome, base_assessment.score);
        record.user_feedback = Some(TrajectoryFeedback {
            label: feedback.verdict.clone(),
            notes: feedback.note.clone(),
            source: Some(feedback.target_type.clone()),
            created_at: Some(feedback.created_at),
        });
        record.assessment = Some(TrajectoryAssessment {
            outcome: verdict_outcome,
            score,
            source: "learning_feedback".to_string(),
            reasoning: format!(
                "Turn label derived from explicit learning feedback verdict '{}'.",
                feedback.verdict
            ),
        });
        record.outcome = verdict_outcome;
    } else if let Some(evaluation) = matched_evaluation {
        let assessment = evaluation_outcome(&evaluation, &base_assessment);
        record.assessment = Some(assessment.clone());
        record.outcome = assessment.outcome;
    } else {
        record.assessment = Some(base_assessment.clone());
        record.outcome = base_assessment.outcome;
    }
}

/// Basic stats summary for the trajectory archive.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrajectoryStats {
    pub log_root: PathBuf,
    pub file_count: usize,
    pub record_count: usize,
    pub session_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
    pub success_count: usize,
    pub failure_count: usize,
    pub neutral_count: usize,
}

/// Appends completed turns to `~/.thinclaw/trajectories/` as JSONL.
#[derive(Debug, Clone)]
pub struct TrajectoryLogger {
    log_root: PathBuf,
}

impl Default for TrajectoryLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryLogger {
    /// Create a logger rooted at the default ThinClaw trajectory directory.
    pub fn new() -> Self {
        Self::with_root(default_trajectory_root())
    }

    /// Create a logger with an explicit root directory.
    pub fn with_root(log_root: impl Into<PathBuf>) -> Self {
        Self {
            log_root: log_root.into(),
        }
    }

    /// Get the configured trajectory root.
    pub fn log_root(&self) -> &Path {
        &self.log_root
    }

    /// Append a single record to the JSONL archive.
    pub async fn append_turn(&self, record: &TrajectoryTurnRecord) -> anyhow::Result<PathBuf> {
        let effective_ts = record.completed_at.unwrap_or(record.started_at);
        let day = effective_ts.format("%Y-%m-%d").to_string();
        let dir = self.log_root.join(day);
        tokio::fs::create_dir_all(&dir).await?;

        let path = dir.join(format!("{}.jsonl", record.session_id));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        use tokio::io::AsyncWriteExt;
        let line = serde_json::to_string(record)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;

        Ok(path)
    }

    /// Load every record found under the trajectory root.
    pub fn load_records(&self) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
        if !self.log_root.exists() {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();
        for path in collect_jsonl_files(&self.log_root)? {
            let content = std::fs::read_to_string(&path)?;
            for line in content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let record: TrajectoryTurnRecord = serde_json::from_str(line)?;
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Summarize the archive for CLI stats output.
    pub fn stats(&self) -> anyhow::Result<TrajectoryStats> {
        let files = if self.log_root.exists() {
            collect_jsonl_files(&self.log_root)?
        } else {
            Vec::new()
        };
        let records = self.load_records()?;
        let mut session_ids = std::collections::BTreeSet::new();
        let mut first_seen: Option<DateTime<Utc>> = None;
        let mut last_seen: Option<DateTime<Utc>> = None;
        let mut success_count = 0;
        let mut failure_count = 0;
        let mut neutral_count = 0;

        for record in &records {
            session_ids.insert(record.session_id);
            let ts = record.completed_at.unwrap_or(record.started_at);
            first_seen = Some(first_seen.map_or(ts, |current| current.min(ts)));
            last_seen = Some(last_seen.map_or(ts, |current| current.max(ts)));
            match record.outcome {
                TrajectoryOutcome::Success => success_count += 1,
                TrajectoryOutcome::Failure => failure_count += 1,
                TrajectoryOutcome::Neutral => neutral_count += 1,
            }
        }

        Ok(TrajectoryStats {
            log_root: self.log_root.clone(),
            file_count: files.len(),
            record_count: records.len(),
            session_count: session_ids.len(),
            first_seen,
            last_seen,
            success_count,
            failure_count,
            neutral_count,
        })
    }
}

fn default_trajectory_root() -> PathBuf {
    crate::platform::resolve_data_dir("trajectories")
}

fn collect_jsonl_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    fn visit(dir: &Path, output: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(&path, output)?;
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                output.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    if root.exists() {
        visit(root, &mut files)?;
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_validator_rejects_transcript_residue() {
        assert!(validate_prompt_content("# Header\nrole: user\nfoo").is_err());
        assert!(validate_prompt_content("# Header\nNormal content").is_ok());
    }

    #[test]
    fn classify_event_prefers_code_when_diff_present() {
        let event = DbLearningEvent {
            id: Uuid::new_v4(),
            user_id: "u".to_string(),
            actor_id: None,
            channel: None,
            thread_id: None,
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: "feedback".to_string(),
            source: "test".to_string(),
            payload: serde_json::json!({"diff": "--- a\n+++ b"}),
            metadata: None,
            created_at: Utc::now(),
        };
        assert_eq!(classify_event(&event), ImprovementClass::Code);
    }

    #[test]
    fn proposal_fingerprint_is_stable_for_identical_input() {
        let files = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];
        let first = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
        let second = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
        assert_eq!(first, second);
    }

    #[test]
    fn proposal_fingerprint_changes_when_diff_changes() {
        let files = vec!["src/lib.rs".to_string()];
        let first = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
        let second = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ c");
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn trajectory_logger_appends_jsonl_records() {
        let root = std::env::temp_dir().join(format!("thinclaw-trajectories-{}", Uuid::new_v4()));
        let logger = TrajectoryLogger::with_root(&root);
        let record = TrajectoryTurnRecord {
            session_id: Uuid::new_v4(),
            thread_id: Uuid::new_v4(),
            user_id: "user-123".to_string(),
            actor_id: "actor-123".to_string(),
            channel: "cli".to_string(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: "direct".to_string(),
            external_thread_id: Some("thread-1".to_string()),
            turn_number: 0,
            user_message: "hello".to_string(),
            assistant_response: Some("hi".to_string()),
            tool_calls: vec![],
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            outcome: TrajectoryOutcome::Success,
            user_feedback: None,
            assessment: Some(TrajectoryAssessment {
                outcome: TrajectoryOutcome::Success,
                score: 0.95,
                source: "test".to_string(),
                reasoning: "positive".to_string(),
            }),
        };

        let path = logger.append_turn(&record).await.expect("append_turn");
        let contents = tokio::fs::read_to_string(path).await.expect("read jsonl");
        assert!(contents.contains("\"user_message\":\"hello\""));
        assert!(contents.contains("\"assistant_response\":\"hi\""));
    }

    #[test]
    fn trajectory_stats_handle_empty_roots() {
        let root = std::env::temp_dir().join(format!("thinclaw-trajectories-{}", Uuid::new_v4()));
        let logger = TrajectoryLogger::with_root(&root);
        let stats = logger.stats().expect("stats");
        assert_eq!(stats.record_count, 0);
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.session_count, 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn hydrate_trajectory_record_prefers_learning_evaluation() {
        let (db, _guard) = crate::testing::test_db().await;
        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let mut record = TrajectoryTurnRecord {
            session_id,
            thread_id,
            user_id: "user-123".to_string(),
            actor_id: "actor-123".to_string(),
            channel: "cli".to_string(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: "direct".to_string(),
            external_thread_id: Some("thread-1".to_string()),
            turn_number: 7,
            user_message: "hello".to_string(),
            assistant_response: Some("hi".to_string()),
            tool_calls: vec![],
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            outcome: TrajectoryOutcome::Success,
            user_feedback: None,
            assessment: Some(TrajectoryAssessment {
                outcome: TrajectoryOutcome::Success,
                score: 0.9,
                source: "heuristic_turn_eval_v1".to_string(),
                reasoning: "fallback".to_string(),
            }),
        };
        let target_id = record.target_id();
        let event = DbLearningEvent {
            id: Uuid::new_v4(),
            user_id: record.user_id.clone(),
            actor_id: Some(record.actor_id.clone()),
            channel: Some(record.channel.clone()),
            thread_id: Some(record.thread_id.to_string()),
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: "trajectory_review".to_string(),
            source: "trajectory_test".to_string(),
            payload: serde_json::json!({
                "target_type": "trajectory_turn",
                "target_id": target_id,
                "thread_id": record.thread_id.to_string(),
                "session_id": record.session_id.to_string(),
                "turn_number": record.turn_number,
            }),
            metadata: None,
            created_at: Utc::now(),
        };
        db.insert_learning_event(&event)
            .await
            .expect("insert learning event");
        db.insert_learning_evaluation(&DbLearningEvaluation {
            id: Uuid::new_v4(),
            learning_event_id: event.id,
            user_id: record.user_id.clone(),
            evaluator: "learning_orchestrator_v1".to_string(),
            status: "poor".to_string(),
            score: Some(0.1),
            details: serde_json::json!({
                "quality_score": 10.0
            }),
            created_at: Utc::now(),
        })
        .await
        .expect("insert learning evaluation");

        hydrate_trajectory_record(&mut record, Some(&db)).await;

        assert_eq!(record.outcome, TrajectoryOutcome::Failure);
        let assessment = record.assessment.expect("assessment");
        assert_eq!(assessment.outcome, TrajectoryOutcome::Failure);
        assert_eq!(
            assessment.source,
            "learning_evaluation:learning_orchestrator_v1"
        );
        assert!(assessment.score <= 0.1);
    }
}

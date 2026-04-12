//! Learning scaffolding and orchestration for ThinClaw's closed-loop improvement system.
//!
//! This module provides:
//! - Core learning-domain types (candidate/risk/decision/proposal state)
//! - Optional external memory providers (Honcho + Zep)
//! - A local-first `LearningOrchestrator` that records evaluations,
//!   creates candidates, applies low-risk mutations, and tracks code proposals.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use crate::db::Database;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion,
    LearningCandidate as DbLearningCandidate, LearningCodeProposal as DbLearningCodeProposal,
    LearningEvaluation as DbLearningEvaluation, LearningEvent as DbLearningEvent,
    LearningFeedbackRecord as DbLearningFeedbackRecord,
};
use crate::settings::LearningSettings;
use crate::skills::registry::SkillRegistry;
use crate::workspace::{Workspace, paths};

/// Broad class of improvement a learning event or candidate belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementClass {
    Memory,
    Skill,
    Prompt,
    Routine,
    Code,
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

impl Default for ImprovementClass {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Risk tier for a potential improvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
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

impl Default for RiskTier {
    fn default() -> Self {
        Self::Low
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
            obj.insert("summary".to_string(), serde_json::json!(self.summary.clone()));
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Draft,
    PendingApproval,
    Approved,
    Applied,
    Rejected,
    RolledBack,
}

impl Default for ProposalState {
    fn default() -> Self {
        Self::Draft
    }
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
        return std::env::var(env_name).ok().filter(|v| !v.trim().is_empty());
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

impl LearningOrchestrator {
    pub fn new(
        store: Arc<dyn Database>,
        workspace: Option<Arc<Workspace>>,
        skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    ) -> Self {
        let providers: Vec<Arc<dyn MemoryProvider>> = vec![
            Arc::new(HonchoProvider),
            Arc::new(ZepProvider),
        ];
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
        self.store
            .insert_learning_feedback(&record)
            .await
            .map_err(|e| e.to_string())
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
            let proposal_id = self.create_code_proposal(event, &candidate).await?;
            outcome.code_proposal_id = Some(proposal_id);
            outcome
                .notes
                .push("high-risk candidate routed to approval-gated code proposal".to_string());
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

        if !matches!(
            target.as_str(),
            paths::SOUL | paths::AGENTS | paths::USER | "SOUL.md" | "AGENTS.md" | "USER.md"
        ) {
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
        workspace.write(&target, content).await.map_err(|e| e.to_string())?;
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
        let before_content = guard.find_by_name(&skill_name).map(|s| s.prompt_content.clone());
        if guard.has(&skill_name) {
            let _ = guard.remove_skill(&skill_name).await;
        }
        guard
            .install_skill(&skill_content)
            .await
            .map_err(|e| e.to_string())?;

        let after_content = guard.find_by_name(&skill_name).map(|s| s.prompt_content.clone());

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
        let proposal = DbLearningCodeProposal {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            status: "proposed".to_string(),
            title: event
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Learning-driven code proposal")
                .to_string(),
            rationale: event
                .payload
                .get("rationale")
                .and_then(|v| v.as_str())
                .or(candidate.summary.as_deref())
                .unwrap_or("Distilled from repeated failures/corrections")
                .to_string(),
            target_files: event
                .payload
                .get("target_files")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect(),
            diff: event
                .payload
                .get("diff")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
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
            let mut metadata = serde_json::json!({"review": {"decision": "reject"}});
            if let Some(note) = note
                && let Some(obj) = metadata.as_object_mut()
            {
                obj.insert("note".to_string(), serde_json::json!(note));
            }
            self.store
                .update_learning_code_proposal(
                    proposal_id,
                    "rejected",
                    None,
                    None,
                    Some(&metadata),
                )
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
            let mut metadata = serde_json::json!({"review": {"decision": "approve"}});
            if let Some(note) = note
                && let Some(obj) = metadata.as_object_mut()
            {
                obj.insert("note".to_string(), serde_json::json!(note));
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

        let branch_name = format!(
            "codex/learning-proposal-{}",
            proposal.id.to_string()[..8].to_string()
        );
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
            proposal.id.to_string()[..8].to_string()
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
                proposal.title,
                proposal.rationale,
                proposal.id
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
    let suspicious_markers = [
        "role: user",
        "role: assistant",
        "tool_result",
        "<tool_call",
    ];
    if suspicious_markers.iter().any(|marker| lowered.contains(marker)) {
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
}

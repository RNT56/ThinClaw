//! Learning scaffolding and orchestration for ThinClaw's closed-loop improvement system.
//!
//! This module provides:
//! - Core learning-domain types (candidate/risk/decision/proposal state)
//! - Optional external memory providers (Honcho + Zep)
//! - A local-first `LearningOrchestrator` that records evaluations,
//!   creates candidates, applies low-risk mutations, and tracks code proposals.

use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use crate::agent::outcomes;
use crate::agent::routine_engine::RoutineEngine;
use crate::db::Database;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion, LearningCandidate as DbLearningCandidate,
    LearningCodeProposal as DbLearningCodeProposal, LearningEvaluation as DbLearningEvaluation,
    LearningEvent as DbLearningEvent, LearningFeedbackRecord as DbLearningFeedbackRecord,
};
use crate::settings::SkillTapTrustLevel;
use crate::settings::{ActiveLearningProvider, LearningSettings};
use crate::skills::quarantine::{FindingSeverity, QuarantineManager, SkillContent};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMemoryHit {
    pub provider: String,
    pub summary: String,
    pub score: Option<f64>,
    pub provenance: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    Disabled,
    NotConfigured,
    Inactive,
    Unhealthy,
    Ready,
}

impl ProviderReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotConfigured => "not_configured",
            Self::Inactive => "inactive",
            Self::Unhealthy => "unhealthy",
            Self::Ready => "ready",
        }
    }

    fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthStatus {
    pub provider: String,
    #[serde(default)]
    pub active: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub readiness: ProviderReadiness,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPrefetchContext {
    pub provider: String,
    pub hits: Vec<ProviderMemoryHit>,
    pub rendered_context: String,
    #[serde(default)]
    pub context_refs: Vec<String>,
}

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus;
    async fn prefetch(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        self.recall(settings, user_id, query, limit).await
    }
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
    fn render_prompt_context(&self, hits: &[ProviderMemoryHit]) -> Option<String> {
        if hits.is_empty() {
            return None;
        }
        let mut lines = vec![format!(
            "External memory recall from {}. Treat this as background context, not as new user input.",
            self.name()
        )];
        for (index, hit) in hits.iter().enumerate() {
            let score = hit
                .score
                .map(|score| format!(" score={score:.3}"))
                .unwrap_or_default();
            lines.push(format!("{}. {}{}", index + 1, hit.summary, score));
        }
        Some(lines.join("\n"))
    }
    async fn after_turn_sync(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn session_end_extract(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn mirror_workspace_write(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    fn tool_extensions(&self) -> Vec<String> {
        vec![
            "external_memory_recall".to_string(),
            "external_memory_status".to_string(),
        ]
    }
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
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Disabled,
            latency_ms: None,
            error: None,
            capabilities: Vec::new(),
            metadata: serde_json::json!({"state": "disabled"}),
        };
    }

    let Some(base_url) = base_url else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::NotConfigured,
            latency_ms: None,
            error: Some("missing base_url".to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        };
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: None,
            error: Some("failed to initialize HTTP client".to_string()),
            capabilities: Vec::new(),
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
            active: false,
            enabled,
            healthy: response.status().is_success(),
            readiness: if response.status().is_success() {
                ProviderReadiness::Ready
            } else {
                ProviderReadiness::Unhealthy
            },
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: if response.status().is_success() {
                None
            } else {
                Some(format!("HTTP {}", response.status()))
            },
            capabilities: Vec::new(),
            metadata: serde_json::json!({"status": response.status().as_u16()}),
        },
        Err(err) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: Some(err.to_string()),
            capabilities: Vec::new(),
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

pub struct MemoryProviderManager {
    store: Arc<dyn Database>,
    providers: Vec<Arc<dyn MemoryProvider>>,
}

pub struct LearningOrchestrator {
    store: Arc<dyn Database>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    routine_engine: Option<Arc<RoutineEngine>>,
    provider_manager: Arc<MemoryProviderManager>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedSkillLifecycle {
    Draft,
    Shadow,
    Proposed,
    Active,
    Frozen,
    RolledBack,
}

impl GeneratedSkillLifecycle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Shadow => "shadow",
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::RolledBack => "rolled_back",
        }
    }
}

const PROPOSAL_SUPPRESSION_WINDOW_HOURS: i64 = 24 * 7;

impl MemoryProviderManager {
    pub fn new(store: Arc<dyn Database>) -> Self {
        let providers: Vec<Arc<dyn MemoryProvider>> =
            vec![Arc::new(HonchoProvider), Arc::new(ZepProvider)];
        Self { store, providers }
    }

    #[cfg(test)]
    fn with_providers(store: Arc<dyn Database>, providers: Vec<Arc<dyn MemoryProvider>>) -> Self {
        Self { store, providers }
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
            let status = self.decorate_provider_status(
                provider,
                &settings,
                provider.health(&settings).await,
            );
            statuses.push(status);
        }
        statuses
    }

    fn active_provider_for_settings<'a>(
        &'a self,
        settings: &LearningSettings,
    ) -> Option<&'a Arc<dyn MemoryProvider>> {
        let target = match settings.providers.active {
            ActiveLearningProvider::None => return None,
            ActiveLearningProvider::Honcho => "honcho",
            ActiveLearningProvider::Zep => "zep",
        };
        self.providers
            .iter()
            .find(|provider| provider.name() == target)
    }

    fn provider_context_refs(hits: &[ProviderMemoryHit]) -> Vec<String> {
        hits.iter()
            .enumerate()
            .map(|(index, hit)| {
                hit.provenance
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        hit.provenance
                            .get("memory_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| format!("{}:{}", hit.provider, index))
            })
            .collect()
    }

    fn decorate_provider_status(
        &self,
        provider: &Arc<dyn MemoryProvider>,
        settings: &LearningSettings,
        mut status: ProviderHealthStatus,
    ) -> ProviderHealthStatus {
        let active_name = self
            .active_provider_for_settings(settings)
            .map(|active| active.name().to_string())
            .unwrap_or_else(|| settings.providers.active.as_str().to_string());
        let is_active = self
            .active_provider_for_settings(settings)
            .is_some_and(|active| active.name() == provider.name());

        status.active = is_active;
        status.capabilities = provider.tool_extensions();
        if !is_active && status.readiness.is_ready() {
            status.readiness = ProviderReadiness::Inactive;
        }
        if !status.metadata.is_object() {
            status.metadata = serde_json::json!({});
        }
        if let Some(obj) = status.metadata.as_object_mut() {
            obj.insert("active".to_string(), serde_json::json!(is_active));
            obj.insert(
                "active_provider".to_string(),
                serde_json::json!(active_name),
            );
            obj.insert(
                "state".to_string(),
                serde_json::json!(status.readiness.as_str()),
            );
        }
        status
    }

    async fn ready_active_provider(
        &self,
        user_id: &str,
    ) -> Option<(
        LearningSettings,
        Arc<dyn MemoryProvider>,
        ProviderHealthStatus,
    )> {
        let settings = self.load_settings_for_user(user_id).await;
        let provider = self.active_provider_for_settings(&settings)?.clone();
        let status =
            self.decorate_provider_status(&provider, &settings, provider.health(&settings).await);
        if !status.readiness.is_ready() {
            tracing::debug!(
                provider = provider.name(),
                readiness = status.readiness.as_str(),
                error = status.error.as_deref().unwrap_or(""),
                "learning provider is not ready; failing closed"
            );
            return None;
        }
        Some((settings, provider, status))
    }

    pub async fn prefetch_provider_context(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Option<ProviderPrefetchContext> {
        let (settings, provider, _) = self.ready_active_provider(user_id).await?;
        let hits = match provider.prefetch(&settings, user_id, query, limit).await {
            Ok(hits) => hits,
            Err(err) => {
                tracing::debug!(
                    provider = provider.name(),
                    user_id = %user_id,
                    error = %err,
                    "learning provider prefetch failed"
                );
                Vec::new()
            }
        };
        let rendered_context = provider.render_prompt_context(&hits)?;
        Some(ProviderPrefetchContext {
            provider: provider.name().to_string(),
            context_refs: Self::provider_context_refs(&hits),
            hits,
            rendered_context,
        })
    }

    pub async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderMemoryHit> {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return Vec::new();
        };
        match provider.recall(&settings, user_id, query, limit).await {
            Ok(hits) => hits,
            Err(err) => {
                tracing::debug!(
                    provider = provider.name(),
                    error = %err,
                    "learning provider recall skipped"
                );
                Vec::new()
            }
        }
    }

    fn run_artifact_payload(artifact: &crate::agent::AgentRunArtifact) -> serde_json::Value {
        serde_json::to_value(artifact).unwrap_or_else(|_| {
            serde_json::json!({
                "run_id": artifact.run_id,
                "source": artifact.source,
                "status": artifact.status,
                "started_at": artifact.started_at,
                "completed_at": artifact.completed_at,
                "failure_reason": artifact.failure_reason,
                "execution_backend": artifact.execution_backend,
                "prompt_snapshot_hash": artifact.prompt_snapshot_hash,
                "ephemeral_overlay_hash": artifact.ephemeral_overlay_hash,
                "provider_context_refs": artifact.provider_context_refs,
                "metadata": artifact.metadata,
            })
        })
    }

    pub async fn after_turn_sync(&self, user_id: &str, artifact: &crate::agent::AgentRunArtifact) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        let payload = Self::run_artifact_payload(artifact);
        if let Err(err) = provider.after_turn_sync(&settings, user_id, &payload).await {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider turn sync skipped"
            );
        }
    }

    pub async fn session_end_extract(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        let payload = Self::run_artifact_payload(artifact);
        if let Err(err) = provider
            .session_end_extract(&settings, user_id, &payload)
            .await
        {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider session-end extract skipped"
            );
        }
    }

    pub async fn mirror_workspace_write(&self, user_id: &str, payload: &serde_json::Value) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        if let Err(err) = provider
            .mirror_workspace_write(&settings, user_id, payload)
            .await
        {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider workspace write mirror skipped"
            );
        }
    }

    pub async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String> {
        self.ready_active_provider(user_id)
            .await
            .map(|(_, provider, _)| provider.tool_extensions())
            .unwrap_or_default()
    }
}

impl LearningOrchestrator {
    pub fn new(
        store: Arc<dyn Database>,
        workspace: Option<Arc<Workspace>>,
        skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    ) -> Self {
        let provider_manager = Arc::new(MemoryProviderManager::new(Arc::clone(&store)));
        Self {
            store,
            workspace,
            skill_registry,
            routine_engine: None,
            provider_manager,
        }
    }

    pub fn with_routine_engine(mut self, routine_engine: Option<Arc<RoutineEngine>>) -> Self {
        self.routine_engine = routine_engine;
        self
    }

    pub fn memory_provider_manager(&self) -> Arc<MemoryProviderManager> {
        Arc::clone(&self.provider_manager)
    }

    pub async fn load_settings_for_user(&self, user_id: &str) -> LearningSettings {
        match self.store.get_all_settings(user_id).await {
            Ok(map) => {
                let settings = crate::settings::Settings::from_db_map(&map);
                let mut learning = settings.learning;
                if settings.desktop_autonomy.is_reckless_enabled() {
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "memory");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "skill");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "prompt");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "routine");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "code");
                    learning.code_proposals.auto_apply_without_review = true;
                    learning.code_proposals.publish_mode = "local_autorollout".to_string();
                }
                learning
            }
            Err(_) => LearningSettings::default(),
        }
    }

    pub async fn provider_health(&self, user_id: &str) -> Vec<ProviderHealthStatus> {
        self.provider_manager.provider_health(user_id).await
    }

    pub async fn prefetch_provider_context(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Option<ProviderPrefetchContext> {
        self.provider_manager
            .prefetch_provider_context(user_id, query, limit)
            .await
    }

    pub async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderMemoryHit> {
        self.provider_manager
            .provider_recall(user_id, query, limit)
            .await
    }

    pub async fn after_turn_sync_to_provider(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        self.provider_manager
            .after_turn_sync(user_id, artifact)
            .await;
    }

    pub async fn session_end_extract(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        self.provider_manager
            .session_end_extract(user_id, artifact)
            .await;
    }

    pub async fn mirror_workspace_write(&self, user_id: &str, payload: &serde_json::Value) {
        self.provider_manager
            .mirror_workspace_write(user_id, payload)
            .await;
    }

    pub async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String> {
        self.provider_manager
            .provider_tool_extensions(user_id)
            .await
    }

    pub async fn review_completed_turn_for_generated_skill(
        &self,
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        _incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Result<Option<String>, String> {
        if turn.state != crate::agent::session::TurnState::Completed {
            return Ok(None);
        }

        let owner_user_id = &session.user_id;
        let workflow_digest = generated_workflow_digest(&turn.user_input, &turn.tool_calls);
        let skill_name = format!("workflow-{}", &workflow_digest[7..19]);
        let existing_candidates = self
            .store
            .list_learning_candidates(owner_user_id, Some("skill"), None, 100)
            .await
            .map_err(|err| err.to_string())?;
        let reuse_count = existing_candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .proposal
                    .get("workflow_digest")
                    .and_then(|value| value.as_str())
                    == Some(workflow_digest.as_str())
                    && candidate.created_at >= Utc::now() - chrono::Duration::days(30)
            })
            .count() as u32
            + 1;

        if !generated_skill_turn_is_eligible(turn, &turn.user_input, reuse_count) {
            return Ok(None);
        }

        let (lifecycle, activation_reason, should_activate) =
            generated_skill_lifecycle_for_reuse(reuse_count);
        let created_at = Utc::now();
        let skill_content = synthesize_generated_skill_markdown(
            &skill_name,
            &turn.user_input,
            &turn.tool_calls,
            lifecycle,
            reuse_count,
            activation_reason.clone(),
        )?;
        let outcome_score = match reuse_count {
            0 | 1 => 0.78,
            2 | 3 => 0.92,
            _ => 0.96,
        };
        let proposal = serde_json::json!({
            "workflow_digest": workflow_digest,
            "provenance": "generated",
            "lifecycle_status": lifecycle.as_str(),
            "reuse_count": reuse_count,
            "outcome_score": outcome_score,
            "activation_reason": activation_reason,
            "skill_content": skill_content,
            "thread_id": thread_id,
            "turn_number": turn.turn_number,
            "tool_count": turn.tool_calls.len(),
            "last_transition_at": created_at,
            "state_history": [generated_skill_transition_entry(
                lifecycle,
                activation_reason.as_deref(),
                None,
                None,
                None,
                created_at,
            )],
        });
        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: owner_user_id.clone(),
            candidate_type: "skill".to_string(),
            risk_tier: RiskTier::Medium.as_str().to_string(),
            confidence: Some(outcome_score),
            target_type: Some("skill".to_string()),
            target_name: Some(skill_name.clone()),
            summary: Some(format!(
                "Generated procedural skill for workflow digest {}",
                &workflow_digest[7..19]
            )),
            proposal: proposal.clone(),
            created_at,
        };
        self.store
            .insert_learning_candidate(&candidate)
            .await
            .map_err(|err| err.to_string())?;
        self.store
            .insert_learning_artifact_version(&DbLearningArtifactVersion {
                id: Uuid::new_v4(),
                candidate_id: Some(candidate.id),
                user_id: owner_user_id.clone(),
                artifact_type: "skill".to_string(),
                artifact_name: skill_name.clone(),
                version_label: Some(Utc::now().to_rfc3339()),
                status: lifecycle.as_str().to_string(),
                diff_summary: Some(match lifecycle {
                    GeneratedSkillLifecycle::Draft => {
                        "Generated procedural skill draft".to_string()
                    }
                    GeneratedSkillLifecycle::Shadow => {
                        "Generated procedural skill shadow candidate".to_string()
                    }
                    GeneratedSkillLifecycle::Proposed => {
                        "Generated procedural skill proposal candidate".to_string()
                    }
                    _ => "Generated procedural skill lifecycle update".to_string(),
                }),
                before_content: None,
                after_content: proposal
                    .get("skill_content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                provenance: serde_json::json!({
                    "provenance": "generated",
                    "workflow_digest": proposal.get("workflow_digest").cloned().unwrap_or(serde_json::Value::Null),
                    "reuse_count": reuse_count,
                    "lifecycle_status": lifecycle.as_str(),
                    "activation_reason": proposal.get("activation_reason").cloned().unwrap_or(serde_json::Value::Null),
                    "duplicate_handling": "new_candidate_per_trace",
                }),
                created_at,
            })
            .await
            .map_err(|err| err.to_string())?;

        if should_activate {
            self.activate_generated_skill(
                Some(&candidate),
                owner_user_id,
                &skill_name,
                proposal
                    .get("skill_content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                reuse_count,
                proposal
                    .get("activation_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("generated_activation"),
                None,
                None,
            )
            .await?;
            return Ok(Some(skill_name));
        }

        Ok(None)
    }

    async fn activate_generated_skill(
        &self,
        candidate: Option<&DbLearningCandidate>,
        user_id: &str,
        skill_name: &str,
        skill_content: &str,
        reuse_count: u32,
        activation_reason: &str,
        feedback_verdict: Option<&str>,
        feedback_note: Option<&str>,
    ) -> Result<(), String> {
        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(());
        };
        let normalized = crate::skills::normalize_line_endings(skill_content);
        let _parsed =
            crate::skills::parser::parse_skill_md(&normalized).map_err(|err| err.to_string())?;
        let quarantine = QuarantineManager::new(crate::platform::resolve_data_dir(
            "generated_skill_quarantine",
        ));
        let quarantined = quarantine
            .quarantine_skill(
                skill_name,
                &SkillContent {
                    raw_content: normalized.clone(),
                    source_kind: "generated".to_string(),
                    source_adapter: "procedural_reviewer".to_string(),
                    source_ref: skill_name.to_string(),
                    source_repo: None,
                    source_url: None,
                    manifest_url: None,
                    manifest_digest: None,
                    path: None,
                    branch: None,
                    commit_sha: None,
                    trust_level: SkillTapTrustLevel::Trusted,
                },
            )
            .await
            .map_err(|err| err.to_string())?;
        let findings = quarantine.scan_quarantined(&quarantined);
        if findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Critical)
        {
            quarantine.cleanup(&quarantined).await;
            return Err(format!(
                "generated skill blocked by static scan: {}",
                findings
                    .iter()
                    .map(|finding| format!("{}:{}", finding.kind, finding.excerpt))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let (install_root, before_content) = {
            let guard = registry.read().await;
            (
                guard.install_root().to_path_buf(),
                guard
                    .find_by_name(skill_name)
                    .map(|skill| skill.prompt_content.clone()),
            )
        };
        let (prepared_name, loaded_skill) =
            SkillRegistry::prepare_install_to_disk(&install_root, skill_name, &normalized)
                .await
                .map_err(|err| err.to_string())?;
        quarantine.cleanup(&quarantined).await;
        let after_content = loaded_skill.prompt_content.clone();

        let existing_remove_path = {
            let guard = registry.read().await;
            if guard.has(skill_name) {
                Some(
                    guard
                        .validate_remove(skill_name)
                        .map_err(|err| err.to_string())?,
                )
            } else {
                None
            }
        };
        if let Some(path) = existing_remove_path.as_ref() {
            SkillRegistry::delete_skill_files(path)
                .await
                .map_err(|err| err.to_string())?;
        }

        let mut guard = registry.write().await;
        if guard.has(skill_name) {
            guard
                .commit_remove(skill_name)
                .map_err(|err| err.to_string())?;
        }
        guard
            .commit_install(&prepared_name, loaded_skill)
            .map_err(|err| err.to_string())?;
        drop(guard);

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: candidate.map(|entry| entry.id),
            user_id: user_id.to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: skill_name.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: GeneratedSkillLifecycle::Active.as_str().to_string(),
            diff_summary: Some("Generated procedural skill activated".to_string()),
            before_content,
            after_content: Some(after_content),
            provenance: serde_json::json!({
                "provenance": "generated",
                "lifecycle_status": GeneratedSkillLifecycle::Active.as_str(),
                "reuse_count": reuse_count,
                "activation_reason": activation_reason,
                "install_pipeline": "prepare_install_to_disk+commit_install",
                "scan_findings": findings,
            }),
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_artifact_version(&version)
            .await
            .map_err(|err| err.to_string())?;
        if let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await {
            tracing::debug!(error = %err, "Generated skill outcome hook skipped");
        }
        if let Some(candidate) = candidate {
            self.update_generated_skill_candidate_proposal(
                candidate,
                GeneratedSkillLifecycle::Active,
                Some(activation_reason),
                feedback_verdict,
                feedback_note,
                Some(version.id),
            )
            .await?;
        }

        Ok(())
    }

    async fn update_generated_skill_candidate_proposal(
        &self,
        candidate: &DbLearningCandidate,
        lifecycle: GeneratedSkillLifecycle,
        activation_reason: Option<&str>,
        feedback_verdict: Option<&str>,
        feedback_note: Option<&str>,
        artifact_version_id: Option<Uuid>,
    ) -> Result<(), String> {
        let proposal = updated_generated_skill_proposal(
            candidate,
            lifecycle,
            activation_reason,
            feedback_verdict,
            feedback_note,
            artifact_version_id,
            Utc::now(),
        );
        self.store
            .update_learning_candidate_proposal(candidate.id, &proposal)
            .await
            .map_err(|err| err.to_string())
    }

    async fn apply_generated_skill_feedback(
        &self,
        user_id: &str,
        target_type: &str,
        target_id: &str,
        verdict: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        if !target_type.eq_ignore_ascii_case("skill") {
            return Ok(());
        }
        let polarity = generated_skill_feedback_polarity(verdict);
        if polarity == 0 {
            return Ok(());
        }

        let Some(candidate) = self
            .store
            .list_learning_candidates(user_id, Some("skill"), None, 200)
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .filter(|candidate| {
                candidate.target_name.as_deref() == Some(target_id)
                    && candidate
                        .proposal
                        .get("provenance")
                        .and_then(|value| value.as_str())
                        == Some("generated")
            })
            .max_by_key(|candidate| candidate.created_at)
        else {
            return Ok(());
        };

        let skill_content = candidate
            .proposal
            .get("skill_content")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let reuse_count = candidate
            .proposal
            .get("reuse_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(1) as u32;

        if polarity > 0 {
            self.activate_generated_skill(
                Some(&candidate),
                user_id,
                target_id,
                skill_content,
                reuse_count,
                "explicit_positive_feedback",
                Some(verdict),
                note,
            )
            .await?;
            return Ok(());
        }

        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(());
        };
        let mut guard = registry.write().await;
        let before_content = guard
            .find_by_name(target_id)
            .map(|skill| skill.prompt_content.clone());
        let removed = if guard.has(target_id) {
            guard.remove_skill(target_id).await.is_ok()
        } else {
            false
        };
        drop(guard);

        let lifecycle = if removed {
            GeneratedSkillLifecycle::RolledBack
        } else {
            GeneratedSkillLifecycle::Frozen
        };
        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: user_id.to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: target_id.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: lifecycle.as_str().to_string(),
            diff_summary: Some(format!(
                "Generated procedural skill {} after feedback verdict '{}'",
                if removed { "rolled back" } else { "frozen" },
                verdict
            )),
            before_content,
            after_content: None,
            provenance: serde_json::json!({
                "provenance": "generated",
                "lifecycle_status": lifecycle.as_str(),
                "activation_reason": "explicit_negative_feedback",
                "feedback_verdict": verdict,
                "feedback_note": note,
            }),
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_artifact_version(&version)
            .await
            .map_err(|err| err.to_string())?;
        self.update_generated_skill_candidate_proposal(
            &candidate,
            lifecycle,
            Some("explicit_negative_feedback"),
            Some(verdict),
            note,
            Some(version.id),
        )
        .await?;

        Ok(())
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
        if let Err(err) = outcomes::observe_feedback(&self.store, &record).await {
            tracing::debug!(user_id = %user_id, error = %err, "Outcome feedback hook skipped");
        }
        if let Err(err) = self
            .apply_generated_skill_feedback(user_id, target_type, target_id, verdict, note)
            .await
        {
            tracing::debug!(
                user_id = %user_id,
                target_type = %target_type,
                target_id = %target_id,
                error = %err,
                "Generated skill feedback hook skipped"
            );
        }

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
                    if class == ImprovementClass::Code
                        && settings.code_proposals.auto_apply_without_review
                    {
                        match self
                            .approve_code_proposal(
                                &event.user_id,
                                proposal_id,
                                Some("auto-approved in reckless_desktop mode"),
                            )
                            .await
                        {
                            Ok(Some(updated)) => {
                                outcome.auto_applied = updated.status == "applied";
                                outcome.notes.push(format!(
                                    "code proposal auto-approved in reckless desktop mode ({})",
                                    updated.status
                                ));
                            }
                            Ok(None) => outcome
                                .notes
                                .push("code proposal disappeared before auto-approval".to_string()),
                            Err(err) => outcome
                                .notes
                                .push(format!("code auto-approval failed: {err}")),
                        }
                    } else {
                        outcome.notes.push(
                            "high-risk candidate routed to approval-gated code proposal"
                                .to_string(),
                        );
                    }
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

    pub async fn route_existing_candidate(
        &self,
        trigger: &str,
        candidate: &DbLearningCandidate,
    ) -> Result<LearningOutcome, String> {
        let settings = self.load_settings_for_user(&candidate.user_id).await;
        let class = ImprovementClass::from_str(&candidate.candidate_type);
        let risk = RiskTier::from_str(&candidate.risk_tier);
        let event_id = candidate.learning_event_id.unwrap_or(candidate.id);
        let mut outcome = LearningOutcome {
            trigger: trigger.to_string(),
            event_id,
            evaluation_id: None,
            candidate_id: Some(candidate.id),
            auto_applied: false,
            code_proposal_id: None,
            notes: Vec::new(),
        };

        if !settings.enabled {
            outcome
                .notes
                .push("learning disabled; outcome candidate persisted only".to_string());
            return Ok(outcome);
        }

        if self.safe_mode_tripped(&settings, &candidate.user_id).await {
            outcome
                .notes
                .push("safe mode is active; outcome candidate held for review".to_string());
            return Ok(outcome);
        }

        if risk.rank() >= RiskTier::High.rank() || class == ImprovementClass::Code {
            match self.create_code_proposal_from_candidate(candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    if class == ImprovementClass::Code
                        && settings.code_proposals.auto_apply_without_review
                    {
                        match self
                            .approve_code_proposal(
                                &candidate.user_id,
                                proposal_id,
                                Some("auto-approved in reckless_desktop mode"),
                            )
                            .await
                        {
                            Ok(Some(updated)) => {
                                outcome.auto_applied = updated.status == "applied";
                                outcome.notes.push(format!(
                                    "outcome code proposal auto-approved in reckless desktop mode ({})",
                                    updated.status
                                ));
                            }
                            Ok(None) => outcome.notes.push(
                                "outcome code proposal disappeared before auto-approval"
                                    .to_string(),
                            ),
                            Err(err) => outcome
                                .notes
                                .push(format!("outcome code auto-approval failed: {err}")),
                        }
                    } else {
                        outcome.notes.push(
                            "outcome candidate routed to approval-gated code proposal".to_string(),
                        );
                    }
                }
                Err(err) => {
                    outcome
                        .notes
                        .push(format!("outcome code proposal suppressed: {err}"));
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
                .auto_apply_candidate(&settings, class, candidate)
                .await
                .unwrap_or(false)
        {
            outcome.auto_applied = true;
            outcome
                .notes
                .push("outcome candidate auto-applied in Tier A".to_string());
        } else {
            outcome
                .notes
                .push("outcome candidate queued for manual review".to_string());
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
        let outcome_stats = self
            .store
            .outcome_summary_stats(user_id)
            .await
            .unwrap_or_default();
        let outcome_ratio = outcome_stats.negative_ratio_last_7d;

        feedback_ratio >= settings.safe_mode.thresholds.negative_feedback_ratio
            || rollback_ratio >= settings.safe_mode.thresholds.rollback_ratio
            || (outcome_stats.evaluated_last_7d >= settings.safe_mode.thresholds.min_samples as u64
                && outcome_ratio >= settings.safe_mode.thresholds.negative_feedback_ratio)
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
            ImprovementClass::Routine => self.auto_apply_routine(candidate).await,
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
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome memory artifact hook skipped");
        }

        Ok(true)
    }

    async fn auto_apply_prompt(&self, candidate: &DbLearningCandidate) -> Result<bool, String> {
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

        if !is_prompt_target_supported(&target) {
            return Ok(false);
        }

        let before = read_prompt_target_content(self.workspace.as_deref(), &target).await?;
        let content =
            if let Some(content) = candidate.proposal.get("content").and_then(|v| v.as_str()) {
                content.to_string()
            } else {
                materialize_prompt_candidate_content(&before, &candidate.proposal, &target)?
            };

        validate_prompt_content(&content)?;
        validate_prompt_target_content(&target, &content)?;
        write_prompt_target_content(self.workspace.as_deref(), &target, &content).await?;
        let after = read_prompt_target_content(self.workspace.as_deref(), &target).await?;

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
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome prompt artifact hook skipped");
        }

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
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome skill artifact hook skipped");
        }

        Ok(true)
    }

    async fn auto_apply_routine(&self, candidate: &DbLearningCandidate) -> Result<bool, String> {
        let Some(engine) = self.routine_engine.as_ref() else {
            return Ok(false);
        };
        let Some(patch) = candidate.proposal.get("routine_patch") else {
            return Ok(false);
        };
        let patch_type = patch
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if patch_type != "notification_noise_reduction" {
            return Ok(false);
        }
        let routine_id = patch
            .get("routine_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "routine patch missing routine_id".to_string())
            .and_then(|value| Uuid::parse_str(value).map_err(|err| err.to_string()))?;

        let Some(mut routine) = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|err| err.to_string())?
        else {
            return Ok(false);
        };

        if !routine.notify.on_success {
            return Ok(false);
        }

        let before = serde_json::to_string_pretty(&routine).map_err(|err| err.to_string())?;
        routine.notify.on_success = false;
        routine.updated_at = Utc::now();
        self.store
            .update_routine(&routine)
            .await
            .map_err(|err| err.to_string())?;
        let after = serde_json::to_string_pretty(&routine).map_err(|err| err.to_string())?;
        engine.refresh_event_cache().await;

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "routine".to_string(),
            artifact_name: routine.name.clone(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-disabled routine success notifications".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({
                "auto_apply": true,
                "class": "routine",
                "patch_type": patch_type,
                "routine_id": routine.id.to_string(),
                "routine_name": routine.name,
                "actor_id": routine.owner_actor_id(),
            }),
            created_at: Utc::now(),
        };
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome routine artifact hook skipped");
        }
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
        if diff.trim().is_empty() {
            return Err("code proposal missing diff".to_string());
        }
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

    async fn create_code_proposal_from_candidate(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<Uuid, String> {
        let event = DbLearningEvent {
            id: candidate.learning_event_id.unwrap_or(candidate.id),
            user_id: candidate.user_id.clone(),
            actor_id: None,
            channel: None,
            thread_id: None,
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: candidate.candidate_type.clone(),
            source: "outcome_backed_learning".to_string(),
            payload: candidate.proposal.clone(),
            metadata: Some(serde_json::json!({
                "source_candidate_id": candidate.id,
                "source": "outcome_backed_learning",
            })),
            created_at: candidate.created_at,
        };
        self.create_code_proposal(&event, candidate).await
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
            if let Err(err) =
                outcomes::observe_proposal_rejection(&self.store, &existing, note).await
            {
                tracing::debug!(proposal_id = %proposal_id, error = %err, "Outcome proposal rejection hook skipped");
            }
            self.store
                .get_learning_code_proposal(user_id, proposal_id)
                .await
                .map_err(|e| e.to_string())
        } else {
            self.approve_code_proposal(user_id, proposal_id, note).await
        }
    }

    async fn approve_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
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
        if matches!(final_status.as_str(), "approved" | "applied")
            && let Some(updated) = self
                .store
                .get_learning_code_proposal(user_id, proposal_id)
                .await
                .map_err(|e| e.to_string())?
            && let Err(err) = outcomes::maybe_create_proposal_contract(&self.store, &updated).await
        {
            tracing::debug!(proposal_id = %proposal_id, error = %err, "Outcome proposal durability hook skipped");
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
        if mode == "local_autorollout" {
            let manager = crate::desktop_autonomy::desktop_autonomy_manager().ok_or_else(|| {
                "local_autorollout requires an active desktop autonomy manager".to_string()
            })?;
            let outcome = manager
                .local_autorollout(
                    &proposal.user_id,
                    proposal.id,
                    &proposal.diff,
                    &proposal.title,
                )
                .await?;

            let candidate_id = proposal
                .metadata
                .get("candidate_id")
                .and_then(|value| value.as_str())
                .and_then(|value| Uuid::parse_str(value).ok());
            let version = DbLearningArtifactVersion {
                id: Uuid::new_v4(),
                candidate_id,
                user_id: proposal.user_id.clone(),
                artifact_type: "code".to_string(),
                artifact_name: outcome.build_id.clone(),
                version_label: Some(outcome.build_id.clone()),
                status: if outcome.promoted {
                    "promoted".to_string()
                } else {
                    "failed".to_string()
                },
                diff_summary: Some(proposal.title.clone()),
                before_content: None,
                after_content: Some(proposal.diff.clone()),
                provenance: serde_json::json!({
                    "publish_mode": "local_autorollout",
                    "proposal_id": proposal.id,
                    "checks": outcome.checks,
                    "metadata": outcome.publish_metadata,
                    "build_dir": outcome.build_dir,
                    "build_id": outcome.build_id,
                    "canary_report_path": outcome.publish_metadata.get("canary_report_path").cloned(),
                    "platform": outcome.publish_metadata.get("platform").cloned(),
                    "bridge_backend": outcome.publish_metadata.get("bridge_backend").cloned(),
                    "providers": outcome.publish_metadata.get("providers").cloned(),
                    "launcher_kind": outcome.publish_metadata.get("launcher_kind").cloned(),
                    "promoted_at": if outcome.promoted { Some(Utc::now()) } else { None },
                    "actor_id": proposal.metadata.get("actor_id").cloned(),
                    "thread_id": proposal.metadata.get("thread_id").cloned(),
                }),
                created_at: Utc::now(),
            };
            let inserted = self.store.insert_learning_artifact_version(&version).await;
            if inserted.is_ok()
                && outcome.promoted
                && let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
            {
                tracing::debug!(error = %err, "Outcome promoted code artifact hook skipped");
            }

            return Ok((
                Some(format!("local_autorollout/{}", outcome.build_id)),
                None,
                serde_json::json!({
                    "status": if outcome.promoted { "promoted" } else { "failed" },
                    "mode": publish_mode,
                    "build_id": outcome.build_id,
                    "build_dir": outcome.build_dir,
                    "checks": outcome.checks,
                    "metadata": outcome.publish_metadata,
                }),
            ));
        }

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

fn ensure_auto_apply_class(classes: &mut Vec<String>, value: &str) {
    if !classes
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(value))
    {
        classes.push(value.to_string());
    }
}

fn generated_skill_turn_is_eligible(
    turn: &crate::agent::session::Turn,
    user_input: &str,
    reuse_count: u32,
) -> bool {
    let distinct_categories = turn
        .tool_calls
        .iter()
        .map(|call| generated_tool_category(&call.name))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let has_multi_tool_pattern = turn.tool_calls.len() >= 3 && distinct_categories >= 2;
    let recovered_from_failure = turn
        .tool_calls
        .iter()
        .any(|call| call.error.is_some() && call.result.is_none());
    let corrected_then_succeeded = detect_generated_skill_correction_signal(user_input)
        && !turn.tool_calls.is_empty()
        && turn.tool_calls.iter().all(|call| call.error.is_none());
    let repeated_workflow_match = reuse_count >= 2;
    has_multi_tool_pattern
        || recovered_from_failure
        || corrected_then_succeeded
        || repeated_workflow_match
}

fn detect_generated_skill_correction_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    [
        "actually",
        "correction:",
        "to clarify",
        "that's incorrect",
        "that is incorrect",
        "not quite",
        "use this instead",
        "please use",
        "instead:",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

fn generated_tool_category(tool_name: &str) -> &'static str {
    match tool_name {
        name if name.contains("file") || name.contains("search") => "files",
        name if name.contains("memory") || name.contains("session") => "memory",
        name if name.contains("http") || name.contains("browser") => "web",
        name if name.contains("skill") || name.contains("prompt") => "learning",
        "execute_code" | "shell" | "process" | "create_job" => "execution",
        _ => "other",
    }
}

fn generated_skill_lifecycle_for_reuse(
    reuse_count: u32,
) -> (GeneratedSkillLifecycle, Option<String>, bool) {
    if reuse_count >= 4 {
        (
            GeneratedSkillLifecycle::Proposed,
            Some("proposal_reuse_threshold".to_string()),
            false,
        )
    } else if reuse_count >= 2 {
        (
            GeneratedSkillLifecycle::Shadow,
            Some("shadow_candidate".to_string()),
            false,
        )
    } else {
        (GeneratedSkillLifecycle::Draft, None, false)
    }
}

fn generated_skill_feedback_polarity(verdict: &str) -> i8 {
    let normalized = verdict.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "helpful" | "approve" | "approved" | "accept" | "accepted" | "good" | "works"
        | "success" | "positive" => 1,
        "harmful" | "reject" | "rejected" | "bad" | "broken" | "regression" | "dont_learn"
        | "negative" | "rollback" | "rolled_back" => -1,
        _ => 0,
    }
}

fn generated_skill_transition_entry(
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    serde_json::json!({
        "status": lifecycle.as_str(),
        "at": transition_at,
        "activation_reason": activation_reason,
        "feedback_verdict": feedback_verdict,
        "feedback_note": feedback_note,
        "artifact_version_id": artifact_version_id,
    })
}

fn updated_generated_skill_proposal(
    candidate: &DbLearningCandidate,
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    let mut proposal = if candidate.proposal.is_object() {
        candidate.proposal.clone()
    } else {
        serde_json::json!({})
    };
    let entry = generated_skill_transition_entry(
        lifecycle,
        activation_reason,
        feedback_verdict,
        feedback_note,
        artifact_version_id,
        transition_at,
    );
    let obj = proposal
        .as_object_mut()
        .expect("generated skill proposal should be object");
    obj.insert("provenance".to_string(), serde_json::json!("generated"));
    obj.insert(
        "lifecycle_status".to_string(),
        serde_json::json!(lifecycle.as_str()),
    );
    obj.insert(
        "last_transition_at".to_string(),
        serde_json::json!(transition_at),
    );
    if let Some(reason) = activation_reason.filter(|value| !value.trim().is_empty()) {
        obj.insert(
            "activation_reason".to_string(),
            serde_json::json!(reason.to_string()),
        );
    }
    if let Some(version_id) = artifact_version_id {
        obj.insert(
            "last_artifact_version_id".to_string(),
            serde_json::json!(version_id),
        );
    }
    if let Some(verdict) = feedback_verdict {
        obj.insert(
            "last_feedback".to_string(),
            serde_json::json!({
                "verdict": verdict,
                "note": feedback_note,
                "at": transition_at,
            }),
        );
    }
    match lifecycle {
        GeneratedSkillLifecycle::Active => {
            obj.insert("activated_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::Frozen => {
            obj.insert("frozen_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::RolledBack => {
            obj.insert(
                "rolled_back_at".to_string(),
                serde_json::json!(transition_at),
            );
        }
        _ => {}
    }
    let history = obj
        .entry("state_history".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !history.is_array() {
        *history = serde_json::json!([]);
    }
    history
        .as_array_mut()
        .expect("state_history should be array")
        .push(entry);
    proposal
}

fn generated_workflow_digest(
    user_input: &str,
    tool_calls: &[crate::agent::session::TurnToolCall],
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(normalize_generated_skill_text(user_input).as_bytes());
    for call in tool_calls {
        hasher.update(b"|tool:");
        hasher.update(call.name.as_bytes());
        hasher.update(b"|params:");
        hasher.update(canonicalize_json_value(&call.parameters).as_bytes());
        hasher.update(b"|status:");
        hasher.update(if call.error.is_some() {
            b"error".as_slice()
        } else {
            b"ok".as_slice()
        });
        hasher.update(b"|signature:");
        hasher.update(compact_tool_outcome_signature(call).as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn normalize_generated_skill_text(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn canonicalize_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string())
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonicalize_json_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| {
                        let value = map
                            .get(key)
                            .map(canonicalize_json_value)
                            .unwrap_or_else(|| "null".to_string());
                        format!(
                            "{}:{}",
                            serde_json::to_string(key).unwrap_or_else(|_| "\"<key>\"".to_string()),
                            value
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn compact_tool_outcome_signature(call: &crate::agent::session::TurnToolCall) -> String {
    use sha2::{Digest, Sha256};

    let signature_input = if let Some(error) = call.error.as_deref() {
        format!("error:{}", normalize_generated_skill_text(error))
    } else if let Some(result) = call.result.as_ref() {
        format!("ok:{}", canonicalize_json_value(result))
    } else {
        "ok:null".to_string()
    };

    let mut hasher = Sha256::new();
    hasher.update(signature_input.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("sha256:{}", &digest[..16])
}

fn synthesize_generated_skill_markdown(
    skill_name: &str,
    user_input: &str,
    tool_calls: &[crate::agent::session::TurnToolCall],
    lifecycle: GeneratedSkillLifecycle,
    reuse_count: u32,
    activation_reason: Option<String>,
) -> Result<String, String> {
    let description = user_input
        .trim()
        .split_whitespace()
        .take(18)
        .collect::<Vec<_>>()
        .join(" ");
    let keywords = tool_calls
        .iter()
        .map(|call| call.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let workflow_steps = tool_calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let parameter_keys = call
                .parameters
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            if parameter_keys.is_empty() {
                format!("{}. Use `{}`.", index + 1, call.name)
            } else {
                format!(
                    "{}. Use `{}` with parameters touching: {}.",
                    index + 1,
                    call.name,
                    parameter_keys
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let yaml_keywords = if keywords.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            keywords
                .iter()
                .map(|keyword| format!("\"{keyword}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let activation_reason = activation_reason.unwrap_or_else(|| "draft".to_string());
    let content = format!(
        "---\nname: {skill_name}\nversion: 0.1.0\ndescription: \"Generated workflow skill for {description}\"\nactivation:\n  keywords: {yaml_keywords}\nmetadata:\n  openclaw:\n    provenance: generated\n    lifecycle_status: {}\n    outcome_score: {}\n    reuse_count: {reuse_count}\n    activation_reason: \"{}\"\n---\n\nYou are a reusable workflow skill distilled from a successful ThinClaw turn.\n\nUse this skill when the user is asking for work that resembles:\n- {description}\n\nPreferred workflow:\n{workflow_steps}\n\nSafety notes:\n- Verify tool results before moving to the next step.\n- Prefer deterministic file/memory reads before mutations.\n- Stop and surface blockers instead of guessing when required tools fail.\n",
        lifecycle.as_str(),
        if reuse_count >= 2 { "0.92" } else { "0.78" },
        activation_reason,
    );
    crate::skills::parser::parse_skill_md(&crate::skills::normalize_line_endings(&content))
        .map_err(|err| err.to_string())?;
    Ok(content)
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
        && matches!(
            target,
            "SOUL.md" | "SOUL.local.md" | "AGENTS.md" | "USER.md"
        )
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

fn validate_prompt_target_content(target: &str, content: &str) -> Result<(), String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return crate::identity::soul::validate_canonical_soul(content);
    }
    if target.eq_ignore_ascii_case(paths::SOUL_LOCAL) {
        return crate::identity::soul::validate_local_overlay(content);
    }
    if target.eq_ignore_ascii_case(paths::AGENTS) {
        let lowered = content.to_ascii_lowercase();
        let required_markers = ["red lines", "ask first", "don't"];
        if required_markers
            .iter()
            .all(|marker| !lowered.contains(marker))
        {
            return Err(format!(
                "{} update rejected: core safety guidance appears to be missing",
                target
            ));
        }
    }
    Ok(())
}

fn is_prompt_target_supported(target: &str) -> bool {
    matches!(
        target,
        paths::SOUL | paths::SOUL_LOCAL | paths::AGENTS | paths::USER
    ) || target
        .to_ascii_lowercase()
        .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()))
}

fn materialize_prompt_candidate_content(
    current: &str,
    proposal: &serde_json::Value,
    target: &str,
) -> Result<String, String> {
    let patch = proposal
        .get("prompt_patch")
        .ok_or_else(|| "prompt candidate missing content".to_string())?;
    let operation = patch
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("replace");
    let base = ensure_prompt_document_root(current, target);
    let next = match operation {
        "replace" => patch
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "prompt patch missing content".to_string())?
            .to_string(),
        "upsert_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            upsert_markdown_section(&base, heading, section_content)
        }
        "append_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            append_markdown_section(&base, heading, section_content)
        }
        "remove_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            remove_markdown_section(&base, heading)?
        }
        other => return Err(format!("unsupported prompt patch operation '{}'", other)),
    };
    Ok(ensure_prompt_trailing_newline(&next))
}

async fn read_prompt_target_content(
    workspace: Option<&Workspace>,
    target: &str,
) -> Result<String, String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return match crate::identity::soul_store::read_home_soul() {
            Ok(content) => Ok(content),
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(String::new()),
            Err(err) => Err(format!("failed to read canonical SOUL.md: {}", err)),
        };
    }

    let Some(workspace) = workspace else {
        return Err(format!(
            "workspace unavailable for prompt target '{}'",
            target
        ));
    };

    Ok(workspace
        .read(target)
        .await
        .ok()
        .map(|doc| doc.content)
        .unwrap_or_default())
}

async fn write_prompt_target_content(
    workspace: Option<&Workspace>,
    target: &str,
    content: &str,
) -> Result<(), String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return crate::identity::soul_store::write_home_soul(content)
            .map_err(|err| format!("failed to update canonical SOUL.md: {}", err));
    }

    let Some(workspace) = workspace else {
        return Err(format!(
            "workspace unavailable for prompt target '{}'",
            target
        ));
    };

    workspace
        .write(target, content)
        .await
        .map(|_| ())
        .map_err(|err| format!("failed to update '{}': {}", target, err))
}

fn ensure_prompt_document_root(current: &str, target: &str) -> String {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        return ensure_prompt_trailing_newline(trimmed);
    }
    if target.ends_with(paths::SOUL_LOCAL) {
        let mut sections = BTreeMap::new();
        for section in crate::identity::soul::LOCAL_SECTIONS {
            sections.insert((*section).to_string(), String::new());
        }
        return crate::identity::soul::render_local_soul_overlay(
            &crate::identity::soul::LocalSoulOverlay { sections },
        );
    }
    if target.ends_with(paths::SOUL) {
        return crate::identity::soul::compose_seeded_soul("balanced").unwrap_or_else(|_| {
            "# SOUL.md - Who You Are\n\n- **Schema:** v2\n- **Seed Pack:** balanced\n\n## Core Truths\n\n## Boundaries\n\n## Vibe\n\n## Default Behaviors\n\n## Continuity\n\n## Change Contract\n"
                .to_string()
        });
    }
    let title = if target.ends_with(paths::USER) {
        "USER.md"
    } else if target.ends_with(paths::AGENTS) {
        "AGENTS.md"
    } else {
        target.rsplit('/').next().unwrap_or("PROMPT.md")
    };
    format!("# {title}\n")
}

fn ensure_prompt_trailing_newline(content: &str) -> String {
    let trimmed = content.trim_end();
    format!("{trimmed}\n")
}

fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title.to_string()))
}

fn find_section_byte_range(doc: &str, heading_name: &str) -> Option<(usize, usize, usize, String)> {
    let target = normalize_heading_name(heading_name);
    let mut offset = 0usize;
    let mut start: Option<(usize, usize, usize, String)> = None;

    for line in doc.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end;

        if let Some((level, title)) = parse_markdown_heading(line) {
            if let Some((start_offset, current_level, _, current_title)) = &start
                && level <= *current_level
            {
                return Some((
                    *start_offset,
                    line_start,
                    *current_level,
                    current_title.clone(),
                ));
            }

            if normalize_heading_name(&title) == target {
                start = Some((line_start, level, line_end, title));
            }
        }
    }

    start.map(|(start_offset, level, _, title)| (start_offset, doc.len(), level, title))
}

fn upsert_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let normalized_content = section_content.trim();
    let body = if normalized_content.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", normalized_content)
    };

    if let Some((start, end, level, title)) = find_section_byte_range(doc, heading) {
        let heading_line = format!("{} {}", "#".repeat(level.max(1)), title.trim());
        let replacement = format!("{heading_line}{body}");
        let mut merged = String::with_capacity(doc.len() + replacement.len());
        merged.push_str(&doc[..start]);
        merged.push_str(replacement.trim_end_matches('\n'));
        merged.push('\n');
        merged.push_str(doc[end..].trim_start_matches('\n'));
        return ensure_prompt_trailing_newline(merged.trim());
    }

    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    if !normalized_content.is_empty() {
        merged.push_str(normalized_content);
        merged.push('\n');
    }
    ensure_prompt_trailing_newline(&merged)
}

fn append_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    let content = section_content.trim();
    if !content.is_empty() {
        merged.push_str(content);
        merged.push('\n');
    }
    ensure_prompt_trailing_newline(&merged)
}

fn remove_markdown_section(doc: &str, heading: &str) -> Result<String, String> {
    let Some((start, end, _, _)) = find_section_byte_range(doc, heading) else {
        return Err(format!("section '{}' not found", heading));
    };

    let mut merged = String::with_capacity(doc.len());
    merged.push_str(&doc[..start]);
    merged.push_str(doc[end..].trim_start_matches('\n'));
    Ok(ensure_prompt_trailing_newline(merged.trim()))
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

/// Outcome classification for a terminal turn trajectory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Success,
    Failure,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryTurnStatus {
    Completed,
    Failed,
    Interrupted,
    Processing,
}

fn default_trajectory_turn_status() -> TrajectoryTurnStatus {
    TrajectoryTurnStatus::Completed
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
    #[serde(default = "default_trajectory_turn_status")]
    pub turn_status: TrajectoryTurnStatus,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<TrajectoryFeedback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<TrajectoryAssessment>,
}

impl TrajectoryTurnRecord {
    /// Build a trajectory record from a terminal thread turn snapshot.
    pub fn from_turn(
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        _thread: &crate::agent::session::Thread,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Self {
        let identity = incoming.resolved_identity();
        Self {
            session_id: session.id,
            thread_id,
            user_id: incoming.user_id.clone(),
            actor_id: identity.actor_id,
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
            turn_status: Self::turn_status(turn),
            outcome: Self::classify_turn(turn),
            failure_reason: turn.error.clone(),
            execution_backend: None,
            llm_provider: None,
            llm_model: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            provider_context_refs: Vec::new(),
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

    pub fn turn_status(turn: &crate::agent::session::Turn) -> TrajectoryTurnStatus {
        match turn.state {
            crate::agent::session::TurnState::Completed => TrajectoryTurnStatus::Completed,
            crate::agent::session::TurnState::Failed => TrajectoryTurnStatus::Failed,
            crate::agent::session::TurnState::Interrupted => TrajectoryTurnStatus::Interrupted,
            crate::agent::session::TurnState::Processing => TrajectoryTurnStatus::Processing,
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
                            "Turn completed with a usable agent response.".to_string()
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

/// Appends terminal turns to `~/.thinclaw/trajectories/` as JSONL.
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
    use crate::agent::session::{Session, Thread, Turn};
    use crate::channels::IncomingMessage;
    use std::sync::{Arc, Mutex};

    use tokio::sync::mpsc;

    use crate::agent::routine::{Routine, RoutineAction, RoutineGuardrails, Trigger};
    use crate::agent::routine_engine::RoutineEngine;
    use crate::config::RoutineConfig;
    use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
    use crate::testing::StubLlm;
    use crate::workspace::Workspace;

    #[derive(Debug)]
    struct TestMemoryProvider {
        name: &'static str,
        hits: Vec<ProviderMemoryHit>,
        recalls: Arc<Mutex<Vec<(String, String, usize)>>>,
        health_status: ProviderHealthStatus,
    }

    #[async_trait]
    impl MemoryProvider for TestMemoryProvider {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn health(&self, _settings: &LearningSettings) -> ProviderHealthStatus {
            self.health_status.clone()
        }

        async fn recall(
            &self,
            _settings: &LearningSettings,
            user_id: &str,
            query: &str,
            limit: usize,
        ) -> Result<Vec<ProviderMemoryHit>, String> {
            self.recalls
                .lock()
                .expect("recall log mutex poisoned")
                .push((user_id.to_string(), query.to_string(), limit));
            Ok(self.hits.iter().take(limit).cloned().collect())
        }

        async fn export_turn(
            &self,
            _settings: &LearningSettings,
            _user_id: &str,
            _payload: &serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn provider_status(
        name: &str,
        readiness: ProviderReadiness,
        healthy: bool,
        error: Option<&str>,
    ) -> ProviderHealthStatus {
        ProviderHealthStatus {
            provider: name.to_string(),
            active: false,
            enabled: readiness != ProviderReadiness::Disabled,
            healthy,
            readiness,
            latency_ms: Some(1),
            error: error.map(str::to_string),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }

    fn generated_skill_test_content(skill_name: &str) -> String {
        synthesize_generated_skill_markdown(
            skill_name,
            "Help the user collect a file summary and write it down.",
            &[crate::agent::session::TurnToolCall {
                name: "shell".to_string(),
                parameters: serde_json::json!({"cmd": "echo hi"}),
                result: Some(serde_json::json!({"stdout": "hi"})),
                error: None,
            }],
            GeneratedSkillLifecycle::Shadow,
            3,
            Some("shadow_candidate".to_string()),
        )
        .expect("generated skill markdown should parse")
    }

    fn generated_skill_candidate(
        user_id: &str,
        skill_name: &str,
        skill_content: &str,
        created_at: DateTime<Utc>,
    ) -> DbLearningCandidate {
        DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: user_id.to_string(),
            candidate_type: "skill".to_string(),
            risk_tier: "medium".to_string(),
            confidence: Some(0.92),
            target_type: Some("skill".to_string()),
            target_name: Some(skill_name.to_string()),
            summary: Some("Generated procedural skill".to_string()),
            proposal: serde_json::json!({
                "workflow_digest": "sha256:test-workflow",
                "provenance": "generated",
                "lifecycle_status": GeneratedSkillLifecycle::Shadow.as_str(),
                "reuse_count": 3,
                "outcome_score": 0.92,
                "activation_reason": "shadow_candidate",
                "skill_content": skill_content,
                "last_transition_at": created_at,
                "state_history": [generated_skill_transition_entry(
                    GeneratedSkillLifecycle::Shadow,
                    Some("shadow_candidate"),
                    None,
                    None,
                    None,
                    created_at,
                )],
            }),
            created_at,
        }
    }

    #[test]
    fn prompt_validator_rejects_transcript_residue() {
        assert!(validate_prompt_content("# Header\nrole: user\nfoo").is_err());
        assert!(validate_prompt_content("# Header\nNormal content").is_ok());
    }

    #[test]
    fn prompt_candidate_patch_materializes_content() {
        let current = "# USER.md\n\n## Preferences\n- concise\n";
        let proposal = serde_json::json!({
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Outcome-Backed Guidance",
                "section_content": "- finish the requested implementation before concluding"
            }
        });

        let next = materialize_prompt_candidate_content(current, &proposal, paths::USER)
            .expect("prompt patch should materialize");

        assert!(next.contains("## Preferences\n- concise"));
        assert!(next.contains("## Outcome-Backed Guidance"));
        assert!(next.contains("finish the requested implementation"));
    }

    #[test]
    fn prompt_candidate_patch_materializes_valid_canonical_soul_when_empty() {
        let proposal = serde_json::json!({
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Outcome-Backed Guidance",
                "section_content": "- call out bad ideas early"
            }
        });

        let next = materialize_prompt_candidate_content("", &proposal, paths::SOUL)
            .expect("prompt patch should materialize");

        assert!(crate::identity::soul::validate_canonical_soul(&next).is_ok());
        assert!(next.contains("## Outcome-Backed Guidance"));
    }

    #[test]
    fn prompt_candidate_patch_materializes_valid_local_overlay_when_empty() {
        let proposal = serde_json::json!({
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Tone Adjustments",
                "section_content": "- stay extra terse for this workspace"
            }
        });

        let next = materialize_prompt_candidate_content("", &proposal, paths::SOUL_LOCAL)
            .expect("prompt patch should materialize");

        assert!(crate::identity::soul::validate_local_overlay(&next).is_ok());
        assert!(next.contains("## Tone Adjustments"));
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
            turn_status: TrajectoryTurnStatus::Completed,
            outcome: TrajectoryOutcome::Success,
            failure_reason: None,
            execution_backend: Some("interactive_chat".to_string()),
            llm_provider: None,
            llm_model: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            provider_context_refs: Vec::new(),
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
    fn trajectory_turn_record_prefers_incoming_actor_identity() {
        let session = Session::new_scoped(
            "user-shared",
            "phone",
            scope_id_from_key("principal:user-shared"),
            ConversationKind::Direct,
        );
        let thread = Thread::new(session.id);
        let incoming = IncomingMessage::new("gateway", "user-shared", "hello").with_identity(
            ResolvedIdentity {
                principal_id: "user-shared".to_string(),
                actor_id: "desktop".to_string(),
                conversation_scope_id: scope_id_from_key("principal:user-shared"),
                conversation_kind: ConversationKind::Direct,
                raw_sender_id: "user-shared".to_string(),
                stable_external_conversation_key:
                    "gateway://direct/user-shared/actor/desktop/thread/thread-a".to_string(),
            },
        );
        let turn = Turn::new(0, "hello", false);

        let record =
            TrajectoryTurnRecord::from_turn(&session, Uuid::new_v4(), &thread, &incoming, &turn);

        assert_eq!(record.actor_id, "desktop");
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
            turn_status: TrajectoryTurnStatus::Completed,
            outcome: TrajectoryOutcome::Success,
            failure_reason: None,
            execution_backend: Some("interactive_chat".to_string()),
            llm_provider: None,
            llm_model: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            provider_context_refs: Vec::new(),
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn auto_apply_routine_records_artifact_version_and_outcome_contract() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "routine-auto-apply-user";

        db.set_setting(user_id, "learning.enabled", &serde_json::json!(true))
            .await
            .expect("set learning.enabled");
        db.set_setting(
            user_id,
            "learning.outcomes.enabled",
            &serde_json::json!(true),
        )
        .await
        .expect("set learning.outcomes.enabled");

        let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
        let (notify_tx, _notify_rx) = mpsc::channel(4);
        let routine_engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            Arc::clone(&db),
            Arc::new(StubLlm::new("ok")),
            Arc::clone(&workspace),
            notify_tx,
            None,
        ));

        let now = Utc::now();
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "Daily outcome digest".to_string(),
            description: "Summarize outcomes".to_string(),
            user_id: user_id.to_string(),
            actor_id: user_id.to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::Lightweight {
                prompt: "Summarize the latest outcome-backed learning signals.".to_string(),
                context_paths: Vec::new(),
                max_tokens: 128,
            },
            guardrails: RoutineGuardrails::default(),
            notify: crate::agent::routine::NotifyConfig {
                user: user_id.to_string(),
                on_success: true,
                ..crate::agent::routine::NotifyConfig::default()
            },
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        };
        db.create_routine(&routine).await.expect("create routine");

        let event = DbLearningEvent {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            actor_id: Some(user_id.to_string()),
            channel: Some("gateway".to_string()),
            thread_id: Some("thread-routine".to_string()),
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: "outcome_candidate".to_string(),
            source: "test".to_string(),
            payload: serde_json::json!({}),
            metadata: None,
            created_at: Utc::now(),
        };
        db.insert_learning_event(&event)
            .await
            .expect("insert learning event");
        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: user_id.to_string(),
            candidate_type: "routine_patch".to_string(),
            risk_tier: "medium".to_string(),
            confidence: Some(0.91),
            target_type: Some("routine".to_string()),
            target_name: Some(routine.name.clone()),
            summary: Some("Disable noisy success notifications for this routine".to_string()),
            proposal: serde_json::json!({
                "routine_patch": {
                    "type": "notification_noise_reduction",
                    "routine_id": routine.id.to_string(),
                    "changes": {
                        "notify": {
                            "on_success": false
                        }
                    }
                }
            }),
            created_at: Utc::now(),
        };
        db.insert_learning_candidate(&candidate)
            .await
            .expect("insert learning candidate");

        let orchestrator =
            LearningOrchestrator::new(Arc::clone(&db), Some(workspace), None::<Arc<_>>)
                .with_routine_engine(Some(routine_engine));

        let applied = orchestrator
            .auto_apply_routine(&candidate)
            .await
            .expect("auto_apply_routine should succeed");
        assert!(applied, "routine patch should auto-apply");

        let updated_routine = db
            .get_routine(routine.id)
            .await
            .expect("get routine")
            .expect("routine should exist");
        assert!(
            !updated_routine.notify.on_success,
            "routine success notifications should be disabled"
        );

        let artifact_versions = db
            .list_learning_artifact_versions(user_id, Some("routine"), Some(&routine.name), 10)
            .await
            .expect("list learning artifact versions");
        assert_eq!(
            artifact_versions.len(),
            1,
            "routine mutation should be ledgered"
        );
        let version = &artifact_versions[0];
        assert_eq!(version.status, "applied");
        assert_eq!(version.artifact_type, "routine");
        assert!(
            version
                .provenance
                .get("patch_type")
                .and_then(|value| value.as_str())
                == Some("notification_noise_reduction")
        );

        let contracts = db
            .list_outcome_contracts(&crate::history::OutcomeContractQuery {
                user_id: user_id.to_string(),
                actor_id: Some(user_id.to_string()),
                status: Some("open".to_string()),
                contract_type: Some("tool_durability".to_string()),
                source_kind: Some("artifact_version".to_string()),
                source_id: Some(version.id.to_string()),
                thread_id: None,
                limit: 10,
            })
            .await
            .expect("list outcome contracts");
        assert_eq!(
            contracts.len(),
            1,
            "routine artifact auto-apply should create a durability contract"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn auto_apply_prompt_materializes_patch_for_actor_user_targets() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "prompt-auto-apply-user";
        let actor_target = paths::actor_user("alice");
        let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
        let orchestrator = LearningOrchestrator::new(
            Arc::clone(&db),
            Some(Arc::clone(&workspace)),
            None::<Arc<_>>,
        );

        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: user_id.to_string(),
            candidate_type: "prompt".to_string(),
            risk_tier: "medium".to_string(),
            confidence: Some(0.88),
            target_type: Some("prompt".to_string()),
            target_name: Some(actor_target.clone()),
            summary: Some("Add outcome-backed prompt guidance".to_string()),
            proposal: serde_json::json!({
                "target": actor_target,
                "prompt_patch": {
                    "operation": "upsert_section",
                    "heading": "Outcome-Backed Guidance",
                    "section_content": "- prefer direct implementation and verification"
                }
            }),
            created_at: Utc::now(),
        };
        db.insert_learning_candidate(&candidate)
            .await
            .expect("insert prompt learning candidate");

        let applied = orchestrator
            .auto_apply_prompt(&candidate)
            .await
            .expect("auto_apply_prompt should succeed");
        assert!(applied, "prompt patch should auto-apply");

        let content = workspace
            .read(&actor_target)
            .await
            .expect("read actor USER.md")
            .content;
        assert!(content.contains("## Outcome-Backed Guidance"));
        assert!(content.contains("prefer direct implementation and verification"));

        let versions = db
            .list_learning_artifact_versions(user_id, Some("prompt"), Some(&actor_target), 10)
            .await
            .expect("list prompt artifact versions");
        assert_eq!(versions.len(), 1, "prompt auto-apply should be ledgered");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn auto_apply_prompt_routes_canonical_soul_to_home_store() {
        let (db, _guard) = crate::testing::test_db().await;
        let temp_home = tempfile::tempdir().expect("temp home");
        let previous_home = std::env::var_os("THINCLAW_HOME");
        unsafe {
            std::env::set_var("THINCLAW_HOME", temp_home.path());
        }

        let user_id = "prompt-auto-apply-soul";
        let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
        crate::identity::soul_store::write_home_soul(
            &crate::identity::soul::compose_seeded_soul("balanced").unwrap(),
        )
        .expect("write initial home soul");
        workspace
            .write(paths::SOUL, "# stale workspace soul should not change")
            .await
            .expect("write stale legacy workspace soul");

        let orchestrator = LearningOrchestrator::new(
            Arc::clone(&db),
            Some(Arc::clone(&workspace)),
            None::<Arc<_>>,
        );

        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: user_id.to_string(),
            candidate_type: "prompt".to_string(),
            risk_tier: "medium".to_string(),
            confidence: Some(0.9),
            target_type: Some("prompt".to_string()),
            target_name: Some(paths::SOUL.to_string()),
            summary: Some("Sharpen canonical soul guidance".to_string()),
            proposal: serde_json::json!({
                "target": paths::SOUL,
                "prompt_patch": {
                    "operation": "upsert_section",
                    "heading": "Outcome-Backed Guidance",
                    "section_content": "- be direct and finish the job"
                }
            }),
            created_at: Utc::now(),
        };
        db.insert_learning_candidate(&candidate)
            .await
            .expect("insert prompt learning candidate");

        let applied = orchestrator
            .auto_apply_prompt(&candidate)
            .await
            .expect("auto_apply_prompt should succeed");
        assert!(applied, "canonical soul patch should auto-apply");

        let home = crate::identity::soul_store::read_home_soul().expect("read home soul");
        assert!(home.contains("## Outcome-Backed Guidance"));
        assert!(home.contains("be direct and finish the job"));

        let workspace_soul = workspace
            .read(paths::SOUL)
            .await
            .expect("read stale workspace soul");
        assert!(
            !workspace_soul
                .content
                .contains("be direct and finish the job"),
            "auto-apply should not write canonical soul changes into workspace SOUL.md"
        );

        let versions = db
            .list_learning_artifact_versions(user_id, Some("prompt"), Some(paths::SOUL), 10)
            .await
            .expect("list prompt artifact versions");
        assert_eq!(
            versions.len(),
            1,
            "canonical soul auto-apply should be ledgered"
        );

        if let Some(previous_home) = previous_home {
            unsafe {
                std::env::set_var("THINCLAW_HOME", previous_home);
            }
        } else {
            unsafe {
                std::env::remove_var("THINCLAW_HOME");
            }
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn create_code_proposal_from_candidate_rejects_empty_diff() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "empty-diff-outcome-user";
        let orchestrator = LearningOrchestrator::new(Arc::clone(&db), None, None::<Arc<_>>);

        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: user_id.to_string(),
            candidate_type: "code".to_string(),
            risk_tier: "critical".to_string(),
            confidence: Some(0.92),
            target_type: Some("code".to_string()),
            target_name: Some("Fix missing diff handling".to_string()),
            summary: Some("Repeated negative durability outcomes".to_string()),
            proposal: serde_json::json!({
                "title": "Fix missing diff handling",
                "rationale": "Outcome-backed durability fix",
                "target_files": ["src/agent/learning.rs"],
                "diff": ""
            }),
            created_at: Utc::now(),
        };

        let err = orchestrator
            .create_code_proposal_from_candidate(&candidate)
            .await
            .expect_err("empty diff should be rejected");
        assert!(err.contains("missing diff"));
    }

    #[test]
    fn generated_skill_lifecycle_requires_shadow_before_activation() {
        let draft = generated_skill_lifecycle_for_reuse(1);
        assert_eq!(draft.0.as_str(), "draft");
        assert_eq!(draft.1, None);
        assert!(!draft.2);

        let shadow = generated_skill_lifecycle_for_reuse(2);
        assert_eq!(shadow.0.as_str(), "shadow");
        assert_eq!(shadow.1.as_deref(), Some("shadow_candidate"));
        assert!(!shadow.2);

        let second_shadow_match = generated_skill_lifecycle_for_reuse(3);
        assert_eq!(second_shadow_match.0.as_str(), "shadow");
        assert_eq!(second_shadow_match.1.as_deref(), Some("shadow_candidate"));
        assert!(!second_shadow_match.2);

        let proposed_threshold = generated_skill_lifecycle_for_reuse(4);
        assert_eq!(proposed_threshold.0.as_str(), "proposed");
        assert_eq!(
            proposed_threshold.1.as_deref(),
            Some("proposal_reuse_threshold")
        );
        assert!(!proposed_threshold.2);
    }

    #[test]
    fn generated_skill_feedback_polarity_maps_positive_and_negative_verdicts() {
        assert_eq!(generated_skill_feedback_polarity("helpful"), 1);
        assert_eq!(generated_skill_feedback_polarity("APPROVED"), 1);
        assert_eq!(generated_skill_feedback_polarity("reject"), -1);
        assert_eq!(generated_skill_feedback_polarity("dont_learn"), -1);
        assert_eq!(generated_skill_feedback_polarity("unclear"), 0);
    }

    #[test]
    fn generated_workflow_digest_distinguishes_parameters_and_outcomes() {
        let first = vec![crate::agent::session::TurnToolCall {
            name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "echo one"}),
            result: Some(serde_json::json!({"stdout": "one"})),
            error: None,
        }];
        let second = vec![crate::agent::session::TurnToolCall {
            name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "echo two"}),
            result: Some(serde_json::json!({"stdout": "two"})),
            error: None,
        }];

        assert_ne!(
            generated_workflow_digest("run the shell command", &first),
            generated_workflow_digest("run the shell command", &second)
        );
    }

    #[test]
    fn generated_workflow_digest_is_stable_for_reordered_object_keys() {
        let first = vec![crate::agent::session::TurnToolCall {
            name: "http".to_string(),
            parameters: serde_json::json!({"url": "https://example.com", "method": "GET"}),
            result: Some(serde_json::json!({"status": 200, "ok": true})),
            error: None,
        }];
        let second = vec![crate::agent::session::TurnToolCall {
            name: "http".to_string(),
            parameters: serde_json::json!({"method": "GET", "url": "https://example.com"}),
            result: Some(serde_json::json!({"ok": true, "status": 200})),
            error: None,
        }];

        assert_eq!(
            generated_workflow_digest("fetch the endpoint", &first),
            generated_workflow_digest("fetch the endpoint", &second)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn prefetch_provider_context_uses_only_the_active_provider() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "provider-prefetch-user";
        db.set_setting(
            user_id,
            "learning.providers.active",
            &serde_json::json!("honcho"),
        )
        .await
        .expect("set active provider");

        let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
        let zep_recalls = Arc::new(Mutex::new(Vec::new()));
        let orchestrator = LearningOrchestrator {
            store: Arc::clone(&db),
            workspace: None,
            skill_registry: None,
            routine_engine: None,
            provider_manager: Arc::new(MemoryProviderManager::with_providers(
                Arc::clone(&db),
                vec![
                    Arc::new(TestMemoryProvider {
                        name: "honcho",
                        hits: vec![ProviderMemoryHit {
                            provider: "honcho".to_string(),
                            summary: "Remembered preference".to_string(),
                            score: Some(0.91),
                            provenance: serde_json::json!({"id": "honcho:1"}),
                        }],
                        recalls: Arc::clone(&honcho_recalls),
                        health_status: provider_status(
                            "honcho",
                            ProviderReadiness::Ready,
                            true,
                            None,
                        ),
                    }),
                    Arc::new(TestMemoryProvider {
                        name: "zep",
                        hits: vec![ProviderMemoryHit {
                            provider: "zep".to_string(),
                            summary: "Should not be used".to_string(),
                            score: Some(0.32),
                            provenance: serde_json::json!({"id": "zep:1"}),
                        }],
                        recalls: Arc::clone(&zep_recalls),
                        health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                    }),
                ],
            )),
        };

        let context = orchestrator
            .prefetch_provider_context(user_id, "summarize my preferences", 3)
            .await
            .expect("active provider should return prefetch context");

        assert_eq!(context.provider, "honcho");
        assert_eq!(context.context_refs, vec!["honcho:1"]);
        assert!(context.rendered_context.contains("honcho"));
        assert_eq!(
            honcho_recalls.lock().expect("honcho recall log").len(),
            1,
            "the selected provider should be queried exactly once"
        );
        assert!(
            zep_recalls.lock().expect("zep recall log").is_empty(),
            "inactive providers must not be queried"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn unhealthy_active_provider_fails_closed_for_prefetch_and_tool_surface() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "provider-health-gating-user";
        db.set_setting(
            user_id,
            "learning.providers.active",
            &serde_json::json!("honcho"),
        )
        .await
        .expect("set active provider");

        let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
        let zep_recalls = Arc::new(Mutex::new(Vec::new()));
        let orchestrator = LearningOrchestrator {
            store: Arc::clone(&db),
            workspace: None,
            skill_registry: None,
            routine_engine: None,
            provider_manager: Arc::new(MemoryProviderManager::with_providers(
                Arc::clone(&db),
                vec![
                    Arc::new(TestMemoryProvider {
                        name: "honcho",
                        hits: vec![ProviderMemoryHit {
                            provider: "honcho".to_string(),
                            summary: "Should not be recalled".to_string(),
                            score: Some(0.11),
                            provenance: serde_json::json!({"id": "honcho:down"}),
                        }],
                        recalls: Arc::clone(&honcho_recalls),
                        health_status: provider_status(
                            "honcho",
                            ProviderReadiness::Unhealthy,
                            false,
                            Some("provider health check failed"),
                        ),
                    }),
                    Arc::new(TestMemoryProvider {
                        name: "zep",
                        hits: vec![ProviderMemoryHit {
                            provider: "zep".to_string(),
                            summary: "Inactive backup".to_string(),
                            score: Some(0.88),
                            provenance: serde_json::json!({"id": "zep:1"}),
                        }],
                        recalls: Arc::clone(&zep_recalls),
                        health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                    }),
                ],
            )),
        };

        let statuses = orchestrator.provider_health(user_id).await;
        let active = statuses
            .iter()
            .find(|status| status.provider == "honcho")
            .expect("active provider status");
        assert!(active.active, "honcho should be marked active");
        assert_eq!(active.readiness, ProviderReadiness::Unhealthy);

        assert!(
            orchestrator
                .prefetch_provider_context(user_id, "remember my preferences", 3)
                .await
                .is_none(),
            "unhealthy providers should not surface prompt recall"
        );
        assert!(
            orchestrator
                .provider_recall(user_id, "remember my preferences", 3)
                .await
                .is_empty(),
            "unhealthy providers should not execute recall calls"
        );
        assert!(
            orchestrator
                .provider_tool_extensions(user_id)
                .await
                .is_empty(),
            "tool extensions should disappear when the active provider is unhealthy"
        );
        assert!(
            honcho_recalls.lock().expect("honcho recall log").is_empty(),
            "prefetch/recall must fail closed before dispatching to an unhealthy provider"
        );
        assert!(
            zep_recalls.lock().expect("zep recall log").is_empty(),
            "inactive backups must not be used automatically"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn positive_feedback_promotes_generated_skill_and_updates_candidate_proposal() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "generated-skill-positive-feedback";
        let created_at = Utc::now();
        let skill_name = "workflow-generated-positive";
        let skill_content = generated_skill_test_content(skill_name);
        let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
        db.insert_learning_candidate(&candidate)
            .await
            .expect("insert learning candidate");

        let user_dir =
            tempfile::tempdir().expect("temporary user dir for generated skill registry");
        let installed_dir =
            tempfile::tempdir().expect("temporary installed dir for generated skill registry");
        let registry = Arc::new(tokio::sync::RwLock::new(
            SkillRegistry::new(user_dir.path().to_path_buf())
                .with_installed_dir(installed_dir.path().to_path_buf()),
        ));
        let orchestrator =
            LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

        orchestrator
            .submit_feedback(
                user_id,
                "skill",
                skill_name,
                "helpful",
                Some("this saved time"),
                None,
            )
            .await
            .expect("positive feedback should activate generated skill");

        assert!(
            registry.read().await.has(skill_name),
            "positive feedback should install the generated skill"
        );

        let persisted = db
            .list_learning_candidates(user_id, Some("skill"), None, 10)
            .await
            .expect("list learning candidates")
            .into_iter()
            .find(|entry| entry.id == candidate.id)
            .expect("updated candidate");
        assert_eq!(
            persisted
                .proposal
                .get("lifecycle_status")
                .and_then(|value| value.as_str()),
            Some("active")
        );
        assert_eq!(
            persisted
                .proposal
                .get("activation_reason")
                .and_then(|value| value.as_str()),
            Some("explicit_positive_feedback")
        );
        assert_eq!(
            persisted
                .proposal
                .get("last_feedback")
                .and_then(|value| value.get("verdict"))
                .and_then(|value| value.as_str()),
            Some("helpful")
        );
        assert!(
            persisted
                .proposal
                .get("state_history")
                .and_then(|value| value.as_array())
                .is_some_and(|entries| entries.len() >= 2),
            "candidate proposal should retain lifecycle history on the canonical record"
        );

        let versions = db
            .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
            .await
            .expect("list learning artifact versions");
        let active_version = versions
            .iter()
            .find(|version| version.status == "active")
            .expect("active artifact version");
        assert_eq!(active_version.candidate_id, Some(candidate.id));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn negative_feedback_rolls_back_generated_skill_and_updates_candidate_proposal() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "generated-skill-negative-feedback";
        let created_at = Utc::now();
        let skill_name = "workflow-generated-negative";
        let skill_content = generated_skill_test_content(skill_name);
        let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
        db.insert_learning_candidate(&candidate)
            .await
            .expect("insert learning candidate");

        let user_dir =
            tempfile::tempdir().expect("temporary user dir for generated skill registry");
        let installed_dir =
            tempfile::tempdir().expect("temporary installed dir for generated skill registry");
        let registry = Arc::new(tokio::sync::RwLock::new(
            SkillRegistry::new(user_dir.path().to_path_buf())
                .with_installed_dir(installed_dir.path().to_path_buf()),
        ));
        registry
            .write()
            .await
            .install_skill(&skill_content)
            .await
            .expect("preinstall generated skill");
        let orchestrator =
            LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

        orchestrator
            .submit_feedback(
                user_id,
                "skill",
                skill_name,
                "reject",
                Some("this introduced drift"),
                None,
            )
            .await
            .expect("negative feedback should update generated skill lifecycle");

        assert!(
            !registry.read().await.has(skill_name),
            "negative feedback should remove the installed generated skill"
        );

        let persisted = db
            .list_learning_candidates(user_id, Some("skill"), None, 10)
            .await
            .expect("list learning candidates")
            .into_iter()
            .find(|entry| entry.id == candidate.id)
            .expect("updated candidate");
        assert_eq!(
            persisted
                .proposal
                .get("lifecycle_status")
                .and_then(|value| value.as_str()),
            Some("rolled_back")
        );
        assert_eq!(
            persisted
                .proposal
                .get("last_feedback")
                .and_then(|value| value.get("verdict"))
                .and_then(|value| value.as_str()),
            Some("reject")
        );
        assert!(
            persisted
                .proposal
                .get("rolled_back_at")
                .and_then(|value| value.as_str())
                .is_some(),
            "candidate proposal should record rollback timing"
        );

        let versions = db
            .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
            .await
            .expect("list learning artifact versions");
        let rollback_version = versions
            .iter()
            .find(|version| version.status == "rolled_back")
            .expect("rollback artifact version");
        assert_eq!(rollback_version.candidate_id, Some(candidate.id));
    }
}

//! Root-independent learning, workspace, skill, and routine mutation ports.
//!
//! This is a staging module for the crate split. It keeps the extracted agent
//! side dependent on serializable DTOs and narrow traits instead of root
//! `thinclaw` settings, workspace, skills, or routine engine adapters.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_types::error::{DatabaseError, RoutineError, WorkspaceError};
use uuid::Uuid;

fn default_true() -> bool {
    true
}

fn default_learning_auto_apply_classes() -> Vec<String> {
    vec![
        "memory".to_string(),
        "skill".to_string(),
        "prompt".to_string(),
    ]
}

fn default_learning_publish_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_learning_skill_synthesis_min_tool_calls() -> u32 {
    3
}

fn default_learning_safe_mode_rollback_ratio() -> f64 {
    0.25
}

fn default_learning_safe_mode_negative_feedback_ratio() -> f64 {
    0.20
}

fn default_learning_safe_mode_min_samples() -> u32 {
    8
}

fn default_learning_reflection_min_tool_calls() -> u32 {
    2
}

fn default_learning_reflection_correction_threshold() -> u32 {
    1
}

fn default_learning_outcomes_evaluation_interval_secs() -> u64 {
    600
}

fn default_learning_outcomes_max_due_per_tick() -> u32 {
    50
}

fn default_learning_outcomes_default_ttl_hours() -> u32 {
    72
}

/// Portable trust tier used by remote skill sources and quarantine metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableSkillSourceTrust {
    Builtin,
    Trusted,
    #[default]
    Community,
}

/// Portable authority ceiling for installed skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableSkillTrust {
    Installed,
    Trusted,
}

/// Canonical memory provider selector without depending on root settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableActiveLearningProvider {
    #[default]
    None,
    Honcho,
    Zep,
}

impl PortableActiveLearningProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Honcho => "honcho",
            Self::Zep => "zep",
        }
    }
}

/// Provider-specific learning memory configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningProviderSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub config: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cadence: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    #[serde(default)]
    pub user_modeling_enabled: bool,
}

/// Learning provider selection and registry-backed provider config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningProvidersSettings {
    #[serde(default)]
    pub active: PortableActiveLearningProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub registry: HashMap<String, PortableLearningProviderSettings>,
    #[serde(default)]
    pub honcho: PortableLearningProviderSettings,
    #[serde(default)]
    pub zep: PortableLearningProviderSettings,
}

impl Default for PortableLearningProvidersSettings {
    fn default() -> Self {
        Self {
            active: PortableActiveLearningProvider::None,
            active_provider: None,
            registry: HashMap::new(),
            honcho: PortableLearningProviderSettings::default(),
            zep: PortableLearningProviderSettings::default(),
        }
    }
}

impl PortableLearningProvidersSettings {
    pub fn active_provider_name(&self) -> Option<String> {
        self.active_provider.clone().or_else(|| match self.active {
            PortableActiveLearningProvider::None => None,
            PortableActiveLearningProvider::Honcho => Some("honcho".to_string()),
            PortableActiveLearningProvider::Zep => Some("zep".to_string()),
        })
    }

    pub fn provider(&self, name: &str) -> Option<&PortableLearningProviderSettings> {
        if let Some(provider) = self.registry.get(name) {
            return Some(provider);
        }
        match name {
            "honcho" => Some(&self.honcho),
            "zep" => Some(&self.zep),
            _ => None,
        }
    }
}

/// Safe-mode thresholds for autonomous mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableLearningSafeModeThresholds {
    #[serde(default = "default_learning_safe_mode_rollback_ratio")]
    pub rollback_ratio: f64,
    #[serde(default = "default_learning_safe_mode_negative_feedback_ratio")]
    pub negative_feedback_ratio: f64,
    #[serde(default = "default_learning_safe_mode_min_samples")]
    pub min_samples: u32,
}

impl Default for PortableLearningSafeModeThresholds {
    fn default() -> Self {
        Self {
            rollback_ratio: default_learning_safe_mode_rollback_ratio(),
            negative_feedback_ratio: default_learning_safe_mode_negative_feedback_ratio(),
            min_samples: default_learning_safe_mode_min_samples(),
        }
    }
}

/// Safe-mode behavior for learning mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableLearningSafeModeSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub thresholds: PortableLearningSafeModeThresholds,
}

impl Default for PortableLearningSafeModeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            thresholds: PortableLearningSafeModeThresholds::default(),
        }
    }
}

/// Turn reflection thresholds used before creating candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningReflectionSettings {
    #[serde(default = "default_learning_reflection_min_tool_calls")]
    pub min_tool_calls: u32,
    #[serde(default = "default_learning_reflection_correction_threshold")]
    pub user_correction_threshold: u32,
}

impl Default for PortableLearningReflectionSettings {
    fn default() -> Self {
        Self {
            min_tool_calls: default_learning_reflection_min_tool_calls(),
            user_correction_threshold: default_learning_reflection_correction_threshold(),
        }
    }
}

/// Generated-skill synthesis controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningSkillSynthesisSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_learning_skill_synthesis_min_tool_calls")]
    pub min_tool_calls: u32,
    #[serde(default)]
    pub auto_apply: bool,
}

impl Default for PortableLearningSkillSynthesisSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_tool_calls: default_learning_skill_synthesis_min_tool_calls(),
            auto_apply: false,
        }
    }
}

/// Gate for autonomous prompt mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningPromptMutationSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for PortableLearningPromptMutationSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Approval-gated code proposal controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningCodeProposalSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_learning_publish_mode")]
    pub publish_mode: String,
    #[serde(default)]
    pub auto_apply_without_review: bool,
}

impl Default for PortableLearningCodeProposalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            publish_mode: default_learning_publish_mode(),
            auto_apply_without_review: false,
        }
    }
}

/// Optional trajectory/export hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningExportSettings {
    #[serde(default)]
    pub enabled: bool,
}

/// Outcome-evaluation controls for deferred consequence checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLearningOutcomeSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_learning_outcomes_evaluation_interval_secs")]
    pub evaluation_interval_secs: u64,
    #[serde(default = "default_learning_outcomes_max_due_per_tick")]
    pub max_due_per_tick: u32,
    #[serde(default = "default_learning_outcomes_default_ttl_hours")]
    pub default_ttl_hours: u32,
    #[serde(default = "default_true")]
    pub llm_assist_enabled: bool,
    #[serde(default = "default_true")]
    pub heartbeat_summary_enabled: bool,
}

impl Default for PortableLearningOutcomeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            evaluation_interval_secs: default_learning_outcomes_evaluation_interval_secs(),
            max_due_per_tick: default_learning_outcomes_max_due_per_tick(),
            default_ttl_hours: default_learning_outcomes_default_ttl_hours(),
            llm_assist_enabled: true,
            heartbeat_summary_enabled: true,
        }
    }
}

/// Root-independent learning settings used by extracted agent logic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableLearningSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_learning_auto_apply_classes")]
    pub auto_apply_classes: Vec<String>,
    #[serde(default)]
    pub safe_mode: PortableLearningSafeModeSettings,
    #[serde(default)]
    pub reflection: PortableLearningReflectionSettings,
    #[serde(default)]
    pub skill_synthesis: PortableLearningSkillSynthesisSettings,
    #[serde(default)]
    pub prompt_mutation: PortableLearningPromptMutationSettings,
    #[serde(default)]
    pub providers: PortableLearningProvidersSettings,
    #[serde(default)]
    pub code_proposals: PortableLearningCodeProposalSettings,
    #[serde(default)]
    pub exports: PortableLearningExportSettings,
    #[serde(default)]
    pub outcomes: PortableLearningOutcomeSettings,
}

impl Default for PortableLearningSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_apply_classes: default_learning_auto_apply_classes(),
            safe_mode: PortableLearningSafeModeSettings::default(),
            reflection: PortableLearningReflectionSettings::default(),
            skill_synthesis: PortableLearningSkillSynthesisSettings::default(),
            prompt_mutation: PortableLearningPromptMutationSettings::default(),
            providers: PortableLearningProvidersSettings::default(),
            code_proposals: PortableLearningCodeProposalSettings::default(),
            exports: PortableLearningExportSettings::default(),
            outcomes: PortableLearningOutcomeSettings::default(),
        }
    }
}

impl PortableLearningSettings {
    pub fn auto_apply_enabled_for(&self, class_name: &str) -> bool {
        self.auto_apply_classes
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(class_name))
    }
}

/// Sparse settings update for UI/tool-driven learning configuration changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LearningSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_apply_classes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_mutation_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_synthesis_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_synthesis_auto_apply: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_proposals_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_proposals_auto_apply_without_review: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcomes_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<Option<String>>,
    #[serde(default)]
    pub provider_updates: HashMap<String, PortableLearningProviderSettings>,
}

/// User learning settings persistence and mutation surface.
#[async_trait]
pub trait LearningSettingsPort: Send + Sync {
    async fn load_learning_settings(
        &self,
        user_id: &str,
    ) -> Result<PortableLearningSettings, DatabaseError>;

    async fn save_learning_settings(
        &self,
        user_id: &str,
        settings: &PortableLearningSettings,
    ) -> Result<(), DatabaseError>;

    async fn patch_learning_settings(
        &self,
        user_id: &str,
        patch: &LearningSettingsPatch,
    ) -> Result<PortableLearningSettings, DatabaseError>;
}

/// Evaluation result for a learning event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningEvaluationRecord {
    pub id: Uuid,
    pub learning_event_id: Uuid,
    pub user_id: String,
    pub evaluator: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Distilled improvement candidate derived from learning events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningCandidateRecord {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub candidate_type: String,
    pub risk_tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub proposal: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningCandidateQuery {
    pub user_id: String,
    pub candidate_type: Option<String>,
    pub risk_tier: Option<String>,
    pub limit: i64,
}

/// Versioned snapshot of a learned artifact mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningArtifactVersionRecord {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_label: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_content: Option<String>,
    pub provenance: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningArtifactVersionQuery {
    pub user_id: String,
    pub artifact_type: Option<String>,
    pub artifact_name: Option<String>,
    pub limit: i64,
}

/// Explicit user/operator feedback on a learning target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningFeedbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningFeedbackQuery {
    pub user_id: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub limit: i64,
}

/// Recorded rollback operation for a learned artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningRollbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version_id: Option<Uuid>,
    pub reason: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningRollbackQuery {
    pub user_id: String,
    pub artifact_type: Option<String>,
    pub artifact_name: Option<String>,
    pub limit: i64,
}

/// Approval-gated code change proposal generated by learning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningCodeProposalRecord {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub status: String,
    pub title: String,
    pub rationale: String,
    pub target_files: Vec<String>,
    pub diff: String,
    pub validation_results: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningCodeProposalQuery {
    pub user_id: String,
    pub status: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LearningCodeProposalPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Persistence surface for learning mutations not covered by LearningOutcomesPort.
#[async_trait]
pub trait LearningMutationPort: Send + Sync {
    async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluationRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidateRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_learning_candidates(
        &self,
        query: &LearningCandidateQuery,
    ) -> Result<Vec<LearningCandidateRecord>, DatabaseError>;

    async fn update_learning_candidate_proposal(
        &self,
        candidate_id: Uuid,
        proposal: &serde_json::Value,
    ) -> Result<(), DatabaseError>;

    async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersionRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_learning_artifact_versions(
        &self,
        query: &LearningArtifactVersionQuery,
    ) -> Result<Vec<LearningArtifactVersionRecord>, DatabaseError>;

    async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_learning_feedback(
        &self,
        query: &LearningFeedbackQuery,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError>;

    async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn list_learning_rollbacks(
        &self,
        query: &LearningRollbackQuery,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError>;

    async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposalRecord,
    ) -> Result<Uuid, DatabaseError>;

    async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposalRecord>, DatabaseError>;

    async fn list_learning_code_proposals(
        &self,
        query: &LearningCodeProposalQuery,
    ) -> Result<Vec<LearningCodeProposalRecord>, DatabaseError>;

    async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        patch: &LearningCodeProposalPatch,
    ) -> Result<(), DatabaseError>;
}

/// Supported workspace artifact targets for prompt and memory mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WorkspaceMutationTarget {
    Soul,
    SoulLocal,
    Agents,
    User,
    Memory,
    Heartbeat,
    DailyLog { date: String },
    Custom { logical_path: String },
}

impl WorkspaceMutationTarget {
    pub fn logical_name(&self) -> String {
        match self {
            Self::Soul => "SOUL.md".to_string(),
            Self::SoulLocal => "SOUL.local.md".to_string(),
            Self::Agents => "AGENTS.md".to_string(),
            Self::User => "USER.md".to_string(),
            Self::Memory => "MEMORY.md".to_string(),
            Self::Heartbeat => "HEARTBEAT.md".to_string(),
            Self::DailyLog { date } => format!("daily_log:{date}"),
            Self::Custom { logical_path } => logical_path.clone(),
        }
    }
}

/// Text edit to apply to a workspace artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkspaceMutationEdit {
    Replace {
        content: String,
    },
    Append {
        content: String,
    },
    ReplaceRange {
        start_line: usize,
        end_line: usize,
        content: String,
    },
    Delete,
}

/// Snapshot of a workspace artifact before or after mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceDocumentSnapshot {
    pub target: WorkspaceMutationTarget,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Request to mutate a workspace artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMutationRequest {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub target: WorkspaceMutationTarget,
    pub edit: WorkspaceMutationEdit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Result of a workspace mutation attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMutationResult {
    pub target: WorkspaceMutationTarget,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<WorkspaceDocumentSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<WorkspaceDocumentSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<LearningArtifactVersionRecord>,
    #[serde(default)]
    pub diagnostics: serde_json::Value,
}

/// Shared severity for mutation and skill quarantine findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationFindingSeverity {
    Info,
    Warning,
    Critical,
}

/// Portable finding shape emitted by mutation scanners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationFinding {
    pub kind: String,
    pub severity: MutationFindingSeverity,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanner_version: Option<String>,
}

/// Summarized finding counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationFindingSummary {
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub warnings: usize,
    #[serde(default)]
    pub critical: usize,
    #[serde(default)]
    pub categories: Vec<String>,
}

/// Quarantine record for a workspace mutation held for review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMutationQuarantineRecord {
    pub id: Uuid,
    pub request: WorkspaceMutationRequest,
    pub summary: MutationFindingSummary,
    pub findings: Vec<MutationFinding>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// Workspace artifact mutation surface.
#[async_trait]
pub trait WorkspaceMutationPort: Send + Sync {
    async fn read_workspace_document(
        &self,
        target: &WorkspaceMutationTarget,
    ) -> Result<Option<WorkspaceDocumentSnapshot>, WorkspaceError>;

    async fn apply_workspace_mutation(
        &self,
        request: WorkspaceMutationRequest,
    ) -> Result<WorkspaceMutationResult, WorkspaceError>;

    async fn quarantine_workspace_mutation(
        &self,
        request: WorkspaceMutationRequest,
        findings: Vec<MutationFinding>,
    ) -> Result<WorkspaceMutationQuarantineRecord, WorkspaceError>;
}

/// File inside a skill package scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillScanFile {
    pub relative_path: String,
    pub content: String,
}

/// Downloaded or generated skill content plus source metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillContent {
    pub raw_content: String,
    pub source_kind: String,
    pub source_adapter: String,
    pub source_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub trust_level: PortableSkillSourceTrust,
}

/// Static scan report for a quarantined skill package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillScanReport {
    pub scanner_version: String,
    pub content_sha256: String,
    pub summary: MutationFindingSummary,
    pub findings: Vec<MutationFinding>,
}

/// Quarantined skill package held before installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuarantinedSkillRecord {
    pub id: Uuid,
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine_path: Option<String>,
    pub content: SkillContent,
    #[serde(default)]
    pub package_files: Vec<SkillScanFile>,
    pub report: SkillScanReport,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillQuarantineDecision {
    Approve,
    Reject,
    Cleanup,
}

/// Request to approve, reject, or clean up a quarantined skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillQuarantineReview {
    pub quarantine_id: Uuid,
    pub decision: SkillQuarantineDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub allow_critical_findings: bool,
}

/// Result of a quarantine review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillQuarantineReviewResult {
    pub quarantine_id: Uuid,
    pub decision: SkillQuarantineDecision,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_path: Option<String>,
    pub cleaned_up: bool,
    pub report: SkillScanReport,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Skill quarantine and scan surface.
#[async_trait]
pub trait SkillQuarantinePort: Send + Sync {
    async fn quarantine_skill(
        &self,
        skill_name: &str,
        content: &SkillContent,
        package_files: &[SkillScanFile],
    ) -> Result<QuarantinedSkillRecord, WorkspaceError>;

    async fn scan_quarantined_skill(
        &self,
        quarantine_id: Uuid,
    ) -> Result<SkillScanReport, WorkspaceError>;

    async fn review_quarantined_skill(
        &self,
        review: SkillQuarantineReview,
    ) -> Result<SkillQuarantineReviewResult, WorkspaceError>;
}

/// Skill registry mutation operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SkillRegistryMutation {
    Create {
        content: String,
        force: bool,
    },
    Delete,
    WriteFile {
        relative_path: String,
        content: String,
    },
    RemoveFile {
        relative_path: String,
    },
    Reload,
    ReloadAll,
    SetTrust {
        trust: PortableSkillTrust,
    },
}

/// Request to mutate the mutable skill registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRegistryMutationRequest {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    pub mutation: SkillRegistryMutation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    #[serde(default)]
    pub allow_quarantine_warnings: bool,
    #[serde(default)]
    pub allow_quarantine_critical: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Result of a skill registry mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRegistryMutationResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    pub changed: bool,
    #[serde(default)]
    pub loaded_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_report: Option<SkillScanReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<LearningArtifactVersionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_content: Option<String>,
    #[serde(default)]
    pub diagnostics: serde_json::Value,
}

/// Mutable skill registry surface.
#[async_trait]
pub trait SkillRegistryMutationPort: Send + Sync {
    async fn apply_skill_registry_mutation(
        &self,
        request: SkillRegistryMutationRequest,
    ) -> Result<SkillRegistryMutationResult, WorkspaceError>;

    async fn reload_all_skills(&self) -> Result<Vec<String>, WorkspaceError>;
}

/// Portable routine notification preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineNotifyConfig {
    pub channel: Option<String>,
    pub user: String,
    pub on_attention: bool,
    pub on_failure: bool,
    pub on_success: bool,
}

impl Default for RoutineNotifyConfig {
    fn default() -> Self {
        Self {
            channel: None,
            user: "default".to_string(),
            on_attention: true,
            on_failure: true,
            on_success: false,
        }
    }
}

/// Sparse patch for routine notification preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineNotificationPolicyPatch {
    pub routine_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default)]
    pub clear_channel: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_attention: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_success: Option<bool>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Routine run status used for notification delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineNotificationStatus {
    Running,
    Ok,
    Attention,
    Failed,
}

/// Notification payload produced by routine execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineNotificationMessage {
    pub routine_id: Uuid,
    pub routine_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub status: RoutineNotificationStatus,
    pub content: String,
    pub notify: RoutineNotifyConfig,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Mutation event emitted after routine config/runtime changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineMutationKind {
    Created,
    Updated,
    Deleted,
    RuntimeUpdated,
    NotificationPolicyUpdated,
    EventCacheRefreshRequested,
}

/// Observer payload for routine mutations that need cache refresh or UI updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineMutationEvent {
    pub routine_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub kind: RoutineMutationKind,
    #[serde(default)]
    pub changed_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

/// Routine mutation helpers not covered by RoutineStorePort storage methods.
#[async_trait]
pub trait RoutineMutationPort: Send + Sync {
    async fn apply_routine_notification_policy(
        &self,
        patch: RoutineNotificationPolicyPatch,
    ) -> Result<RoutineMutationEvent, RoutineError>;

    async fn notify_routine_mutation(
        &self,
        event: RoutineMutationEvent,
    ) -> Result<(), RoutineError>;

    async fn refresh_routine_event_cache(&self) -> Result<(), RoutineError>;
}

/// Routine notification delivery surface.
#[async_trait]
pub trait RoutineNotificationPort: Send + Sync {
    async fn send_routine_notification(
        &self,
        notification: RoutineNotificationMessage,
    ) -> Result<(), RoutineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_settings_defaults_match_root_behavioral_defaults() {
        let settings = PortableLearningSettings::default();

        assert!(settings.enabled);
        assert_eq!(
            settings.auto_apply_classes,
            vec![
                "memory".to_string(),
                "skill".to_string(),
                "prompt".to_string()
            ]
        );
        assert!(settings.auto_apply_enabled_for("PROMPT"));
        assert!(settings.prompt_mutation.enabled);
        assert!(settings.skill_synthesis.enabled);
        assert_eq!(settings.skill_synthesis.min_tool_calls, 3);
        assert!(!settings.skill_synthesis.auto_apply);
        assert!(settings.code_proposals.enabled);
        assert_eq!(settings.code_proposals.publish_mode, "branch_pr_draft");
        assert!(!settings.code_proposals.auto_apply_without_review);
        assert!(settings.outcomes.enabled);
        assert_eq!(settings.outcomes.evaluation_interval_secs, 600);
        assert_eq!(settings.outcomes.max_due_per_tick, 50);
        assert_eq!(settings.outcomes.default_ttl_hours, 72);
    }

    #[test]
    fn learning_settings_deserialize_defaults_for_sparse_json() {
        let settings: PortableLearningSettings =
            serde_json::from_value(serde_json::json!({})).expect("settings");

        assert!(settings.enabled);
        assert!(settings.prompt_mutation.enabled);
        assert_eq!(
            settings.providers.active,
            PortableActiveLearningProvider::None
        );
        assert_eq!(settings.providers.active_provider_name(), None);
    }

    #[test]
    fn skill_scan_report_serializes_snake_case_findings() {
        let report = SkillScanReport {
            scanner_version: "skill_quarantine_v2".to_string(),
            content_sha256: "sha256:abc".to_string(),
            summary: MutationFindingSummary {
                total: 1,
                warnings: 1,
                critical: 0,
                categories: vec!["network_fetch".to_string()],
            },
            findings: vec![MutationFinding {
                kind: "network_fetch".to_string(),
                severity: MutationFindingSeverity::Warning,
                excerpt: "curl".to_string(),
                rule_id: Some("network_fetch.001".to_string()),
                file: Some("SKILL.md".to_string()),
                line: Some(1),
                recommendation: None,
                scanner_version: Some("skill_quarantine_v2".to_string()),
            }],
        };

        let value = serde_json::to_value(&report).expect("report json");
        assert_eq!(value["findings"][0]["severity"], "warning");
        assert_eq!(value["summary"]["warnings"], 1);
        assert_eq!(value["findings"][0]["file"], "SKILL.md");
    }

    #[test]
    fn workspace_mutation_target_has_stable_logical_names() {
        assert_eq!(WorkspaceMutationTarget::Soul.logical_name(), "SOUL.md");
        assert_eq!(
            WorkspaceMutationTarget::DailyLog {
                date: "2026-06-01".to_string()
            }
            .logical_name(),
            "daily_log:2026-06-01"
        );
        assert_eq!(
            WorkspaceMutationTarget::Custom {
                logical_path: "notes/custom.md".to_string()
            }
            .logical_name(),
            "notes/custom.md"
        );
    }

    #[test]
    fn mutation_traits_are_object_safe() {
        let _: Option<&dyn LearningSettingsPort> = None;
        let _: Option<&dyn LearningMutationPort> = None;
        let _: Option<&dyn WorkspaceMutationPort> = None;
        let _: Option<&dyn SkillQuarantinePort> = None;
        let _: Option<&dyn SkillRegistryMutationPort> = None;
        let _: Option<&dyn RoutineMutationPort> = None;
        let _: Option<&dyn RoutineNotificationPort> = None;
    }
}

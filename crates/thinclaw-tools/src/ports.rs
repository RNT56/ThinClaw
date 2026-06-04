//! Root-independent host operation ports for generic tools.
//!
//! These ports let crate-owned tools describe the host services they need
//! without depending on the root application's database, workspace, sidecars,
//! or registries.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thinclaw_tools_core::ToolArtifact;
use thinclaw_types::JobContext;
use uuid::Uuid;

/// Common identity and routing scope for host-backed tool operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOperationScope {
    pub principal_id: String,
    pub actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl ToolOperationScope {
    pub fn new(principal_id: impl Into<String>, actor_id: impl Into<String>) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            conversation_id: None,
            thread_id: None,
            job_id: None,
            metadata: serde_json::Value::Null,
        }
    }
}

pub fn job_context_from_tool_scope(scope: ToolOperationScope, title: &str) -> JobContext {
    let ToolOperationScope {
        principal_id,
        actor_id,
        conversation_id,
        thread_id,
        job_id,
        metadata,
    } = scope;

    let mut ctx = JobContext::with_identity(principal_id, actor_id, title, title);
    if let Some(job_id) = job_id {
        ctx.job_id = job_id;
    }
    ctx.conversation_id = conversation_id;
    ctx.metadata = metadata;
    if let Some(thread_id) = thread_id {
        if let Some(metadata) = ctx.metadata.as_object_mut() {
            metadata
                .entry("thread_id")
                .or_insert_with(|| serde_json::json!(thread_id));
        } else {
            ctx.metadata = serde_json::json!({ "thread_id": thread_id });
        }
    }
    ctx
}

pub fn tool_scope_from_job_context(ctx: &JobContext) -> ToolOperationScope {
    let actor_id = ctx
        .actor_id
        .clone()
        .unwrap_or_else(|| ctx.principal_id.clone());
    let mut scope = ToolOperationScope::new(ctx.principal_id.clone(), actor_id);
    scope.conversation_id = ctx.conversation_id;
    scope.job_id = Some(ctx.job_id);
    scope.thread_id = ctx
        .metadata
        .get("thread_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    scope.metadata = ctx.metadata.clone();
    scope
}

/// Error boundary for host service adapters used by generic tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolHostError {
    #[error("not found: {resource}")]
    NotFound { resource: String },
    #[error("permission denied: {reason}")]
    PermissionDenied { reason: String },
    #[error("invalid request: {reason}")]
    InvalidRequest { reason: String },
    #[error("host service unavailable: {service}")]
    Unavailable { service: String },
    #[error("operation failed: {reason}")]
    OperationFailed { reason: String },
}

/// Portable job state exposed to generic job tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolJobStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Submitted,
    Accepted,
    Failed,
    Stuck,
    Cancelled,
    Abandoned,
}

/// Request to create a host-managed job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCreateJobRequest {
    pub scope: ToolOperationScope,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Host job snapshot returned to generic tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolJobSnapshot {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub status: ToolJobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolJobQuery {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub statuses: Vec<ToolJobStatus>,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolJobActionRequest {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolJobActionResult {
    pub output: serde_json::Value,
}

/// Host operations needed by generic job tools.
#[async_trait]
pub trait JobToolHostPort: Send + Sync {
    async fn create_job(
        &self,
        request: ToolCreateJobRequest,
    ) -> Result<ToolJobSnapshot, ToolHostError>;

    async fn load_job(
        &self,
        scope: ToolOperationScope,
        job_id: Uuid,
    ) -> Result<Option<ToolJobSnapshot>, ToolHostError>;

    async fn list_jobs(&self, query: ToolJobQuery) -> Result<Vec<ToolJobSnapshot>, ToolHostError>;

    async fn send_job_prompt(
        &self,
        scope: ToolOperationScope,
        job_id: Uuid,
        content: Option<String>,
        done: bool,
    ) -> Result<ToolJobSnapshot, ToolHostError>;

    async fn create_job_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;

    async fn list_jobs_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;

    async fn job_status_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;

    async fn cancel_job_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;

    async fn job_events_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;

    async fn job_prompt_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolMemoryScope {
    #[default]
    Auto,
    Shared,
    Actor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMemoryReadRequest {
    pub scope: ToolOperationScope,
    pub path: String,
    pub memory_scope: ToolMemoryScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMemoryWriteRequest {
    pub scope: ToolOperationScope,
    pub path: String,
    pub memory_scope: ToolMemoryScope,
    pub content: String,
    #[serde(default = "default_true")]
    pub append: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMemorySearchRequest {
    pub scope: ToolOperationScope,
    pub query: String,
    #[serde(default = "default_memory_limit")]
    pub limit: usize,
    #[serde(default)]
    pub include_group_history: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMemoryEntry {
    pub path: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMemoryActionRequest {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMemoryActionResult {
    pub output: serde_json::Value,
}

/// Host operations needed by generic memory tools.
#[async_trait]
pub trait MemoryToolHostPort: Send + Sync {
    async fn read_memory(
        &self,
        request: ToolMemoryReadRequest,
    ) -> Result<ToolMemoryEntry, ToolHostError>;

    async fn write_memory(
        &self,
        request: ToolMemoryWriteRequest,
    ) -> Result<ToolMemoryEntry, ToolHostError>;

    async fn search_memory(
        &self,
        request: ToolMemorySearchRequest,
    ) -> Result<Vec<ToolMemoryEntry>, ToolHostError>;

    async fn delete_memory(
        &self,
        scope: ToolOperationScope,
        path: String,
        memory_scope: ToolMemoryScope,
    ) -> Result<(), ToolHostError>;

    async fn search_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError>;

    async fn write_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError>;

    async fn read_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError>;

    async fn tree_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError>;

    async fn delete_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSkillTrust {
    Installed,
    Trusted,
    #[default]
    Community,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub trust: ToolSkillTrust,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillRead {
    pub name: String,
    pub version: String,
    pub description: String,
    pub trust: ToolSkillTrust,
    pub source_tier: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillSnapshotResult {
    pub path: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillQuery {
    pub scope: ToolOperationScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillSearchRequest {
    pub scope: ToolOperationScope,
    pub query: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillSearchCatalogEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub score: f64,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillSearchRemoteEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: String,
    pub source_label: String,
    pub source_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub trust_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillSearchLocalEntry {
    pub name: String,
    pub description: String,
    pub trust: String,
    pub source_tier: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillSearchResult {
    #[serde(default)]
    pub catalog: Vec<ToolSkillSearchCatalogEntry>,
    #[serde(default)]
    pub remote: Vec<ToolSkillSearchRemoteEntry>,
    #[serde(default)]
    pub local: Vec<ToolSkillSearchLocalEntry>,
    pub registry_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[async_trait]
pub trait SkillSearchToolHostPort: Send + Sync {
    async fn search_skills(
        &self,
        request: ToolSkillSearchRequest,
    ) -> Result<ToolSkillSearchResult, ToolHostError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillInstallActionRequest {
    pub scope: ToolOperationScope,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillUpdateActionRequest {
    pub scope: ToolOperationScope,
    pub name: String,
    #[serde(default)]
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillMutationActionResult {
    pub output: serde_json::Value,
}

#[async_trait]
pub trait SkillInstallToolHostPort: Send + Sync {
    async fn install_skill_action(
        &self,
        request: ToolSkillInstallActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError>;

    async fn update_skill_action(
        &self,
        request: ToolSkillUpdateActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillInstallRequest {
    pub scope: ToolOperationScope,
    pub name: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub approve_risky: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSkillCheckSource {
    InlineContent { content: String },
    Path { path: String },
    Url { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillCheckRequest {
    pub scope: ToolOperationScope,
    pub source: ToolSkillCheckSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillCheckResult {
    pub output: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillRemoveResult {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTrustMutationRequest {
    pub scope: ToolOperationScope,
    pub name: String,
    pub target_trust: ToolSkillTrust,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTrustMutationResult {
    pub name: String,
    pub trust: ToolSkillTrust,
    pub source_tier: String,
}

/// Host operations needed by generic skill tools.
#[async_trait]
pub trait SkillToolHostPort: Send + Sync {
    async fn list_skills(
        &self,
        query: ToolSkillQuery,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError>;

    async fn inspect_skill(
        &self,
        scope: ToolOperationScope,
        name: String,
        include_content: bool,
        include_files: bool,
        audit: bool,
    ) -> Result<serde_json::Value, ToolHostError>;

    async fn read_skill(
        &self,
        scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRead, ToolHostError>;

    async fn install_skill(
        &self,
        request: ToolSkillInstallRequest,
    ) -> Result<ToolSkillSummary, ToolHostError>;

    async fn check_skill(
        &self,
        request: ToolSkillCheckRequest,
    ) -> Result<ToolSkillCheckResult, ToolHostError>;

    async fn remove_skill(
        &self,
        scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRemoveResult, ToolHostError>;

    async fn promote_skill_trust(
        &self,
        request: ToolSkillTrustMutationRequest,
    ) -> Result<ToolSkillTrustMutationResult, ToolHostError>;

    async fn audit_skills(
        &self,
        scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<serde_json::Value>, ToolHostError>;

    async fn reload_skills(
        &self,
        scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError>;

    async fn snapshot_skills(
        &self,
        scope: ToolOperationScope,
    ) -> Result<ToolSkillSnapshotResult, ToolHostError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillPublishRequest {
    pub scope: ToolOperationScope,
    pub name: String,
    pub target_repo: String,
    #[serde(default = "default_true")]
    pub dry_run: bool,
    #[serde(default)]
    pub remote_write: bool,
    #[serde(default)]
    pub confirm_remote_write: bool,
    #[serde(default)]
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSkillPublishResult {
    pub status: String,
    pub name: String,
    pub target_repo: String,
    pub tap_path: String,
    pub package_path: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    pub package_hash: String,
    #[serde(default)]
    pub files: Vec<serde_json::Value>,
    #[serde(default)]
    pub findings: Vec<serde_json::Value>,
    pub trust: String,
    pub source_tier: String,
    #[serde(default)]
    pub source: serde_json::Value,
    #[serde(default)]
    pub remote_write_plan: serde_json::Value,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait SkillPublishToolHostPort: Send + Sync {
    async fn publish_skill(
        &self,
        request: ToolSkillPublishRequest,
    ) -> Result<ToolSkillPublishResult, ToolHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSkillTapTrust {
    Builtin,
    Trusted,
    #[default]
    Community,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTap {
    pub repo: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub trust_level: ToolSkillTapTrust,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapQuery {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub include_health: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapList {
    pub taps: Vec<ToolSkillTap>,
    pub count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_enabled: Option<bool>,
}

impl ToolSkillTapList {
    pub fn new(taps: Vec<ToolSkillTap>, hub_enabled: Option<bool>) -> Self {
        Self {
            count: taps.len(),
            taps,
            hub_enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapAddRequest {
    pub scope: ToolOperationScope,
    pub repo: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub trust_level: ToolSkillTapTrust,
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapRemoveRequest {
    pub scope: ToolOperationScope,
    pub repo: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapRefreshRequest {
    pub scope: ToolOperationScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapMutationResult {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<ToolSkillTap>,
    pub tap_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSkillTapRefreshResult {
    pub status: String,
    pub tap_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub hub_enabled: bool,
}

#[async_trait]
pub trait SkillTapToolHostPort: Send + Sync {
    async fn list_skill_taps(
        &self,
        query: ToolSkillTapQuery,
    ) -> Result<ToolSkillTapList, ToolHostError>;

    async fn add_skill_tap(
        &self,
        request: ToolSkillTapAddRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError>;

    async fn remove_skill_tap(
        &self,
        request: ToolSkillTapRemoveRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError>;

    async fn refresh_skill_taps(
        &self,
        request: ToolSkillTapRefreshRequest,
    ) -> Result<ToolSkillTapRefreshResult, ToolHostError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningFeedbackRequest {
    pub scope: ToolOperationScope,
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningHistoryQuery {
    pub scope: ToolOperationScope,
    pub kind: String,
    #[serde(default = "default_learning_limit")]
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningProposalReview {
    pub scope: ToolOperationScope,
    pub proposal_id: Uuid,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningRecord {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningActionRequest {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolLearningActionResult {
    pub output: serde_json::Value,
}

/// Host operations needed by generic learning tools.
#[async_trait]
pub trait LearningToolHostPort: Send + Sync {
    async fn record_feedback(
        &self,
        request: ToolLearningFeedbackRequest,
    ) -> Result<ToolLearningRecord, ToolHostError>;

    async fn list_learning_history(
        &self,
        query: ToolLearningHistoryQuery,
    ) -> Result<Vec<ToolLearningRecord>, ToolHostError>;

    async fn review_learning_proposal(
        &self,
        review: ToolLearningProposalReview,
    ) -> Result<ToolLearningRecord, ToolHostError>;

    async fn prompt_manage_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn skill_manage_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn learning_status_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn learning_outcomes_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn learning_history_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn learning_feedback_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;

    async fn learning_proposal_review_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRoutineTriggerRequest {
    pub scope: ToolOperationScope,
    pub routine_id: Uuid,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRoutineSummary {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Host operations needed by generic routine tools.
#[async_trait]
pub trait RoutineToolHostPort: Send + Sync {
    async fn list_routines(
        &self,
        scope: ToolOperationScope,
    ) -> Result<Vec<ToolRoutineSummary>, ToolHostError>;

    async fn trigger_routine(
        &self,
        request: ToolRoutineTriggerRequest,
    ) -> Result<ToolRoutineSummary, ToolHostError>;

    async fn set_routine_enabled(
        &self,
        scope: ToolOperationScope,
        routine_id: Uuid,
        enabled: bool,
    ) -> Result<ToolRoutineSummary, ToolHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolComfySidecarState {
    #[default]
    Unknown,
    Starting,
    Ready,
    Busy,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolComfyStatus {
    pub state: ToolComfySidecarState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolComfyWorkflowRequest {
    pub scope: ToolOperationScope,
    pub workflow: serde_json::Value,
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolComfyWorkflowResult {
    pub run_id: Uuid,
    pub status: String,
    #[serde(default)]
    pub outputs: Vec<serde_json::Value>,
    #[serde(default)]
    pub cost_usd: Option<Decimal>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolComfyActionRequest {
    pub scope: ToolOperationScope,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolComfyActionResult {
    pub output: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ToolArtifact>,
}

/// Host operations needed by generic ComfyUI tools.
#[async_trait]
pub trait ComfyUiToolHostPort: Send + Sync {
    async fn comfy_status(
        &self,
        scope: ToolOperationScope,
    ) -> Result<ToolComfyStatus, ToolHostError>;

    async fn run_comfy_workflow(
        &self,
        request: ToolComfyWorkflowRequest,
    ) -> Result<ToolComfyWorkflowResult, ToolHostError>;

    async fn image_generate_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError>;

    async fn comfy_health_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError>;

    async fn comfy_check_deps_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError>;

    async fn comfy_run_workflow_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError>;

    async fn comfy_manage_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError>;
}

fn default_true() -> bool {
    true
}

fn default_memory_limit() -> usize {
    10
}

fn default_learning_limit() -> i64 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_serializes_without_optional_noise() {
        let scope = ToolOperationScope::new("principal", "actor");
        let value = serde_json::to_value(&scope).expect("scope should serialize");

        assert_eq!(value["principal_id"], "principal");
        assert_eq!(value["actor_id"], "actor");
        assert!(value.get("job_id").is_none());
    }

    #[test]
    fn job_context_from_tool_scope_preserves_routing_identity_and_metadata() {
        let mut scope = ToolOperationScope::new("principal", "actor");
        scope.conversation_id = Some(Uuid::new_v4());
        scope.thread_id = Some("thread-1".to_string());
        scope.job_id = Some(Uuid::new_v4());
        scope.metadata = serde_json::json!({ "existing": true });

        let ctx = job_context_from_tool_scope(scope.clone(), "tool call");

        assert_eq!(ctx.user_id, "principal");
        assert_eq!(ctx.principal_id, "principal");
        assert_eq!(ctx.actor_id.as_deref(), Some("actor"));
        assert_eq!(ctx.title, "tool call");
        assert_eq!(ctx.description, "tool call");
        assert_eq!(ctx.conversation_id, scope.conversation_id);
        assert_eq!(ctx.job_id, scope.job_id.unwrap());
        assert_eq!(ctx.metadata["existing"], true);
        assert_eq!(ctx.metadata["thread_id"], "thread-1");
    }

    #[test]
    fn tool_scope_from_job_context_preserves_routing_identity_and_metadata() {
        let mut ctx = JobContext::with_identity("principal", "actor", "tool call", "tool call");
        ctx.conversation_id = Some(Uuid::new_v4());
        ctx.metadata = serde_json::json!({
            "thread_id": "thread-1",
            "existing": true
        });

        let scope = tool_scope_from_job_context(&ctx);

        assert_eq!(scope.principal_id, "principal");
        assert_eq!(scope.actor_id, "actor");
        assert_eq!(scope.conversation_id, ctx.conversation_id);
        assert_eq!(scope.job_id, Some(ctx.job_id));
        assert_eq!(scope.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(scope.metadata["existing"], true);
    }

    #[test]
    fn memory_write_defaults_to_append() {
        let request: ToolMemoryWriteRequest = serde_json::from_value(serde_json::json!({
            "scope": ToolOperationScope::new("u", "a"),
            "path": "MEMORY.md",
            "memory_scope": "shared",
            "content": "hello"
        }))
        .expect("request should deserialize");

        assert!(request.append);
        assert_eq!(request.memory_scope, ToolMemoryScope::Shared);
    }

    #[test]
    fn skill_publish_request_defaults_to_dry_run() {
        let request: ToolSkillPublishRequest = serde_json::from_value(serde_json::json!({
            "scope": ToolOperationScope::new("u", "a"),
            "name": "writer",
            "target_repo": "owner/skills"
        }))
        .expect("request should deserialize");

        assert!(request.dry_run);
        assert!(!request.remote_write);
        assert!(!request.confirm_remote_write);
        assert!(!request.approve_risky);
    }

    #[test]
    fn skill_tap_contracts_preserve_wire_shapes() {
        let tap = ToolSkillTap {
            repo: "owner/skills".to_string(),
            path: "packs/core".to_string(),
            branch: Some("main".to_string()),
            trust_level: ToolSkillTapTrust::Trusted,
        };
        let list = ToolSkillTapList::new(vec![tap.clone()], Some(true));
        let value = serde_json::to_value(list).expect("list should serialize");

        assert_eq!(
            value,
            serde_json::json!({
                "taps": [{
                    "repo": "owner/skills",
                    "path": "packs/core",
                    "branch": "main",
                    "trust_level": "trusted"
                }],
                "count": 1,
                "hub_enabled": true
            })
        );

        let add_request: ToolSkillTapAddRequest = serde_json::from_value(serde_json::json!({
            "scope": ToolOperationScope::new("u", "a"),
            "repo": "owner/skills"
        }))
        .expect("add request should deserialize");

        assert_eq!(add_request.path, "");
        assert_eq!(add_request.trust_level, ToolSkillTapTrust::Community);
        assert!(!add_request.replace);

        let result = ToolSkillTapRefreshResult {
            status: "refreshed".to_string(),
            tap_count: 1,
            repo: Some("owner/skills".to_string()),
            path: None,
            hub_enabled: true,
        };
        let value = serde_json::to_value(result).expect("refresh should serialize");
        assert_eq!(
            value,
            serde_json::json!({
                "status": "refreshed",
                "tap_count": 1,
                "repo": "owner/skills",
                "hub_enabled": true
            })
        );
    }

    #[test]
    fn host_traits_are_object_safe() {
        fn assert_object_safe<T: ?Sized + Send + Sync>() {}

        assert_object_safe::<dyn JobToolHostPort>();
        assert_object_safe::<dyn MemoryToolHostPort>();
        assert_object_safe::<dyn SkillToolHostPort>();
        assert_object_safe::<dyn SkillPublishToolHostPort>();
        assert_object_safe::<dyn SkillTapToolHostPort>();
        assert_object_safe::<dyn LearningToolHostPort>();
        assert_object_safe::<dyn RoutineToolHostPort>();
        assert_object_safe::<dyn ComfyUiToolHostPort>();
    }
}

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::repo_projects as repo_projects_api;
use crate::db::Database;
use crate::secrets::SecretsStore;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolMetadata, ToolOutput, ToolSideEffectLevel,
};

type SharedSecrets = Arc<dyn SecretsStore + Send + Sync>;

fn user_id(ctx: &thinclaw_types::JobContext) -> &str {
    if !ctx.principal_id.trim().is_empty() {
        &ctx.principal_id
    } else if !ctx.user_id.trim().is_empty() {
        &ctx.user_id
    } else {
        repo_projects_api::default_user_id()
    }
}

fn output<T: serde::Serialize>(
    started: Instant,
    result: crate::api::ApiResult<T>,
) -> Result<ToolOutput, ToolError> {
    let value = result.map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
    let json = serde_json::to_value(value)
        .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
    Ok(ToolOutput::success(json, started.elapsed()))
}

fn parse_project_id(params: &serde_json::Value) -> Result<Uuid, ToolError> {
    let id = params
        .get("project_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidParameters("project_id is required".to_string()))?;
    Uuid::parse_str(id)
        .map_err(|_| ToolError::InvalidParameters("project_id must be a UUID".to_string()))
}

#[derive(Debug, Deserialize)]
struct CreateParams {
    name: String,
    repo_url: String,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApproveParams {
    project_id: String,
    approval_id: String,
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

pub struct RepoProjectCreateTool {
    store: Arc<dyn Database>,
}

impl RepoProjectCreateTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectCreateTool {
    fn name(&self) -> &str {
        "repo_project_create"
    }

    fn description(&self) -> &str {
        "Create a durable GitHub repository project and enroll its first repository."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "repo_url": { "type": "string", "description": "GitHub repository as owner/repo or github.com/owner/repo" },
                "default_branch": { "type": "string" },
                "local_path": { "type": "string" },
                "description": { "type": "string" }
            },
            "required": ["name", "repo_url"]
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: crate::tools::tool::ToolApprovalClass::Conditional,
            parallel_safe: false,
            ..ToolMetadata::default()
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let params: CreateParams = serde_json::from_value(params)
            .map_err(|error| ToolError::InvalidParameters(error.to_string()))?;
        let input = repo_projects_api::RepoProjectCreateInput {
            name: params.name,
            repo_url: params.repo_url,
            default_branch: params.default_branch,
            local_path: params.local_path,
            description: params.description,
        };
        output(
            started,
            repo_projects_api::create_project(&self.store, user_id(ctx), input).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

pub struct RepoProjectPlanTool {
    store: Arc<dyn Database>,
}

impl RepoProjectPlanTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectPlanTool {
    fn name(&self) -> &str {
        "repo_project_plan"
    }

    fn description(&self) -> &str {
        "Move a repository project into durable planning state for supervisor decomposition."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        project_id_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let project_id = parse_project_id(&params)?;
        output(
            started,
            repo_projects_api::plan_project(&self.store, user_id(ctx), project_id).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct RepoProjectStatusTool {
    store: Arc<dyn Database>,
}

impl RepoProjectStatusTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectStatusTool {
    fn name(&self) -> &str {
        "repo_project_status"
    }

    fn description(&self) -> &str {
        "Read repository project status, backlog, workers, PRs, and merge gates."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string", "description": "Optional project UUID. Omit to list all projects." }
            },
            "required": []
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::read_only()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        if params.get("project_id").is_some() {
            let project_id = parse_project_id(&params)?;
            output(
                started,
                repo_projects_api::get_project(&self.store, project_id).await,
            )
        } else {
            output(started, repo_projects_api::list_projects(&self.store).await)
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct RepoProjectPauseTool {
    store: Arc<dyn Database>,
}

impl RepoProjectPauseTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectPauseTool {
    fn name(&self) -> &str {
        "repo_project_pause"
    }

    fn description(&self) -> &str {
        "Pause a running repository project supervisor."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        project_id_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let project_id = parse_project_id(&params)?;
        output(
            started,
            repo_projects_api::pause_project(&self.store, user_id(ctx), project_id).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct RepoProjectResumeTool {
    store: Arc<dyn Database>,
}

impl RepoProjectResumeTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectResumeTool {
    fn name(&self) -> &str {
        "repo_project_resume"
    }

    fn description(&self) -> &str {
        "Resume a paused repository project supervisor."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        project_id_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let project_id = parse_project_id(&params)?;
        output(
            started,
            repo_projects_api::resume_project(&self.store, user_id(ctx), project_id).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct RepoProjectApproveTool {
    store: Arc<dyn Database>,
}

impl RepoProjectApproveTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectApproveTool {
    fn name(&self) -> &str {
        "repo_project_approve"
    }

    fn description(&self) -> &str {
        "Record a human approval or rejection for a repository project plan or blocker."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "approval_id": { "type": "string" },
                "decision": { "type": "string", "enum": ["approve", "reject"] },
                "note": { "type": "string" }
            },
            "required": ["project_id", "approval_id", "decision"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let params: ApproveParams = serde_json::from_value(params)
            .map_err(|error| ToolError::InvalidParameters(error.to_string()))?;
        let project_id = Uuid::parse_str(&params.project_id)
            .map_err(|_| ToolError::InvalidParameters("project_id must be a UUID".to_string()))?;
        let input = repo_projects_api::RepoApprovalInput {
            approval_id: params.approval_id,
            decision: params.decision,
            note: params.note,
        };
        output(
            started,
            repo_projects_api::approve_project(&self.store, user_id(ctx), project_id, input).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── Setup / enrollment / credentials ────────────────────────────────────

pub struct RepoProjectSetupTool {
    store: Arc<dyn Database>,
    secrets: Option<SharedSecrets>,
}

impl RepoProjectSetupTool {
    pub fn new(store: Arc<dyn Database>, secrets: Option<SharedSecrets>) -> Self {
        Self { store, secrets }
    }
}

#[async_trait]
impl Tool for RepoProjectSetupTool {
    fn name(&self) -> &str {
        "repo_project_setup"
    }

    fn description(&self) -> &str {
        "Enable and configure the repository project supervisor (feature flag, GitHub App / token \
         credential references, auto-merge and concurrency policy) and report setup readiness. \
         Call with no fields to just read current readiness. Secret VALUES are never set here — \
         store those with repo_project_set_credential and reference them by name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "enabled": { "type": "boolean", "description": "Turn the supervisor on/off." },
                "app_id": { "type": "integer", "description": "GitHub App id." },
                "installation_id": { "type": "integer", "description": "GitHub App installation id." },
                "private_key_secret": { "type": "string", "description": "Name of the secret holding the GitHub App PEM private key." },
                "webhook_secret_secret": { "type": "string", "description": "Name of the secret holding the GitHub webhook secret." },
                "default_coding_backend": { "type": "string", "enum": ["worker", "claude_code", "codex_code"] },
                "auto_merge_default": { "type": "boolean" },
                "max_concurrent_projects": { "type": "integer" },
                "max_concurrent_tasks_per_project": { "type": "integer" },
                "watchdog_interval_secs": { "type": "integer" },
                "workspace_base_dir": { "type": "string" }
            },
            "required": []
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: crate::tools::tool::ToolApprovalClass::Conditional,
            parallel_safe: false,
            ..ToolMetadata::default()
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let input: repo_projects_api::RepoProjectsConfigureInput =
            serde_json::from_value(params)
                .map_err(|error| ToolError::InvalidParameters(error.to_string()))?;
        output(
            started,
            repo_projects_api::configure_supervisor(
                &self.store,
                self.secrets.as_ref(),
                user_id(ctx),
                input,
            )
            .await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[derive(Debug, Deserialize)]
struct CredentialParams {
    name: String,
    value: String,
}

pub struct RepoProjectSetCredentialTool {
    secrets: SharedSecrets,
}

impl RepoProjectSetCredentialTool {
    pub fn new(secrets: SharedSecrets) -> Self {
        Self { secrets }
    }
}

#[async_trait]
impl Tool for RepoProjectSetCredentialTool {
    fn name(&self) -> &str {
        "repo_project_set_credential"
    }

    fn description(&self) -> &str {
        "Securely store a GitHub credential (e.g. a personal access token or the GitHub App PEM \
         private key) in the encrypted secrets store under a name. The value is encrypted at rest \
         and never written to settings, events, or logs. Reference it from repo_project_setup by \
         name (e.g. github_token, repo_projects_github_private_key)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Secret name to store under (e.g. github_token)." },
                "value": { "type": "string", "description": "The credential value (token or PEM key). Stored encrypted." }
            },
            "required": ["name", "value"]
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: crate::tools::tool::ToolApprovalClass::Conditional,
            parallel_safe: false,
            ..ToolMetadata::default()
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let params: CredentialParams = serde_json::from_value(params)
            .map_err(|error| ToolError::InvalidParameters(error.to_string()))?;
        output(
            started,
            repo_projects_api::store_repo_credential(
                &self.secrets,
                user_id(ctx),
                params.name,
                params.value,
            )
            .await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[derive(Debug, Deserialize)]
struct EnrollParams {
    project_id: String,
    repo_url: String,
    #[serde(default)]
    default_branch: Option<String>,
}

pub struct RepoProjectEnrollTool {
    store: Arc<dyn Database>,
}

impl RepoProjectEnrollTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RepoProjectEnrollTool {
    fn name(&self) -> &str {
        "repo_project_enroll"
    }

    fn description(&self) -> &str {
        "Enroll an additional GitHub repository into an existing repository project."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "repo_url": { "type": "string", "description": "owner/repo or github.com/owner/repo" },
                "default_branch": { "type": "string" }
            },
            "required": ["project_id", "repo_url"]
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: crate::tools::tool::ToolApprovalClass::Conditional,
            parallel_safe: false,
            ..ToolMetadata::default()
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &thinclaw_types::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let started = Instant::now();
        let params: EnrollParams = serde_json::from_value(params)
            .map_err(|error| ToolError::InvalidParameters(error.to_string()))?;
        let project_id = Uuid::parse_str(&params.project_id)
            .map_err(|_| ToolError::InvalidParameters("project_id must be a UUID".to_string()))?;
        let input = repo_projects_api::RepoEnrollInput {
            repo_url: params.repo_url,
            default_branch: params.default_branch,
        };
        output(
            started,
            repo_projects_api::enroll_repo(&self.store, user_id(ctx), project_id, input).await,
        )
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

impl std::fmt::Debug for RepoProjectSetupTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectSetupTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectSetCredentialTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectSetCredentialTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectEnrollTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectEnrollTool")
            .finish_non_exhaustive()
    }
}

fn project_id_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "project_id": { "type": "string" }
        },
        "required": ["project_id"]
    })
}

impl std::fmt::Debug for RepoProjectCreateTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectCreateTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectPlanTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectPlanTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectStatusTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectStatusTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectPauseTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectPauseTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectResumeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectResumeTool")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepoProjectApproveTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProjectApproveTool")
            .finish_non_exhaustive()
    }
}

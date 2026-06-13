use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::repo_projects as repo_projects_api;
use crate::db::Database;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolMetadata, ToolOutput, ToolSideEffectLevel,
};

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

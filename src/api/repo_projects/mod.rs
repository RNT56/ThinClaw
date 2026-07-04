//! Repository project supervisor API.
//!
//! This module is intentionally framework-free: desktop commands and gateway
//! handlers can both call it with an `Arc<dyn Database>`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{ApiError, ApiResult};
use crate::db::Database;
use crate::settings::Settings;
use thinclaw_repo_projects::{
    CodingBackend, GitHubAuthMode, MergeGateDecision, ProjectPolicy, RepoProject, RepoProjectEvent,
    RepoProjectEventKind, RepoProjectRepo, RepoProjectState, RepoProjectTask, RepoProjectTaskState,
    RepoWorkerRun, RepoWorkerRunState, RepoWriteMode, repo_local_path_fragment,
    validate_project_state_transition, validate_task_state_transition,
};

const LOCAL_USER_ID: &str = "local_user";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectSetupItem {
    pub key: String,
    pub label: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBacklogItem {
    pub id: String,
    pub title: String,
    pub priority: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoWorkerRunView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backlog_id: Option<String>,
    pub agent: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPullRequestView {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoCiCheckView {
    pub id: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMergeGateView {
    pub id: String,
    pub label: String,
    pub state: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectView {
    pub id: String,
    pub name: String,
    pub repo_url: String,
    pub default_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: String,
    pub active_runs: usize,
    pub queued_items: usize,
    pub open_prs: usize,
    pub merge_gate_state: String,
    pub github_app: String,
    pub docker_agents: String,
    pub credentials: String,
    pub concurrency_limit: u32,
    pub write_mode: String,
    pub auto_merge_policy: String,
    pub notifications: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup_checklist: Vec<RepoProjectSetupItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backlog: Vec<RepoBacklogItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worker_runs: Vec<RepoWorkerRunView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_requests: Vec<RepoPullRequestView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ci_checks: Vec<RepoCiCheckView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_gates: Vec<RepoMergeGateView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectsListResponse {
    pub projects: Vec<RepoProjectView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectResponse {
    pub project: Option<RepoProjectView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectCreateInput {
    pub name: String,
    pub repo_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub write_mode: Option<RepoWriteMode>,
    #[serde(default)]
    pub fork_owner: Option<String>,
    #[serde(default)]
    pub fork_repo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBacklogEnqueueInput {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoApprovalInput {
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectCommandResponse {
    pub ok: bool,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<RepoProjectView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<RepoWorkerRunView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectEventView {
    pub id: String,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectEventsResponse {
    pub events: Vec<RepoProjectEventView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectMergeGatesResponse {
    pub gates: Vec<RepoMergeGateView>,
}

struct RepoProjectParts {
    repos: Vec<RepoProjectRepo>,
    tasks: Vec<RepoProjectTask>,
    worker_runs: Vec<RepoWorkerRun>,
    merge_gates: Vec<(Uuid, MergeGateDecision)>,
}

pub async fn list_projects(store: &Arc<dyn Database>) -> ApiResult<RepoProjectsListResponse> {
    let projects = store
        .list_repo_projects()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let mut views = Vec::with_capacity(projects.len());
    for project in projects {
        let parts = load_project_parts(store, project.id).await?;
        views.push(project_view(&project, &parts));
    }

    Ok(RepoProjectsListResponse { projects: views })
}

pub async fn get_project(
    store: &Arc<dyn Database>,
    project_id: Uuid,
) -> ApiResult<RepoProjectResponse> {
    let Some(project) = store
        .get_repo_project(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
    else {
        return Ok(RepoProjectResponse { project: None });
    };
    let parts = load_project_parts(store, project.id).await?;
    Ok(RepoProjectResponse {
        project: Some(project_view(&project, &parts)),
    })
}

pub async fn create_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    input: RepoProjectCreateInput,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;

    let name = non_empty(input.name, "name")?;
    let repo_url = non_empty(input.repo_url, "repo_url")?;
    let (owner, repo_name) = parse_github_repo_url(&repo_url)?;
    let default_branch = input
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("main")
        .to_string();
    let mut policy = default_policy_from_settings(store, user_id).await;
    if let Some(write_mode) = input.write_mode {
        policy.write_mode = write_mode;
    }
    let installation_id =
        default_installation_id_from_settings(store, user_id, policy.github_auth_mode).await;
    let now = Utc::now();
    let project_id = Uuid::new_v4();
    let repo_id = Uuid::new_v4();
    let local_path = match input.local_path {
        Some(path) if !path.trim().is_empty() => Some(path.trim().to_string()),
        _ => Some(
            repo_local_path_fragment(&owner, &repo_name)
                .map_err(ApiError::InvalidInput)?
                .to_string_lossy()
                .to_string(),
        ),
    };
    let project = RepoProject {
        id: project_id,
        slug: unique_project_slug(&name, project_id),
        name,
        state: RepoProjectState::Draft,
        policy,
        description: input
            .description
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        current_run_id: None,
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
    };
    let repo = RepoProjectRepo {
        id: repo_id,
        project_id,
        owner,
        repo: repo_name,
        github_repo_id: None,
        installation_id,
        default_branch: default_branch.clone(),
        base_branch: Some(default_branch),
        enrolled: true,
        local_path,
        auth_mode: project.policy.github_auth_mode,
        metadata: repo_metadata(
            &repo_url,
            project.policy.write_mode,
            input.fork_owner.as_deref(),
            input.fork_repo.as_deref(),
        )?,
        created_at: now,
        updated_at: now,
    };

    store
        .create_repo_project(&project)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    store
        .upsert_repo_project_repo(&repo)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    append_project_event(
        store,
        project.id,
        None,
        None,
        RepoProjectEventKind::ProjectCreated,
        "Repository project created",
        serde_json::json!({ "name": project.name, "slug": project.slug }),
    )
    .await?;
    append_project_event(
        store,
        project.id,
        Some(repo.id),
        None,
        RepoProjectEventKind::RepoEnrolled,
        "Repository enrolled",
        serde_json::json!({ "owner": repo.owner, "repo": repo.repo }),
    )
    .await?;

    let parts = load_project_parts(store, project.id).await?;
    Ok(RepoProjectCommandResponse {
        ok: true,
        message: Some("Repository project created".to_string()),
        project: Some(project_view(&project, &parts)),
        run: None,
    })
}

pub async fn start_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    transition_project(
        store,
        project_id,
        RepoProjectState::Active,
        "Repository project started",
        |project, now| {
            if project.started_at.is_none() {
                project.started_at = Some(now);
            }
        },
    )
    .await
}

pub async fn plan_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    transition_project(
        store,
        project_id,
        RepoProjectState::Planning,
        "Repository project planning requested",
        |_project, _now| {},
    )
    .await
}

pub async fn pause_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    transition_project(
        store,
        project_id,
        RepoProjectState::Paused,
        "Repository project paused",
        |_project, _now| {},
    )
    .await
}

pub async fn resume_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    transition_project(
        store,
        project_id,
        RepoProjectState::Active,
        "Repository project resumed",
        |_project, _now| {},
    )
    .await
}

pub async fn cancel_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    transition_project(
        store,
        project_id,
        RepoProjectState::Cancelled,
        "Repository project cancelled",
        |project, now| project.completed_at = Some(now),
    )
    .await
}

pub async fn approve_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
    input: RepoApprovalInput,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    let project = project_required(store, project_id).await?;
    let approved = input.decision.eq_ignore_ascii_case("approve");
    append_project_event(
        store,
        project.id,
        None,
        None,
        if approved {
            RepoProjectEventKind::ProjectStateChanged
        } else {
            RepoProjectEventKind::MergeDenied
        },
        if approved {
            "Project approval recorded"
        } else {
            "Project approval rejected"
        },
        serde_json::json!({
            "approval_id": input.approval_id,
            "decision": input.decision,
            "note": input.note,
        }),
    )
    .await?;

    let response = if approved
        && matches!(
            project.state,
            RepoProjectState::Draft | RepoProjectState::Planning | RepoProjectState::AwaitingHuman
        ) {
        transition_project(
            store,
            project_id,
            RepoProjectState::Active,
            "Repository project approved and activated",
            |project, now| {
                if project.started_at.is_none() {
                    project.started_at = Some(now);
                }
            },
        )
        .await?
    } else {
        let parts = load_project_parts(store, project.id).await?;
        RepoProjectCommandResponse {
            ok: approved,
            message: Some(if approved {
                "Project approval recorded".to_string()
            } else {
                "Project approval rejected".to_string()
            }),
            project: Some(project_view(&project, &parts)),
            run: None,
        }
    };

    Ok(response)
}

pub async fn enqueue_task(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
    input: RepoBacklogEnqueueInput,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    let project = project_required(store, project_id).await?;
    if matches!(
        project.state,
        RepoProjectState::Completed | RepoProjectState::Failed | RepoProjectState::Cancelled
    ) {
        return Err(ApiError::InvalidInput(format!(
            "Cannot enqueue tasks for project in state '{}'",
            state_label(project.state)
        )));
    }

    let repos = store
        .list_repo_project_repos(project.id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let repo = repos
        .iter()
        .find(|repo| repo.enrolled)
        .or_else(|| repos.first())
        .ok_or_else(|| {
            ApiError::InvalidInput("Project has no enrolled repositories".to_string())
        })?;

    let task = crate::repo_projects::build_queued_task(
        &project,
        repo,
        non_empty(input.title, "title")?,
        input
            .description
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        priority_value(input.priority.as_deref()),
        input.labels,
    )
    .map_err(ApiError::InvalidInput)?;
    validate_task_state_transition(RepoProjectTaskState::Queued, task.state)
        .map_err(|error| ApiError::InvalidInput(error.to_string()))?;
    store
        .upsert_repo_project_task(&task)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    append_project_event(
        store,
        project.id,
        Some(repo.id),
        Some(task.id),
        RepoProjectEventKind::TaskCreated,
        "Repository project task queued",
        serde_json::json!({
            "title": task.title,
            "branch_name": task.branch_name,
            "coding_backend": enum_label(task.coding_backend),
        }),
    )
    .await?;

    let parts = load_project_parts(store, project.id).await?;
    Ok(RepoProjectCommandResponse {
        ok: true,
        message: Some("Task queued".to_string()),
        project: Some(project_view(&project, &parts)),
        run: None,
    })
}

pub async fn list_events(
    store: &Arc<dyn Database>,
    project_id: Uuid,
    limit: i64,
) -> ApiResult<RepoProjectEventsResponse> {
    let events = store
        .list_repo_project_events(project_id, limit)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(RepoProjectEventsResponse {
        events: events.into_iter().map(event_view).collect(),
    })
}

pub async fn list_merge_gates(
    store: &Arc<dyn Database>,
    project_id: Uuid,
) -> ApiResult<RepoProjectMergeGatesResponse> {
    let Some(project) = store
        .get_repo_project(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
    else {
        return Err(ApiError::SessionNotFound(format!(
            "Repository project {project_id} was not found"
        )));
    };
    let parts = load_project_parts(store, project.id).await?;
    Ok(RepoProjectMergeGatesResponse {
        gates: merge_gate_views(&project, &parts),
    })
}

// ── Supervisor setup / configuration ────────────────────────────────────

type SharedSecrets = Arc<dyn crate::secrets::SecretsStore + Send + Sync>;

/// Fields an operator/agent can set to configure the supervisor. All optional;
/// only provided fields are written. Secret *values* are never stored here —
/// only the *names* of secrets held in the encrypted secrets store.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoProjectsConfigureInput {
    pub enabled: Option<bool>,
    pub app_id: Option<u64>,
    pub installation_id: Option<u64>,
    pub private_key_secret: Option<String>,
    pub webhook_secret_secret: Option<String>,
    pub app_slug: Option<String>,
    pub default_coding_backend: Option<String>,
    pub default_write_mode: Option<String>,
    pub auto_merge_default: Option<bool>,
    pub max_concurrent_projects: Option<usize>,
    pub max_concurrent_tasks_per_project: Option<usize>,
    pub watchdog_interval_secs: Option<u64>,
    pub workspace_base_dir: Option<String>,
}

/// A snapshot of how ready the supervisor is for live runs.
#[derive(Debug, Clone, Serialize)]
pub struct RepoProjectsReadiness {
    pub enabled: bool,
    pub credential_mode: String,
    pub app_id: Option<u64>,
    pub installation_id: Option<u64>,
    pub private_key_secret: Option<String>,
    pub webhook_secret_secret: Option<String>,
    pub app_slug: Option<String>,
    /// GitHub App install URL for the connector "Connect" action, when an
    /// `app_slug` is configured. Sending the user here lets them install the
    /// App and pick all or specific repos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_url: Option<String>,
    pub auto_merge_default: bool,
    pub default_coding_backend: String,
    pub default_write_mode: String,
    pub max_concurrent_projects: usize,
    pub max_concurrent_tasks_per_project: usize,
    pub watchdog_interval_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token_secret_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_fork_token_secret_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key_secret_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret_present: Option<bool>,
    pub ready_for_live_runs: bool,
    pub checklist: Vec<RepoProjectSetupItem>,
}

async fn set_repo_setting(
    store: &Arc<dyn Database>,
    user_id: &str,
    key: &str,
    value: serde_json::Value,
) -> ApiResult<()> {
    store
        .set_setting(user_id, key, &value)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))
}

/// Write the provided supervisor settings (feature flag, GitHub App config,
/// policy defaults) and return the resulting readiness. Does not touch secret
/// values — store those with [`store_repo_credential`].
pub async fn configure_supervisor(
    store: &Arc<dyn Database>,
    secrets: Option<&SharedSecrets>,
    user_id: &str,
    input: RepoProjectsConfigureInput,
) -> ApiResult<RepoProjectsReadiness> {
    if let Some(value) = input.enabled {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.enabled",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.app_id {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.github_app.app_id",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.installation_id {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.github_app.installation_id",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.private_key_secret {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.github_app.private_key_secret",
            serde_json::json!(value.trim()),
        )
        .await?;
    }
    if let Some(value) = input.webhook_secret_secret {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.github_app.webhook_secret_secret",
            serde_json::json!(value.trim()),
        )
        .await?;
    }
    if let Some(value) = input.app_slug {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.github_app.app_slug",
            serde_json::json!(value.trim()),
        )
        .await?;
    }
    if let Some(value) = input.default_coding_backend {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.default_coding_backend",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.default_write_mode {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.default_write_mode",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.auto_merge_default {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.auto_merge_default",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.max_concurrent_projects {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.max_concurrent_projects",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.max_concurrent_tasks_per_project {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.max_concurrent_tasks_per_project",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.watchdog_interval_secs {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.watchdog_interval_secs",
            serde_json::json!(value),
        )
        .await?;
    }
    if let Some(value) = input.workspace_base_dir {
        set_repo_setting(
            store,
            user_id,
            "repo_projects.workspace_base_dir",
            serde_json::json!(value.trim()),
        )
        .await?;
    }
    repo_projects_readiness(store, secrets, user_id).await
}

/// Report the supervisor's configuration + (when a secrets store is provided)
/// whether the referenced credentials actually exist.
pub async fn repo_projects_readiness(
    store: &Arc<dyn Database>,
    secrets: Option<&SharedSecrets>,
    user_id: &str,
) -> ApiResult<RepoProjectsReadiness> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let rp = Settings::from_db_map(&map).repo_projects;
    let app = rp.github_app.clone();

    let secret_present = |name: Option<String>| async {
        match (secrets, name) {
            (Some(store), Some(name)) if !name.trim().is_empty() => {
                Some(store.exists(user_id, name.trim()).await.unwrap_or(false))
            }
            _ => None,
        }
    };
    let private_key_secret_present = secret_present(app.private_key_secret.clone()).await;
    let webhook_secret_present = secret_present(app.webhook_secret_secret.clone()).await;
    let github_token_secret_present = match secrets {
        Some(store) => Some(store.exists(user_id, "github_token").await.unwrap_or(false)),
        None => None,
    };
    let github_fork_token_secret_present = match secrets {
        Some(store) => Some(
            store
                .exists(user_id, "github_fork_token")
                .await
                .unwrap_or(false),
        ),
        None => None,
    };

    let app_ready = app.app_id.is_some()
        && app.private_key_secret.is_some()
        && private_key_secret_present != Some(false);
    let default_write_mode = write_mode_from_setting(&rp.default_write_mode);
    let credential_mode = if app_ready {
        "github_app"
    } else if github_token_secret_present == Some(true) {
        "github_token"
    } else {
        "none"
    };
    let supervisor_credentials_ready = app_ready || github_token_secret_present == Some(true);
    let worker_write_credentials_ready = match default_write_mode {
        RepoWriteMode::ReadOnlyClone => true,
        RepoWriteMode::ForkPr => github_fork_token_secret_present == Some(true),
        RepoWriteMode::MaintainerBranchPr | RepoWriteMode::MaintainerAutoMerge => {
            github_token_secret_present == Some(true)
        }
    };
    let ready_for_live_runs =
        rp.enabled && supervisor_credentials_ready && worker_write_credentials_ready;

    let item = |key: &str, label: &str, state: &str, detail: Option<String>| RepoProjectSetupItem {
        key: key.to_string(),
        label: label.to_string(),
        state: state.to_string(),
        detail,
    };
    let checklist = vec![
        item(
            "feature_flag",
            "Supervisor enabled",
            if rp.enabled { "complete" } else { "pending" },
            (!rp.enabled).then(|| "Set repo_projects.enabled to true.".to_string()),
        ),
        item(
            "credentials",
            "GitHub credentials",
            if supervisor_credentials_ready && worker_write_credentials_ready {
                "complete"
            } else {
                "pending"
            },
            Some(credential_detail(
                credential_mode,
                default_write_mode,
                supervisor_credentials_ready,
                github_token_secret_present,
                github_fork_token_secret_present,
            )),
        ),
        item(
            "webhook",
            "Webhook secret",
            match (app.webhook_secret_secret.is_some(), webhook_secret_present) {
                (true, Some(false)) => "pending",
                (true, _) => "complete",
                (false, _) => "optional",
            },
            app.webhook_secret_secret.clone(),
        ),
        item(
            "coding_backend",
            "Coding backend",
            "complete",
            Some(rp.default_coding_backend.clone()),
        ),
        item(
            "write_mode",
            "Write mode default",
            "complete",
            Some(rp.default_write_mode.clone()),
        ),
        item(
            "auto_merge",
            "Auto-merge default",
            if rp.auto_merge_default {
                "enabled"
            } else {
                "disabled"
            },
            None,
        ),
    ];

    Ok(RepoProjectsReadiness {
        enabled: rp.enabled,
        credential_mode: credential_mode.to_string(),
        app_id: app.app_id,
        installation_id: app.installation_id,
        private_key_secret: app.private_key_secret,
        webhook_secret_secret: app.webhook_secret_secret,
        install_url: github_app_install_url(app.app_slug.as_deref()),
        app_slug: app.app_slug,
        auto_merge_default: rp.auto_merge_default,
        default_coding_backend: rp.default_coding_backend,
        default_write_mode: rp.default_write_mode,
        max_concurrent_projects: rp.max_concurrent_projects,
        max_concurrent_tasks_per_project: rp.max_concurrent_tasks_per_project,
        watchdog_interval_secs: rp.watchdog_interval_secs,
        github_token_secret_present,
        github_fork_token_secret_present,
        private_key_secret_present,
        webhook_secret_present,
        ready_for_live_runs,
        checklist,
    })
}

/// Securely store a GitHub credential value into the encrypted secrets store
/// under `name`. The plaintext is never persisted in settings or events.
pub async fn store_repo_credential(
    secrets: &SharedSecrets,
    user_id: &str,
    name: String,
    value: String,
) -> ApiResult<RepoCredentialStored> {
    let name = non_empty(name, "name")?;
    let value = non_empty(value, "value")?;
    let params = crate::secrets::CreateSecretParams::new(name.clone(), value)
        .with_provider("github")
        .with_created_by("repo_project_setup");
    secrets
        .create(user_id, params)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(RepoCredentialStored { ok: true, name })
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoCredentialStored {
    pub ok: bool,
    pub name: String,
}

/// Request body for storing a GitHub credential. The `value` is encrypted into
/// the secrets store and never echoed back or written to settings/events.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoCredentialInput {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoEnrollInput {
    pub repo_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub fork_owner: Option<String>,
    #[serde(default)]
    pub fork_repo: Option<String>,
}

/// Enroll an additional GitHub repository into an existing project.
pub async fn enroll_repo(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
    input: RepoEnrollInput,
) -> ApiResult<RepoProjectCommandResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;
    let project = project_required(store, project_id).await?;
    let repo_url = non_empty(input.repo_url, "repo_url")?;
    let (owner, repo_name) = parse_github_repo_url(&repo_url)?;
    let default_branch = input
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("main")
        .to_string();
    let installation_id =
        default_installation_id_from_settings(store, user_id, project.policy.github_auth_mode)
            .await;
    let now = Utc::now();
    let repo = RepoProjectRepo {
        id: Uuid::new_v4(),
        project_id,
        owner: owner.clone(),
        repo: repo_name.clone(),
        github_repo_id: None,
        installation_id,
        default_branch: default_branch.clone(),
        base_branch: Some(default_branch),
        enrolled: true,
        local_path: Some(
            repo_local_path_fragment(&owner, &repo_name)
                .map_err(ApiError::InvalidInput)?
                .to_string_lossy()
                .to_string(),
        ),
        auth_mode: project.policy.github_auth_mode,
        metadata: repo_metadata(
            &repo_url,
            project.policy.write_mode,
            input.fork_owner.as_deref(),
            input.fork_repo.as_deref(),
        )?,
        created_at: now,
        updated_at: now,
    };
    store
        .upsert_repo_project_repo(&repo)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    append_project_event(
        store,
        project_id,
        Some(repo.id),
        None,
        RepoProjectEventKind::RepoEnrolled,
        "Repository enrolled",
        serde_json::json!({ "owner": repo.owner, "repo": repo.repo }),
    )
    .await?;
    let parts = load_project_parts(store, project_id).await?;
    Ok(RepoProjectCommandResponse {
        ok: true,
        message: Some(format!("Enrolled {}/{}", repo.owner, repo.repo)),
        project: Some(project_view(&project, &parts)),
        run: None,
    })
}

// ── GitHub connector: repo discovery + selection ─────────────────────────

/// Build the GitHub App install URL from a configured app slug. This is the
/// entry point of the connector "Connect" flow: the user installs the App and
/// grants access to all or specific repositories.
fn github_app_install_url(app_slug: Option<&str>) -> Option<String> {
    let slug = app_slug.map(str::trim).filter(|slug| !slug.is_empty())?;
    Some(format!("https://github.com/apps/{slug}/installations/new"))
}

/// A repository the connected GitHub credential can act on, annotated with
/// whether it is already enrolled in a ThinClaw project.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectableRepoPermissions {
    pub pull: bool,
    pub triage: bool,
    pub push: bool,
    pub maintain: bool,
    pub admin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectableRepo {
    pub owner: String,
    pub repo: String,
    pub full_name: String,
    pub private: bool,
    pub archived: bool,
    pub default_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_url: Option<String>,
    pub permissions: ConnectableRepoPermissions,
    pub recommended_write_mode: String,
    /// True when this repo is already under supervision.
    pub enrolled: bool,
    /// The project id this repo is enrolled in, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectableReposResponse {
    /// How the listing was authenticated: "github_app" or "github_token".
    pub source: String,
    /// Authenticated GitHub user login, when discovery used a user token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated_user: Option<String>,
    pub total: usize,
    pub repos: Vec<ConnectableRepo>,
}

/// List the repositories the configured GitHub credential can act on, so the
/// connector UI (or the agent) can present a repo picker. Builds an
/// authenticated client from settings + the secrets store; GitHub App auth
/// lists the installation's repos, a `github_token` lists the owner's repos.
pub async fn list_connectable_repos(
    store: &Arc<dyn Database>,
    secrets: &SharedSecrets,
    user_id: &str,
) -> ApiResult<ConnectableReposResponse> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let app = Settings::from_db_map(&map).repo_projects.github_app;
    let provider = crate::repo_projects::github_provider::SecretsRepoGitHubClientProvider::build(
        Arc::clone(secrets),
        user_id,
        "https://api.github.com",
        app.app_id,
        app.installation_id,
        app.private_key_secret.clone(),
        "github_token",
    )
    .await;
    list_connectable_repos_with_provider(store, &provider).await
}

const CONNECTOR_REPO_PAGE_SIZE: u32 = 100;
const CONNECTOR_REPO_MAX_PAGES: u32 = 20;

async fn list_connectable_repos_with_provider(
    store: &Arc<dyn Database>,
    provider: &dyn crate::repo_projects::github_provider::RepoGitHubClientProvider,
) -> ApiResult<ConnectableReposResponse> {
    use crate::repo_projects::github::{GitHubListQuery, GitHubUserReposQuery};

    let (client, mode) = provider
        .discovery_client()
        .await
        .map_err(ApiError::Internal)?;
    let authenticated_user = match mode {
        GitHubAuthMode::UserToken | GitHubAuthMode::GhCli => client
            .get_authenticated_user()
            .await
            .ok()
            .map(|user| user.login),
        GitHubAuthMode::GitHubApp => None,
    };

    let mut raw = Vec::new();
    let mut page = 1u32;
    loop {
        let batch = match mode {
            GitHubAuthMode::GitHubApp => {
                client
                    .list_installation_repositories(&GitHubListQuery {
                        page: Some(page),
                        per_page: Some(CONNECTOR_REPO_PAGE_SIZE),
                    })
                    .await
                    .map_err(|error| ApiError::Internal(error.to_string()))?
                    .repositories
            }
            _ => client
                .list_user_repositories(&GitHubUserReposQuery {
                    affiliation: Some("owner,collaborator,organization_member".to_string()),
                    sort: Some("updated".to_string()),
                    direction: Some("desc".to_string()),
                    page: Some(page),
                    per_page: Some(CONNECTOR_REPO_PAGE_SIZE),
                })
                .await
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        };
        let received = batch.len() as u32;
        raw.extend(batch);
        if received < CONNECTOR_REPO_PAGE_SIZE || page >= CONNECTOR_REPO_MAX_PAGES {
            break;
        }
        page += 1;
    }

    let enrolled = enrolled_repo_index(store).await?;
    let mut repos = Vec::with_capacity(raw.len());
    for repo in raw {
        let permissions = connectable_permissions(repo.permissions.as_ref());
        let recommended_write_mode =
            recommended_write_mode_for_repo(&repo, mode, authenticated_user.as_deref());
        let owner = repo
            .owner
            .as_ref()
            .map(|owner| owner.login.clone())
            .or_else(|| repo.full_name.split('/').next().map(ToString::to_string))
            .unwrap_or_default();
        let name = if repo.name.is_empty() {
            repo.full_name
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string()
        } else {
            repo.name.clone()
        };
        let project_id = enrolled
            .get(&(owner.to_ascii_lowercase(), name.to_ascii_lowercase()))
            .cloned();
        repos.push(ConnectableRepo {
            owner,
            repo: name,
            full_name: repo.full_name,
            private: repo.private,
            archived: repo.archived,
            default_branch: repo.default_branch.unwrap_or_else(|| "main".to_string()),
            html_url: repo.html_url,
            permissions,
            recommended_write_mode: write_mode_label(recommended_write_mode),
            enrolled: project_id.is_some(),
            project_id,
        });
    }

    Ok(ConnectableReposResponse {
        source: connector_source_label(mode),
        authenticated_user,
        total: repos.len(),
        repos,
    })
}

fn connector_source_label(mode: GitHubAuthMode) -> String {
    match mode {
        GitHubAuthMode::GitHubApp => "github_app",
        GitHubAuthMode::UserToken => "github_token",
        GitHubAuthMode::GhCli => "gh_cli",
    }
    .to_string()
}

fn connectable_permissions(
    permissions: Option<&crate::repo_projects::github::GitHubRepositoryPermissions>,
) -> ConnectableRepoPermissions {
    let permissions = permissions.cloned().unwrap_or_default();
    ConnectableRepoPermissions {
        pull: permissions.pull,
        triage: permissions.triage,
        push: permissions.push,
        maintain: permissions.maintain,
        admin: permissions.admin,
    }
}

fn recommended_write_mode_for_repo(
    repo: &crate::repo_projects::github::GitHubRepository,
    mode: GitHubAuthMode,
    authenticated_user: Option<&str>,
) -> RepoWriteMode {
    use crate::repo_projects::github::GitHubRepoPermission;

    if repo.archived || repo.disabled {
        return RepoWriteMode::ReadOnlyClone;
    }
    if !repo.private {
        return if matches!(mode, GitHubAuthMode::UserToken | GitHubAuthMode::GhCli)
            && authenticated_user.is_some()
        {
            RepoWriteMode::ForkPr
        } else {
            RepoWriteMode::ReadOnlyClone
        };
    }
    if repo.has_permission(GitHubRepoPermission::Push) {
        RepoWriteMode::MaintainerBranchPr
    } else {
        RepoWriteMode::ReadOnlyClone
    }
}

/// Map of `(owner_lc, repo_lc) -> project_id` for every repo already enrolled
/// in some project, so the connector can mark which repos are already taken.
async fn enrolled_repo_index(
    store: &Arc<dyn Database>,
) -> ApiResult<std::collections::HashMap<(String, String), String>> {
    let mut index = std::collections::HashMap::new();
    let projects = store
        .list_repo_projects()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    for project in projects {
        let repos = store
            .list_repo_project_repos(project.id)
            .await
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        for repo in repos.into_iter().filter(|repo| repo.enrolled) {
            index.insert(
                (
                    repo.owner.to_ascii_lowercase(),
                    repo.repo.to_ascii_lowercase(),
                ),
                project.id.to_string(),
            );
        }
    }
    Ok(index)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoConnectInput {
    /// `owner/repo` identifiers to bring under supervision.
    #[serde(default)]
    pub repos: Vec<String>,
    /// When true, connect every repository the credential can access.
    #[serde(default)]
    pub all: bool,
    /// Optional override. Without this, each discovered repo uses its safe
    /// recommended write mode and undiscovered explicit repos use the configured
    /// supervisor default.
    #[serde(default)]
    pub write_mode: Option<RepoWriteMode>,
    #[serde(default)]
    pub fork_owner: Option<String>,
    #[serde(default)]
    pub fork_repo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoConnectResponse {
    pub ok: bool,
    pub connected: Vec<String>,
    pub skipped: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct RepoConnectSelection {
    owner: String,
    repo: String,
    default_branch: Option<String>,
    write_mode: Option<RepoWriteMode>,
}

/// Bring the selected repositories under supervision by creating a draft
/// project for each repo that is not already enrolled. With `all = true`,
/// every (non-archived) connectable repo is selected. This is the "select all
/// or specific repos" step of the connector flow; callers then `start_project`
/// to engage.
pub async fn connect_repos(
    store: &Arc<dyn Database>,
    secrets: &SharedSecrets,
    user_id: &str,
    input: RepoConnectInput,
) -> ApiResult<RepoConnectResponse> {
    ensure_repo_projects_enabled(store, user_id).await?;

    let listing = if input.all {
        Some(list_connectable_repos(store, secrets, user_id).await?)
    } else {
        list_connectable_repos(store, secrets, user_id).await.ok()
    };
    let discovered = listing
        .as_ref()
        .map(|listing| {
            listing
                .repos
                .iter()
                .map(|repo| {
                    (
                        (
                            repo.owner.to_ascii_lowercase(),
                            repo.repo.to_ascii_lowercase(),
                        ),
                        repo.clone(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut wanted: Vec<RepoConnectSelection> = Vec::new();
    if input.all
        && let Some(listing) = listing.as_ref()
    {
        for repo in listing
            .repos
            .iter()
            .filter(|repo| !repo.archived && !repo.enrolled)
        {
            wanted.push(RepoConnectSelection {
                owner: repo.owner.clone(),
                repo: repo.repo.clone(),
                default_branch: Some(repo.default_branch.clone()),
                write_mode: Some(write_mode_from_setting(&repo.recommended_write_mode)),
            });
        }
    }
    for raw in &input.repos {
        let (owner, repo) = parse_github_repo_url(raw)?;
        let discovered = discovered.get(&(owner.to_ascii_lowercase(), repo.to_ascii_lowercase()));
        wanted.push(RepoConnectSelection {
            owner,
            repo,
            default_branch: discovered.map(|repo| repo.default_branch.clone()),
            write_mode: discovered
                .map(|repo| write_mode_from_setting(&repo.recommended_write_mode)),
        });
    }
    if wanted.is_empty() {
        return Err(ApiError::InvalidInput(
            "Provide one or more repos, or set all=true to connect every accessible repository."
                .to_string(),
        ));
    }

    let enrolled = enrolled_repo_index(store).await?;
    let mut connected = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for selection in wanted {
        let key = (
            selection.owner.to_ascii_lowercase(),
            selection.repo.to_ascii_lowercase(),
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let full = format!("{}/{}", selection.owner, selection.repo);
        if enrolled.contains_key(&key) {
            skipped.push(full);
            continue;
        }
        let write_mode = input.write_mode.or(selection.write_mode);
        let fork_owner = input.fork_owner.clone().or_else(|| {
            write_mode
                .filter(|mode| *mode == RepoWriteMode::ForkPr)
                .and_then(|_| {
                    listing
                        .as_ref()
                        .and_then(|listing| listing.authenticated_user.clone())
                })
        });
        let fork_repo = input
            .fork_repo
            .clone()
            .or_else(|| fork_owner.as_ref().map(|_| selection.repo.clone()));
        create_project(
            store,
            user_id,
            RepoProjectCreateInput {
                name: selection.repo.clone(),
                repo_url: full.clone(),
                default_branch: selection.default_branch,
                local_path: None,
                description: None,
                write_mode,
                fork_owner,
                fork_repo,
            },
        )
        .await?;
        connected.push(full);
    }

    let message = format!(
        "Connected {} repo(s){}",
        connected.len(),
        if skipped.is_empty() {
            String::new()
        } else {
            format!(", skipped {} already enrolled", skipped.len())
        }
    );
    Ok(RepoConnectResponse {
        ok: true,
        connected,
        skipped,
        message,
    })
}

async fn transition_project<F>(
    store: &Arc<dyn Database>,
    project_id: Uuid,
    next_state: RepoProjectState,
    message: &str,
    apply: F,
) -> ApiResult<RepoProjectCommandResponse>
where
    F: FnOnce(&mut RepoProject, DateTime<Utc>),
{
    let mut project = project_required(store, project_id).await?;
    let previous_state = project.state;
    validate_project_state_transition(previous_state, next_state)
        .map_err(|error| ApiError::InvalidInput(error.to_string()))?;

    let now = Utc::now();
    project.state = next_state;
    project.updated_at = now;
    apply(&mut project, now);
    store
        .update_repo_project(&project)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    append_project_event(
        store,
        project.id,
        None,
        None,
        RepoProjectEventKind::ProjectStateChanged,
        message,
        serde_json::json!({
            "from": state_label(previous_state),
            "to": state_label(next_state),
        }),
    )
    .await?;

    let parts = load_project_parts(store, project.id).await?;
    Ok(RepoProjectCommandResponse {
        ok: true,
        message: Some(message.to_string()),
        project: Some(project_view(&project, &parts)),
        run: None,
    })
}

async fn project_required(store: &Arc<dyn Database>, project_id: Uuid) -> ApiResult<RepoProject> {
    store
        .get_repo_project(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!("Repository project {project_id} was not found"))
        })
}

async fn load_project_parts(
    store: &Arc<dyn Database>,
    project_id: Uuid,
) -> ApiResult<RepoProjectParts> {
    let repos = store
        .list_repo_project_repos(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let tasks = store
        .list_repo_project_tasks(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let worker_runs = store
        .list_repo_worker_runs(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let merge_gates = store
        .list_repo_merge_gate_decisions(project_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(RepoProjectParts {
        repos,
        tasks,
        worker_runs,
        merge_gates,
    })
}

async fn append_project_event(
    store: &Arc<dyn Database>,
    project_id: Uuid,
    repo_id: Option<Uuid>,
    task_id: Option<Uuid>,
    kind: RepoProjectEventKind,
    message: &str,
    details: serde_json::Value,
) -> ApiResult<()> {
    let event = RepoProjectEvent {
        id: Uuid::new_v4(),
        project_id,
        repo_id,
        task_id,
        project_run_id: None,
        worker_run_id: None,
        kind,
        message: message.to_string(),
        details,
        created_at: Utc::now(),
    };
    store
        .append_repo_project_event(&event)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))
}

async fn ensure_repo_projects_enabled(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<Settings> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let settings = Settings::from_db_map(&map);
    if !settings.repo_projects.enabled {
        return Err(ApiError::FeatureDisabled(
            "Repository projects are disabled. Set repo_projects.enabled=true to enable the supervisor.".to_string(),
        ));
    }
    Ok(settings)
}

async fn default_policy_from_settings(store: &Arc<dyn Database>, user_id: &str) -> ProjectPolicy {
    let Ok(map) = store.get_all_settings(user_id).await else {
        return ProjectPolicy::default();
    };
    let settings = Settings::from_db_map(&map);
    ProjectPolicy {
        auto_merge: settings.repo_projects.auto_merge_default,
        write_mode: write_mode_from_setting(&settings.repo_projects.default_write_mode),
        default_coding_backend: coding_backend_from_setting(
            &settings.repo_projects.default_coding_backend,
        ),
        github_auth_mode: if settings.repo_projects.github_app.app_id.is_some() {
            GitHubAuthMode::GitHubApp
        } else {
            GitHubAuthMode::UserToken
        },
        max_parallel_tasks: settings
            .repo_projects
            .max_concurrent_tasks_per_project
            .max(1) as u32,
        ..ProjectPolicy::default()
    }
}

/// Resolve the configured GitHub App installation id (as the persisted `i64`)
/// when the project will authenticate as a GitHub App. Returns `None` for the
/// personal-access-token path or when no installation id is configured; in
/// those cases the repo's `installation_id` stays `None` and the client
/// provider falls back to the global default / token auth. A later webhook
/// delivery (`find_project_id_for_repo`) backfills the precise per-repo
/// installation id when one was not discoverable at enroll time.
async fn default_installation_id_from_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
    auth_mode: GitHubAuthMode,
) -> Option<i64> {
    if !matches!(auth_mode, GitHubAuthMode::GitHubApp) {
        return None;
    }
    let map = store.get_all_settings(user_id).await.ok()?;
    let installation_id = Settings::from_db_map(&map)
        .repo_projects
        .github_app
        .installation_id?;
    i64::try_from(installation_id).ok()
}

fn project_view(project: &RepoProject, parts: &RepoProjectParts) -> RepoProjectView {
    let repo = parts
        .repos
        .iter()
        .find(|repo| repo.enrolled)
        .or_else(|| parts.repos.first());
    let tasks = &parts.tasks;
    let active_runs = parts
        .worker_runs
        .iter()
        .filter(|run| run.state == RepoWorkerRunState::Running)
        .count();
    let queued_items = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.state,
                RepoProjectTaskState::Queued
                    | RepoProjectTaskState::Planning
                    | RepoProjectTaskState::Ready
            )
        })
        .count();
    let open_prs = tasks
        .iter()
        .filter(|task| {
            task.pull_request_number.is_some() && task.state != RepoProjectTaskState::Done
        })
        .count();
    let gates = merge_gate_views(project, parts);

    RepoProjectView {
        id: project.id.to_string(),
        name: project.name.clone(),
        repo_url: repo
            .map(|repo| format!("github.com/{}/{}", repo.owner, repo.repo))
            .unwrap_or_else(|| "github.com/unknown/unknown".to_string()),
        default_branch: repo
            .map(|repo| repo.default_branch.clone())
            .unwrap_or_else(|| "main".to_string()),
        local_path: repo.and_then(|repo| repo.local_path.clone()),
        description: project.description.clone(),
        state: project_state_view(project.state),
        active_runs,
        queued_items,
        open_prs,
        merge_gate_state: aggregate_gate_state(&gates),
        github_app: github_app_state(project, repo),
        docker_agents: "ready".to_string(),
        credentials: credentials_state(project, repo),
        concurrency_limit: project.policy.max_parallel_tasks.max(1),
        write_mode: write_mode_label(project.policy.write_mode),
        auto_merge_policy: if project.policy.auto_merge
            && project.policy.write_mode.allows_auto_merge()
        {
            "green_checks".to_string()
        } else {
            "manual".to_string()
        },
        notifications: "disabled".to_string(),
        updated_at: Some(project.updated_at.to_rfc3339()),
        setup_checklist: setup_checklist(project, repo),
        backlog: tasks.iter().map(backlog_item).collect(),
        worker_runs: parts.worker_runs.iter().map(worker_run_view).collect(),
        pull_requests: tasks
            .iter()
            .filter(|task| task.pull_request_number.is_some() || task.pull_request_url.is_some())
            .map(pull_request_view)
            .collect(),
        ci_checks: Vec::new(),
        merge_gates: gates,
    }
}

fn setup_checklist(
    project: &RepoProject,
    repo: Option<&RepoProjectRepo>,
) -> Vec<RepoProjectSetupItem> {
    vec![
        RepoProjectSetupItem {
            key: "github_app".to_string(),
            label: "GitHub App".to_string(),
            state: if matches!(project.policy.github_auth_mode, GitHubAuthMode::GitHubApp) {
                "pending".to_string()
            } else {
                "complete".to_string()
            },
            detail: Some(if matches!(project.policy.github_auth_mode, GitHubAuthMode::GitHubApp) {
                "Configured for GitHub App auth; installation verification is handled by webhook/API setup.".to_string()
            } else {
                "Using token or gh fallback until GitHub App credentials are configured.".to_string()
            }),
        },
        RepoProjectSetupItem {
            key: "docker_agents".to_string(),
            label: "Docker coding agents".to_string(),
            state: "complete".to_string(),
            detail: Some("Supervisor records are ready for sandbox job dispatch.".to_string()),
        },
        RepoProjectSetupItem {
            key: "credentials".to_string(),
            label: "Credentials".to_string(),
            state: credentials_state(project, repo),
            detail: Some("Short-lived GitHub/model credential injection is enforced at worker dispatch.".to_string()),
        },
        RepoProjectSetupItem {
            key: "concurrency".to_string(),
            label: "Concurrency".to_string(),
            state: "complete".to_string(),
            detail: Some(format!("Limit set to {}", project.policy.max_parallel_tasks.max(1))),
        },
        RepoProjectSetupItem {
            key: "write_mode".to_string(),
            label: "Write mode".to_string(),
            state: "complete".to_string(),
            detail: Some(write_mode_detail(project.policy.write_mode)),
        },
        RepoProjectSetupItem {
            key: "auto_merge_policy".to_string(),
            label: "Auto-merge policy".to_string(),
            state: if project.policy.auto_merge && project.policy.write_mode.allows_auto_merge() {
                "complete".to_string()
            } else {
                "pending".to_string()
            },
            detail: Some(if project.policy.auto_merge && project.policy.write_mode.allows_auto_merge() {
                "Guarded auto-merge enabled for this project.".to_string()
            } else {
                "Manual merge required unless project policy is changed.".to_string()
            }),
        },
        RepoProjectSetupItem {
            key: "notifications".to_string(),
            label: "Notifications".to_string(),
            state: "pending".to_string(),
            detail: Some("Notification routing is recorded by events and can be connected to channel settings.".to_string()),
        },
    ]
}

fn merge_gate_views(project: &RepoProject, parts: &RepoProjectParts) -> Vec<RepoMergeGateView> {
    let mut gates = Vec::new();
    gates.push(RepoMergeGateView {
        id: "gate-policy".to_string(),
        label: "Auto-merge/write policy".to_string(),
        state: if project.policy.auto_merge && project.policy.write_mode.allows_auto_merge() {
            "passed".to_string()
        } else {
            "pending".to_string()
        },
        required: false,
        detail: Some(
            if project.policy.auto_merge && project.policy.write_mode.allows_auto_merge() {
                "Guarded auto-merge is enabled.".to_string()
            } else {
                format!(
                    "Project requires manual merge in {} mode.",
                    project.policy.write_mode.as_str()
                )
            },
        ),
        updated_at: Some(project.updated_at.to_rfc3339()),
    });

    for (task_id, decision) in &parts.merge_gates {
        gates.push(RepoMergeGateView {
            id: format!("gate-{task_id}"),
            label: format!("Merge gate {}", short_uuid(*task_id)),
            state: if decision.approved {
                "passed".to_string()
            } else {
                "blocked".to_string()
            },
            required: true,
            detail: Some(if decision.approved {
                format!("Approved for {} merge", enum_label(decision.merge_method))
            } else {
                let reasons = decision
                    .reasons
                    .iter()
                    .map(|reason| enum_label(*reason))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Denied: {reasons}")
            }),
            updated_at: Some(project.updated_at.to_rfc3339()),
        });
    }

    gates
}

fn backlog_item(task: &RepoProjectTask) -> RepoBacklogItem {
    RepoBacklogItem {
        id: task.id.to_string(),
        title: task.title.clone(),
        priority: priority_label(task.priority).to_string(),
        state: task_state_view(task.state),
        owner: task.assigned_worker_id.clone(),
        labels: task.labels.clone(),
        created_at: Some(task.created_at.to_rfc3339()),
        updated_at: Some(task.updated_at.to_rfc3339()),
    }
}

fn worker_run_view(run: &RepoWorkerRun) -> RepoWorkerRunView {
    let end = run.completed_at.unwrap_or_else(Utc::now);
    RepoWorkerRunView {
        id: run.id.to_string(),
        backlog_id: Some(run.task_id.to_string()),
        agent: run.worker_id.clone(),
        state: worker_state_view(run.state),
        branch: Some(run.branch_name.clone()),
        started_at: run.started_at.map(|value| value.to_rfc3339()),
        updated_at: Some(run.updated_at.to_rfc3339()),
        duration_secs: run
            .started_at
            .map(|started| (end - started).num_seconds().max(0)),
        last_event: run.summary.clone(),
    }
}

fn pull_request_view(task: &RepoProjectTask) -> RepoPullRequestView {
    RepoPullRequestView {
        id: task
            .pull_request_number
            .map(|number| format!("pr-{number}"))
            .unwrap_or_else(|| format!("task-{}", task.id)),
        title: task.title.clone(),
        number: task.pull_request_number,
        url: task.pull_request_url.clone(),
        branch: Some(task.branch_name.clone()),
        state: if task.state == RepoProjectTaskState::Done {
            "merged".to_string()
        } else {
            "open".to_string()
        },
        author: task.assigned_worker_id.clone(),
        updated_at: Some(task.updated_at.to_rfc3339()),
    }
}

fn event_view(event: RepoProjectEvent) -> RepoProjectEventView {
    RepoProjectEventView {
        id: event.id.to_string(),
        event_type: enum_label(event.kind),
        created_at: Some(event.created_at.to_rfc3339()),
        data: Some(serde_json::json!({
            "project_id": event.project_id,
            "repo_id": event.repo_id,
            "task_id": event.task_id,
            "project_run_id": event.project_run_id,
            "worker_run_id": event.worker_run_id,
            "message": event.message,
            "details": event.details,
        })),
    }
}

fn aggregate_gate_state(gates: &[RepoMergeGateView]) -> String {
    if gates
        .iter()
        .any(|gate| gate.state == "blocked" || gate.state == "failed")
    {
        "blocked".to_string()
    } else if gates.iter().all(|gate| gate.state == "passed") {
        "passed".to_string()
    } else {
        "pending".to_string()
    }
}

fn github_app_state(project: &RepoProject, repo: Option<&RepoProjectRepo>) -> String {
    if matches!(project.policy.github_auth_mode, GitHubAuthMode::GitHubApp)
        || repo
            .map(|repo| matches!(repo.auth_mode, GitHubAuthMode::GitHubApp))
            .unwrap_or(false)
    {
        "pending".to_string()
    } else {
        "connected".to_string()
    }
}

fn credentials_state(project: &RepoProject, repo: Option<&RepoProjectRepo>) -> String {
    if project.policy.write_mode == RepoWriteMode::ReadOnlyClone {
        return "ready".to_string();
    }
    if matches!(project.policy.github_auth_mode, GitHubAuthMode::GitHubApp)
        || repo
            .map(|repo| matches!(repo.auth_mode, GitHubAuthMode::GitHubApp))
            .unwrap_or(false)
    {
        "partial".to_string()
    } else {
        "ready".to_string()
    }
}

fn credential_detail(
    credential_mode: &str,
    write_mode: RepoWriteMode,
    supervisor_credentials_ready: bool,
    github_token_secret_present: Option<bool>,
    github_fork_token_secret_present: Option<bool>,
) -> String {
    if !supervisor_credentials_ready {
        return "Configure a GitHub App (app_id + private key secret) or store a github_token secret."
            .to_string();
    }
    if write_mode == RepoWriteMode::ForkPr && github_fork_token_secret_present != Some(true) {
        return "Store github_fork_token so workers can push only to forks in fork_pr mode."
            .to_string();
    }
    if matches!(
        write_mode,
        RepoWriteMode::MaintainerBranchPr | RepoWriteMode::MaintainerAutoMerge
    ) && credential_mode == "github_app"
        && github_token_secret_present != Some(true)
    {
        return "Store github_token so sandbox workers can push upstream task branches in maintainer modes."
            .to_string();
    }
    match (credential_mode, write_mode) {
        ("github_app", RepoWriteMode::ForkPr) => {
            "GitHub App handles upstream PR/CI; github_fork_token handles worker fork pushes."
                .to_string()
        }
        ("github_token", RepoWriteMode::ForkPr) => {
            "github_token handles upstream PR/CI; github_fork_token handles worker fork pushes."
                .to_string()
        }
        ("github_app", _) => "GitHub App installation token".to_string(),
        ("github_token", _) => "github_token secret".to_string(),
        _ => "GitHub credentials configured.".to_string(),
    }
}

fn write_mode_detail(mode: RepoWriteMode) -> String {
    match mode {
        RepoWriteMode::ReadOnlyClone => {
            "Workers receive no GitHub write credential; they can only report findings."
        }
        RepoWriteMode::ForkPr => {
            "Workers push to a fork credential and the supervisor opens a PR against upstream."
        }
        RepoWriteMode::MaintainerBranchPr => {
            "Workers may push task branches to the upstream repo; humans merge PRs."
        }
        RepoWriteMode::MaintainerAutoMerge => {
            "Workers may push task branches upstream and the supervisor may merge after gates pass."
        }
    }
    .to_string()
}

fn project_state_view(state: RepoProjectState) -> String {
    match state {
        RepoProjectState::Draft => "setup_required",
        RepoProjectState::Planning => "queued",
        RepoProjectState::Active => "running",
        RepoProjectState::Blocked | RepoProjectState::AwaitingHuman => "blocked",
        RepoProjectState::Paused => "paused",
        RepoProjectState::Completed => "completed",
        RepoProjectState::Failed => "failed",
        RepoProjectState::Cancelled => "cancelled",
    }
    .to_string()
}

fn task_state_view(state: RepoProjectTaskState) -> String {
    match state {
        RepoProjectTaskState::Done => "done".to_string(),
        RepoProjectTaskState::WaitingCi => "waiting_ci".to_string(),
        RepoProjectTaskState::WaitingReview => "waiting_review".to_string(),
        other => enum_label(other),
    }
}

fn worker_state_view(state: RepoWorkerRunState) -> String {
    match state {
        RepoWorkerRunState::Succeeded => "completed".to_string(),
        other => enum_label(other),
    }
}

fn state_label(state: RepoProjectState) -> String {
    enum_label(state)
}

fn enum_label<T>(value: T) -> String
where
    T: Serialize,
{
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_github_repo_url(input: &str) -> ApiResult<(String, String)> {
    let mut value = input.trim().trim_end_matches('/').to_string();
    if value.ends_with(".git") {
        value.truncate(value.len() - 4);
    }
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
        "github.com/",
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest.to_string();
            break;
        }
    }
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return Err(ApiError::InvalidInput(
            "repo_url must identify a GitHub repository as owner/repo or github.com/owner/repo"
                .to_string(),
        ));
    }
    repo_local_path_fragment(parts[0], parts[1]).map_err(ApiError::InvalidInput)?;
    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn repo_metadata(
    input_repo_url: &str,
    write_mode: RepoWriteMode,
    fork_owner: Option<&str>,
    fork_repo: Option<&str>,
) -> ApiResult<serde_json::Value> {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "input_repo_url".to_string(),
        serde_json::Value::String(input_repo_url.to_string()),
    );
    metadata.insert(
        "write_mode".to_string(),
        serde_json::Value::String(write_mode.as_str().to_string()),
    );

    let fork_owner = fork_owner
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let fork_repo = fork_repo
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    match (fork_owner, fork_repo) {
        (Some(owner), Some(repo)) => {
            repo_local_path_fragment(&owner, &repo).map_err(ApiError::InvalidInput)?;
            metadata.insert("fork_owner".to_string(), serde_json::Value::String(owner));
            metadata.insert("fork_repo".to_string(), serde_json::Value::String(repo));
        }
        (Some(owner), None) => {
            repo_local_path_fragment(&owner, "repo").map_err(ApiError::InvalidInput)?;
            metadata.insert("fork_owner".to_string(), serde_json::Value::String(owner));
        }
        (None, Some(_)) => {
            return Err(ApiError::InvalidInput(
                "fork_repo requires fork_owner".to_string(),
            ));
        }
        (None, None) => {}
    }

    Ok(serde_json::Value::Object(metadata))
}

fn unique_project_slug(name: &str, id: Uuid) -> String {
    let mut slug = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch == '-' || ch == '_' || ch.is_whitespace() {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "project".to_string();
    }
    format!("{}-{}", slug.trim_matches('-'), short_uuid(id))
}

fn short_uuid(id: Uuid) -> String {
    id.simple().to_string()[..8].to_string()
}

fn non_empty(value: String, field: &str) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidInput(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn coding_backend_from_setting(value: &str) -> CodingBackend {
    match value.trim().to_ascii_lowercase().as_str() {
        "codex_code" | "codex" => CodingBackend::CodexCode,
        "claude_code" | "claude" => CodingBackend::ClaudeCode,
        _ => CodingBackend::Worker,
    }
}

fn write_mode_from_setting(value: &str) -> RepoWriteMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "read_only_clone" | "read-only-clone" | "read_only" | "readonly" => {
            RepoWriteMode::ReadOnlyClone
        }
        "maintainer_branch_pr" | "maintainer-branch-pr" | "branch_pr" | "branch" => {
            RepoWriteMode::MaintainerBranchPr
        }
        "maintainer_auto_merge" | "maintainer-auto-merge" | "auto_merge" | "auto" => {
            RepoWriteMode::MaintainerAutoMerge
        }
        _ => RepoWriteMode::ForkPr,
    }
}

fn write_mode_label(mode: RepoWriteMode) -> String {
    mode.as_str().to_string()
}

fn priority_value(priority: Option<&str>) -> i32 {
    match priority.unwrap_or("medium").to_ascii_lowercase().as_str() {
        "urgent" => 100,
        "high" => 50,
        "low" => -10,
        _ => 0,
    }
}

fn priority_label(priority: i32) -> &'static str {
    if priority >= 75 {
        "urgent"
    } else if priority >= 25 {
        "high"
    } else if priority < 0 {
        "low"
    } else {
        "medium"
    }
}

pub fn default_user_id() -> &'static str {
    LOCAL_USER_ID
}

#[cfg(all(test, feature = "libsql"))]
mod tests;

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
    RepoWorkerRun, RepoWorkerRunState, branch_name_for_task, repo_local_path_fragment,
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
    let policy = default_policy_from_settings(store, user_id).await;
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
        installation_id: None,
        default_branch: default_branch.clone(),
        base_branch: Some(default_branch),
        enrolled: true,
        local_path,
        auth_mode: project.policy.github_auth_mode,
        metadata: serde_json::json!({ "input_repo_url": repo_url }),
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

    let task_id = Uuid::new_v4();
    let branch_name =
        branch_name_for_task(&project.slug, task_id).map_err(ApiError::InvalidInput)?;
    let now = Utc::now();
    let task = RepoProjectTask {
        id: task_id,
        project_id: project.id,
        repo_id: repo.id,
        title: non_empty(input.title, "title")?,
        body: input
            .description
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        state: RepoProjectTaskState::Queued,
        coding_backend: project.policy.default_coding_backend,
        base_branch: repo
            .base_branch
            .clone()
            .unwrap_or_else(|| repo.default_branch.clone()),
        branch_name,
        head_sha: None,
        pull_request_number: None,
        pull_request_url: None,
        github_issue_number: None,
        assigned_worker_id: None,
        priority: priority_value(input.priority.as_deref()),
        labels: input.labels,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        queued_at: Some(now),
        started_at: None,
        completed_at: None,
    };
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
        auto_merge_policy: if project.policy.auto_merge {
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
            key: "auto_merge_policy".to_string(),
            label: "Auto-merge policy".to_string(),
            state: if project.policy.auto_merge {
                "complete".to_string()
            } else {
                "pending".to_string()
            },
            detail: Some(if project.policy.auto_merge {
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
        label: "Auto-merge policy".to_string(),
        state: if project.policy.auto_merge {
            "passed".to_string()
        } else {
            "pending".to_string()
        },
        required: false,
        detail: Some(if project.policy.auto_merge {
            "Guarded auto-merge is enabled.".to_string()
        } else {
            "Project requires manual merge.".to_string()
        }),
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

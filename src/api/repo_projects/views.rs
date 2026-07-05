//! repo_projects API: view/label projection helpers.

use super::*;

pub(super) fn project_view(project: &RepoProject, parts: &RepoProjectParts) -> RepoProjectView {
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

pub(super) fn setup_checklist(
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

pub(super) fn merge_gate_views(
    project: &RepoProject,
    parts: &RepoProjectParts,
) -> Vec<RepoMergeGateView> {
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

pub(super) fn backlog_item(task: &RepoProjectTask) -> RepoBacklogItem {
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

pub(super) fn worker_run_view(run: &RepoWorkerRun) -> RepoWorkerRunView {
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

pub(super) fn pull_request_view(task: &RepoProjectTask) -> RepoPullRequestView {
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

pub(super) fn event_view(event: RepoProjectEvent) -> RepoProjectEventView {
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

pub(super) fn aggregate_gate_state(gates: &[RepoMergeGateView]) -> String {
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

pub(super) fn github_app_state(project: &RepoProject, repo: Option<&RepoProjectRepo>) -> String {
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

pub(super) fn credentials_state(project: &RepoProject, repo: Option<&RepoProjectRepo>) -> String {
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

pub(super) fn credential_detail(
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

pub(super) fn write_mode_detail(mode: RepoWriteMode) -> String {
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

pub(super) fn project_state_view(state: RepoProjectState) -> String {
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

pub(super) fn task_state_view(state: RepoProjectTaskState) -> String {
    match state {
        RepoProjectTaskState::Done => "done".to_string(),
        RepoProjectTaskState::WaitingCi => "waiting_ci".to_string(),
        RepoProjectTaskState::WaitingReview => "waiting_review".to_string(),
        other => enum_label(other),
    }
}

pub(super) fn worker_state_view(state: RepoWorkerRunState) -> String {
    match state {
        RepoWorkerRunState::Succeeded => "completed".to_string(),
        other => enum_label(other),
    }
}

pub(super) fn state_label(state: RepoProjectState) -> String {
    enum_label(state)
}

pub(super) fn enum_label<T>(value: T) -> String
where
    T: Serialize,
{
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn parse_github_repo_url(input: &str) -> ApiResult<(String, String)> {
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

pub(super) fn repo_metadata(
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

pub(super) fn unique_project_slug(name: &str, id: Uuid) -> String {
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

pub(super) fn short_uuid(id: Uuid) -> String {
    id.simple().to_string()[..8].to_string()
}

pub(super) fn non_empty(value: String, field: &str) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidInput(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

pub(super) fn coding_backend_from_setting(value: &str) -> CodingBackend {
    match value.trim().to_ascii_lowercase().as_str() {
        "codex_code" | "codex" => CodingBackend::CodexCode,
        "claude_code" | "claude" => CodingBackend::ClaudeCode,
        _ => CodingBackend::Worker,
    }
}

pub(super) fn write_mode_from_setting(value: &str) -> RepoWriteMode {
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

pub(super) fn write_mode_label(mode: RepoWriteMode) -> String {
    mode.as_str().to_string()
}

pub(super) fn priority_value(priority: Option<&str>) -> i32 {
    match priority.unwrap_or("medium").to_ascii_lowercase().as_str() {
        "urgent" => 100,
        "high" => 50,
        "low" => -10,
        _ => 0,
    }
}

pub(super) fn priority_label(priority: i32) -> &'static str {
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

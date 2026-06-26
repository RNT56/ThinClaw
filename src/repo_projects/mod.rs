//! Durable GitHub repo project supervision primitives.
//!
//! This module contains runtime pieces used by the project supervisor. The
//! shared data model lives in `thinclaw-repo-projects` once that crate is
//! wired into the workspace.

pub mod ci;
pub mod executor;
pub mod github;
pub mod github_provider;
pub mod merge_gate;
pub mod pipeline;
pub mod planner;
pub mod prompts;
pub mod subagent_planner;
pub mod supervisor;
pub mod workspace;

#[cfg(all(test, feature = "libsql"))]
mod pipeline_tests;

use chrono::Utc;
use thinclaw_repo_projects::{
    RepoProject, RepoProjectRepo, RepoProjectTask, RepoProjectTaskState, branch_name_for_task,
};
use uuid::Uuid;

/// Build a fresh `Queued` task for a project/repo pair with a supervisor-derived
/// branch name. Shared by the operator-facing `enqueue_task` API and the
/// autonomous planner so both produce identically-shaped tasks (avoiding the
/// copy-paste literal drift the audit flagged elsewhere).
///
/// Returns an error only when the branch name cannot be derived from the project
/// slug + task id (an invalid slug fragment).
pub(crate) fn build_queued_task(
    project: &RepoProject,
    repo: &RepoProjectRepo,
    title: String,
    body: Option<String>,
    priority: i32,
    labels: Vec<String>,
) -> Result<RepoProjectTask, String> {
    let task_id = Uuid::new_v4();
    let branch_name = branch_name_for_task(&project.slug, task_id)?;
    let now = Utc::now();
    Ok(RepoProjectTask {
        id: task_id,
        project_id: project.id,
        repo_id: repo.id,
        title,
        body,
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
        priority,
        labels,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        queued_at: Some(now),
        started_at: None,
        completed_at: None,
    })
}

/// Shallow-merge the keys of `patch` (an object) into `current` (treated as an
/// object), returning a new JSON object value. Non-object inputs are treated as
/// empty objects. Shared by the executor and the GitHub pipeline so task
/// metadata accumulates consistently across subsystems.
pub(crate) fn merge_metadata(
    current: &serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    let mut root = current.as_object().cloned().unwrap_or_default();
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            root.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(root)
}

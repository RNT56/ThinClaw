//! Autonomous task-planning port for the repo project supervisor.
//!
//! When a project enters [`RepoProjectState::Planning`] (or `Draft`) with no
//! tasks, the supervisor needs to decompose the project goal into concrete,
//! dispatchable tasks. That decomposition wants an LLM/subagent, which is a
//! heavy host dependency the dependency-light supervisor store must not pull in
//! directly. Following the ports-and-adapters spine of the codebase, the
//! capability is expressed here as a narrow trait; the concrete LLM-backed
//! adapter lives in the root app wiring and is injected behind
//! `DatabaseRepoSupervisorStore::with_planner`.
//!
//! When no planner is wired (e.g. no LLM stack is available, or the operator
//! opts out), the supervisor falls back to an explicit "awaiting human plan"
//! status rather than silently stalling a `Planning` project forever.

use thinclaw_repo_projects::{RepoProject, RepoProjectRepo};
use uuid::Uuid;

/// A single planned unit of work the supervisor will persist as a `Queued`
/// task. The `repo_id` must reference one of the project's enrolled repos; an
/// out-of-range id is dropped by the supervisor with a warning rather than
/// fabricating a task against an unknown repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTask {
    /// Short, imperative task title (becomes the PR title).
    pub title: String,
    /// Task body / acceptance criteria (becomes the PR body); optional.
    pub body: Option<String>,
    /// The enrolled repo this task targets.
    pub repo_id: Uuid,
}

impl PlannedTask {
    pub fn new(repo_id: Uuid, title: impl Into<String>, body: Option<String>) -> Self {
        Self {
            title: title.into(),
            body,
            repo_id,
        }
    }
}

/// Decomposes a project goal into concrete tasks. Implementations are expected
/// to be one-shot and side-effect-free with respect to project state — the
/// supervisor owns persistence and state transitions; the planner only returns
/// the proposed task drafts.
#[async_trait::async_trait]
pub trait RepoTaskPlanner: Send + Sync {
    /// Produce a non-empty task list for the project, or an error string the
    /// supervisor surfaces (and treats as "could not plan" → awaiting human).
    /// Returning an empty `Vec` is allowed and treated the same as the
    /// no-planner fallback (the project moves to `AwaitingHuman`).
    async fn plan(
        &self,
        project: &RepoProject,
        repos: &[RepoProjectRepo],
    ) -> Result<Vec<PlannedTask>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Deterministic planner: returns a fixed list of tasks targeting the first
    /// enrolled repo.
    struct FixedPlanner {
        tasks: Vec<(String, Option<String>)>,
    }

    #[async_trait::async_trait]
    impl RepoTaskPlanner for FixedPlanner {
        async fn plan(
            &self,
            _project: &RepoProject,
            repos: &[RepoProjectRepo],
        ) -> Result<Vec<PlannedTask>, String> {
            let repo_id = repos
                .first()
                .map(|repo| repo.id)
                .ok_or_else(|| "no enrolled repos".to_string())?;
            Ok(self
                .tasks
                .iter()
                .map(|(title, body)| PlannedTask::new(repo_id, title.clone(), body.clone()))
                .collect())
        }
    }

    #[tokio::test]
    async fn fixed_planner_targets_first_repo() {
        let planner = Arc::new(FixedPlanner {
            tasks: vec![("Add CI".to_string(), None)],
        });
        let project = sample_project();
        let repo = sample_repo(project.id);
        let planned = planner
            .plan(&project, std::slice::from_ref(&repo))
            .await
            .unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].repo_id, repo.id);
        assert_eq!(planned[0].title, "Add CI");
    }

    fn sample_project() -> RepoProject {
        use thinclaw_repo_projects::{ProjectPolicy, RepoProjectState};
        let now = chrono::Utc::now();
        RepoProject {
            id: Uuid::new_v4(),
            slug: "proj".to_string(),
            name: "Proj".to_string(),
            state: RepoProjectState::Planning,
            policy: ProjectPolicy::default(),
            description: Some("Ship the thing".to_string()),
            current_run_id: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        }
    }

    fn sample_repo(project_id: Uuid) -> RepoProjectRepo {
        use thinclaw_repo_projects::GitHubAuthMode;
        let now = chrono::Utc::now();
        RepoProjectRepo {
            id: Uuid::new_v4(),
            project_id,
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            github_repo_id: None,
            installation_id: None,
            default_branch: "main".to_string(),
            base_branch: Some("main".to_string()),
            enrolled: true,
            local_path: None,
            auth_mode: GitHubAuthMode::UserToken,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

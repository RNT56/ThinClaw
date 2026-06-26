//! Concrete [`RepoTaskPlanner`] backed by a one-shot planning subagent (F-06).
//!
//! When `REPO_PROJECTS_AUTOPLAN` is enabled and a [`SubagentExecutor`] is
//! available, the supervisor decomposes a project goal into concrete tasks by
//! spawning a single, tool-less reasoning subagent that returns a strict JSON
//! task list. The adapter is deliberately conservative: any failure (no
//! executor, spawn error, non-JSON output, or an empty plan) degrades to the
//! existing no-planner fallback — the supervisor moves the project to
//! `AwaitingHuman` rather than fabricating work.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_repo_projects::{RepoProject, RepoProjectRepo};

use super::planner::{PlannedTask, RepoTaskPlanner};
use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};

/// LLM/subagent-backed task planner.
pub struct SubagentRepoTaskPlanner {
    executor: Arc<SubagentExecutor>,
    /// Owner identity used as the spawned subagent's parent user.
    owner_user_id: String,
}

impl SubagentRepoTaskPlanner {
    pub fn new(executor: Arc<SubagentExecutor>, owner_user_id: impl Into<String>) -> Self {
        Self {
            executor,
            owner_user_id: owner_user_id.into(),
        }
    }

    /// Build the strict-JSON planning prompt for `project` over `repos`.
    fn planning_prompt(project: &RepoProject, repos: &[RepoProjectRepo]) -> String {
        let repo_list = repos
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "{i}: {}/{} (default branch {})",
                    r.owner, r.repo, r.default_branch
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let goal = project
            .description
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or(&project.name);
        format!(
            "You are planning concrete engineering tasks for an autonomous repo-project \
             supervisor.\nProject: {name}\nGoal: {goal}\n\nEnrolled repositories \
             (index: owner/repo):\n{repo_list}\n\nDecompose the goal into 1-5 small, \
             independently-shippable tasks. Respond with ONLY a JSON array (no prose, no code \
             fences) of objects with this shape: {{\"title\": string (imperative, <= 72 chars), \
             \"body\": string (acceptance criteria), \"repo_index\": integer}}. Use repo_index to \
             select the target repository from the list above. Return [] if you cannot produce a \
             sensible plan.",
            name = project.name,
        )
    }
}

#[async_trait]
impl RepoTaskPlanner for SubagentRepoTaskPlanner {
    async fn plan(
        &self,
        project: &RepoProject,
        repos: &[RepoProjectRepo],
    ) -> Result<Vec<PlannedTask>, String> {
        if repos.is_empty() {
            return Err("no enrolled repos to plan against".to_string());
        }

        let request = SubagentSpawnRequest {
            name: "repo-task-planner".to_string(),
            task: Self::planning_prompt(project, repos),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            // Pure reasoning: the planner proposes tasks, it does not act.
            allowed_tools: Some(Vec::new()),
            allowed_skills: Some(Vec::new()),
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: Some(180),
            wait: true,
        };

        let result = self
            .executor
            .spawn(
                request,
                "repo-projects",
                &serde_json::json!({}),
                &self.owner_user_id,
                None,
                None,
            )
            .await
            .map_err(|e| format!("planner subagent failed to run: {e}"))?;

        if !result.success {
            return Err(result
                .error
                .unwrap_or_else(|| "planner subagent returned failure".to_string()));
        }

        parse_planned_tasks(&result.response, repos)
    }
}

/// Parse the subagent's JSON response into [`PlannedTask`]s. Tolerant of
/// surrounding prose / code fences: extracts the first top-level JSON array.
fn parse_planned_tasks(
    response: &str,
    repos: &[RepoProjectRepo],
) -> Result<Vec<PlannedTask>, String> {
    let json_str = extract_json_array(response)
        .ok_or_else(|| "planner did not return a JSON array".to_string())?;

    #[derive(serde::Deserialize)]
    struct RawTask {
        title: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        repo_index: usize,
    }

    let raw: Vec<RawTask> =
        serde_json::from_str(json_str).map_err(|e| format!("invalid planner JSON: {e}"))?;

    let tasks = raw
        .into_iter()
        .filter_map(|t| {
            // Out-of-range repo_index falls back to the first enrolled repo; the
            // supervisor will further drop any task whose repo_id is unknown.
            let repo = repos.get(t.repo_index).or_else(|| repos.first())?;
            let title = t.title.trim();
            if title.is_empty() {
                return None;
            }
            let body = t.body.filter(|b| !b.trim().is_empty());
            Some(PlannedTask::new(repo.id, title, body))
        })
        .collect();
    Ok(tasks)
}

/// Extract the substring spanning the first `[` to the last `]`, if any.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_repo_projects::{GitHubAuthMode, RepoProjectRepo};
    use uuid::Uuid;

    fn repo(owner: &str) -> RepoProjectRepo {
        let now = chrono::Utc::now();
        RepoProjectRepo {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            owner: owner.to_string(),
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

    #[test]
    fn parses_plain_json_array() {
        let repos = vec![repo("acme"), repo("globex")];
        let resp = r#"[{"title":"Add CI","body":"green build","repo_index":1}]"#;
        let tasks = parse_planned_tasks(resp, &repos).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Add CI");
        assert_eq!(tasks[0].body.as_deref(), Some("green build"));
        assert_eq!(tasks[0].repo_id, repos[1].id);
    }

    #[test]
    fn tolerates_prose_and_fences_and_defaults_repo() {
        let repos = vec![repo("acme")];
        let resp = "Here is the plan:\n```json\n[{\"title\":\"Do thing\"}]\n```\nThanks!";
        let tasks = parse_planned_tasks(resp, &repos).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].repo_id, repos[0].id); // default repo_index 0
        assert!(tasks[0].body.is_none());
    }

    #[test]
    fn out_of_range_index_falls_back_to_first_repo() {
        let repos = vec![repo("acme")];
        let resp = r#"[{"title":"X","repo_index":9}]"#;
        let tasks = parse_planned_tasks(resp, &repos).unwrap();
        assert_eq!(tasks[0].repo_id, repos[0].id);
    }

    #[test]
    fn empty_array_yields_no_tasks() {
        let repos = vec![repo("acme")];
        assert!(parse_planned_tasks("[]", &repos).unwrap().is_empty());
    }

    #[test]
    fn blank_titles_are_dropped() {
        let repos = vec![repo("acme")];
        let resp = r#"[{"title":"   "},{"title":"Real"}]"#;
        let tasks = parse_planned_tasks(resp, &repos).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Real");
    }

    #[test]
    fn non_json_response_errors() {
        let repos = vec![repo("acme")];
        assert!(parse_planned_tasks("no json here", &repos).is_err());
    }
}

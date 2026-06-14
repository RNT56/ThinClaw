//! Sandbox-backed execution for repo project tasks.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::sandbox_jobs::{DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS, SandboxJobSpec};
use crate::sandbox_types::{ContainerJobManager, CredentialGrant, JobMode};
use thinclaw_repo_projects::{
    CodingBackend, RepoProject, RepoProjectEvent, RepoProjectEventKind, RepoProjectRepo,
    RepoProjectRun, RepoProjectRunState, RepoProjectTask, RepoProjectTaskState, RepoWorkerRun,
    RepoWorkerRunState, task_short_id, validate_task_state_transition,
};

use tokio::sync::broadcast;

use crate::channels::web::types::SseEvent;

use super::ci::{CiClassification, CiSuiteClassification, failure_kind_label};
use super::prompts::{RepoTaskPacketInput, build_implementation_packet, build_review_packet};
use super::workspace::RepoWorkspaceProvisioner;

const SUPERVISOR_ACTOR_ID: &str = "repo_project_supervisor";
const GITHUB_SECRET_NAME: &str = "github_token";

#[derive(Debug, Clone)]
pub struct RepoProjectExecutorConfig {
    pub workspace_base_dir: PathBuf,
    pub principal_id: String,
    pub actor_id: String,
}

impl Default for RepoProjectExecutorConfig {
    fn default() -> Self {
        Self {
            workspace_base_dir: RepoWorkspaceProvisioner::default_base_dir(),
            principal_id: SUPERVISOR_ACTOR_ID.to_string(),
            actor_id: SUPERVISOR_ACTOR_ID.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoProjectDispatchResult {
    pub worker_run_id: Uuid,
    pub job_id: Uuid,
    pub mode: JobMode,
    pub worktree_dir: PathBuf,
}

#[derive(Clone)]
pub struct RepoProjectExecutor {
    db: Arc<dyn Database>,
    job_manager: Option<Arc<ContainerJobManager>>,
    workspace: RepoWorkspaceProvisioner,
    principal_id: String,
    actor_id: String,
    sse: Option<broadcast::Sender<SseEvent>>,
}

impl RepoProjectExecutor {
    pub fn new(
        db: Arc<dyn Database>,
        job_manager: Option<Arc<ContainerJobManager>>,
        config: RepoProjectExecutorConfig,
    ) -> Self {
        Self {
            db,
            job_manager,
            workspace: RepoWorkspaceProvisioner::new(config.workspace_base_dir),
            principal_id: config.principal_id,
            actor_id: config.actor_id,
            sse: None,
        }
    }

    /// Wire an SSE broadcast sender so worker-run and task transitions surface
    /// live to the operator UI.
    pub fn with_sse(mut self, sse: Option<broadcast::Sender<SseEvent>>) -> Self {
        self.sse = sse;
        self
    }

    pub async fn dispatch_task(
        &self,
        project: &mut RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<Option<RepoProjectDispatchResult>, String> {
        if !matches!(
            task.state,
            RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
        ) {
            return Ok(None);
        }
        if let Some(existing) = self.active_worker_for_task(project.id, task.id).await? {
            return Ok(Some(existing));
        }
        self.dispatch_core(project, repo, task, None, &[]).await
    }

    /// Re-dispatch a task that is waiting on CI back into a sandbox worker with
    /// the CI failure classification injected into the packet, so the coding
    /// agent can target the specific failure. Bounded-attempt accounting is owned
    /// by the GitHub pipeline before this is called.
    pub async fn redispatch_repair_task(
        &self,
        project: &mut RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        ci: &CiSuiteClassification,
    ) -> Result<Option<RepoProjectDispatchResult>, String> {
        if task.state != RepoProjectTaskState::WaitingCi {
            return Ok(None);
        }
        if let Some(existing) = self.active_worker_for_task(project.id, task.id).await? {
            return Ok(Some(existing));
        }
        let primary = ci.checks.iter().find(|check| check.failure_kind.is_some());
        let failure_label = ci
            .primary_failure_kind
            .map(failure_kind_label)
            .unwrap_or("unknown");
        let extra_context: Vec<(&str, &str)> = vec![
            ("ci_status", "failing"),
            ("ci_primary_failure", failure_label),
            ("ci_summary", ci.summary.as_str()),
        ];
        self.dispatch_core(project, repo, task, primary, &extra_context)
            .await
    }

    async fn dispatch_core(
        &self,
        project: &mut RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        ci: Option<&CiClassification>,
        extra_context: &[(&str, &str)],
    ) -> Result<Option<RepoProjectDispatchResult>, String> {
        let Some(job_manager) = self.job_manager.as_ref() else {
            self.block_task(task, "Sandbox job manager is unavailable")
                .await?;
            return Err("sandbox job manager unavailable".to_string());
        };

        let mode = select_job_mode(task.coding_backend, job_manager);
        let short_id = task_short_id(task.id);
        let base_branch = task.base_branch.clone();
        let remote_url = repo_remote_url(repo);

        self.workspace
            .clone_or_fetch(&repo.owner, &repo.repo, &remote_url, &base_branch)
            .await
            .map_err(|error| format!("workspace clone/fetch failed: {error}"))?;
        let worktree = self
            .workspace
            .create_task_worktree(
                &repo.owner,
                &repo.repo,
                &project.slug,
                &short_id,
                &base_branch,
            )
            .await
            .map_err(|error| format!("workspace worktree failed: {error}"))?;

        let now = Utc::now();
        let worker_run_id = Uuid::new_v4();
        let project_run_id = project.current_run_id.unwrap_or(worker_run_id);
        if project.current_run_id.is_none() {
            project.current_run_id = Some(project_run_id);
            project.updated_at = now;
            self.db
                .update_repo_project(project)
                .await
                .map_err(|error| error.to_string())?;

            // Open a durable run record for this supervisor work session.
            let run = RepoProjectRun {
                id: project_run_id,
                project_id: project.id,
                state: RepoProjectRunState::Running,
                trigger: "supervisor".to_string(),
                summary: None,
                tasks_seen: 0,
                tasks_queued: 0,
                tasks_completed: 0,
                tasks_failed: 0,
                metadata: serde_json::json!({}),
                created_at: now,
                started_at: Some(now),
                completed_at: None,
            };
            self.db
                .upsert_repo_project_run(&run)
                .await
                .map_err(|error| error.to_string())?;
            append_event(
                self.db.as_ref(),
                project.id,
                None,
                None,
                None,
                RepoProjectEventKind::ProjectRunStarted,
                "Repository project run started",
                serde_json::json!({ "run_id": project_run_id }),
            )
            .await?;
        }

        let job_id = Uuid::new_v4();
        let mut worker_run = RepoWorkerRun {
            id: worker_run_id,
            project_id: project.id,
            project_run_id,
            repo_id: repo.id,
            task_id: task.id,
            state: RepoWorkerRunState::Queued,
            coding_backend: task.coding_backend,
            worker_id: format!("repo-project-supervisor-{short_id}"),
            branch_name: task.branch_name.clone(),
            job_id: Some(job_id.to_string()),
            commit_sha: None,
            exit_code: None,
            summary: None,
            metadata: serde_json::json!({
                "repo": format!("{}/{}", repo.owner, repo.repo),
                "mode": mode.as_str(),
                "base_branch": base_branch,
                "worktree_dir": worktree.worktree_dir,
            }),
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        };

        self.db
            .upsert_repo_worker_run(&worker_run)
            .await
            .map_err(|error| error.to_string())?;
        append_event(
            self.db.as_ref(),
            project.id,
            Some(repo.id),
            Some(task.id),
            Some(worker_run.id),
            RepoProjectEventKind::WorkerRunQueued,
            "Repository task worker queued",
            serde_json::json!({
                "job_id": job_id,
                "mode": mode.as_str(),
                "branch": task.branch_name,
            }),
        )
        .await?;
        self.emit_worker_run(&worker_run, "Worker queued");

        let worktree_path = worktree.worktree_dir.display().to_string();
        let task_packet = build_implementation_packet(
            RepoTaskPacketInput {
                project,
                repo,
                task,
                worktree_path: Some(&worktree_path),
                ci,
                merge_gate: None,
                extra_context,
            },
            task.coding_backend,
        );
        let task_prompt =
            wrap_packet_prompt(&task_packet.prompt, &task.base_branch, &task.branch_name);

        let mut spec = SandboxJobSpec::new(
            format!("{}: {}", project.name, task.title),
            task_prompt,
            self.principal_id.clone(),
            self.actor_id.clone(),
            Some(worktree.worktree_dir.display().to_string()),
            mode,
        );
        spec.interactive = false;
        spec.idle_timeout_secs = DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS;
        spec.metadata = serde_json::json!({
            "repo_project_id": project.id,
            "repo_project_slug": project.slug,
            "repo_project_task_id": task.id,
            "repo_worker_run_id": worker_run.id,
            "repo": format!("{}/{}", repo.owner, repo.repo),
            "base_branch": task.base_branch,
            "branch_name": task.branch_name,
            "pull_request_number": task.pull_request_number,
            "source": "repo_project_supervisor",
            "task_packet": task_packet.metadata,
        });

        let credential_grants = repo_project_credential_grants();
        let credential_grants_json =
            serde_json::to_string(&credential_grants).unwrap_or_else(|_| "[]".to_string());
        self.db
            .save_sandbox_job(&SandboxJobRecord {
                id: job_id,
                spec: spec.clone(),
                status: "creating".to_string(),
                success: None,
                failure_reason: None,
                created_at: now,
                started_at: None,
                completed_at: None,
                credential_grants_json,
            })
            .await
            .map_err(|error| error.to_string())?;

        if let Err(error) = job_manager
            .create_job(job_id, spec.clone(), credential_grants)
            .await
        {
            let message = format!("failed to create sandbox job: {error}");
            self.db
                .update_sandbox_job_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some(&message),
                    None,
                    Some(now),
                )
                .await
                .map_err(|error| error.to_string())?;
            worker_run.state = RepoWorkerRunState::Failed;
            worker_run.summary = Some(message.clone());
            worker_run.updated_at = Utc::now();
            worker_run.completed_at = Some(worker_run.updated_at);
            self.db
                .upsert_repo_worker_run(&worker_run)
                .await
                .map_err(|error| error.to_string())?;
            self.block_task(task, &message).await?;
            return Err(message);
        }

        self.db
            .update_sandbox_job_status(job_id, "running", None, None, Some(now), None)
            .await
            .map_err(|error| error.to_string())?;
        worker_run.state = RepoWorkerRunState::Running;
        worker_run.started_at = Some(now);
        worker_run.updated_at = now;
        self.db
            .upsert_repo_worker_run(&worker_run)
            .await
            .map_err(|error| error.to_string())?;
        self.emit_worker_run(&worker_run, "Worker started");

        task.state = RepoProjectTaskState::Running;
        task.assigned_worker_id = Some(worker_run.worker_id.clone());
        task.started_at = Some(now);
        task.updated_at = now;
        task.metadata = merge_task_metadata(
            &task.metadata,
            serde_json::json!({
                "worker_run_id": worker_run.id,
                "sandbox_job_id": job_id,
                "worktree_dir": worktree.worktree_dir,
                "job_mode": mode.as_str(),
            }),
        );
        self.db
            .upsert_repo_project_task(task)
            .await
            .map_err(|error| error.to_string())?;
        append_event(
            self.db.as_ref(),
            project.id,
            Some(repo.id),
            Some(task.id),
            Some(worker_run.id),
            RepoProjectEventKind::WorkerRunStarted,
            "Repository task worker started",
            serde_json::json!({
                "job_id": job_id,
                "mode": mode.as_str(),
                "worktree_dir": worktree.worktree_dir,
            }),
        )
        .await?;
        append_event(
            self.db.as_ref(),
            project.id,
            Some(repo.id),
            Some(task.id),
            Some(worker_run.id),
            RepoProjectEventKind::TaskStateChanged,
            "Repository task is running",
            serde_json::json!({ "state": "running" }),
        )
        .await?;
        self.emit_task(task, "Task running");

        Ok(Some(RepoProjectDispatchResult {
            worker_run_id,
            job_id,
            mode,
            worktree_dir: worktree.worktree_dir,
        }))
    }

    /// Dispatch a one-shot sandbox review of the task's pull request. The review
    /// worker checks out the *pushed* branch content (not a fresh branch off
    /// base) and is instructed to post findings as a PR review comment. It does
    /// not change the task's state, and `sync_worker_runs` skips it when
    /// reconciling task transitions.
    pub async fn dispatch_review_task(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &RepoProjectTask,
        backend: CodingBackend,
    ) -> Result<Option<RepoProjectDispatchResult>, String> {
        let Some(job_manager) = self.job_manager.as_ref() else {
            return Ok(None);
        };
        if self.has_active_review_worker(project.id, task.id).await? {
            return Ok(None);
        }

        let mode = select_job_mode(backend, job_manager);
        let short_id = task_short_id(task.id);
        let remote_url = repo_remote_url(repo);

        self.workspace
            .clone_or_fetch(&repo.owner, &repo.repo, &remote_url, &task.base_branch)
            .await
            .map_err(|error| format!("workspace clone/fetch failed: {error}"))?;
        let worktree = self
            .workspace
            .create_review_worktree(&repo.owner, &repo.repo, &short_id, &task.branch_name)
            .await
            .map_err(|error| format!("review worktree failed: {error}"))?;

        let now = Utc::now();
        let worker_run_id = Uuid::new_v4();
        let project_run_id = project.current_run_id.unwrap_or(worker_run_id);
        let job_id = Uuid::new_v4();
        let worktree_path = worktree.worktree_dir.display().to_string();
        let packet = build_review_packet(
            RepoTaskPacketInput {
                project,
                repo,
                task,
                worktree_path: Some(&worktree_path),
                ci: None,
                merge_gate: None,
                extra_context: &[],
            },
            backend,
        );
        let prompt =
            wrap_review_prompt(&packet.prompt, &task.base_branch, task.pull_request_number);

        let mut worker_run = RepoWorkerRun {
            id: worker_run_id,
            project_id: project.id,
            project_run_id,
            repo_id: repo.id,
            task_id: task.id,
            state: RepoWorkerRunState::Queued,
            coding_backend: backend,
            worker_id: format!("repo-project-reviewer-{short_id}"),
            branch_name: task.branch_name.clone(),
            job_id: Some(job_id.to_string()),
            commit_sha: task.head_sha.clone(),
            exit_code: None,
            summary: None,
            metadata: serde_json::json!({
                "review": true,
                "mode": mode.as_str(),
                "worktree_dir": worktree.worktree_dir,
            }),
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        };
        self.db
            .upsert_repo_worker_run(&worker_run)
            .await
            .map_err(|error| error.to_string())?;
        append_event(
            self.db.as_ref(),
            project.id,
            Some(repo.id),
            Some(task.id),
            Some(worker_run.id),
            RepoProjectEventKind::WorkerRunQueued,
            "Repository review worker queued",
            serde_json::json!({ "job_id": job_id, "mode": mode.as_str(), "review": true }),
        )
        .await?;
        self.emit_worker_run(&worker_run, "Review worker queued");

        let mut spec = SandboxJobSpec::new(
            format!("{}: review {}", project.name, task.title),
            prompt,
            self.principal_id.clone(),
            self.actor_id.clone(),
            Some(worktree_path.clone()),
            mode,
        );
        spec.interactive = false;
        spec.idle_timeout_secs = DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS;
        spec.metadata = serde_json::json!({
            "repo_project_id": project.id,
            "repo_project_task_id": task.id,
            "repo_worker_run_id": worker_run.id,
            "repo": format!("{}/{}", repo.owner, repo.repo),
            "pull_request_number": task.pull_request_number,
            "review": true,
            "source": "repo_project_supervisor",
            "task_packet": packet.metadata,
        });

        let credential_grants = repo_project_credential_grants();
        let credential_grants_json =
            serde_json::to_string(&credential_grants).unwrap_or_else(|_| "[]".to_string());
        self.db
            .save_sandbox_job(&SandboxJobRecord {
                id: job_id,
                spec: spec.clone(),
                status: "creating".to_string(),
                success: None,
                failure_reason: None,
                created_at: now,
                started_at: None,
                completed_at: None,
                credential_grants_json,
            })
            .await
            .map_err(|error| error.to_string())?;

        if let Err(error) = job_manager
            .create_job(job_id, spec.clone(), credential_grants)
            .await
        {
            let message = format!("failed to create review job: {error}");
            self.db
                .update_sandbox_job_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some(&message),
                    None,
                    Some(now),
                )
                .await
                .map_err(|error| error.to_string())?;
            worker_run.state = RepoWorkerRunState::Failed;
            worker_run.summary = Some(message.clone());
            worker_run.updated_at = Utc::now();
            worker_run.completed_at = Some(worker_run.updated_at);
            self.db
                .upsert_repo_worker_run(&worker_run)
                .await
                .map_err(|error| error.to_string())?;
            return Err(message);
        }

        self.db
            .update_sandbox_job_status(job_id, "running", None, None, Some(now), None)
            .await
            .map_err(|error| error.to_string())?;
        worker_run.state = RepoWorkerRunState::Running;
        worker_run.started_at = Some(now);
        worker_run.updated_at = now;
        self.db
            .upsert_repo_worker_run(&worker_run)
            .await
            .map_err(|error| error.to_string())?;
        self.emit_worker_run(&worker_run, "Review worker started");

        Ok(Some(RepoProjectDispatchResult {
            worker_run_id,
            job_id,
            mode,
            worktree_dir: worktree.worktree_dir,
        }))
    }

    async fn has_active_review_worker(
        &self,
        project_id: Uuid,
        task_id: Uuid,
    ) -> Result<bool, String> {
        let runs = self
            .db
            .list_repo_worker_runs(project_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(runs.iter().any(|run| {
            run.task_id == task_id
                && is_review_run(run)
                && matches!(
                    run.state,
                    RepoWorkerRunState::Queued | RepoWorkerRunState::Running
                )
        }))
    }

    pub async fn sync_worker_runs(&self, project_id: Uuid) -> Result<(), String> {
        let mut worker_runs = self
            .db
            .list_repo_worker_runs(project_id)
            .await
            .map_err(|error| error.to_string())?;
        for worker_run in worker_runs.iter_mut().filter(|run| {
            matches!(
                run.state,
                RepoWorkerRunState::Queued | RepoWorkerRunState::Running
            )
        }) {
            let Some(job_id) = worker_run
                .job_id
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok())
            else {
                continue;
            };
            let Some(job) = self
                .db
                .get_sandbox_job(job_id)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            if !crate::sandbox_jobs::is_terminal_sandbox_status(&job.status) {
                continue;
            }

            let now = Utc::now();
            let success = job.success.unwrap_or(job.status == "completed");
            worker_run.state = if success {
                RepoWorkerRunState::Succeeded
            } else {
                RepoWorkerRunState::Failed
            };
            worker_run.summary = job.failure_reason.clone().or_else(|| {
                Some(if success {
                    "Sandbox job completed".to_string()
                } else {
                    format!("Sandbox job ended with status '{}'", job.status)
                })
            });
            worker_run.updated_at = now;
            worker_run.completed_at = Some(now);
            self.db
                .upsert_repo_worker_run(worker_run)
                .await
                .map_err(|error| error.to_string())?;
            self.emit_worker_run(
                worker_run,
                if success {
                    "Worker succeeded"
                } else {
                    "Worker failed"
                },
            );

            // Review workers post findings to the PR and do not drive task
            // state; only implementation workers transition the task.
            if is_review_run(worker_run) {
                append_event(
                    self.db.as_ref(),
                    worker_run.project_id,
                    Some(worker_run.repo_id),
                    Some(worker_run.task_id),
                    Some(worker_run.id),
                    RepoProjectEventKind::WorkerRunCompleted,
                    if success {
                        "Repository review worker completed"
                    } else {
                        "Repository review worker failed"
                    },
                    serde_json::json!({ "job_id": job_id, "status": job.status, "review": true }),
                )
                .await?;
                continue;
            }

            if let Some(mut task) = self
                .db
                .get_repo_project_task(worker_run.task_id)
                .await
                .map_err(|error| error.to_string())?
            {
                let next_state = if success {
                    RepoProjectTaskState::WaitingCi
                } else {
                    RepoProjectTaskState::Failed
                };
                if validate_task_state_transition(task.state, next_state).is_ok() {
                    task.state = next_state;
                    task.updated_at = now;
                    if !success {
                        task.completed_at = Some(now);
                    }
                    task.metadata = merge_task_metadata(
                        &task.metadata,
                        serde_json::json!({
                            "last_sandbox_status": job.status,
                            "last_sandbox_success": success,
                        }),
                    );
                    self.db
                        .upsert_repo_project_task(&task)
                        .await
                        .map_err(|error| error.to_string())?;
                    append_event(
                        self.db.as_ref(),
                        task.project_id,
                        Some(task.repo_id),
                        Some(task.id),
                        Some(worker_run.id),
                        RepoProjectEventKind::WorkerRunCompleted,
                        if success {
                            "Repository task worker completed"
                        } else {
                            "Repository task worker failed"
                        },
                        serde_json::json!({
                            "job_id": job_id,
                            "status": job.status,
                            "success": success,
                            "message": worker_run.summary,
                        }),
                    )
                    .await?;
                    append_event(
                        self.db.as_ref(),
                        task.project_id,
                        Some(task.repo_id),
                        Some(task.id),
                        Some(worker_run.id),
                        RepoProjectEventKind::TaskStateChanged,
                        if success {
                            "Repository task is waiting for CI"
                        } else {
                            "Repository task failed"
                        },
                        serde_json::json!({ "state": state_label(task.state) }),
                    )
                    .await?;
                    self.emit_task(
                        &task,
                        if success {
                            "Worker completed; waiting for CI"
                        } else {
                            "Worker failed"
                        },
                    );
                }
            }
        }
        Ok(())
    }

    async fn active_worker_for_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
    ) -> Result<Option<RepoProjectDispatchResult>, String> {
        let runs = self
            .db
            .list_repo_worker_runs(project_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(runs.into_iter().find_map(|run| {
            if run.task_id != task_id
                || !matches!(
                    run.state,
                    RepoWorkerRunState::Queued | RepoWorkerRunState::Running
                )
            {
                return None;
            }
            let job_id = run.job_id.as_deref().and_then(|value| value.parse().ok())?;
            Some(RepoProjectDispatchResult {
                worker_run_id: run.id,
                job_id,
                mode: job_mode_from_backend(run.coding_backend),
                worktree_dir: run
                    .metadata
                    .get("worktree_dir")
                    .and_then(|value| value.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_default(),
            })
        }))
    }

    async fn block_task(&self, task: &mut RepoProjectTask, reason: &str) -> Result<(), String> {
        let previous = task.state;
        if validate_task_state_transition(previous, RepoProjectTaskState::Blocked).is_err() {
            return Ok(());
        }
        let now = Utc::now();
        task.state = RepoProjectTaskState::Blocked;
        task.updated_at = now;
        task.metadata = merge_task_metadata(
            &task.metadata,
            serde_json::json!({ "blocked_reason": reason }),
        );
        self.db
            .upsert_repo_project_task(task)
            .await
            .map_err(|error| error.to_string())?;
        append_event(
            self.db.as_ref(),
            task.project_id,
            Some(task.repo_id),
            Some(task.id),
            None,
            RepoProjectEventKind::TaskStateChanged,
            "Repository task blocked",
            serde_json::json!({
                "from": state_label(previous),
                "to": "blocked",
                "reason": reason,
            }),
        )
        .await?;
        self.emit_task(task, reason);
        Ok(())
    }

    fn emit_task(&self, task: &RepoProjectTask, message: &str) {
        self.emit_sse(SseEvent::RepoTaskUpdated {
            project_id: task.project_id.to_string(),
            task_id: task.id.to_string(),
            state: state_label(task.state).to_string(),
            message: message.to_string(),
        });
    }

    fn emit_worker_run(&self, run: &RepoWorkerRun, message: &str) {
        self.emit_sse(SseEvent::RepoWorkerRunUpdated {
            project_id: run.project_id.to_string(),
            worker_run_id: run.id.to_string(),
            state: worker_run_state_label(run.state).to_string(),
            message: message.to_string(),
        });
    }

    fn emit_sse(&self, event: SseEvent) {
        if let Some(sender) = self.sse.as_ref() {
            let _ = sender.send(event);
        }
    }
}

pub fn build_repo_task_prompt(
    project: &RepoProject,
    repo: &RepoProjectRepo,
    task: &RepoProjectTask,
    worktree_path: Option<&str>,
) -> String {
    let packet = build_implementation_packet(
        RepoTaskPacketInput {
            project,
            repo,
            task,
            worktree_path,
            ci: None,
            merge_gate: None,
            extra_context: &[],
        },
        task.coding_backend,
    );
    wrap_packet_prompt(&packet.prompt, &task.base_branch, &task.branch_name)
}

/// Wrap a deterministic task packet prompt with the ThinClaw sandbox execution
/// rules. Shared by the initial dispatch and CI-repair re-dispatch so both
/// enforce identical branch/PR safety constraints.
fn wrap_packet_prompt(packet_prompt: &str, base_branch: &str, branch_name: &str) -> String {
    format!(
        r#"{packet_prompt}

## ThinClaw Execution Rules
- Work only in /workspace, which is an isolated git worktree for this task.
- Do not push directly to the base branch.
- Keep all autonomous code changes on branch `{branch_name}`.
- Make focused commits with clear messages.
- Run the relevant tests/checks you changed or can reasonably infer.
- Push `{branch_name}` to origin when credentials are available.
- Open or update a pull request targeting `{base_branch}` when GitHub credentials are available.
- Report blockers explicitly in the final message when credentials, tests, or required context are missing.
"#,
    )
}

pub fn select_job_mode(backend: CodingBackend, job_manager: &ContainerJobManager) -> JobMode {
    match backend {
        CodingBackend::CodexCode if job_manager.codex_code_enabled() => JobMode::CodexCode,
        CodingBackend::CodexCode if job_manager.claude_code_enabled() => JobMode::ClaudeCode,
        CodingBackend::ClaudeCode if job_manager.claude_code_enabled() => JobMode::ClaudeCode,
        CodingBackend::ClaudeCode if job_manager.codex_code_enabled() => JobMode::CodexCode,
        _ => JobMode::Worker,
    }
}

fn job_mode_from_backend(backend: CodingBackend) -> JobMode {
    match backend {
        CodingBackend::CodexCode => JobMode::CodexCode,
        CodingBackend::ClaudeCode => JobMode::ClaudeCode,
        CodingBackend::Worker => JobMode::Worker,
    }
}

fn repo_project_credential_grants() -> Vec<CredentialGrant> {
    vec![
        CredentialGrant {
            secret_name: GITHUB_SECRET_NAME.to_string(),
            env_var: "GITHUB_TOKEN".to_string(),
        },
        CredentialGrant {
            secret_name: GITHUB_SECRET_NAME.to_string(),
            env_var: "GH_TOKEN".to_string(),
        },
    ]
}

fn repo_remote_url(repo: &RepoProjectRepo) -> String {
    repo.metadata
        .get("clone_url")
        .and_then(|value| value.as_str())
        .or_else(|| {
            repo.metadata
                .get("input_repo_url")
                .and_then(|value| value.as_str())
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{}/{}.git", repo.owner, repo.repo))
}

fn is_review_run(run: &RepoWorkerRun) -> bool {
    run.metadata
        .get("review")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Wrap a review packet prompt with read-only review execution rules.
fn wrap_review_prompt(
    packet_prompt: &str,
    base_branch: &str,
    pull_request_number: Option<u64>,
) -> String {
    let pr_line = match pull_request_number {
        Some(number) => format!(
            "- Post your findings as a review comment on pull request #{number} using the GitHub \
             CLI (`gh pr review` / `gh pr comment`)."
        ),
        None => {
            "- Summarize your findings in the final message; no pull request is associated yet."
                .to_string()
        }
    };
    format!(
        r#"{packet_prompt}

## ThinClaw Review Rules
- /workspace is a read-only checkout of the pushed task branch (the changes under review) targeting `{base_branch}`.
- Do not modify code, push commits, or open/merge pull requests.
- Prioritize correctness, security, regression risk, and missing tests; cite concrete files and lines.
{pr_line}
- If GitHub credentials are unavailable, return the review in your final message instead.
"#,
    )
}

fn merge_task_metadata(current: &serde_json::Value, patch: serde_json::Value) -> serde_json::Value {
    super::merge_metadata(current, patch)
}

fn worker_run_state_label(state: RepoWorkerRunState) -> &'static str {
    match state {
        RepoWorkerRunState::Queued => "queued",
        RepoWorkerRunState::Running => "running",
        RepoWorkerRunState::Succeeded => "succeeded",
        RepoWorkerRunState::Failed => "failed",
        RepoWorkerRunState::Cancelled => "cancelled",
    }
}

async fn append_event(
    db: &dyn Database,
    project_id: Uuid,
    repo_id: Option<Uuid>,
    task_id: Option<Uuid>,
    worker_run_id: Option<Uuid>,
    kind: RepoProjectEventKind,
    message: &str,
    details: serde_json::Value,
) -> Result<(), String> {
    db.append_repo_project_event(&RepoProjectEvent {
        id: Uuid::new_v4(),
        project_id,
        repo_id,
        task_id,
        project_run_id: None,
        worker_run_id,
        kind,
        message: message.to_string(),
        details,
        created_at: Utc::now(),
    })
    .await
    .map_err(|error| error.to_string())
}

fn state_label(state: RepoProjectTaskState) -> &'static str {
    match state {
        RepoProjectTaskState::Queued => "queued",
        RepoProjectTaskState::Planning => "planning",
        RepoProjectTaskState::Ready => "ready",
        RepoProjectTaskState::Running => "running",
        RepoProjectTaskState::WaitingCi => "waiting_ci",
        RepoProjectTaskState::WaitingReview => "waiting_review",
        RepoProjectTaskState::Blocked => "blocked",
        RepoProjectTaskState::Done => "done",
        RepoProjectTaskState::Failed => "failed",
        RepoProjectTaskState::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use thinclaw_repo_projects::{GitHubAuthMode, ProjectPolicy};

    fn sample_project() -> RepoProject {
        RepoProject {
            id: Uuid::new_v4(),
            slug: "sample".to_string(),
            name: "Sample".to_string(),
            state: thinclaw_repo_projects::RepoProjectState::Active,
            policy: ProjectPolicy::default(),
            description: None,
            current_run_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        }
    }

    fn sample_repo(project_id: Uuid) -> RepoProjectRepo {
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_task(project_id: Uuid, repo_id: Uuid) -> RepoProjectTask {
        RepoProjectTask {
            id: Uuid::new_v4(),
            project_id,
            repo_id,
            title: "Fix CI".to_string(),
            body: Some("Make the tests pass.".to_string()),
            state: RepoProjectTaskState::Queued,
            coding_backend: CodingBackend::CodexCode,
            base_branch: "main".to_string(),
            branch_name: "thinclaw/sample/abc123".to_string(),
            head_sha: None,
            pull_request_number: None,
            pull_request_url: None,
            github_issue_number: None,
            assigned_worker_id: None,
            priority: 0,
            labels: vec![],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            queued_at: Some(Utc::now()),
            started_at: None,
            completed_at: None,
        }
    }

    #[test]
    fn prompt_contains_branch_and_no_base_push_rule() {
        let project = sample_project();
        let repo = sample_repo(project.id);
        let task = sample_task(project.id, repo.id);
        let prompt = build_repo_task_prompt(&project, &repo, &task, Some("/tmp/worktree"));

        assert!(prompt.contains("repo: acme/widgets"));
        assert!(prompt.contains("task_branch: thinclaw/sample/abc123"));
        assert!(prompt.contains("Do not push directly to the base branch"));
        assert!(prompt.contains("Open or update a pull request"));
    }

    #[test]
    fn task_metadata_merge_preserves_existing_keys() {
        let merged = merge_task_metadata(
            &serde_json::json!({"a": 1, "b": false}),
            serde_json::json!({"b": true, "c": "new"}),
        );
        assert_eq!(merged["a"], 1);
        assert_eq!(merged["b"], true);
        assert_eq!(merged["c"], "new");
    }
}

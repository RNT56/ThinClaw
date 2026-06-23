//! Bounded reconcile loop scaffolding for the repo project supervisor.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use thinclaw_repo_projects::{
    RepoProject, RepoProjectEvent, RepoProjectEventKind, RepoProjectRepo, RepoProjectRunState,
    RepoProjectState, RepoProjectTask, RepoProjectTaskState, validate_project_state_transition,
    validate_task_state_transition,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::repo_projects::executor::RepoProjectExecutor;
use crate::repo_projects::pipeline::{GitHubPipeline, PipelineOutcome};
use crate::repo_projects::planner::RepoTaskPlanner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoSupervisorWakeReason {
    Manual,
    Watchdog,
    GitHubWebhook { delivery_id: String },
    JobCompleted { job_id: Uuid },
    RoutineTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSupervisorWake {
    pub project_id: Option<Uuid>,
    pub reason: RepoSupervisorWakeReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoSupervisorDecision {
    Idle,
    NeedsPlanning { project_id: Uuid },
    DispatchTask { project_id: Uuid, task_id: Uuid },
    WaitForCi { project_id: Uuid, task_id: Uuid },
    AwaitingReview { project_id: Uuid, task_id: Uuid },
    Merged { project_id: Uuid, task_id: Uuid },
    AwaitingHuman { project_id: Uuid, reason: String },
    Blocked { project_id: Uuid, reason: String },
    Completed { project_id: Uuid },
}

#[async_trait::async_trait]
pub trait RepoSupervisorStore: Send + Sync {
    async fn reconcile_project(
        &self,
        project_id: Option<Uuid>,
        reason: RepoSupervisorWakeReason,
    ) -> Result<Vec<RepoSupervisorDecision>, String>;

    /// Recover in-flight state after a supervisor restart: reconcile completed
    /// sandbox jobs and surface tasks that were left running with no worker
    /// record. The default is a no-op for non-database stores.
    async fn recover(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Default per-process ceilings, matching `RepoProjectsConfig::default`. Used
/// when the store is constructed without explicit limits (e.g. in tests).
const DEFAULT_MAX_CONCURRENT_PROJECTS: usize = 1;
const DEFAULT_MAX_CONCURRENT_TASKS_PER_PROJECT: usize = 1;

#[derive(Clone)]
pub struct DatabaseRepoSupervisorStore {
    db: Arc<dyn Database>,
    executor: Option<RepoProjectExecutor>,
    pipeline: Option<GitHubPipeline>,
    sse: Option<broadcast::Sender<SseEvent>>,
    planner: Option<Arc<dyn RepoTaskPlanner>>,
    /// Process-global ceiling on how many projects may advance dispatch per
    /// reconcile. Caps total host load across projects.
    max_concurrent_projects: usize,
    /// Process-global ceiling on concurrently-running tasks per project. Clamps
    /// the per-project `ProjectPolicy.max_parallel_tasks`.
    max_concurrent_tasks_per_project: usize,
}

impl DatabaseRepoSupervisorStore {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self {
            db,
            executor: None,
            pipeline: None,
            sse: None,
            planner: None,
            max_concurrent_projects: DEFAULT_MAX_CONCURRENT_PROJECTS,
            max_concurrent_tasks_per_project: DEFAULT_MAX_CONCURRENT_TASKS_PER_PROJECT,
        }
    }

    pub fn with_executor(mut self, executor: RepoProjectExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Wire the GitHub PR/CI/merge pipeline that advances `WaitingCi` and
    /// `WaitingReview` tasks. Without it those tasks simply wait.
    pub fn with_pipeline(mut self, pipeline: GitHubPipeline) -> Self {
        self.pipeline = Some(pipeline);
        self
    }

    pub fn with_sse(mut self, sse: Option<broadcast::Sender<SseEvent>>) -> Self {
        self.sse = sse;
        self
    }

    /// Inject the autonomous task planner. When absent, `NeedsPlanning`
    /// projects fall back to an explicit `AwaitingHuman` status instead of
    /// stalling silently.
    pub fn with_planner(mut self, planner: Option<Arc<dyn RepoTaskPlanner>>) -> Self {
        self.planner = planner;
        self
    }

    /// Set the process-global concurrency ceilings (typically from
    /// `RepoProjectsConfig`). Values below 1 are clamped to 1.
    pub fn with_limits(
        mut self,
        max_concurrent_projects: usize,
        max_concurrent_tasks_per_project: usize,
    ) -> Self {
        self.max_concurrent_projects = max_concurrent_projects.max(1);
        self.max_concurrent_tasks_per_project = max_concurrent_tasks_per_project.max(1);
        self
    }
}

#[async_trait::async_trait]
impl RepoSupervisorStore for DatabaseRepoSupervisorStore {
    async fn reconcile_project(
        &self,
        project_id: Option<Uuid>,
        reason: RepoSupervisorWakeReason,
    ) -> Result<Vec<RepoSupervisorDecision>, String> {
        let projects = if let Some(project_id) = project_id {
            self.db
                .get_repo_project(project_id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            self.db
                .list_repo_projects()
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|project| {
                    matches!(
                        project.state,
                        RepoProjectState::Draft
                            | RepoProjectState::Planning
                            | RepoProjectState::Active
                            | RepoProjectState::Blocked
                            | RepoProjectState::AwaitingHuman
                    )
                })
                .collect()
        };

        let mut decisions = Vec::new();
        // Process-global ceiling: how many projects may advance dispatch this
        // reconcile. A project that only logs Idle/Blocked does not consume a
        // slot; a project that dispatches a task or plans does.
        let mut projects_advanced = 0usize;
        for mut project in projects {
            let decisions_before = decisions.len();
            if let Some(executor) = self.executor.as_ref() {
                executor.sync_worker_runs(project.id).await?;
            }
            let mut tasks = self
                .db
                .list_repo_project_tasks(project.id)
                .await
                .map_err(|error| error.to_string())?;
            let repos = self
                .db
                .list_repo_project_repos(project.id)
                .await
                .map_err(|error| error.to_string())?;
            // Once the per-reconcile project budget is spent, stop advancing
            // dispatch/planning for additional projects; pipeline advancement of
            // already-in-flight tasks below is unaffected for the projects that
            // did get a slot.
            let dispatch_budget_available = projects_advanced < self.max_concurrent_projects;
            match project.state {
                RepoProjectState::Draft | RepoProjectState::Planning if tasks.is_empty() => {
                    decisions.push(RepoSupervisorDecision::NeedsPlanning {
                        project_id: project.id,
                    });
                    if dispatch_budget_available {
                        projects_advanced += 1;
                        self.plan_or_await_human(&mut project, &repos, &mut decisions)
                            .await?;
                    }
                }
                RepoProjectState::Active | RepoProjectState::Planning => {
                    // 1. Advance GitHub-driven tasks (PR/CI/review/merge). These
                    //    may transition WaitingCi -> Running (repair), -> Done
                    //    (merged), or -> Blocked.
                    for task in tasks.iter_mut() {
                        if matches!(
                            task.state,
                            RepoProjectTaskState::WaitingCi | RepoProjectTaskState::WaitingReview
                        ) {
                            self.advance_pipeline_task(&project, &repos, task, &mut decisions)
                                .await?;
                        }
                    }

                    // 2. Dispatch queued/ready tasks into sandbox workers, up to
                    //    the effective per-project concurrency cap, respecting
                    //    the per-reconcile project budget.
                    let has_dispatchable = tasks.iter().any(|task| {
                        matches!(
                            task.state,
                            RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
                        )
                    });
                    if has_dispatchable && dispatch_budget_available {
                        projects_advanced += 1;
                        self.dispatch_next_task(&mut project, &repos, &mut tasks, &mut decisions)
                            .await?;
                    } else if !has_dispatchable
                        && !tasks.is_empty()
                        && tasks
                            .iter()
                            .all(|task| task.state == RepoProjectTaskState::Done)
                    {
                        // 3. Every task merged/done — complete the project.
                        self.complete_project(&mut project, &tasks).await?;
                        decisions.push(RepoSupervisorDecision::Completed {
                            project_id: project.id,
                        });
                    }
                }
                RepoProjectState::Blocked | RepoProjectState::AwaitingHuman => {
                    decisions.push(RepoSupervisorDecision::Blocked {
                        project_id: project.id,
                        reason: format!(
                            "project is {} after {:?}",
                            state_label(project.state),
                            reason
                        ),
                    });
                }
                RepoProjectState::Paused
                | RepoProjectState::Completed
                | RepoProjectState::Failed
                | RepoProjectState::Cancelled => decisions.push(RepoSupervisorDecision::Idle),
                // Draft with a non-empty backlog (the empty case is handled by
                // the guarded arm above): normalize to Planning so the next
                // reconcile dispatches the existing tasks.
                RepoProjectState::Draft => {
                    decisions.push(RepoSupervisorDecision::NeedsPlanning {
                        project_id: project.id,
                    });
                    if dispatch_budget_available {
                        projects_advanced += 1;
                        self.transition_project_state(
                            &mut project,
                            RepoProjectState::Active,
                            "Repository project activated",
                        )
                        .await?;
                    }
                }
            }

            if decisions.len() == decisions_before {
                decisions.push(RepoSupervisorDecision::Idle);
            }
        }

        if decisions.is_empty() {
            decisions.push(RepoSupervisorDecision::Idle);
        }
        Ok(decisions)
    }

    async fn recover(&self) -> Result<(), String> {
        let projects = self
            .db
            .list_repo_projects()
            .await
            .map_err(|error| error.to_string())?;
        for project in projects {
            if !matches!(
                project.state,
                RepoProjectState::Draft
                    | RepoProjectState::Planning
                    | RepoProjectState::Active
                    | RepoProjectState::Blocked
                    | RepoProjectState::AwaitingHuman
            ) {
                continue;
            }
            // Reconcile any sandbox jobs that finished while the supervisor was
            // down.
            if let Some(executor) = self.executor.as_ref()
                && let Err(error) = executor.sync_worker_runs(project.id).await
            {
                tracing::warn!(project_id = %project.id, error = %error, "recovery worker-run sync failed");
            }
            let tasks = self
                .db
                .list_repo_project_tasks(project.id)
                .await
                .map_err(|error| error.to_string())?;
            let worker_runs = self
                .db
                .list_repo_worker_runs(project.id)
                .await
                .map_err(|error| error.to_string())?;
            for mut task in tasks {
                if task.state != RepoProjectTaskState::Running {
                    continue;
                }
                let has_worker_run = worker_runs.iter().any(|run| run.task_id == task.id);
                if !has_worker_run
                    && validate_task_state_transition(task.state, RepoProjectTaskState::Blocked)
                        .is_ok()
                {
                    task.state = RepoProjectTaskState::Blocked;
                    task.updated_at = Utc::now();
                    self.db
                        .upsert_repo_project_task(&task)
                        .await
                        .map_err(|error| error.to_string())?;
                    append_supervisor_event(
                        self.db.as_ref(),
                        project.id,
                        Some(task.repo_id),
                        Some(task.id),
                        RepoProjectEventKind::TaskStateChanged,
                        "Task was running with no worker record after restart; blocked for review",
                        serde_json::json!({ "to": "blocked", "reason": "orphaned_by_restart" }),
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }
}

impl DatabaseRepoSupervisorStore {
    /// Advance a single `WaitingCi`/`WaitingReview` task through the GitHub
    /// pipeline, dispatching a sandbox repair when CI fails and a repair is
    /// requested.
    async fn advance_pipeline_task(
        &self,
        project: &RepoProject,
        repos: &[RepoProjectRepo],
        task: &mut RepoProjectTask,
        decisions: &mut Vec<RepoSupervisorDecision>,
    ) -> Result<(), String> {
        let Some(pipeline) = self.pipeline.as_ref() else {
            // No GitHub pipeline configured — the task simply waits.
            decisions.push(RepoSupervisorDecision::WaitForCi {
                project_id: project.id,
                task_id: task.id,
            });
            return Ok(());
        };
        let Some(repo) = repos.iter().find(|repo| repo.id == task.repo_id) else {
            decisions.push(RepoSupervisorDecision::Blocked {
                project_id: project.id,
                reason: format!("task {} has no matching repo", task.id),
            });
            return Ok(());
        };

        match pipeline.advance_task(project, repo, task).await {
            Ok(PipelineOutcome::CiRepairRequested(suite)) => {
                if let Some(executor) = self.executor.as_ref() {
                    let mut project = project.clone();
                    match executor
                        .redispatch_repair_task(&mut project, repo, task, &suite)
                        .await
                    {
                        Ok(_) => decisions.push(RepoSupervisorDecision::DispatchTask {
                            project_id: project.id,
                            task_id: task.id,
                        }),
                        Err(error) => decisions.push(RepoSupervisorDecision::Blocked {
                            project_id: project.id,
                            reason: error,
                        }),
                    }
                } else {
                    decisions.push(RepoSupervisorDecision::Blocked {
                        project_id: project.id,
                        reason: "CI repair requested but no sandbox executor is available"
                            .to_string(),
                    });
                }
            }
            Ok(PipelineOutcome::ReviewRequested { backend }) => {
                if let Some(executor) = self.executor.as_ref()
                    && let Err(error) = executor
                        .dispatch_review_task(project, repo, task, backend)
                        .await
                {
                    tracing::warn!(error = %error, "failed to dispatch sandbox review task");
                }
                decisions.push(RepoSupervisorDecision::AwaitingReview {
                    project_id: project.id,
                    task_id: task.id,
                });
            }
            Ok(outcome) => decisions.push(pipeline_decision(project.id, task.id, &outcome)),
            Err(error) => decisions.push(RepoSupervisorDecision::Blocked {
                project_id: project.id,
                reason: error,
            }),
        }
        Ok(())
    }

    /// Dispatch `Queued`/`Ready` tasks into sandbox workers, up to the effective
    /// per-project concurrency cap.
    ///
    /// The cap is the persisted per-project `ProjectPolicy.max_parallel_tasks`
    /// clamped by the process-global `max_concurrent_tasks_per_project` ceiling
    /// (the per-project knob is authoritative, the env knob caps host load). We
    /// count tasks already in `Running` (CI/review states are GitHub-bound, not
    /// sandbox-bound, so they do not consume a worker slot) and dispatch until
    /// the cap is reached or no dispatchable task remains. Counting actual
    /// `Running` tasks — rather than relying on a one-dispatch-per-tick cadence —
    /// keeps the limit correct across consecutive reconciles and faster wakes.
    async fn dispatch_next_task(
        &self,
        project: &mut RepoProject,
        repos: &[RepoProjectRepo],
        tasks: &mut [RepoProjectTask],
        decisions: &mut Vec<RepoSupervisorDecision>,
    ) -> Result<(), String> {
        let cap = (project.policy.max_parallel_tasks as usize)
            .min(self.max_concurrent_tasks_per_project)
            .max(1);
        let mut running = tasks
            .iter()
            .filter(|task| task.state == RepoProjectTaskState::Running)
            .count();

        // Already at (or over) the effective concurrency cap: nothing to do this
        // tick. Returning here keeps the cap honored across consecutive
        // reconciles (it does not rely on the one-dispatch-per-tick cadence).
        if running >= cap {
            return Ok(());
        }
        // No dispatchable task: nothing to do.
        let has_dispatchable = tasks.iter().any(|task| {
            matches!(
                task.state,
                RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
            )
        });
        if !has_dispatchable {
            return Ok(());
        }

        let Some(executor) = self.executor.as_ref() else {
            decisions.push(RepoSupervisorDecision::AwaitingHuman {
                project_id: project.id,
                reason: "no sandbox executor available to dispatch tasks".to_string(),
            });
            return Ok(());
        };

        while running < cap {
            let Some(pos) = tasks.iter().position(|task| {
                matches!(
                    task.state,
                    RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
                )
            }) else {
                break;
            };
            let task_id = tasks[pos].id;
            let repo_id = tasks[pos].repo_id;

            let Some(repo) = repos.iter().find(|repo| repo.id == repo_id) else {
                append_supervisor_event(
                    self.db.as_ref(),
                    project.id,
                    None,
                    Some(task_id),
                    RepoProjectEventKind::TaskStateChanged,
                    "Repository task has no matching enrolled repo",
                    serde_json::json!({ "task_id": task_id }),
                )
                .await?;
                decisions.push(RepoSupervisorDecision::Blocked {
                    project_id: project.id,
                    reason: format!("task {task_id} has no matching repo"),
                });
                // Mark the orphaned task non-dispatchable for this pass so the
                // loop does not spin on it; the next reconcile re-evaluates.
                tasks[pos].state = RepoProjectTaskState::Blocked;
                continue;
            };

            let mut dispatch_task = tasks[pos].clone();
            let result = executor
                .dispatch_task(project, repo, &mut dispatch_task)
                .await;
            // Write the (possibly mutated) task back so the next iteration's
            // position scan does not re-select it. A successful dispatch moves
            // the task to `Running`; an adopted pre-existing worker may leave it
            // unchanged, so guard against re-selecting an unmoved task below.
            let still_dispatchable = matches!(
                dispatch_task.state,
                RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
            );
            match result {
                Ok(Some(outcome)) => {
                    append_supervisor_event(
                        self.db.as_ref(),
                        project.id,
                        Some(repo.id),
                        Some(dispatch_task.id),
                        RepoProjectEventKind::TaskStateChanged,
                        "Repository task dispatched to sandbox worker",
                        serde_json::json!({
                            "job_id": outcome.job_id,
                            "worker_run_id": outcome.worker_run_id,
                            "mode": outcome.mode.as_str(),
                        }),
                    )
                    .await?;
                    tasks[pos] = dispatch_task;
                    decisions.push(RepoSupervisorDecision::DispatchTask {
                        project_id: project.id,
                        task_id,
                    });
                }
                Ok(None) => {
                    tasks[pos] = dispatch_task;
                    decisions.push(RepoSupervisorDecision::DispatchTask {
                        project_id: project.id,
                        task_id,
                    });
                }
                Err(error) => {
                    decisions.push(RepoSupervisorDecision::Blocked {
                        project_id: project.id,
                        reason: error,
                    });
                    // Avoid re-selecting the same task in this loop.
                    tasks[pos].state = RepoProjectTaskState::Blocked;
                    continue;
                }
            }
            // If the task is still Queued/Ready (e.g. an already-active worker was
            // adopted without a state change), it still occupies a sandbox slot;
            // stop here rather than spin re-selecting it this pass.
            if still_dispatchable {
                break;
            }
            running += 1;
        }
        Ok(())
    }

    /// Act on a `NeedsPlanning` project. With a planner wired, decompose the
    /// project goal into `Queued` tasks and move the project to `Active`. Without
    /// a planner (or when planning yields nothing / errors), transition the
    /// project to `AwaitingHuman` so it never silently stalls in `Planning`.
    ///
    /// Idempotent: the caller only invokes this while the project has no tasks,
    /// and we re-check, so a burst of wakes cannot duplicate tasks.
    async fn plan_or_await_human(
        &self,
        project: &mut RepoProject,
        repos: &[RepoProjectRepo],
        decisions: &mut Vec<RepoSupervisorDecision>,
    ) -> Result<(), String> {
        // Defensive idempotency guard: never plan over an existing backlog.
        let existing = self
            .db
            .list_repo_project_tasks(project.id)
            .await
            .map_err(|error| error.to_string())?;
        if !existing.is_empty() {
            return Ok(());
        }

        // Normalize Draft -> Planning so the subsequent transitions are valid
        // (Draft cannot move directly to AwaitingHuman).
        if project.state == RepoProjectState::Draft {
            self.transition_project_state(
                project,
                RepoProjectState::Planning,
                "Repository project planning started",
            )
            .await?;
        }

        let planned = match self.planner.as_ref() {
            Some(planner) => match planner.plan(project, repos).await {
                Ok(planned) => planned,
                Err(error) => {
                    tracing::warn!(project_id = %project.id, error = %error, "repo task planner failed");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        if planned.is_empty() {
            // No planner, or nothing to plan: surface an actionable human-facing
            // status rather than leaving the project parked in Planning forever.
            self.transition_project_state(
                project,
                RepoProjectState::AwaitingHuman,
                "Project needs a plan; add tasks to proceed.",
            )
            .await?;
            decisions.push(RepoSupervisorDecision::AwaitingHuman {
                project_id: project.id,
                reason: "no planner configured; awaiting human-provided tasks".to_string(),
            });
            return Ok(());
        }

        // Persist each planned task as a Queued task, recording a TaskCreated
        // event + SSE in lockstep so the WebUI consumer reflects the new backlog.
        let mut created = 0usize;
        for task_draft in planned {
            let Some(repo) = repos.iter().find(|repo| repo.id == task_draft.repo_id) else {
                tracing::warn!(
                    project_id = %project.id,
                    repo_id = %task_draft.repo_id,
                    "planner returned a task for an unknown repo; dropping"
                );
                continue;
            };
            let task = crate::repo_projects::build_queued_task(
                project,
                repo,
                task_draft.title,
                task_draft.body,
                0,
                Vec::new(),
            )?;
            self.db
                .upsert_repo_project_task(&task)
                .await
                .map_err(|error| error.to_string())?;
            append_supervisor_event(
                self.db.as_ref(),
                project.id,
                Some(repo.id),
                Some(task.id),
                RepoProjectEventKind::TaskCreated,
                "Repository project task planned",
                serde_json::json!({
                    "title": task.title,
                    "branch_name": task.branch_name,
                    "source": "planner",
                }),
            )
            .await?;
            self.emit_task(&task, "Task planned");
            decisions.push(RepoSupervisorDecision::DispatchTask {
                project_id: project.id,
                task_id: task.id,
            });
            created += 1;
        }

        if created == 0 {
            // Every planned task referenced an unknown repo — fall back to human.
            self.transition_project_state(
                project,
                RepoProjectState::AwaitingHuman,
                "Planner produced no usable tasks; add tasks to proceed.",
            )
            .await?;
            decisions.push(RepoSupervisorDecision::AwaitingHuman {
                project_id: project.id,
                reason: "planner produced no usable tasks".to_string(),
            });
            return Ok(());
        }

        // Move the project to Active so the next reconcile dispatches the new
        // backlog.
        self.transition_project_state(
            project,
            RepoProjectState::Active,
            "Repository project plan ready",
        )
        .await?;
        Ok(())
    }

    /// Transition a project to `next`, persisting the change, appending a
    /// `ProjectStateChanged` event, and broadcasting SSE in lockstep. A no-op
    /// when the transition is invalid (logged) or already in the target state.
    async fn transition_project_state(
        &self,
        project: &mut RepoProject,
        next: RepoProjectState,
        message: &str,
    ) -> Result<(), String> {
        if project.state == next {
            return Ok(());
        }
        if validate_project_state_transition(project.state, next).is_err() {
            tracing::warn!(
                project_id = %project.id,
                from = state_label(project.state),
                to = state_label(next),
                "skipping invalid project state transition"
            );
            return Ok(());
        }
        let now = Utc::now();
        let from = project.state;
        project.state = next;
        project.updated_at = now;
        if next == RepoProjectState::Active && project.started_at.is_none() {
            project.started_at = Some(now);
        }
        self.db
            .update_repo_project(project)
            .await
            .map_err(|error| error.to_string())?;
        append_supervisor_event(
            self.db.as_ref(),
            project.id,
            None,
            None,
            RepoProjectEventKind::ProjectStateChanged,
            message,
            serde_json::json!({
                "from": state_label(from),
                "to": state_label(next),
            }),
        )
        .await?;
        self.emit_project(project, message);
        Ok(())
    }

    async fn complete_project(
        &self,
        project: &mut RepoProject,
        tasks: &[RepoProjectTask],
    ) -> Result<(), String> {
        if validate_project_state_transition(project.state, RepoProjectState::Completed).is_err() {
            return Ok(());
        }
        let now = Utc::now();

        // Close the durable run record, recording final task tallies.
        if let Some(run_id) = project.current_run_id
            && let Some(mut run) = self
                .db
                .get_repo_project_run(run_id)
                .await
                .map_err(|error| error.to_string())?
        {
            run.state = RepoProjectRunState::Completed;
            run.tasks_seen = tasks.len() as u32;
            run.tasks_completed = tasks
                .iter()
                .filter(|task| task.state == RepoProjectTaskState::Done)
                .count() as u32;
            run.tasks_failed = tasks
                .iter()
                .filter(|task| {
                    matches!(
                        task.state,
                        RepoProjectTaskState::Failed | RepoProjectTaskState::Cancelled
                    )
                })
                .count() as u32;
            run.completed_at = Some(now);
            self.db
                .upsert_repo_project_run(&run)
                .await
                .map_err(|error| error.to_string())?;
            append_supervisor_event(
                self.db.as_ref(),
                project.id,
                None,
                None,
                RepoProjectEventKind::ProjectRunCompleted,
                "Repository project run completed",
                serde_json::json!({
                    "run_id": run_id,
                    "tasks_completed": run.tasks_completed,
                    "tasks_failed": run.tasks_failed,
                }),
            )
            .await?;
        }

        project.state = RepoProjectState::Completed;
        project.completed_at = Some(now);
        project.updated_at = now;
        self.db
            .update_repo_project(project)
            .await
            .map_err(|error| error.to_string())?;
        append_supervisor_event(
            self.db.as_ref(),
            project.id,
            None,
            None,
            RepoProjectEventKind::ProjectStateChanged,
            "All repository tasks complete",
            serde_json::json!({ "state": "completed" }),
        )
        .await?;
        self.emit_project(project, "Project completed");
        Ok(())
    }

    fn emit_project(&self, project: &RepoProject, message: &str) {
        if let Some(sender) = self.sse.as_ref() {
            let _ = sender.send(SseEvent::RepoProjectUpdated {
                project_id: project.id.to_string(),
                state: state_label(project.state).to_string(),
                message: message.to_string(),
            });
        }
    }

    fn emit_task(&self, task: &RepoProjectTask, message: &str) {
        if let Some(sender) = self.sse.as_ref() {
            let _ = sender.send(SseEvent::RepoTaskUpdated {
                project_id: task.project_id.to_string(),
                task_id: task.id.to_string(),
                state: task_state_label(task.state).to_string(),
                message: message.to_string(),
            });
        }
    }
}

fn pipeline_decision(
    project_id: Uuid,
    task_id: Uuid,
    outcome: &PipelineOutcome,
) -> RepoSupervisorDecision {
    match outcome {
        PipelineOutcome::Skipped => RepoSupervisorDecision::Idle,
        PipelineOutcome::WaitingForCi => RepoSupervisorDecision::WaitForCi {
            project_id,
            task_id,
        },
        PipelineOutcome::PullRequestMissing => RepoSupervisorDecision::AwaitingHuman {
            project_id,
            reason: format!("task {task_id} branch was not pushed to origin"),
        },
        PipelineOutcome::AdvancedToReview | PipelineOutcome::MergeGateRecorded { .. } => {
            RepoSupervisorDecision::AwaitingReview {
                project_id,
                task_id,
            }
        }
        PipelineOutcome::CiRepairRequested(_) | PipelineOutcome::ReviewRequested { .. } => {
            RepoSupervisorDecision::DispatchTask {
                project_id,
                task_id,
            }
        }
        PipelineOutcome::Merged { .. } => RepoSupervisorDecision::Merged {
            project_id,
            task_id,
        },
        PipelineOutcome::AwaitingHuman { reason } => RepoSupervisorDecision::AwaitingHuman {
            project_id,
            reason: reason.clone(),
        },
    }
}

#[derive(Clone)]
pub struct ProjectSupervisor {
    store: Arc<dyn RepoSupervisorStore>,
    wake_tx: mpsc::Sender<RepoSupervisorWake>,
}

impl ProjectSupervisor {
    pub fn new(
        store: Arc<dyn RepoSupervisorStore>,
        buffer: usize,
    ) -> (Self, mpsc::Receiver<RepoSupervisorWake>) {
        let (wake_tx, wake_rx) = mpsc::channel(buffer.max(1));
        (Self { store, wake_tx }, wake_rx)
    }

    pub async fn wake(
        &self,
        project_id: Option<Uuid>,
        reason: RepoSupervisorWakeReason,
    ) -> Result<(), String> {
        self.wake_tx
            .send(RepoSupervisorWake { project_id, reason })
            .await
            .map_err(|error| format!("repo project supervisor is not running: {error}"))
    }

    pub fn store(&self) -> &Arc<dyn RepoSupervisorStore> {
        &self.store
    }
}

pub async fn run_project_supervisor_loop(
    store: Arc<dyn RepoSupervisorStore>,
    mut wake_rx: mpsc::Receiver<RepoSupervisorWake>,
    watchdog_interval: Duration,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut watchdog = tokio::time::interval(watchdog_interval);
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Restart recovery: reconcile jobs that finished while we were down and
    // surface orphaned in-flight tasks before steady-state reconciliation.
    if let Err(error) = store.recover().await {
        tracing::warn!(error = %error, "repo project supervisor recovery failed");
    }

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("repo project supervisor shutting down");
                return;
            }
            _ = watchdog.tick() => {
                if let Err(error) = reconcile_once(&store, None, RepoSupervisorWakeReason::Watchdog).await {
                    tracing::warn!(error = %error, "repo project watchdog reconcile failed");
                }
            }
            maybe_wake = wake_rx.recv() => {
                let Some(wake) = maybe_wake else {
                    tracing::info!("repo project supervisor wake channel closed");
                    return;
                };
                if let Err(error) = reconcile_once(&store, wake.project_id, wake.reason).await {
                    tracing::warn!(error = %error, "repo project reconcile failed");
                }
            }
        }
    }
}

async fn reconcile_once(
    store: &Arc<dyn RepoSupervisorStore>,
    project_id: Option<Uuid>,
    reason: RepoSupervisorWakeReason,
) -> Result<(), String> {
    let decisions = store.reconcile_project(project_id, reason).await?;
    for decision in decisions {
        tracing::info!(?decision, "repo project supervisor decision");
    }
    Ok(())
}

fn state_label(state: RepoProjectState) -> &'static str {
    match state {
        RepoProjectState::Draft => "draft",
        RepoProjectState::Planning => "planning",
        RepoProjectState::Active => "active",
        RepoProjectState::Blocked => "blocked",
        RepoProjectState::Paused => "paused",
        RepoProjectState::AwaitingHuman => "awaiting_human",
        RepoProjectState::Completed => "completed",
        RepoProjectState::Failed => "failed",
        RepoProjectState::Cancelled => "cancelled",
    }
}

fn task_state_label(state: RepoProjectTaskState) -> &'static str {
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

async fn append_supervisor_event(
    db: &dyn Database,
    project_id: Uuid,
    repo_id: Option<Uuid>,
    task_id: Option<Uuid>,
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
        worker_run_id: None,
        kind,
        message: message.to_string(),
        details,
        created_at: chrono::Utc::now(),
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestStore {
        calls: Mutex<Vec<RepoSupervisorWakeReason>>,
    }

    #[async_trait::async_trait]
    impl RepoSupervisorStore for TestStore {
        async fn reconcile_project(
            &self,
            _project_id: Option<Uuid>,
            reason: RepoSupervisorWakeReason,
        ) -> Result<Vec<RepoSupervisorDecision>, String> {
            self.calls.lock().unwrap().push(reason);
            Ok(vec![RepoSupervisorDecision::Idle])
        }
    }

    #[tokio::test]
    async fn supervisor_enqueues_manual_wake() {
        let store = Arc::new(TestStore::default());
        let (supervisor, mut rx) = ProjectSupervisor::new(store, 4);
        supervisor
            .wake(None, RepoSupervisorWakeReason::Manual)
            .await
            .unwrap();
        let wake = rx.recv().await.unwrap();
        assert_eq!(wake.reason, RepoSupervisorWakeReason::Manual);
    }
}

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

#[derive(Clone)]
pub struct DatabaseRepoSupervisorStore {
    db: Arc<dyn Database>,
    executor: Option<RepoProjectExecutor>,
    pipeline: Option<GitHubPipeline>,
    sse: Option<broadcast::Sender<SseEvent>>,
}

impl DatabaseRepoSupervisorStore {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self {
            db,
            executor: None,
            pipeline: None,
            sse: None,
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
            match project.state {
                RepoProjectState::Draft | RepoProjectState::Planning if tasks.is_empty() => {
                    decisions.push(RepoSupervisorDecision::NeedsPlanning {
                        project_id: project.id,
                    });
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

                    // 2. Dispatch one queued/ready task into a sandbox worker.
                    let has_dispatchable = tasks.iter().any(|task| {
                        matches!(
                            task.state,
                            RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
                        )
                    });
                    if has_dispatchable {
                        self.dispatch_next_task(&mut project, &repos, &mut tasks, &mut decisions)
                            .await?;
                    } else if !tasks.is_empty()
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
                RepoProjectState::Draft => decisions.push(RepoSupervisorDecision::NeedsPlanning {
                    project_id: project.id,
                }),
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
            Ok(outcome) => decisions.push(pipeline_decision(project.id, task.id, &outcome)),
            Err(error) => decisions.push(RepoSupervisorDecision::Blocked {
                project_id: project.id,
                reason: error,
            }),
        }
        Ok(())
    }

    /// Dispatch the first `Queued`/`Ready` task into a sandbox worker.
    async fn dispatch_next_task(
        &self,
        project: &mut RepoProject,
        repos: &[RepoProjectRepo],
        tasks: &mut [RepoProjectTask],
        decisions: &mut Vec<RepoSupervisorDecision>,
    ) -> Result<(), String> {
        let Some(pos) = tasks.iter().position(|task| {
            matches!(
                task.state,
                RepoProjectTaskState::Queued | RepoProjectTaskState::Ready
            )
        }) else {
            return Ok(());
        };
        let task_id = tasks[pos].id;
        let repo_id = tasks[pos].repo_id;

        let Some(executor) = self.executor.as_ref() else {
            decisions.push(RepoSupervisorDecision::AwaitingHuman {
                project_id: project.id,
                reason: "no sandbox executor available to dispatch tasks".to_string(),
            });
            return Ok(());
        };
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
            return Ok(());
        };

        let mut dispatch_task = tasks[pos].clone();
        match executor
            .dispatch_task(project, repo, &mut dispatch_task)
            .await
        {
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
            Ok(None) => decisions.push(RepoSupervisorDecision::DispatchTask {
                project_id: project.id,
                task_id,
            }),
            Err(error) => decisions.push(RepoSupervisorDecision::Blocked {
                project_id: project.id,
                reason: error,
            }),
        }
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
        PipelineOutcome::CiRepairRequested(_) => RepoSupervisorDecision::DispatchTask {
            project_id,
            task_id,
        },
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

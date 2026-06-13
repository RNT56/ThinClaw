//! End-to-end tests for the GitHub PR/CI/merge pipeline against an in-process
//! fake GitHub server. These exercise the full single-repo loop the supervisor
//! drives: ensure PR -> failing CI -> green CI -> merge-gate two-phase ->
//! guarded squash merge, plus the auto-merge-disabled and restart-recovery
//! paths. No real network or Docker is involved.

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde_json::{Value, json};
use thinclaw_repo_projects::{
    CodingBackend, GitHubAuthMode, MergeMethod, ProjectPolicy, RepoProject, RepoProjectRepo,
    RepoProjectRun, RepoProjectRunState, RepoProjectState, RepoProjectTask, RepoProjectTaskState,
    RepoWorkerRun, RepoWorkerRunState,
};
use uuid::Uuid;

use crate::db::Database;
use crate::testing::test_db;

use super::github_provider::FixedTokenGitHubClientProvider;
use super::pipeline::{GitHubPipeline, PipelineConfig, PipelineOutcome};
use super::supervisor::{
    DatabaseRepoSupervisorStore, RepoSupervisorStore, RepoSupervisorWakeReason,
};

const HEAD_SHA: &str = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
const MERGE_SHA: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";

#[derive(Default)]
struct FakeGitHubState {
    ci_green: bool,
    merges: u32,
    comments: u32,
    deleted_branch: bool,
}

type SharedFake = Arc<Mutex<FakeGitHubState>>;

/// Spawn the fake GitHub server on an ephemeral port and return its base URL.
async fn spawn_fake_github(state: SharedFake) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake github listener");
    let addr = listener.local_addr().expect("local addr");
    let app = Router::new().fallback(fake_github).with_state(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn fake_github(State(state): State<SharedFake>, method: Method, uri: Uri) -> Response {
    let path = uri.path().to_string();
    let mut fake = state.lock().expect("fake github mutex");

    // Order matters: match the most specific suffixes first.
    if method == Method::POST && path.ends_with("/comments") {
        fake.comments += 1;
        return json_ok(json!({ "id": 1, "body": "triage" }));
    }
    if path.ends_with("/reviews") {
        return json_ok(json!([]));
    }
    if method == Method::PUT && path.ends_with("/merge") {
        fake.merges += 1;
        return json_ok(json!({
            "sha": MERGE_SHA,
            "merged": true,
            "message": "Pull Request successfully merged"
        }));
    }
    if path.ends_with("/check-runs") {
        let conclusion = if fake.ci_green { "success" } else { "failure" };
        return json_ok(json!({
            "total_count": 1,
            "check_runs": [{
                "id": 1,
                "name": "test",
                "head_sha": HEAD_SHA,
                "status": "completed",
                "conclusion": conclusion,
                "output": { "summary": "test result: FAILED. assertion failed" }
            }]
        }));
    }
    if path.contains("/compare/") {
        return json_ok(json!({
            "status": "ahead",
            "ahead_by": 1,
            "behind_by": 0,
            "total_commits": 1
        }));
    }
    if path.ends_with("/actions/runs") {
        return json_ok(json!({ "total_count": 0, "workflow_runs": [] }));
    }
    if method == Method::DELETE && path.contains("/git/refs/") {
        fake.deleted_branch = true;
        return StatusCode::NO_CONTENT.into_response();
    }
    if path.contains("/git/ref/") {
        return json_ok(json!({
            "ref": "refs/heads/thinclaw/proj/abc123",
            "node_id": "REF",
            "url": "http://example/ref",
            "object": { "type": "commit", "sha": HEAD_SHA, "url": "http://example/commit" }
        }));
    }
    if method == Method::POST && path.ends_with("/pulls") {
        return json_ok(pull_request_json(fake.merges > 0));
    }
    if method == Method::GET && path.ends_with("/pulls") {
        // Discovery: no pre-existing PR, force the create path.
        return json_ok(json!([]));
    }
    if path.contains("/pulls/") {
        return json_ok(pull_request_json(fake.merges > 0));
    }

    (
        StatusCode::NOT_FOUND,
        Json(json!({ "message": "not found" })),
    )
        .into_response()
}

fn pull_request_json(merged: bool) -> Value {
    json!({
        "id": 1,
        "number": 42,
        "state": if merged { "closed" } else { "open" },
        "title": "Fix CI",
        "head": { "ref": "thinclaw/proj/abc123", "sha": HEAD_SHA },
        "base": { "ref": "main", "sha": "basesha" },
        "html_url": "http://example/pull/42",
        "mergeable": true,
        "merged": merged,
        "merged_at": if merged { Some(Utc::now().to_rfc3339()) } else { None }
    })
}

fn json_ok(value: Value) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

fn sample_project(auto_merge: bool) -> RepoProject {
    let now = Utc::now();
    RepoProject {
        id: Uuid::new_v4(),
        slug: "proj".to_string(),
        name: "Proj".to_string(),
        state: RepoProjectState::Active,
        policy: ProjectPolicy {
            auto_merge,
            merge_method: MergeMethod::Squash,
            default_coding_backend: CodingBackend::CodexCode,
            github_auth_mode: GitHubAuthMode::UserToken,
            max_parallel_tasks: 1,
        },
        description: None,
        current_run_id: None,
        created_at: now,
        updated_at: now,
        started_at: Some(now),
        completed_at: None,
    }
}

fn sample_repo(project_id: Uuid) -> RepoProjectRepo {
    let now = Utc::now();
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
        metadata: json!({}),
        created_at: now,
        updated_at: now,
    }
}

fn sample_task(project_id: Uuid, repo_id: Uuid, state: RepoProjectTaskState) -> RepoProjectTask {
    let now = Utc::now();
    RepoProjectTask {
        id: Uuid::new_v4(),
        project_id,
        repo_id,
        title: "Fix CI".to_string(),
        body: Some("Make the tests pass.".to_string()),
        state,
        coding_backend: CodingBackend::CodexCode,
        base_branch: "main".to_string(),
        branch_name: "thinclaw/proj/abc123".to_string(),
        head_sha: None,
        pull_request_number: None,
        pull_request_url: None,
        github_issue_number: None,
        assigned_worker_id: None,
        priority: 0,
        labels: vec![],
        metadata: json!({}),
        created_at: now,
        updated_at: now,
        queued_at: Some(now),
        started_at: Some(now),
        completed_at: None,
    }
}

async fn seed(
    db: &Arc<dyn Database>,
    auto_merge: bool,
    task_state: RepoProjectTaskState,
) -> (RepoProject, RepoProjectRepo, Uuid) {
    let project = sample_project(auto_merge);
    let repo = sample_repo(project.id);
    let task = sample_task(project.id, repo.id, task_state);
    let task_id = task.id;
    db.create_repo_project(&project).await.unwrap();
    db.upsert_repo_project_repo(&repo).await.unwrap();
    db.upsert_repo_project_task(&task).await.unwrap();
    (project, repo, task_id)
}

fn pipeline(db: Arc<dyn Database>, base_url: &str) -> GitHubPipeline {
    pipeline_with(db, base_url, PipelineConfig::default())
}

fn pipeline_with(db: Arc<dyn Database>, base_url: &str, config: PipelineConfig) -> GitHubPipeline {
    let provider = Arc::new(FixedTokenGitHubClientProvider::new(base_url, "test-token"));
    GitHubPipeline::new(db, provider, config)
}

#[tokio::test]
async fn full_pr_loop_ensures_pr_repairs_ci_then_auto_merges_exactly_once() {
    let (db, _guard) = test_db().await;
    let fake = SharedFake::default();
    let base_url = spawn_fake_github(Arc::clone(&fake)).await;
    let (project, repo, task_id) = seed(&db, true, RepoProjectTaskState::WaitingCi).await;
    let pipeline = pipeline(Arc::clone(&db), &base_url);

    // 1. CI is failing: ensure a PR exists, classify, request a repair.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::CiRepairRequested(_)),
        "expected CiRepairRequested, got {outcome:?}"
    );
    assert_eq!(task.pull_request_number, Some(42), "PR should be opened");
    assert_eq!(task.state, RepoProjectTaskState::WaitingCi);
    assert_eq!(fake.lock().unwrap().comments, 1, "CI triage comment posted");
    assert_eq!(fake.lock().unwrap().merges, 0, "no merge while CI red");

    // 2. CI goes green: advance to review.
    fake.lock().unwrap().ci_green = true;
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::AdvancedToReview),
        "expected AdvancedToReview, got {outcome:?}"
    );
    assert_eq!(task.state, RepoProjectTaskState::WaitingReview);

    // 3. First review pass records the merge-gate audit event (gate_event_missing).
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            PipelineOutcome::MergeGateRecorded { approved: false }
        ),
        "expected first-pass gate record, got {outcome:?}"
    );
    assert_eq!(
        fake.lock().unwrap().merges,
        0,
        "no merge before gate recorded"
    );

    // 4. Second review pass: gate is satisfied, guarded auto-merge fires.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::Merged { .. }),
        "expected Merged, got {outcome:?}"
    );
    assert_eq!(task.state, RepoProjectTaskState::Done);
    assert_eq!(fake.lock().unwrap().merges, 1, "merged exactly once");
    assert!(
        fake.lock().unwrap().deleted_branch,
        "branch deleted after merge"
    );

    // 5. Idempotency: a further tick must not merge again.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::Skipped),
        "done task should be skipped, got {outcome:?}"
    );
    assert_eq!(fake.lock().unwrap().merges, 1, "no duplicate merge");
}

#[tokio::test]
async fn auto_merge_disabled_holds_for_human_and_never_merges() {
    let (db, _guard) = test_db().await;
    let fake = SharedFake::default();
    fake.lock().unwrap().ci_green = true;
    let base_url = spawn_fake_github(Arc::clone(&fake)).await;
    let (project, repo, task_id) = seed(&db, false, RepoProjectTaskState::WaitingReview).await;

    // Pre-set the PR number so the task is squarely in review.
    let mut seeded = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    seeded.pull_request_number = Some(42);
    seeded.head_sha = Some(HEAD_SHA.to_string());
    db.upsert_repo_project_task(&seeded).await.unwrap();

    let pipeline = pipeline(Arc::clone(&db), &base_url);

    // First pass records the gate (denied: auto_merge_disabled + gate_event_missing).
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            PipelineOutcome::MergeGateRecorded { approved: false }
        ),
        "expected gate record, got {outcome:?}"
    );

    // Second pass: only auto_merge_disabled remains -> awaiting human, never merged.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::AwaitingHuman { .. }),
        "expected AwaitingHuman, got {outcome:?}"
    );
    assert_eq!(
        fake.lock().unwrap().merges,
        0,
        "auto-merge disabled never merges"
    );
    assert_eq!(task.state, RepoProjectTaskState::WaitingReview);
}

#[tokio::test]
async fn recovery_blocks_running_task_without_worker_record() {
    let (db, _guard) = test_db().await;
    let (_project, _repo, task_id) = seed(&db, false, RepoProjectTaskState::Running).await;

    // No worker runs were recorded for the running task (simulating a crash).
    let store = DatabaseRepoSupervisorStore::new(Arc::clone(&db));
    store.recover().await.unwrap();

    let task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.state,
        RepoProjectTaskState::Blocked,
        "orphaned running task should be blocked on recovery"
    );
}

#[tokio::test]
async fn supervisor_reconcile_drives_waiting_ci_to_merge_and_completes_run() {
    let (db, _guard) = test_db().await;
    let fake = SharedFake::default();
    fake.lock().unwrap().ci_green = true;
    let base_url = spawn_fake_github(Arc::clone(&fake)).await;

    // Seed an active, auto-merge project with an open run and one waiting-CI task.
    let mut project = sample_project(true);
    let run_id = Uuid::new_v4();
    project.current_run_id = Some(run_id);
    let repo = sample_repo(project.id);
    let task = sample_task(project.id, repo.id, RepoProjectTaskState::WaitingCi);
    let task_id = task.id;
    db.create_repo_project(&project).await.unwrap();
    db.upsert_repo_project_repo(&repo).await.unwrap();
    db.upsert_repo_project_task(&task).await.unwrap();
    db.upsert_repo_project_run(&RepoProjectRun {
        id: run_id,
        project_id: project.id,
        state: RepoProjectRunState::Running,
        trigger: "test".to_string(),
        summary: None,
        tasks_seen: 0,
        tasks_queued: 0,
        tasks_completed: 0,
        tasks_failed: 0,
        metadata: json!({}),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: None,
    })
    .await
    .unwrap();

    let store = DatabaseRepoSupervisorStore::new(Arc::clone(&db))
        .with_pipeline(pipeline(Arc::clone(&db), &base_url));

    // Drive the reconcile loop: WaitingCi -> WaitingReview -> gate -> merge -> done -> project complete.
    for _ in 0..6 {
        store
            .reconcile_project(Some(project.id), RepoSupervisorWakeReason::Manual)
            .await
            .unwrap();
    }

    let task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.state,
        RepoProjectTaskState::Done,
        "task should be merged/done"
    );
    assert_eq!(
        fake.lock().unwrap().merges,
        1,
        "merged exactly once via reconcile"
    );

    let project = db.get_repo_project(project.id).await.unwrap().unwrap();
    assert_eq!(
        project.state,
        RepoProjectState::Completed,
        "project should complete once all tasks are done"
    );

    let run = db.get_repo_project_run(run_id).await.unwrap().unwrap();
    assert_eq!(
        run.state,
        RepoProjectRunState::Completed,
        "run should be closed"
    );
    assert_eq!(
        run.tasks_completed, 1,
        "run should record the completed task"
    );
}

#[tokio::test]
async fn review_summary_comment_is_posted_once_per_head_sha_when_enabled() {
    let (db, _guard) = test_db().await;
    let fake = SharedFake::default();
    fake.lock().unwrap().ci_green = true;
    let base_url = spawn_fake_github(Arc::clone(&fake)).await;
    // auto_merge off so the task stays in review and the summary path is exercised.
    let (project, repo, task_id) = seed(&db, false, RepoProjectTaskState::WaitingReview).await;

    let mut seeded = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    seeded.pull_request_number = Some(42);
    seeded.head_sha = Some(HEAD_SHA.to_string());
    db.upsert_repo_project_task(&seeded).await.unwrap();

    let pipeline = pipeline_with(
        Arc::clone(&db),
        &base_url,
        PipelineConfig {
            post_review_summary: true,
            ..PipelineConfig::default()
        },
    );

    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    let after_first = fake.lock().unwrap().comments;
    assert_eq!(after_first, 1, "review-readiness summary posted once");

    // Second pass on the same head SHA must not post again.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert_eq!(
        fake.lock().unwrap().comments,
        after_first,
        "summary is one-shot per head SHA"
    );
}

#[tokio::test]
async fn review_requested_once_per_head_sha_when_reviewer_configured() {
    let (db, _guard) = test_db().await;
    let fake = SharedFake::default();
    fake.lock().unwrap().ci_green = true;
    let base_url = spawn_fake_github(Arc::clone(&fake)).await;
    let (project, repo, task_id) = seed(&db, true, RepoProjectTaskState::WaitingReview).await;
    let mut seeded = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    seeded.pull_request_number = Some(42);
    seeded.head_sha = Some(HEAD_SHA.to_string());
    db.upsert_repo_project_task(&seeded).await.unwrap();

    let pipeline = pipeline_with(
        Arc::clone(&db),
        &base_url,
        PipelineConfig {
            reviewer_backend: Some(CodingBackend::Worker),
            ..PipelineConfig::default()
        },
    );

    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        matches!(outcome, PipelineOutcome::ReviewRequested { .. }),
        "first review pass requests a review, got {outcome:?}"
    );
    assert_eq!(fake.lock().unwrap().merges, 0, "no merge before review");

    // Second pass: review already requested for this head SHA -> proceeds to gate.
    let mut task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    let outcome = pipeline
        .advance_task(&project, &repo, &mut task)
        .await
        .unwrap();
    assert!(
        !matches!(outcome, PipelineOutcome::ReviewRequested { .. }),
        "sandbox review is one-shot per head SHA, got {outcome:?}"
    );
}

#[tokio::test]
async fn sync_worker_runs_skips_review_runs_and_keeps_task_in_review() {
    let (db, _guard) = test_db().await;
    let (project, repo, task_id) = seed(&db, true, RepoProjectTaskState::WaitingReview).await;
    let now = Utc::now();

    // A completed review sandbox job and its review-marked worker run.
    let job_id = Uuid::new_v4();
    db.save_sandbox_job(&crate::history::SandboxJobRecord {
        id: job_id,
        spec: crate::sandbox_jobs::SandboxJobSpec::new(
            "review",
            "review",
            "principal",
            "actor",
            None,
            crate::sandbox_types::JobMode::Worker,
        ),
        status: "completed".to_string(),
        success: Some(true),
        failure_reason: None,
        created_at: now,
        started_at: Some(now),
        completed_at: Some(now),
        credential_grants_json: "[]".to_string(),
    })
    .await
    .unwrap();

    let run_id = Uuid::new_v4();
    db.upsert_repo_worker_run(&RepoWorkerRun {
        id: run_id,
        project_id: project.id,
        project_run_id: run_id,
        repo_id: repo.id,
        task_id,
        state: RepoWorkerRunState::Running,
        coding_backend: CodingBackend::Worker,
        worker_id: "repo-project-reviewer-x".to_string(),
        branch_name: "thinclaw/proj/abc123".to_string(),
        job_id: Some(job_id.to_string()),
        commit_sha: None,
        exit_code: None,
        summary: None,
        metadata: json!({ "review": true }),
        created_at: now,
        updated_at: now,
        started_at: Some(now),
        completed_at: None,
    })
    .await
    .unwrap();

    let executor = super::executor::RepoProjectExecutor::new(
        Arc::clone(&db),
        None,
        super::executor::RepoProjectExecutorConfig::default(),
    );
    executor.sync_worker_runs(project.id).await.unwrap();

    let runs = db.list_repo_worker_runs(project.id).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].state,
        RepoWorkerRunState::Succeeded,
        "review worker run is still synced to terminal"
    );
    let task = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.state,
        RepoProjectTaskState::WaitingReview,
        "a review run must not transition the task out of review"
    );
}

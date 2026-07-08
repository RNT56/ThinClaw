//! Backend contract tests for the repo project supervisor store. Runs against
//! whichever backend `contract_db_or_skip` selects (Postgres when
//! `DATABASE_BACKEND=postgres`), exercising the durable model end to end —
//! including the run records and webhook delivery idempotency.

use chrono::Utc;
use uuid::Uuid;

use thinclaw_repo_projects::{
    CodingBackend, GitHubAuthMode, MergeGateDecision, MergeGateDenialReason, MergeMethod,
    ProjectPolicy, RepoProject, RepoProjectEvent, RepoProjectEventKind, RepoProjectRepo,
    RepoProjectRun, RepoProjectRunState, RepoProjectState, RepoProjectTask, RepoProjectTaskState,
    RepoWebhookDelivery, RepoWorkerRun, RepoWorkerRunState, RepoWriteMode,
};

use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn repo_project_store_full_lifecycle_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let db = &ctx.db;
    let now = Utc::now();

    let project_id = Uuid::new_v4();
    let project = RepoProject {
        id: project_id,
        slug: "contract".to_string(),
        name: "Contract".to_string(),
        state: RepoProjectState::Active,
        policy: ProjectPolicy {
            auto_merge: true,
            write_mode: RepoWriteMode::MaintainerAutoMerge,
            merge_method: MergeMethod::Squash,
            default_coding_backend: CodingBackend::CodexCode,
            github_auth_mode: GitHubAuthMode::GitHubApp,
            max_parallel_tasks: 2,
        },
        description: Some("contract fixture".to_string()),
        current_run_id: None,
        created_at: now,
        updated_at: now,
        started_at: Some(now),
        completed_at: None,
    };
    db.create_repo_project(&project)
        .await
        .expect("create project");
    let loaded = db
        .get_repo_project(project_id)
        .await
        .expect("get project")
        .expect("project exists");
    assert_eq!(loaded.state, RepoProjectState::Active);
    assert!(loaded.policy.auto_merge);
    assert!(!db.list_repo_projects().await.expect("list").is_empty());

    let repo_id = Uuid::new_v4();
    let repo = RepoProjectRepo {
        id: repo_id,
        project_id,
        owner: "owner".to_string(),
        repo: "repo".to_string(),
        github_repo_id: Some(7),
        installation_id: Some(99),
        default_branch: "main".to_string(),
        base_branch: Some("main".to_string()),
        enrolled: true,
        local_path: None,
        auth_mode: GitHubAuthMode::GitHubApp,
        metadata: serde_json::json!({ "k": "v" }),
        created_at: now,
        updated_at: now,
    };
    db.upsert_repo_project_repo(&repo)
        .await
        .expect("upsert repo");
    let repos = db
        .list_repo_project_repos(project_id)
        .await
        .expect("list repos");
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].installation_id, Some(99));

    let task_id = Uuid::new_v4();
    let mut task = RepoProjectTask {
        id: task_id,
        project_id,
        repo_id,
        title: "Task".to_string(),
        body: Some("body".to_string()),
        state: RepoProjectTaskState::WaitingCi,
        coding_backend: CodingBackend::CodexCode,
        base_branch: "main".to_string(),
        branch_name: "thinclaw/contract/abc".to_string(),
        head_sha: Some("deadbeef".to_string()),
        pull_request_number: Some(11),
        pull_request_url: Some("https://example/pull/11".to_string()),
        github_issue_number: None,
        assigned_worker_id: None,
        priority: 5,
        labels: vec!["x".to_string()],
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        queued_at: Some(now),
        started_at: None,
        completed_at: None,
    };
    db.upsert_repo_project_task(&task)
        .await
        .expect("upsert task");
    task.state = RepoProjectTaskState::WaitingReview;
    db.upsert_repo_project_task(&task)
        .await
        .expect("update task");
    let fetched_task = db
        .get_repo_project_task(task_id)
        .await
        .expect("get task")
        .expect("task exists");
    assert_eq!(fetched_task.state, RepoProjectTaskState::WaitingReview);
    assert_eq!(fetched_task.pull_request_number, Some(11));

    let run_id = Uuid::new_v4();
    let mut run = RepoProjectRun {
        id: run_id,
        project_id,
        state: RepoProjectRunState::Running,
        trigger: "supervisor".to_string(),
        summary: None,
        tasks_seen: 0,
        tasks_queued: 1,
        tasks_completed: 0,
        tasks_failed: 0,
        metadata: serde_json::json!({}),
        created_at: now,
        started_at: Some(now),
        completed_at: None,
    };
    db.upsert_repo_project_run(&run).await.expect("insert run");
    run.state = RepoProjectRunState::Completed;
    run.tasks_completed = 1;
    run.completed_at = Some(now);
    db.upsert_repo_project_run(&run).await.expect("update run");
    let fetched_run = db
        .get_repo_project_run(run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(fetched_run.state, RepoProjectRunState::Completed);
    assert_eq!(fetched_run.tasks_completed, 1);
    assert_eq!(
        db.list_repo_project_runs(project_id)
            .await
            .expect("runs")
            .len(),
        1
    );

    let worker_run = RepoWorkerRun {
        id: Uuid::new_v4(),
        project_id,
        project_run_id: run_id,
        repo_id,
        task_id,
        state: RepoWorkerRunState::Succeeded,
        coding_backend: CodingBackend::CodexCode,
        worker_id: "w1".to_string(),
        branch_name: "thinclaw/contract/abc".to_string(),
        job_id: Some("job-1".to_string()),
        commit_sha: Some("deadbeef".to_string()),
        exit_code: Some(0),
        summary: Some("ok".to_string()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        started_at: Some(now),
        completed_at: Some(now),
    };
    db.upsert_repo_worker_run(&worker_run)
        .await
        .expect("upsert worker run");
    assert_eq!(
        db.list_repo_worker_runs(project_id)
            .await
            .expect("worker runs")
            .len(),
        1
    );

    let event = RepoProjectEvent {
        id: Uuid::new_v4(),
        project_id,
        repo_id: Some(repo_id),
        task_id: Some(task_id),
        project_run_id: Some(run_id),
        worker_run_id: None,
        kind: RepoProjectEventKind::MergeGateEvaluated,
        message: "gate".to_string(),
        details: serde_json::json!({ "approved": false }),
        created_at: now,
    };
    db.append_repo_project_event(&event)
        .await
        .expect("append event");
    // Idempotent append: same id is a no-op.
    db.append_repo_project_event(&event)
        .await
        .expect("re-append event");
    let events = db
        .list_repo_project_events(project_id, 10)
        .await
        .expect("list events");
    assert_eq!(events.len(), 1, "event append is idempotent by id");

    let decision = MergeGateDecision::denied(
        vec![MergeGateDenialReason::ChecksNotGreen],
        MergeMethod::Squash,
    );
    db.upsert_repo_merge_gate_decision(project_id, task_id, &decision)
        .await
        .expect("upsert gate");
    let gates = db
        .list_repo_merge_gate_decisions(project_id)
        .await
        .expect("list gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].0, task_id);
    assert!(!gates[0].1.approved);

    // Webhook delivery idempotency.
    let delivery = RepoWebhookDelivery {
        delivery_id: format!("delivery-{}", Uuid::new_v4().simple()),
        event: "pull_request".to_string(),
        action: Some("opened".to_string()),
        repository_full_name: Some("owner/repo".to_string()),
        installation_id: Some(99),
        raw_payload_base64: None,
        signature_header: None,
        received_at: now,
    };
    assert!(
        db.record_repo_webhook_delivery(&delivery)
            .await
            .expect("record delivery"),
        "first delivery is new"
    );
    assert!(
        !db.record_repo_webhook_delivery(&delivery)
            .await
            .expect("record delivery again"),
        "redelivery is a duplicate"
    );
    assert!(
        !db.list_repo_webhook_deliveries(50)
            .await
            .expect("list deliveries")
            .is_empty()
    );
    assert_eq!(
        db.get_repo_webhook_delivery(&delivery.delivery_id)
            .await
            .expect("get delivery"),
        Some(delivery.clone())
    );
    assert!(
        db.get_repo_webhook_delivery("missing-delivery")
            .await
            .expect("get missing delivery")
            .is_none()
    );

    // Cleanup so repeated contract runs against a shared schema stay isolated.
    db.delete_repo_project(project_id)
        .await
        .expect("delete project");
    assert!(
        db.get_repo_project(project_id)
            .await
            .expect("get")
            .is_none()
    );
}

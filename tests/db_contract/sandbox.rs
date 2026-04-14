use chrono::Utc;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn sandbox_job_crud_and_summary_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("sandbox_user");
    let actor = fixtures::actor_name("sandbox");
    let mut job = fixtures::sandbox_job_record(&user, &actor, "creating");

    ctx.db
        .save_sandbox_job(&job)
        .await
        .expect("save_sandbox_job should succeed");

    let loaded = ctx
        .db
        .get_sandbox_job(job.id)
        .await
        .expect("get_sandbox_job should succeed")
        .expect("sandbox job should exist");
    assert_eq!(loaded.status, "creating");

    let all = ctx
        .db
        .list_sandbox_jobs()
        .await
        .expect("list_sandbox_jobs should succeed");
    assert!(!all.is_empty(), "job list should not be empty");

    let started = Utc::now();
    let completed = started + chrono::Duration::seconds(1);
    ctx.db
        .update_sandbox_job_status(
            job.id,
            "completed",
            Some(true),
            None,
            Some(started),
            Some(completed),
        )
        .await
        .expect("update_sandbox_job_status should succeed");

    let summary = ctx
        .db
        .sandbox_job_summary_for_user(&user)
        .await
        .expect("sandbox_job_summary_for_user should succeed");
    assert_eq!(summary.total, 1);
    assert_eq!(summary.completed, 1);

    // Default helper method in trait: actor-scoped filtering.
    let actor_jobs = ctx
        .db
        .list_sandbox_jobs_for_actor(&user, &actor)
        .await
        .expect("list_sandbox_jobs_for_actor should succeed");
    assert_eq!(actor_jobs.len(), 1);

    let belongs = ctx
        .db
        .sandbox_job_belongs_to_actor(job.id, &user, &actor)
        .await
        .expect("sandbox_job_belongs_to_actor should succeed");
    assert!(belongs);

    job.status = "failed".to_string();
    job.success = Some(false);
    job.failure_reason = Some("simulated".to_string());
    ctx.db
        .save_sandbox_job(&job)
        .await
        .expect("upsert save_sandbox_job should succeed");
}

#[tokio::test]
async fn sandbox_job_events_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("sandbox_events_user");
    let actor = fixtures::actor_name("sandbox_events");
    let job = fixtures::sandbox_job_record(&user, &actor, "running");

    ctx.db
        .save_sandbox_job(&job)
        .await
        .expect("save_sandbox_job should succeed");

    ctx.db
        .save_job_event(job.id, "status", &serde_json::json!({"status":"running"}))
        .await
        .expect("save_job_event should succeed");
    ctx.db
        .save_job_event(job.id, "log", &serde_json::json!({"line":"hello"}))
        .await
        .expect("save_job_event should succeed");

    let all_events = ctx
        .db
        .list_job_events(job.id, None)
        .await
        .expect("list_job_events should succeed");
    assert!(all_events.len() >= 2);

    let latest_only = ctx
        .db
        .list_job_events(job.id, Some(1))
        .await
        .expect("list_job_events with limit should succeed");
    assert_eq!(latest_only.len(), 1);
}

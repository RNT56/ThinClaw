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
    let duplicate = ctx.db.save_sandbox_job(&job).await;
    assert!(
        duplicate.is_err(),
        "sandbox job IDs are immutable admission keys and must not be upserted"
    );

    let retained = ctx
        .db
        .get_sandbox_job(job.id)
        .await
        .expect("reload sandbox job")
        .expect("sandbox job should remain");
    assert_eq!(retained.status, "completed");
}

#[tokio::test]
async fn sandbox_terminal_transition_and_result_event_are_atomic() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("sandbox_terminal_race_user");
    let actor = fixtures::actor_name("sandbox_terminal_race");
    let job = fixtures::sandbox_job_record(&user, &actor, "running");
    ctx.db
        .save_sandbox_job(&job)
        .await
        .expect("insert sandbox race fixture");

    let completed_at = Utc::now();
    let completed_data = serde_json::json!({"status":"completed","winner":"completed"});
    let cancelled_data = serde_json::json!({"status":"cancelled","winner":"cancelled"});
    let (completed, cancelled) = tokio::join!(
        ctx.db.finalize_sandbox_job_status(
            job.id,
            "completed",
            true,
            None,
            completed_at,
            &completed_data,
        ),
        ctx.db.finalize_sandbox_job_status(
            job.id,
            "cancelled",
            false,
            Some("cancel race"),
            completed_at,
            &cancelled_data,
        ),
    );
    let completed = completed.expect("completed finalizer");
    let cancelled = cancelled.expect("cancelled finalizer");
    assert_ne!(completed, cancelled, "exactly one finalizer must win");

    let stored = ctx
        .db
        .get_sandbox_job(job.id)
        .await
        .expect("reload finalized sandbox job")
        .expect("sandbox job should exist");
    assert_eq!(
        stored.status,
        if completed { "completed" } else { "cancelled" }
    );

    let events = ctx
        .db
        .list_job_events(job.id, None)
        .await
        .expect("load atomic result event");
    let result_events = events
        .iter()
        .filter(|event| event.event_type == "result")
        .collect::<Vec<_>>();
    assert_eq!(result_events.len(), 1);
    assert_eq!(
        result_events[0].data["winner"],
        if completed { "completed" } else { "cancelled" }
    );

    // A late creator must not resurrect the terminal job.
    ctx.db
        .update_sandbox_job_status(job.id, "running", None, None, Some(Utc::now()), None)
        .await
        .expect("late running update is an idempotent no-op");
    assert_eq!(
        ctx.db
            .get_sandbox_job(job.id)
            .await
            .expect("reload terminal job")
            .expect("terminal job exists")
            .status,
        stored.status
    );
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

#[tokio::test]
async fn stale_sandbox_cleanup_is_runtime_scoped_and_legacy_safe() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("sandbox_cleanup_scope_user");
    let actor = fixtures::actor_name("sandbox_cleanup_scope");
    let owned_scope = uuid::Uuid::new_v4().simple().to_string();
    let foreign_scope = uuid::Uuid::new_v4().simple().to_string();

    let mut owned = fixtures::sandbox_job_record(&user, &actor, "running");
    owned.spec.runtime_scope = Some(owned_scope.clone());
    let mut foreign = fixtures::sandbox_job_record(&user, &actor, "creating");
    foreign.spec.runtime_scope = Some(foreign_scope);
    let legacy = fixtures::sandbox_job_record(&user, &actor, "running");

    for job in [&owned, &foreign, &legacy] {
        ctx.db
            .save_sandbox_job(job)
            .await
            .expect("insert stale-cleanup fixture");
    }

    assert_eq!(
        ctx.db
            .cleanup_stale_sandbox_jobs(&owned_scope)
            .await
            .expect("scoped stale cleanup"),
        1
    );

    let owned = ctx
        .db
        .get_sandbox_job(owned.id)
        .await
        .expect("load owned job")
        .expect("owned job exists");
    let foreign = ctx
        .db
        .get_sandbox_job(foreign.id)
        .await
        .expect("load foreign job")
        .expect("foreign job exists");
    let legacy = ctx
        .db
        .get_sandbox_job(legacy.id)
        .await
        .expect("load legacy job")
        .expect("legacy job exists");

    assert_eq!(owned.status, "interrupted");
    assert_eq!(foreign.status, "creating");
    assert_eq!(legacy.status, "running");
}

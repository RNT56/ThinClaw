use rust_decimal::Decimal;
use thinclaw::context::JobState;
use thinclaw::history::LlmCallRecord;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn job_store_lifecycle_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("jobs_user");
    let actor = fixtures::actor_name("jobs");
    let job = fixtures::job_context(&user, &actor);
    let job_id = job.job_id;

    ctx.db
        .save_job(&job)
        .await
        .expect("save_job should succeed");

    let loaded = ctx
        .db
        .get_job(job_id)
        .await
        .expect("get_job should succeed")
        .expect("job should exist");
    assert_eq!(loaded.title, job.title);

    ctx.db
        .update_job_status(job_id, JobState::InProgress, None)
        .await
        .expect("update_job_status should succeed");
    let loaded = ctx
        .db
        .get_job(job_id)
        .await
        .expect("get_job should succeed")
        .expect("job should exist");
    assert_eq!(loaded.state, JobState::InProgress);

    let action = fixtures::action_record(0, "echo");
    ctx.db
        .save_action(job_id, &action)
        .await
        .expect("save_action should succeed");
    let actions = ctx
        .db
        .get_job_actions(job_id)
        .await
        .expect("get_job_actions should succeed");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].tool_name, "echo");

    let llm_call = LlmCallRecord {
        job_id: Some(job_id),
        conversation_id: None,
        provider: "openai",
        model: "gpt-5-mini",
        input_tokens: 42,
        output_tokens: 7,
        cost: Decimal::new(11, 4),
        purpose: Some("contract"),
    };
    let llm_call_id = ctx
        .db
        .record_llm_call(&llm_call)
        .await
        .expect("record_llm_call should succeed");
    assert_ne!(llm_call_id, Uuid::nil());

    let snapshot_id = ctx
        .db
        .save_estimation_snapshot(
            job_id,
            "coding",
            &["echo".to_string(), "search".to_string()],
            Decimal::new(125, 2),
            300,
            Decimal::new(1000, 2),
        )
        .await
        .expect("save_estimation_snapshot should succeed");
    ctx.db
        .update_estimation_actuals(
            snapshot_id,
            Decimal::new(100, 2),
            240,
            Some(Decimal::new(1200, 2)),
        )
        .await
        .expect("update_estimation_actuals should succeed");
}

#[tokio::test]
async fn job_store_stuck_detection_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("jobs_stuck");
    let actor = fixtures::actor_name("jobs_stuck");
    let job = fixtures::job_context(&user, &actor);
    let job_id = job.job_id;

    ctx.db
        .save_job(&job)
        .await
        .expect("save_job should succeed");
    ctx.db
        .mark_job_stuck(job_id)
        .await
        .expect("mark_job_stuck should succeed");

    let stuck = ctx
        .db
        .get_stuck_jobs()
        .await
        .expect("get_stuck_jobs should succeed");
    assert!(
        stuck.contains(&job_id),
        "stuck jobs should include the marked job"
    );
}

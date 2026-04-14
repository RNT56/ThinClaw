use chrono::Utc;
use thinclaw::agent::routine::RunStatus;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn routine_crud_and_runtime_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("routine_user");
    let actor = fixtures::actor_name("routine");
    let routine = fixtures::routine(&user, &actor);

    ctx.db
        .create_routine(&routine)
        .await
        .expect("create_routine should succeed");

    let loaded = ctx
        .db
        .get_routine(routine.id)
        .await
        .expect("get_routine should succeed")
        .expect("routine should exist");
    assert_eq!(loaded.name, routine.name);

    let by_name = ctx
        .db
        .get_routine_by_name(&user, &routine.name)
        .await
        .expect("get_routine_by_name should succeed");
    assert!(by_name.is_some());

    // Default helper methods for actor filtering should work.
    let by_name_actor = ctx
        .db
        .get_routine_by_name_for_actor(&user, &actor, &routine.name)
        .await
        .expect("get_routine_by_name_for_actor should succeed");
    assert!(by_name_actor.is_some());

    let actor_list = ctx
        .db
        .list_routines_for_actor(&user, &actor)
        .await
        .expect("list_routines_for_actor should succeed");
    assert_eq!(actor_list.len(), 1);

    let now = Utc::now();
    ctx.db
        .update_routine_runtime(
            routine.id,
            now,
            Some(now),
            1,
            0,
            &serde_json::json!({"ok":true}),
        )
        .await
        .expect("update_routine_runtime should succeed");
}

#[tokio::test]
async fn routine_runs_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("routine_runs_user");
    let actor = fixtures::actor_name("routine_runs");
    let routine = fixtures::routine(&user, &actor);

    ctx.db
        .create_routine(&routine)
        .await
        .expect("create_routine should succeed");

    let run = fixtures::routine_run(routine.id, RunStatus::Running);
    ctx.db
        .create_routine_run(&run)
        .await
        .expect("create_routine_run should succeed");

    let running_before = ctx
        .db
        .count_running_routine_runs(routine.id)
        .await
        .expect("count_running_routine_runs should succeed");
    assert_eq!(running_before, 1);

    ctx.db
        .complete_routine_run(run.id, RunStatus::Ok, Some("done"), Some(123))
        .await
        .expect("complete_routine_run should succeed");

    let runs = ctx
        .db
        .list_routine_runs(routine.id, 10)
        .await
        .expect("list_routine_runs should succeed");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Ok);

    let running_after = ctx
        .db
        .count_running_routine_runs(routine.id)
        .await
        .expect("count_running_routine_runs should succeed");
    assert_eq!(running_after, 0);

    let deleted = ctx
        .db
        .delete_routine(routine.id)
        .await
        .expect("delete_routine should succeed");
    assert!(deleted);
}

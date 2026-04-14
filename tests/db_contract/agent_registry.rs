use crate::db_contract::fixtures;
use crate::db_contract::support::{contract_db_or_skip, unique_id};

#[tokio::test]
async fn agent_registry_crud_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let agent_id = unique_id("agent_registry");
    let mut record = fixtures::agent_workspace(&agent_id);

    ctx.db
        .save_agent_workspace(&record)
        .await
        .expect("save_agent_workspace should succeed");

    let loaded = ctx
        .db
        .get_agent_workspace(&agent_id)
        .await
        .expect("get_agent_workspace should succeed")
        .expect("agent workspace should exist");
    assert_eq!(loaded.agent_id, agent_id);

    let list = ctx
        .db
        .list_agent_workspaces()
        .await
        .expect("list_agent_workspaces should succeed");
    assert!(
        list.iter().any(|entry| entry.agent_id == agent_id),
        "saved workspace should be present in list"
    );

    record.display_name = "Updated Contract Agent".to_string();
    ctx.db
        .update_agent_workspace(&record)
        .await
        .expect("update_agent_workspace should succeed");
    let updated = ctx
        .db
        .get_agent_workspace(&agent_id)
        .await
        .expect("get_agent_workspace should succeed")
        .expect("agent workspace should still exist");
    assert_eq!(updated.display_name, "Updated Contract Agent");

    let deleted = ctx
        .db
        .delete_agent_workspace(&agent_id)
        .await
        .expect("delete_agent_workspace should succeed");
    assert!(deleted, "delete_agent_workspace should return true");
}

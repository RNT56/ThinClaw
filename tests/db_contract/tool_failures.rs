use crate::db_contract::support::{contract_db_or_skip, unique_id};

#[tokio::test]
async fn tool_failure_threshold_and_repair_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let tool = unique_id("contract_tool");
    let other = unique_id("contract_other_tool");

    ctx.db
        .record_tool_failure(&tool, "first failure")
        .await
        .expect("record_tool_failure should succeed");
    ctx.db
        .record_tool_failure(&tool, "second failure")
        .await
        .expect("record_tool_failure should succeed");
    ctx.db
        .record_tool_failure(&other, "single failure")
        .await
        .expect("record_tool_failure should succeed");

    let broken = ctx
        .db
        .get_broken_tools(2)
        .await
        .expect("get_broken_tools should succeed");
    assert!(
        broken.iter().any(|entry| entry.name == tool),
        "tool with two failures should appear above threshold"
    );
    assert!(
        broken.iter().all(|entry| entry.name != other),
        "tool with single failure should not appear at threshold=2"
    );

    ctx.db
        .increment_repair_attempts(&tool)
        .await
        .expect("increment_repair_attempts should succeed");
    ctx.db
        .mark_tool_repaired(&tool)
        .await
        .expect("mark_tool_repaired should succeed");

    let repaired_view = ctx
        .db
        .get_broken_tools(1)
        .await
        .expect("get_broken_tools should succeed after repair");
    assert!(
        repaired_view.iter().all(|entry| entry.name != tool),
        "repaired tool should not be considered broken"
    );
}

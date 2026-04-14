use std::collections::HashMap;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn settings_crud_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("settings_user");

    ctx.db
        .set_setting(&user, "theme", &serde_json::json!("dark"))
        .await
        .expect("set_setting should succeed");

    let value = ctx
        .db
        .get_setting(&user, "theme")
        .await
        .expect("get_setting should succeed");
    assert_eq!(value, Some(serde_json::json!("dark")));

    let full = ctx
        .db
        .get_setting_full(&user, "theme")
        .await
        .expect("get_setting_full should succeed")
        .expect("setting row should exist");
    assert_eq!(full.key, "theme");
    assert_eq!(full.value, serde_json::json!("dark"));

    let listed = ctx
        .db
        .list_settings(&user)
        .await
        .expect("list_settings should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "theme");

    assert!(
        ctx.db
            .has_settings(&user)
            .await
            .expect("has_settings should succeed")
    );

    let deleted = ctx
        .db
        .delete_setting(&user, "theme")
        .await
        .expect("delete_setting should succeed");
    assert!(deleted, "delete should report one row removed");
}

#[tokio::test]
async fn settings_bulk_roundtrip_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("settings_bulk");

    let mut expected = HashMap::new();
    expected.insert("timezone".to_string(), serde_json::json!("Europe/Berlin"));
    expected.insert("sound".to_string(), serde_json::json!(true));
    expected.insert("temperature".to_string(), serde_json::json!(0.7));

    ctx.db
        .set_all_settings(&user, &expected)
        .await
        .expect("set_all_settings should succeed");

    let actual = ctx
        .db
        .get_all_settings(&user)
        .await
        .expect("get_all_settings should succeed");
    assert_eq!(actual, expected);
}

use crate::db_contract::fixtures;
use crate::db_contract::support::{contract_db_or_skip, unique_id};
use thinclaw::db::{IdentityRegistryStore, IdentityStore};

#[tokio::test]
async fn identity_registry_actor_and_endpoint_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let principal = fixtures::user("identity_principal");
    let actor_display = fixtures::actor_name("contract");
    let created = ctx
        .db
        .create_actor(&fixtures::new_actor_record(&principal, &actor_display))
        .await
        .expect("create_actor should succeed");

    let loaded = IdentityRegistryStore::get_actor(ctx.db.as_ref(), created.actor_id)
        .await
        .expect("get_actor should succeed")
        .expect("actor must exist");
    assert_eq!(loaded.display_name, actor_display);
    assert_eq!(loaded.principal_id, principal);

    let endpoint_external = unique_id("endpoint_user");
    let endpoint =
        fixtures::new_actor_endpoint_record(created.actor_id, "telegram", &endpoint_external);
    let saved_endpoint = ctx
        .db
        .upsert_actor_endpoint(&endpoint)
        .await
        .expect("upsert_actor_endpoint should succeed");
    assert_eq!(saved_endpoint.endpoint.channel, "telegram");

    let resolved = ctx
        .db
        .resolve_actor_for_endpoint("telegram", &endpoint_external)
        .await
        .expect("resolve_actor_for_endpoint should succeed")
        .expect("endpoint should resolve to actor");
    assert_eq!(resolved.actor_id, created.actor_id);

    let deleted = ctx
        .db
        .delete_actor_endpoint("telegram", &endpoint_external)
        .await
        .expect("delete_actor_endpoint should succeed");
    assert!(deleted, "endpoint should be deleted");
}

#[tokio::test]
async fn identity_store_adapter_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let principal = fixtures::user("identity_adapter");
    let actor = ctx
        .db
        .create_actor(&fixtures::new_actor_record(&principal, "Adapter Actor"))
        .await
        .expect("create_actor should succeed");

    let fetched_via_adapter =
        IdentityStore::get_actor(ctx.db.as_ref(), &actor.actor_id.to_string())
            .await
            .expect("IdentityStore::get_actor should succeed")
            .expect("actor must exist");
    assert_eq!(fetched_via_adapter.actor_id, actor.actor_id);

    let bad_uuid_err = IdentityStore::get_actor(ctx.db.as_ref(), "not-a-uuid").await;
    assert!(
        bad_uuid_err.is_err(),
        "IdentityStore adapter should reject invalid UUID strings"
    );

    let ext_id = unique_id("adapter_endpoint");
    ctx.db
        .link_actor_endpoint(
            &actor.actor_id.to_string(),
            "discord",
            &ext_id,
            &serde_json::json!({"source":"adapter"}),
            "approved",
        )
        .await
        .expect("IdentityStore::link_actor_endpoint should succeed");

    let endpoints =
        IdentityStore::list_actor_endpoints(ctx.db.as_ref(), &actor.actor_id.to_string())
            .await
            .expect("IdentityStore::list_actor_endpoints should succeed");
    assert_eq!(endpoints.len(), 1);
}

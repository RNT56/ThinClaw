use thinclaw::workspace::SearchConfig;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn workspace_document_and_listing_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("workspace_user");
    let path = "projects/contract/README.md";

    let doc = ctx
        .db
        .get_or_create_document_by_path(&user, None, path)
        .await
        .expect("get_or_create_document_by_path should succeed");
    assert_eq!(doc.path, path);

    ctx.db
        .update_document(doc.id, "# Contract\n\nThis is a workspace contract test.")
        .await
        .expect("update_document should succeed");

    let loaded = ctx
        .db
        .get_document_by_path(&user, None, path)
        .await
        .expect("get_document_by_path should succeed");
    assert!(loaded.content.contains("workspace contract"));

    let by_id = ctx
        .db
        .get_document_by_id(doc.id)
        .await
        .expect("get_document_by_id should succeed");
    assert_eq!(by_id.path, path);

    let entries = ctx
        .db
        .list_directory(&user, None, "projects")
        .await
        .expect("list_directory should succeed");
    assert!(
        !entries.is_empty(),
        "directory listing should include the contract subtree"
    );

    let paths = ctx
        .db
        .list_all_paths(&user, None)
        .await
        .expect("list_all_paths should succeed");
    assert!(paths.iter().any(|p| p == path));

    let docs = ctx
        .db
        .list_documents(&user, None)
        .await
        .expect("list_documents should succeed");
    assert!(docs.iter().any(|d| d.id == doc.id));
}

#[tokio::test]
async fn workspace_chunks_and_search_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("workspace_chunks_user");
    let doc = ctx
        .db
        .get_or_create_document_by_path(&user, None, "notes/contract.md")
        .await
        .expect("create document should succeed");

    let first_chunk_id = ctx
        .db
        .insert_chunk(doc.id, 0, "alpha contract query token", None, None)
        .await
        .expect("insert_chunk should succeed");

    let pending = ctx
        .db
        .get_chunks_requiring_embedding(&user, None, "contract-model", 4, 10)
        .await
        .expect("get_chunks_requiring_embedding should succeed");
    assert!(pending.iter().any(|chunk| chunk.id == first_chunk_id));

    // Replace with two chunks using the trait method (default helper on PG, transaction on libSQL).
    ctx.db
        .replace_chunks(
            doc.id,
            &[
                (0, "replacement contract chunk one".to_string(), None),
                (1, "replacement chunk two".to_string(), None),
            ],
            None,
        )
        .await
        .expect("replace_chunks should succeed");

    let config = SearchConfig::default().with_limit(5).fts_only();
    let hits = ctx
        .db
        .hybrid_search(&user, None, "contract", None, None, &config)
        .await
        .expect("hybrid_search should succeed");
    assert!(!hits.is_empty(), "expected at least one hybrid search hit");
}

#[tokio::test]
async fn workspace_vector_profiles_never_mix_dimensions_or_models() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user = fixtures::user("workspace_vector_profiles");
    let dimension_four = ctx
        .db
        .get_or_create_document_by_path(&user, None, "vectors/model-a-4.md")
        .await
        .expect("create four-dimensional document");
    let dimension_six = ctx
        .db
        .get_or_create_document_by_path(&user, None, "vectors/model-a-6.md")
        .await
        .expect("create six-dimensional document");
    let other_model = ctx
        .db
        .get_or_create_document_by_path(&user, None, "vectors/model-b-4.md")
        .await
        .expect("create second-model document");

    let vector4 = vec![1.0, 0.1, 0.0, 0.0];
    let vector6 = vec![1.0, 0.1, 0.0, 0.0, 0.0, 0.0];
    ctx.db
        .replace_chunks(
            dimension_four.id,
            &[(
                0,
                "model a dimension four".to_string(),
                Some(vector4.clone()),
            )],
            Some("model-a"),
        )
        .await
        .expect("store model-a dimension-four vector");
    ctx.db
        .replace_chunks(
            dimension_six.id,
            &[(
                0,
                "model a dimension six".to_string(),
                Some(vector6.clone()),
            )],
            Some("model-a"),
        )
        .await
        .expect("store model-a dimension-six vector");
    ctx.db
        .replace_chunks(
            other_model.id,
            &[(
                0,
                "model b dimension four".to_string(),
                Some(vector4.clone()),
            )],
            Some("model-b"),
        )
        .await
        .expect("store model-b dimension-four vector");

    let config = SearchConfig::default().vector_only().with_limit(10);
    let model_a_four = ctx
        .db
        .hybrid_search(
            &user,
            None,
            "unused",
            Some(&vector4),
            Some("model-a"),
            &config,
        )
        .await
        .expect("model-a dimension-four search should be safe");
    assert_eq!(model_a_four.len(), 1);
    assert_eq!(model_a_four[0].content, "model a dimension four");

    let model_a_six = ctx
        .db
        .hybrid_search(
            &user,
            None,
            "unused",
            Some(&vector6),
            Some("model-a"),
            &config,
        )
        .await
        .expect("model-a dimension-six search should be safe");
    assert_eq!(model_a_six.len(), 1);
    assert_eq!(model_a_six[0].content, "model a dimension six");

    let model_b_four = ctx
        .db
        .hybrid_search(
            &user,
            None,
            "unused",
            Some(&vector4),
            Some("model-b"),
            &config,
        )
        .await
        .expect("model-b dimension-four search should be safe");
    assert_eq!(model_b_four.len(), 1);
    assert_eq!(model_b_four[0].content, "model b dimension four");

    let stale = ctx
        .db
        .get_chunks_requiring_embedding(&user, None, "model-a", 4, 10)
        .await
        .expect("profile-aware backfill query should succeed");
    assert_eq!(stale.len(), 2, "wrong dimension and wrong model are stale");

    let invalid = [1.0, f32::NAN];
    assert!(
        ctx.db
            .hybrid_search(
                &user,
                None,
                "unused",
                Some(&invalid),
                Some("model-a"),
                &config,
            )
            .await
            .is_err(),
        "non-finite query vectors must fail before comparison"
    );

    assert!(
        ctx.db
            .hybrid_search(&user, None, "unused", Some(&vector4), None, &config)
            .await
            .is_err(),
        "vectors without a model identity must not search across semantic spaces"
    );
    assert!(
        ctx.db
            .insert_chunk(
                dimension_four.id,
                99,
                "missing profile",
                Some(&vector4),
                None,
            )
            .await
            .is_err(),
        "vectors without a model identity must not be persisted"
    );
}

#[tokio::test]
async fn workspace_delete_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("workspace_delete_user");
    let path = format!("trash/{}.md", Uuid::new_v4().simple());

    let doc = ctx
        .db
        .get_or_create_document_by_path(&user, None, &path)
        .await
        .expect("create document should succeed");
    ctx.db
        .insert_chunk(doc.id, 0, "to be deleted", None, None)
        .await
        .expect("insert_chunk should succeed");

    ctx.db
        .delete_document_by_path(&user, None, &path)
        .await
        .expect("delete_document_by_path should succeed");

    let fetch = ctx.db.get_document_by_path(&user, None, &path).await;
    assert!(fetch.is_err(), "deleted document should not be retrievable");
}

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
        .insert_chunk(doc.id, 0, "alpha contract query token", None)
        .await
        .expect("insert_chunk should succeed");

    let pending = ctx
        .db
        .get_chunks_without_embeddings(&user, None, 10)
        .await
        .expect("get_chunks_without_embeddings should succeed");
    assert!(pending.iter().any(|chunk| chunk.id == first_chunk_id));

    // Replace with two chunks using the trait method (default helper on PG, transaction on libSQL).
    ctx.db
        .replace_chunks(
            doc.id,
            &[
                (0, "replacement contract chunk one".to_string(), None),
                (1, "replacement chunk two".to_string(), None),
            ],
        )
        .await
        .expect("replace_chunks should succeed");

    let config = SearchConfig::default().with_limit(5).fts_only();
    let hits = ctx
        .db
        .hybrid_search(&user, None, "contract", None, &config)
        .await
        .expect("hybrid_search should succeed");
    assert!(!hits.is_empty(), "expected at least one hybrid search hit");
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
        .insert_chunk(doc.id, 0, "to be deleted", None)
        .await
        .expect("insert_chunk should succeed");

    ctx.db
        .delete_document_by_path(&user, None, &path)
        .await
        .expect("delete_document_by_path should succeed");

    let fetch = ctx.db.get_document_by_path(&user, None, &path).await;
    assert!(fetch.is_err(), "deleted document should not be retrievable");
}

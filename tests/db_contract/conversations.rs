use thinclaw::history::ConversationKind;
use uuid::Uuid;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn conversation_message_flow_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("conversation_user");
    let channel = "repl";

    let conversation_id = ctx
        .db
        .create_conversation(channel, &user, Some("thread-contract"))
        .await
        .expect("create_conversation should succeed");
    assert_ne!(conversation_id, Uuid::nil());

    ctx.db
        .add_conversation_message(conversation_id, "user", "hello contract world")
        .await
        .expect("add_conversation_message should succeed");
    ctx.db
        .add_conversation_message_with_attribution(
            conversation_id,
            "assistant",
            "contract response",
            Some("assistant-1"),
            Some("Assistant"),
            Some("raw-id"),
            Some(&serde_json::json!({"source":"contract"})),
        )
        .await
        .expect("add_conversation_message_with_attribution should succeed");

    let messages = ctx
        .db
        .list_conversation_messages(conversation_id)
        .await
        .expect("list_conversation_messages should succeed");
    assert_eq!(messages.len(), 2);

    let (page, has_more) = ctx
        .db
        .list_conversation_messages_paginated(conversation_id, None, 1)
        .await
        .expect("list_conversation_messages_paginated should succeed");
    assert_eq!(page.len(), 1);
    assert!(has_more);

    let previews = ctx
        .db
        .list_conversations_with_preview(&user, channel, 10)
        .await
        .expect("list_conversations_with_preview should succeed");
    assert!(!previews.is_empty());

    let search_hits = ctx
        .db
        .search_conversation_messages(&user, "contract", None, Some(channel), None, 10)
        .await
        .expect("search_conversation_messages should succeed");
    assert!(!search_hits.is_empty(), "expected at least one search hit");

    ctx.db
        .update_conversation_identity(
            conversation_id,
            Some("actor-1"),
            Some(Uuid::new_v4()),
            ConversationKind::Direct,
            Some("repl:contract"),
        )
        .await
        .expect("update_conversation_identity should succeed");
}

#[tokio::test]
async fn conversation_learning_flow_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let user = fixtures::user("conversation_learning_user");
    let conversation_id = ctx
        .db
        .create_conversation("repl", &user, Some("learning-thread"))
        .await
        .expect("create_conversation should succeed");
    let message_id = ctx
        .db
        .add_conversation_message(conversation_id, "user", "learning candidate")
        .await
        .expect("add_conversation_message should succeed");

    let mut event = fixtures::learning_event(&user, Some(conversation_id), Some(message_id));
    event.thread_id = Some("learning-thread".to_string());
    let event_id = ctx
        .db
        .insert_learning_event(&event)
        .await
        .expect("insert_learning_event should succeed");

    let eval = fixtures::learning_evaluation(&user, event_id);
    ctx.db
        .insert_learning_evaluation(&eval)
        .await
        .expect("insert_learning_evaluation should succeed");

    let candidate = fixtures::learning_candidate(&user, event_id);
    let candidate_id = ctx
        .db
        .insert_learning_candidate(&candidate)
        .await
        .expect("insert_learning_candidate should succeed");

    let artifact = fixtures::learning_artifact_version(&user, candidate_id);
    ctx.db
        .insert_learning_artifact_version(&artifact)
        .await
        .expect("insert_learning_artifact_version should succeed");

    let proposal = fixtures::learning_code_proposal(&user, event_id);
    ctx.db
        .insert_learning_code_proposal(&proposal)
        .await
        .expect("insert_learning_code_proposal should succeed");

    let events = ctx
        .db
        .list_learning_events(&user, None, Some("repl"), Some("learning-thread"), 20)
        .await
        .expect("list_learning_events should succeed");
    assert!(events.iter().any(|entry| entry.id == event_id));

    let evals = ctx
        .db
        .list_learning_evaluations(&user, 20)
        .await
        .expect("list_learning_evaluations should succeed");
    assert!(
        evals
            .iter()
            .any(|entry| entry.learning_event_id == event_id)
    );

    let proposals = ctx
        .db
        .list_learning_code_proposals(&user, Some("proposed"), 20)
        .await
        .expect("list_learning_code_proposals should succeed");
    assert!(
        proposals
            .iter()
            .any(|entry| entry.learning_event_id == Some(event_id))
    );
}

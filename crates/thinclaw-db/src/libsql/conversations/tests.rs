use super::*;
use crate::Database;

async fn database() -> (tempfile::TempDir, LibSqlBackend) {
    let directory = tempfile::tempdir().unwrap();
    let backend = LibSqlBackend::new_local(&directory.path().join("conversations.db"))
        .await
        .unwrap();
    backend.run_migrations().await.unwrap();
    (directory, backend)
}

#[tokio::test]
async fn direct_conversation_acl_is_actor_scoped() {
    let (_directory, backend) = database().await;
    let id = backend
        .create_conversation("test", "owner", Some("thread"))
        .await
        .unwrap();
    let scope = Uuid::new_v4();
    backend
        .update_conversation_identity(
            id,
            Some("owner"),
            Some("alice"),
            Some(scope),
            ConversationKind::Direct,
            Some("test:thread"),
        )
        .await
        .unwrap();

    assert!(
        backend
            .conversation_belongs_to_identity(
                id,
                "owner",
                "alice",
                scope,
                ConversationKind::Direct,
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .conversation_belongs_to_identity(id, "owner", "bob", scope, ConversationKind::Direct,)
            .await
            .unwrap()
    );
    assert!(
        !backend
            .conversation_belongs_to_identity(
                id,
                "other-owner",
                "alice",
                scope,
                ConversationKind::Direct,
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn group_acl_uses_exact_scope_and_preserves_first_speaker() {
    let (_directory, backend) = database().await;
    let id = backend
        .create_conversation("test", "owner", Some("group-thread"))
        .await
        .unwrap();
    let scope = Uuid::new_v4();
    backend
        .update_conversation_identity(
            id,
            Some("owner"),
            Some("alice"),
            Some(scope),
            ConversationKind::Group,
            Some("test:group-thread"),
        )
        .await
        .unwrap();
    backend
        .update_conversation_identity(
            id,
            Some("owner"),
            Some("bob"),
            Some(scope),
            ConversationKind::Group,
            Some("test:group-thread"),
        )
        .await
        .unwrap();

    let conn = backend.connect().await.unwrap();
    let mut rows = conn
        .query(
            "SELECT actor_id FROM conversations WHERE id = ?1",
            params![id.to_string()],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<String>(0).unwrap(), "alice");

    assert!(
        backend
            .conversation_belongs_to_identity(id, "owner", "bob", scope, ConversationKind::Group,)
            .await
            .unwrap()
    );
    assert!(
        !backend
            .conversation_belongs_to_identity(
                id,
                "owner",
                "bob",
                Uuid::new_v4(),
                ConversationKind::Group,
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .conversation_belongs_to_identity(id, "owner", "bob", scope, ConversationKind::Direct,)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn native_ingress_lookup_restores_only_the_exact_identity_scope() {
    let (_directory, backend) = database().await;
    let group_id = backend
        .create_conversation("matrix", "owner", Some("room:non-uuid"))
        .await
        .unwrap();
    let group_scope = Uuid::new_v4();
    backend
        .update_conversation_identity(
            group_id,
            Some("owner"),
            Some("alice"),
            Some(group_scope),
            ConversationKind::Group,
            Some("matrix:group:room:non-uuid"),
        )
        .await
        .unwrap();

    assert_eq!(
        backend
            .find_latest_conversation_for_ingress(
                "owner",
                "bob",
                group_scope,
                ConversationKind::Group,
                "matrix",
                Some("room:non-uuid"),
            )
            .await
            .unwrap(),
        Some(group_id)
    );
    assert!(
        backend
            .find_latest_conversation_for_ingress(
                "owner",
                "bob",
                Uuid::new_v4(),
                ConversationKind::Group,
                "matrix",
                Some("room:non-uuid"),
            )
            .await
            .unwrap()
            .is_none()
    );

    let direct_id = backend
        .create_conversation("signal", "owner", Some("side-thread"))
        .await
        .unwrap();
    let direct_scope = Uuid::new_v4();
    backend
        .update_conversation_identity(
            direct_id,
            Some("owner"),
            Some("alice"),
            Some(direct_scope),
            ConversationKind::Direct,
            Some("direct:owner:alice"),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .find_latest_conversation_for_ingress(
                "owner",
                "alice",
                direct_scope,
                ConversationKind::Direct,
                "signal",
                Some("side-thread"),
            )
            .await
            .unwrap(),
        Some(direct_id)
    );
    assert!(
        backend
            .find_latest_conversation_for_ingress(
                "owner",
                "bob",
                direct_scope,
                ConversationKind::Direct,
                "signal",
                Some("side-thread"),
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn conversation_message_window_is_bounded_and_chronological() {
    let (_directory, backend) = database().await;
    let id = backend
        .create_conversation("test", "owner", Some("window-thread"))
        .await
        .unwrap();
    for index in 0..5 {
        backend
            .add_conversation_message(id, "user", &format!("message-{index}"))
            .await
            .unwrap();
    }

    assert_eq!(backend.count_conversation_messages(id).await.unwrap(), 5);
    let window = backend
        .list_conversation_messages_window(id, 2, 2)
        .await
        .unwrap();
    assert_eq!(
        window
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        vec!["message-2", "message-3"]
    );
    let first = backend
        .list_conversation_messages_window(id, -10, 1)
        .await
        .unwrap();
    assert_eq!(first[0].content, "message-0");
    assert!(
        backend
            .list_conversation_messages_window(id, 0, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn effective_user_instruction_updates_exact_row_without_replacing_raw_content() {
    let (_directory, backend) = database().await;
    let conversation_id = backend
        .create_conversation("test", "owner", Some("hook-thread"))
        .await
        .unwrap();
    let user_message_id = backend
        .add_conversation_message_with_attribution(
            conversation_id,
            "user",
            "raw transcript",
            None,
            None,
            None,
            Some(&serde_json::json!({"source": "channel"})),
        )
        .await
        .unwrap();
    let other_message_id = backend
        .add_conversation_message(conversation_id, "assistant", "unchanged")
        .await
        .unwrap();

    backend
        .set_effective_user_instruction(conversation_id, user_message_id, "model-visible rewrite")
        .await
        .unwrap();

    let messages = backend
        .list_conversation_messages(conversation_id)
        .await
        .unwrap();
    let user = messages
        .iter()
        .find(|message| message.id == user_message_id)
        .unwrap();
    assert_eq!(user.content, "raw transcript");
    assert_eq!(user.metadata["source"], "channel");
    assert_eq!(
        user.metadata["_thinclaw_effective_user_instruction_version"],
        1
    );
    assert_eq!(
        user.metadata["_thinclaw_effective_user_instruction"],
        "model-visible rewrite"
    );
    assert!(
        backend
            .set_effective_user_instruction(
                conversation_id,
                other_message_id,
                "must not update assistant rows",
            )
            .await
            .is_err()
    );
}

//! Conversion policy between thread-store ports and history records.

use thinclaw_history::{ConversationMessage, ConversationSummary};
use uuid::Uuid;

use crate::ports::{ThreadMessage, ThreadSummary};

pub fn thread_message_from_history(
    conversation_id: Uuid,
    message: ConversationMessage,
) -> ThreadMessage {
    ThreadMessage {
        id: message.id,
        conversation_id,
        role: message.role,
        content: message.content,
        actor_id: message.actor_id,
        actor_display_name: message.actor_display_name,
        raw_sender_id: message.raw_sender_id,
        metadata: message.metadata,
        created_at: message.created_at,
    }
}

pub fn thread_summary_from_history(summary: ConversationSummary) -> ThreadSummary {
    ThreadSummary {
        id: summary.id,
        user_id: summary.user_id,
        channel: summary.channel,
        thread_id: summary.stable_external_conversation_key,
        title: summary.title.clone(),
        preview: summary.title,
        message_count: summary.message_count,
        updated_at: summary.last_activity,
        metadata: serde_json::json!({
            "conversation_kind": summary.conversation_kind.as_str(),
            "conversation_scope_id": summary.conversation_scope_id.map(|id| id.to_string()),
            "actor_id": summary.actor_id,
            "thread_type": summary.thread_type,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use thinclaw_history::{ConversationKind, ConversationSummary};

    #[test]
    fn thread_summary_adapter_preserves_recall_fields() {
        let id = Uuid::new_v4();
        let scope_id = Uuid::new_v4();
        let now = Utc::now();
        let summary = thread_summary_from_history(ConversationSummary {
            id,
            user_id: "user-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            conversation_scope_id: Some(scope_id),
            conversation_kind: ConversationKind::Direct,
            channel: "web".to_string(),
            title: Some("hello world".to_string()),
            message_count: 3,
            started_at: now,
            last_activity: now,
            thread_type: Some("assistant".to_string()),
            handoff: None,
            stable_external_conversation_key: Some("thread-1".to_string()),
        });

        assert_eq!(summary.id, id);
        assert_eq!(summary.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(summary.preview.as_deref(), Some("hello world"));
        assert_eq!(summary.metadata["conversation_kind"], "direct");
        assert_eq!(
            summary.metadata["conversation_scope_id"],
            scope_id.to_string()
        );
    }
}

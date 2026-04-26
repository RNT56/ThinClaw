//! Types for Gmail API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Gmail tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GmailAction {
    /// List messages in the mailbox.
    ListMessages {
        /// Gmail search query (same syntax as the Gmail search box).
        /// Examples: "from:alice@example.com", "subject:meeting", "is:unread",
        /// "after:2025/01/01 before:2025/02/01".
        #[serde(default)]
        query: Option<String>,
        /// Maximum number of messages to return (default: 20).
        #[serde(default = "default_max_results")]
        max_results: u32,
        /// Label IDs to filter by (e.g., "INBOX", "SENT", "DRAFT").
        #[serde(default)]
        label_ids: Vec<String>,
    },

    /// Get a specific message with full content.
    GetMessage {
        /// The message ID.
        message_id: String,
    },

    /// Send an email.
    SendMessage {
        /// Recipient email address(es), comma-separated.
        to: String,
        /// Email subject.
        subject: String,
        /// Email body (plain text).
        body: String,
        /// CC recipients, comma-separated.
        #[serde(default)]
        cc: Option<String>,
        /// BCC recipients, comma-separated.
        #[serde(default)]
        bcc: Option<String>,
    },

    /// Create a draft email.
    CreateDraft {
        /// Recipient email address(es), comma-separated.
        to: String,
        /// Email subject.
        subject: String,
        /// Email body (plain text).
        body: String,
        /// CC recipients, comma-separated.
        #[serde(default)]
        cc: Option<String>,
        /// BCC recipients, comma-separated.
        #[serde(default)]
        bcc: Option<String>,
    },

    /// Reply to an existing message.
    ReplyToMessage {
        /// The message ID to reply to.
        message_id: String,
        /// Reply body (plain text).
        body: String,
        /// If true, reply to all recipients. Default: false.
        #[serde(default)]
        reply_all: bool,
    },

    /// Move a message to trash.
    TrashMessage {
        /// The message ID to trash.
        message_id: String,
    },
}

fn default_max_results() -> u32 {
    20
}

/// A Gmail message summary (from list endpoint).
#[derive(Debug, Serialize)]
pub struct MessageSummary {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
    pub snippet: String,
    pub label_ids: Vec<String>,
    pub is_unread: bool,
}

/// A full Gmail message (from get endpoint).
#[derive(Debug, Serialize)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<String>,
    pub date: String,
    pub body: String,
    pub snippet: String,
    pub label_ids: Vec<String>,
    pub is_unread: bool,
}

/// Result from list_messages.
#[derive(Debug, Serialize)]
pub struct ListMessagesResult {
    pub messages: Vec<MessageSummary>,
    pub result_size_estimate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// Result from send_message or reply_to_message.
#[derive(Debug, Serialize)]
pub struct SendResult {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

/// Result from create_draft.
#[derive(Debug, Serialize)]
pub struct DraftResult {
    pub id: String,
    pub message_id: String,
}

/// Result from trash_message.
#[derive(Debug, Serialize)]
pub struct TrashResult {
    pub id: String,
    pub trashed: bool,
}

#[cfg(test)]
mod tests {
    use super::GmailAction;

    #[test]
    fn list_messages_defaults_max_results_and_labels() {
        let action: GmailAction = serde_json::from_value(serde_json::json!({
            "action": "list_messages"
        }))
        .unwrap();

        match action {
            GmailAction::ListMessages {
                query,
                max_results,
                label_ids,
            } => {
                assert_eq!(query, None);
                assert_eq!(max_results, 20);
                assert!(label_ids.is_empty());
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn reply_to_message_defaults_reply_all_to_false() {
        let action: GmailAction = serde_json::from_value(serde_json::json!({
            "action": "reply_to_message",
            "message_id": "msg-123",
            "body": "Thanks"
        }))
        .unwrap();

        match action {
            GmailAction::ReplyToMessage {
                message_id,
                body,
                reply_all,
            } => {
                assert_eq!(message_id, "msg-123");
                assert_eq!(body, "Thanks");
                assert!(!reply_all);
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }
}

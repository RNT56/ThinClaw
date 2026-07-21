//! Chat / thread-scoped proxy methods: send, abort, session lifecycle,
//! history, approval resolution, transcript export, and compaction.

use super::core::{remote_thread_id, required_remote_thread_id, RemoteGatewayProxy};

impl RemoteGatewayProxy {
    // ── Chat ─────────────────────────────────────────────────────────────────

    /// Send a chat message.
    ///
    /// Remote gateway endpoint: POST /api/chat/send
    /// Body: { session_key, message, stream }
    pub async fn send_message(
        &self,
        session_key: &str,
        text: &str,
    ) -> Result<serde_json::Value, String> {
        let thread_id = if session_key == "agent:main" || session_key.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(session_key.to_string())
        };
        self.post_json(
            "/api/chat/send",
            &serde_json::json!({
                "thread_id": thread_id,
                "content": text,
            }),
        )
        .await
    }

    /// Abort a running chat turn.
    pub async fn abort_chat(&self, session_key: &str) -> Result<(), String> {
        self.post_json(
            "/api/chat/abort",
            &serde_json::json!({ "thread_id": remote_thread_id(session_key) }),
        )
        .await
        .map(|_| ())
    }

    /// Delete a chat session/thread.
    pub async fn delete_session(&self, session_key: &str) -> Result<(), String> {
        if session_key == "agent:main" {
            return Err(Self::unavailable(
                "session delete",
                "the gateway assistant thread is pinned and cannot be deleted",
            ));
        }
        self.delete_json(&format!(
            "/api/chat/thread/{}",
            urlencoding::encode(session_key)
        ))
        .await
        .map(|_| ())
    }

    /// Reset (clear history of) a chat session.
    pub async fn reset_session(&self, session_key: &str) -> Result<(), String> {
        let thread_id = required_remote_thread_id(session_key, "session reset")?;
        self.post_json(
            &format!("/api/chat/thread/{}/reset", urlencoding::encode(&thread_id)),
            &serde_json::json!({ "thread_id": thread_id }),
        )
        .await
        .map(|_| ())
    }

    /// Get all chat sessions/threads.
    pub async fn get_sessions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/chat/threads").await
    }

    /// Get chat history for a session.
    pub async fn get_history(
        &self,
        session_key: &str,
        limit: u32,
        before: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let mut query = format!("/api/chat/history?limit={}", limit);
        if session_key != "agent:main" && !session_key.trim().is_empty() {
            query.push_str("&thread_id=");
            query.push_str(&urlencoding::encode(session_key));
        }
        if let Some(before) = before {
            query.push_str("&before=");
            query.push_str(&urlencoding::encode(before));
        }
        self.get_json(&query).await
    }

    /// Resolve a tool approval request.
    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        approved: bool,
        allow_session: bool,
    ) -> Result<serde_json::Value, String> {
        let action = if approved && allow_session {
            "always"
        } else if approved {
            "approve"
        } else {
            "deny"
        };
        self.post_json(
            "/api/chat/approval",
            &serde_json::json!({
                "request_id": approval_id,
                "action": action,
            }),
        )
        .await
    }

    // ── Export ───────────────────────────────────────────────────────────────

    /// Export a session as a formatted transcript.
    ///
    /// The root gateway has history retrieval but no transcript export endpoint.
    pub async fn export_session(
        &self,
        session_key: &str,
        format: &str,
    ) -> Result<serde_json::Value, String> {
        let thread_id = required_remote_thread_id(session_key, "session export")?;
        self.get_json(&format!(
            "/api/chat/thread/{}/export?format={}",
            urlencoding::encode(&thread_id),
            urlencoding::encode(format)
        ))
        .await
    }

    pub async fn compact_session(&self, session_key: &str) -> Result<serde_json::Value, String> {
        let thread_id = required_remote_thread_id(session_key, "session compaction")?;
        self.post_json(
            &format!(
                "/api/chat/thread/{}/compact",
                urlencoding::encode(&thread_id)
            ),
            &serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }
}

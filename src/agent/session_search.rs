//! Session-search compatibility facade.
//!
//! Transcript windowing and summarization rendering live in `thinclaw-agent`.
//! Root keeps the concrete database adapter and legacy constructor shape.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::session_search::SessionSearchStore;
use thinclaw_history::{ConversationMessage, SessionSearchHit};
use uuid::Uuid;

use crate::db::Database;

pub use thinclaw_agent::session_search::{
    SessionSearchRender, normalize_query_terms, raw_hit_payload, truncate_around_matches,
};

#[derive(Clone)]
pub struct SessionSearchService {
    inner: thinclaw_agent::session_search::SessionSearchService,
}

impl Default for SessionSearchService {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionSearchService {
    pub fn new() -> Self {
        Self {
            inner: thinclaw_agent::session_search::SessionSearchService::new(),
        }
    }

    pub fn with_summarizer(mut self, summarizer: Arc<dyn crate::llm::LlmProvider>) -> Self {
        self.inner = self.inner.with_summarizer(summarizer);
        self
    }

    pub fn summarizer_configured(&self) -> bool {
        self.inner.summarizer_configured()
    }

    pub async fn render_results(
        &self,
        store: &Arc<dyn Database>,
        query: &str,
        hits: Vec<SessionSearchHit>,
        summarize_sessions: bool,
    ) -> SessionSearchRender {
        let store: Arc<dyn SessionSearchStore> = Arc::new(RootSessionSearchStore {
            store: Arc::clone(store),
        });
        self.inner
            .render_results(store, query, hits, summarize_sessions)
            .await
    }
}

struct RootSessionSearchStore {
    store: Arc<dyn Database>,
}

#[async_trait]
impl SessionSearchStore for RootSessionSearchStore {
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> anyhow::Result<Vec<ConversationMessage>> {
        self.store
            .list_conversation_messages(conversation_id)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn truncate_around_matches_returns_recent_tail_without_matches() {
        let transcript = "a".repeat(5_000);
        let truncated = truncate_around_matches(&transcript, &["needle"], 600);
        assert_eq!(truncated.len(), 600);
        assert!(truncated.chars().all(|c| c == 'a'));
    }

    #[test]
    fn truncate_around_matches_windows_around_multiple_hits() {
        let transcript = format!(
            "{} alpha {} beta {} gamma {}",
            "x".repeat(900),
            "y".repeat(2_800),
            "z".repeat(2_800),
            "q".repeat(2_800),
        );
        let truncated = truncate_around_matches(&transcript, &["alpha", "beta", "gamma"], 3_000);
        assert!(truncated.contains("alpha"));
        assert!(truncated.contains("beta"));
        assert!(truncated.contains("gamma"));
        assert!(truncated.contains("\n...\n"));
    }

    #[test]
    fn normalize_query_terms_deduplicates_and_strips_punctuation() {
        let terms = normalize_query_terms("Error, error! build? a");
        assert_eq!(terms, vec!["error".to_string(), "build".to_string()]);
    }

    #[cfg(feature = "libsql")]
    fn make_hit(
        conversation_id: Uuid,
        message_id: Uuid,
        created_at: chrono::DateTime<Utc>,
        score: f64,
    ) -> SessionSearchHit {
        SessionSearchHit {
            conversation_id,
            message_id,
            user_id: "user-1".to_string(),
            actor_id: None,
            channel: "repl".to_string(),
            thread_id: Some("thread-1".to_string()),
            conversation_kind: crate::history::ConversationKind::Direct,
            role: "user".to_string(),
            content: "error while building project".to_string(),
            excerpt: "error while building".to_string(),
            metadata: serde_json::json!({}),
            created_at,
            score: Some(score),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn render_results_batches_summaries_by_conversation() {
        let (db, _guard) = crate::testing::test_db().await;
        let conv_a = db
            .create_conversation("repl", "user-1", Some("thread-1"))
            .await
            .expect("create conv a");
        let conv_b = db
            .create_conversation("repl", "user-1", Some("thread-1"))
            .await
            .expect("create conv b");

        let msg_a = db
            .add_conversation_message(conv_a, "user", "error while building")
            .await
            .expect("insert message a");
        let msg_b = db
            .add_conversation_message(conv_b, "assistant", "fixed after retry")
            .await
            .expect("insert message b");

        let summarizer = Arc::new(crate::testing::StubLlm::new("summary output"));
        let service = SessionSearchService::new()
            .with_summarizer(Arc::clone(&summarizer) as Arc<dyn crate::llm::LlmProvider>);

        let now = Utc::now();
        let hits = vec![
            make_hit(conv_a, msg_a, now, 0.9),
            make_hit(conv_b, msg_b, now, 0.8),
        ];

        let rendered = service.render_results(&db, "error retry", hits, true).await;

        assert!(rendered.summarized);
        assert!(!rendered.fallback);
        assert_eq!(rendered.results.len(), 2);
        assert_eq!(summarizer.calls(), 2, "one summary call per conversation");
        for entry in &rendered.results {
            assert!(entry.get("summary").is_some());
            assert!(entry.get("fallback_hits").is_some());
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn render_results_uses_raw_fallback_when_summarizer_fails() {
        let (db, _guard) = crate::testing::test_db().await;
        let conv = db
            .create_conversation("repl", "user-1", Some("thread-1"))
            .await
            .expect("create conversation");
        let msg = db
            .add_conversation_message(conv, "user", "error happened")
            .await
            .expect("insert message");

        let summarizer = Arc::new(crate::testing::StubLlm::failing("failing-summarizer"));
        let service = SessionSearchService::new()
            .with_summarizer(Arc::clone(&summarizer) as Arc<dyn crate::llm::LlmProvider>);

        let hits = vec![make_hit(conv, msg, Utc::now(), 0.91)];
        let rendered = service.render_results(&db, "error", hits, true).await;

        assert!(!rendered.summarized);
        assert!(rendered.fallback);
        assert_eq!(rendered.results.len(), 1);
        assert!(rendered.results[0].get("message_id").is_some());
        assert_eq!(summarizer.calls(), 1);
    }
}

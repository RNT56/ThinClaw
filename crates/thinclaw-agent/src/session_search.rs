//! Session-search helpers and summarization service.
//!
//! Keeps transcript windowing + cheap-model summarization out of the tool
//! implementation so the tool remains a thin orchestration layer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_history::{ConversationMessage, SessionSearchHit};
use thinclaw_llm_core::{ChatMessage, CompletionRequest, LlmProvider};
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SessionSearchRender {
    pub results: Vec<serde_json::Value>,
    pub summarized: bool,
    pub fallback: bool,
}

#[async_trait]
pub trait SessionSearchStore: Send + Sync {
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> anyhow::Result<Vec<ConversationMessage>>;
}

#[derive(Default, Clone)]
pub struct SessionSearchService {
    summarizer: Option<Arc<dyn LlmProvider>>,
}

impl SessionSearchService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_summarizer(mut self, summarizer: Arc<dyn LlmProvider>) -> Self {
        self.summarizer = Some(summarizer);
        self
    }

    pub fn summarizer_configured(&self) -> bool {
        self.summarizer.is_some()
    }

    pub async fn render_results(
        &self,
        store: Arc<dyn SessionSearchStore>,
        query: &str,
        hits: Vec<SessionSearchHit>,
        summarize_sessions: bool,
    ) -> SessionSearchRender {
        let raw_fallback = hits.iter().map(raw_hit_payload).collect::<Vec<_>>();
        if !summarize_sessions {
            return SessionSearchRender {
                results: raw_fallback,
                summarized: false,
                fallback: false,
            };
        }

        let Some(summarizer) = self.summarizer.as_ref().map(Arc::clone) else {
            return SessionSearchRender {
                results: raw_fallback,
                summarized: false,
                fallback: false,
            };
        };

        let query_terms = normalize_query_terms(query);
        let query_term_refs = query_terms.iter().map(String::as_str).collect::<Vec<_>>();

        let mut per_conversation: Vec<(Uuid, SessionSearchHit, Vec<SessionSearchHit>)> = Vec::new();
        let mut grouped: HashMap<Uuid, Vec<SessionSearchHit>> = HashMap::new();
        for hit in hits {
            grouped.entry(hit.conversation_id).or_default().push(hit);
        }
        for (_conversation_id, group_hits) in grouped {
            let mut ordered_hits = group_hits;
            ordered_hits.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.created_at.cmp(&a.created_at))
            });
            if let Some(primary) = ordered_hits.first().cloned() {
                per_conversation.push((primary.conversation_id, primary, ordered_hits));
            }
        }
        per_conversation.sort_by(|(_, left, _), (_, right, _)| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        per_conversation.truncate(5);

        let max_chars = (100_000 / per_conversation.len().max(1)).max(4_000);
        let mut tasks = JoinSet::new();
        for (conversation_id, primary, grouped_hits) in per_conversation {
            let store = Arc::clone(&store);
            let summarizer = Arc::clone(&summarizer);
            let query_text = query.to_string();
            let query_terms = query_term_refs
                .iter()
                .map(|term| (*term).to_string())
                .collect::<Vec<_>>();
            tasks.spawn(async move {
                let transcript_messages = store
                    .list_conversation_messages(conversation_id)
                    .await
                    .map_err(|e| format!("load transcript failed: {}", e))?;
                let transcript = format_transcript(&transcript_messages);
                let local_refs = query_terms.iter().map(String::as_str).collect::<Vec<_>>();
                let windowed = truncate_around_matches(&transcript, &local_refs, max_chars);
                let prompt = format!(
                    "Summarize what happened in this conversation, focusing on: {}.\nBe concise: 3-5 bullet points max.\nCall out decisions, blockers, and user intent if present.\n\nConversation transcript:\n{}",
                    query_text,
                    windowed
                );
                let request = CompletionRequest::new(vec![
                    ChatMessage::system(
                        "You summarize conversation transcripts for search results. Stay factual, concise, and grounded in the transcript.",
                    ),
                    ChatMessage::user(prompt),
                ])
                .with_max_tokens(220);
                let summary = summarizer
                    .complete(request)
                    .await
                    .map(|response| response.content)
                    .map_err(|e| format!("summary failed: {}", e))?;

                Ok::<serde_json::Value, String>(serde_json::json!({
                    "conversation_id": primary.conversation_id,
                    "user_id": primary.user_id,
                    "actor_id": primary.actor_id,
                    "channel": primary.channel,
                    "thread_id": primary.thread_id,
                    "conversation_kind": primary.conversation_kind.as_str(),
                    "latest_match_at": primary.created_at.to_rfc3339(),
                    "top_score": primary.score,
                    "match_count": grouped_hits.len(),
                    "message_count": transcript_messages.len(),
                    "summary": summary.trim(),
                    "fallback_hits": grouped_hits.iter().take(3).map(raw_hit_payload).collect::<Vec<_>>(),
                }))
            });
        }

        let mut summarized_results = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(summary)) => summarized_results.push(summary),
                Ok(Err(err)) => {
                    tracing::warn!(
                        error = %err,
                        "session_search summarization failed; using fallback hit payload"
                    )
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "session_search summarization task panicked"
                    )
                }
            }
        }

        if summarized_results.is_empty() {
            SessionSearchRender {
                results: raw_fallback,
                summarized: false,
                fallback: true,
            }
        } else {
            SessionSearchRender {
                results: summarized_results,
                summarized: true,
                fallback: false,
            }
        }
    }
}

pub fn normalize_query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split_whitespace()
        .map(|term| {
            term.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|term| term.len() >= 2)
        .filter(|term| seen.insert(term.clone()))
        .collect()
}

fn clamp_char_boundary_start(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn clamp_char_boundary_end(s: &str, mut idx: usize) -> usize {
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx.min(s.len())
}

pub fn truncate_around_matches(transcript: &str, query_terms: &[&str], max_chars: usize) -> String {
    if transcript.is_empty() || max_chars == 0 {
        return String::new();
    }

    let total_chars = transcript.chars().count();
    if total_chars <= max_chars {
        return transcript.to_string();
    }

    let normalized_terms: Vec<String> = query_terms
        .iter()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect();
    if normalized_terms.is_empty() {
        let tail_start = total_chars.saturating_sub(max_chars);
        return transcript.chars().skip(tail_start).collect();
    }

    let lowercase = transcript.to_ascii_lowercase();
    let mut positions = Vec::new();
    for term in &normalized_terms {
        let mut start = 0usize;
        while let Some(found) = lowercase[start..].find(term) {
            positions.push(start + found);
            start += found + term.len().max(1);
            if start >= lowercase.len() {
                break;
            }
        }
    }

    if positions.is_empty() {
        let tail_start = total_chars.saturating_sub(max_chars);
        return transcript.chars().skip(tail_start).collect();
    }

    positions.sort_unstable();
    positions.dedup();

    let window_count = positions.len().max(1);
    let bytes_per_window = (max_chars / window_count).max(200).min(transcript.len());
    let mut windows = positions
        .into_iter()
        .map(|pos| {
            let half = bytes_per_window / 2;
            let start = clamp_char_boundary_start(transcript, pos.saturating_sub(half));
            let end = clamp_char_boundary_end(transcript, (pos + half).min(transcript.len()));
            (start, end)
        })
        .collect::<Vec<_>>();
    windows.sort_unstable_by_key(|(start, _)| *start);

    let mut merged = Vec::new();
    for (start, end) in windows {
        if let Some((_, prev_end)) = merged.last_mut()
            && start <= *prev_end
        {
            *prev_end = (*prev_end).max(end);
            continue;
        }
        merged.push((start, end));
    }

    let mut parts = Vec::new();
    for (start, end) in merged {
        let segment = transcript[start..end].trim();
        if !segment.is_empty() {
            parts.push(segment.to_string());
        }
    }
    if parts.is_empty() {
        let tail_start = total_chars.saturating_sub(max_chars);
        return transcript.chars().skip(tail_start).collect();
    }
    parts.join("\n...\n")
}

fn format_transcript(messages: &[ConversationMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            format!(
                "[{} @ {}]\n{}",
                message.role,
                message.created_at.to_rfc3339(),
                message.content.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn raw_hit_payload(hit: &SessionSearchHit) -> serde_json::Value {
    serde_json::json!({
        "conversation_id": hit.conversation_id,
        "message_id": hit.message_id,
        "user_id": hit.user_id,
        "actor_id": hit.actor_id,
        "channel": hit.channel,
        "thread_id": hit.thread_id,
        "conversation_kind": hit.conversation_kind.as_str(),
        "role": hit.role,
        "created_at": hit.created_at.to_rfc3339(),
        "score": hit.score,
        "excerpt": hit.excerpt,
        "metadata": hit.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
}

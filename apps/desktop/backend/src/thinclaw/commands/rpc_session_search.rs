//! RPC command — cross-session transcript search.
//!
//! Surfaces the core session-search capability (full-text search across stored
//! conversation transcripts, with windowed excerpts and optional LLM
//! summarization) to the desktop. Two steps, both from the core:
//!   1. `Database::search_conversation_messages` produces the raw hits.
//!   2. `SessionSearchService::render_results` windows/renders them (and
//!      optionally summarizes matching sessions via the primary provider).
//!
//! Local (embedded) mode only: the search runs against the local conversation
//! database. In remote-gateway mode it returns a typed `BridgeError::Unavailable`
//! (the gateway exposes its own transcript search that is not yet proxied here).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::thinclaw::bridge::{gated, BridgeError, RouteMode};
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const USER_ID: &str = "local_user";

/// Rendered cross-session search results.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SessionSearchResult {
    /// One rendered hit per result (raw JSON: session/thread id, excerpt, score, …).
    pub results: Vec<serde_json::Value>,
    /// Whether matching sessions were LLM-summarized.
    pub summarized: bool,
    /// Whether the renderer fell back to raw excerpts (e.g. no summarizer).
    pub fallback: bool,
}

/// Search stored conversation transcripts for `query` and render windowed
/// excerpts; optionally summarize matching sessions.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_session_search(
    ironclaw: State<'_, ThinClawRuntimeState>,
    query: String,
    limit: Option<u32>,
    summarize: Option<bool>,
) -> Result<SessionSearchResult, BridgeError> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(gated(
            "session transcript search",
            "desktop transcript search runs against the local conversation database; \
             remote-gateway transcript search is not yet bridged",
            "run the desktop in local (embedded) mode to search session transcripts",
            RouteMode::LocalOnly,
        ));
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("conversation database not available")?;
    let limit = limit.unwrap_or(20).clamp(1, 200) as i64;
    let summarize = summarize.unwrap_or(false);

    let hits = store
        .search_conversation_messages(USER_ID, &query, None, None, None, limit)
        .await
        .map_err(|e| e.to_string())?;

    let mut service = thinclaw_core::agent::session_search::SessionSearchService::new();
    if summarize {
        if let Ok(llm) = ironclaw.llm_runtime().await {
            service = service.with_summarizer(llm.primary_handle());
        }
    }

    let render = service.render_results(store, &query, hits, summarize).await;
    Ok(SessionSearchResult {
        results: render.results,
        summarized: render.summarized,
        fallback: render.fallback,
    })
}

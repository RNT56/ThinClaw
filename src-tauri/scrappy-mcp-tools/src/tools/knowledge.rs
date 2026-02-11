use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub content: String,
    pub url: Option<String>,
    pub score: f32,
    pub source_type: String, // web, file, db, etc.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeResult {
    pub hits: Vec<SearchHit>,
    pub summary: Option<String>,
    pub references: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Retrieve context from the remote knowledge base (RAG).
pub async fn rag_query(
    client: &McpClient,
    query: &str,
    top_k: Option<usize>,
) -> McpResult<KnowledgeResult> {
    let mut params = serde_json::json!({ "query": query });
    if let Some(k) = top_k {
        params["top_k"] = serde_json::json!(k);
    }
    client.call_tool("rag_query", params).await
}

/// Search for available sources/datasets.
pub async fn get_sources(client: &McpClient) -> McpResult<Vec<String>> {
    client.call_tool("get_sources", serde_json::json!({})).await
}

/// Search within a specific source/domain.
pub async fn search_knowledge(
    client: &McpClient,
    query: &str,
    source_id: &str,
) -> McpResult<Vec<SearchHit>> {
    client
        .call_tool(
            "search_knowledge",
            serde_json::json!({ "query": query, "source_id": source_id }),
        )
        .await
}

use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryResult {
    pub original_length: usize,
    pub summary: String,
    pub key_points: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Summarize lengthy text content.
pub async fn summarize_text(
    client: &McpClient,
    text: &str,
    target_length: Option<&str>, // "short", "medium", "long"
) -> McpResult<SummaryResult> {
    let mut params = serde_json::json!({ "text": text });
    if let Some(tl) = target_length {
        params["target_length"] = serde_json::json!(tl);
    }
    client.call_tool("summarize_text", params).await
}

/// Extract key entities or structured data from text.
pub async fn extract_entities(
    client: &McpClient,
    text: &str,
    entity_types: &[&str], // ["person", "org", "location"]
) -> McpResult<Vec<serde_json::Value>> {
    client
        .call_tool(
            "extract_entities",
            serde_json::json!({ "text": text, "entity_types": entity_types }),
        )
        .await
}

/// Re-rank a list of documents based on relevance to a query.
pub async fn rerank_documents(
    client: &McpClient,
    query: &str,
    documents: Vec<String>,
) -> McpResult<Vec<(usize, f32)>> {
    client
        .call_tool(
            "rerank_documents",
            serde_json::json!({ "query": query, "documents": documents }),
        )
        .await
}

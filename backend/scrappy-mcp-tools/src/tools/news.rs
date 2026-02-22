use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub source: String,
    pub published_at: String,
    pub summary: Option<String>,
    pub author: Option<String>,
    pub sentiment: Option<String>, // "positive", "negative", "neutral"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsSource {
    pub id: String,
    pub name: String,
    pub category: String,
    pub language: String,
    pub country: String,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Get latest news by category or general headlines.
pub async fn get_news(
    client: &McpClient,
    category: Option<&str>,
    limit: Option<usize>,
) -> McpResult<Vec<NewsItem>> {
    let mut params = serde_json::json!({});
    if let Some(c) = category {
        params["category"] = serde_json::json!(c);
    }
    if let Some(l) = limit {
        params["limit"] = serde_json::json!(l);
    }
    client.call_tool("get_news", params).await
}

/// Search news by query string.
pub async fn search_news(
    client: &McpClient,
    query: &str,
    limit: Option<usize>,
) -> McpResult<Vec<NewsItem>> {
    let mut params = serde_json::json!({ "query": query });
    if let Some(l) = limit {
        params["limit"] = serde_json::json!(l);
    }
    client.call_tool("search_news", params).await
}

/// Get top headlines for a specific country.
pub async fn get_headlines(
    client: &McpClient,
    country: &str,
    limit: Option<usize>,
) -> McpResult<Vec<NewsItem>> {
    let mut params = serde_json::json!({ "country": country });
    if let Some(l) = limit {
        params["limit"] = serde_json::json!(l);
    }
    client.call_tool("get_headlines", params).await
}

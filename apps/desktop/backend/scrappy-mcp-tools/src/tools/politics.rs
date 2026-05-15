use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoliticalEvent {
    pub title: String,
    pub description: String,
    pub date: String,
    pub location: Option<String>,
    pub importance: String, // "critical", "high", "medium", "low"
    pub parties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResult {
    pub pollster: String,
    pub date: String,
    pub candidates: Vec<(String, f32)>,
    pub margin_of_error: Option<f32>,
    pub sample_size: Option<usize>,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Get upcoming political events (elections, key bills).
pub async fn get_political_events(
    client: &McpClient,
    country: Option<&str>,
) -> McpResult<Vec<PoliticalEvent>> {
    let mut params = serde_json::json!({});
    if let Some(c) = country {
        params["country"] = serde_json::json!(c);
    }
    client.call_tool("get_political_events", params).await
}

/// Search for legislation or policy docs.
pub async fn search_legislation(
    client: &McpClient,
    query: &str,
    jurisdiction: Option<&str>,
) -> McpResult<Vec<serde_json::Value>> {
    let mut params = serde_json::json!({ "query": query });
    if let Some(j) = jurisdiction {
        params["jurisdiction"] = serde_json::json!(j);
    }
    client.call_tool("search_legislation", params).await
}

/// Get latest polling data for an upcoming election.
pub async fn get_election_polls(
    client: &McpClient,
    election_id: &str,
) -> McpResult<Vec<PollResult>> {
    client
        .call_tool(
            "get_election_polls",
            serde_json::json!({ "election_id": election_id }),
        )
        .await
}

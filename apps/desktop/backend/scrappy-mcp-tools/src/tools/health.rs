use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedicalStudy {
    pub title: String,
    pub authors: Vec<String>,
    pub journal: String,
    pub date: String,
    pub abstract_text: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStat {
    pub topic: String,
    pub metric: String,
    pub value: f64,
    pub unit: String,
    pub year: u16,
    pub region: String,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Search medical databases (PubMed, CDC, WHO).
pub async fn search_medical_research(
    client: &McpClient,
    query: &str,
    limit: Option<usize>,
) -> McpResult<Vec<MedicalStudy>> {
    let mut params = serde_json::json!({ "query": query });
    if let Some(l) = limit {
        params["limit"] = serde_json::json!(l);
    }
    client.call_tool("search_medical_research", params).await
}

/// Get public health statistics for a region.
pub async fn get_health_stats(
    client: &McpClient,
    topic: &str,
    region: Option<&str>,
) -> McpResult<Vec<HealthStat>> {
    let mut params = serde_json::json!({ "topic": topic });
    if let Some(r) = region {
        params["region"] = serde_json::json!(r);
    }
    client.call_tool("get_health_stats", params).await
}

/// Lookup drug information (interactions, side effects).
pub async fn get_drug_info(client: &McpClient, drug_name: &str) -> McpResult<serde_json::Value> {
    client
        .call_tool(
            "get_drug_info",
            serde_json::json!({ "drug_name": drug_name }),
        )
        .await
}

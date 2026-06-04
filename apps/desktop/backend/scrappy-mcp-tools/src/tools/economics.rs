use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicIndicator {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub country: String,
    pub period: String,
    pub previous: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicReport {
    pub title: String,
    pub indicators: Vec<EconomicIndicator>,
    pub summary: String,
    pub source: String,
    pub published_at: String,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Get key economic indicators for a country (GDP, Inflation, Unemployment).
pub async fn get_economic_data(
    client: &McpClient,
    country: &str,
    indicator: Option<&str>,
) -> McpResult<Vec<EconomicIndicator>> {
    let mut params = serde_json::json!({ "country": country });
    if let Some(ind) = indicator {
        params["indicator"] = serde_json::json!(ind);
    }
    client.call_tool("get_economic_data", params).await
}

/// Compare economic metrics between two or more countries.
pub async fn compare_economies(
    client: &McpClient,
    countries: &[&str],
    indicator: &str,
) -> McpResult<serde_json::Value> {
    client
        .call_tool(
            "compare_economies",
            serde_json::json!({ "countries": countries, "indicator": indicator }),
        )
        .await
}

/// Search for upcoming economic calendar events.
pub async fn get_economic_calendar(
    client: &McpClient,
    country: Option<&str>,
    impact: Option<&str>, // "low", "medium", "high"
) -> McpResult<Vec<EconomicReport>> {
    let mut params = serde_json::json!({});
    if let Some(c) = country {
        params["country"] = serde_json::json!(c);
    }
    if let Some(i) = impact {
        params["impact"] = serde_json::json!(i);
    }
    client.call_tool("get_economic_calendar", params).await
}

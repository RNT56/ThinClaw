use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockPrice {
    pub symbol: String,
    pub name: Option<String>,
    pub price: f64,
    pub change: f64,
    pub change_percent: f64,
    pub volume: u64,
    pub market_cap: Option<f64>,
    pub asset_type: String,
    pub currency: String,
    pub timestamp: String,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub pe_ratio: Option<f64>,
    pub dividend_yield: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSummary {
    pub indices: Vec<StockPrice>,
    pub top_stocks: Vec<StockPrice>,
    pub top_crypto: Vec<StockPrice>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolCategory {
    pub count: usize,
    pub symbols: Vec<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedSymbols {
    pub total_tracked: usize,
    pub categories: HashMap<String, SymbolCategory>,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// Get current price for any financial symbol.
pub async fn get_stock_price(client: &McpClient, symbol: &str) -> McpResult<StockPrice> {
    client
        .call_tool("get_stock_price", serde_json::json!({ "symbol": symbol }))
        .await
}

/// Get overall market summary with indices, top stocks, and crypto.
pub async fn get_market_summary(client: &McpClient) -> McpResult<MarketSummary> {
    client
        .call_tool("get_market_summary", serde_json::json!({}))
        .await
}

/// Get current cryptocurrency prices.
pub async fn get_crypto_prices(client: &McpClient, symbols: &[&str]) -> McpResult<Vec<StockPrice>> {
    client
        .call_tool(
            "get_crypto_prices",
            serde_json::json!({ "symbols": symbols }),
        )
        .await
}

/// Search for financial symbols by name or ticker.
pub async fn search_financial_symbols(
    client: &McpClient,
    query: &str,
    asset_type: Option<&str>,
) -> McpResult<Vec<StockPrice>> {
    let mut params = serde_json::json!({ "query": query });
    if let Some(at) = asset_type {
        params["asset_type"] = serde_json::json!(at);
    }
    client.call_tool("search_financial_symbols", params).await
}

/// List all symbols tracked by periodic scraping.
pub async fn list_tracked_symbols(client: &McpClient, category: &str) -> McpResult<TrackedSymbols> {
    client
        .call_tool(
            "list_tracked_symbols",
            serde_json::json!({ "category": category }),
        )
        .await
}

/// Batch price query – fetches multiple symbols concurrently.
pub async fn get_stock_price_batch(
    client: &McpClient,
    symbols: &[&str],
) -> McpResult<Vec<StockPrice>> {
    let futures: Vec<_> = symbols.iter().map(|s| get_stock_price(client, s)).collect();

    let results = futures::future::join_all(futures).await;
    results.into_iter().collect()
}

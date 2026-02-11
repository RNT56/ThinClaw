use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::debug;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum DetailLevel {
    /// Just category names and tool counts
    Categories,
    /// Tool names with one-line descriptions
    Names,
    /// Full definitions with parameter schemas
    Full,
}

impl DetailLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Categories => "categories",
            Self::Names => "names",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCategory {
    pub description: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub categories: Option<HashMap<String, ToolCategory>>,
    #[serde(default)]
    pub tools: Option<Vec<ToolInfo>>,
}

/// A standard MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ---------------------------------------------------------------------------
// search_tools client helper
// ---------------------------------------------------------------------------

pub async fn search_tools(
    client: &McpClient,
    query: &str,
    detail: DetailLevel,
) -> McpResult<SearchResult> {
    debug!(
        "[discovery] search_tools query='{}' detail='{}'",
        query,
        detail.as_str()
    );
    client
        .call_tool(
            "search_tools",
            serde_json::json!({
                "query": query,
                "detail": detail.as_str(),
            }),
        )
        .await
}

// ---------------------------------------------------------------------------
// Registry cache
// ---------------------------------------------------------------------------

pub struct ToolRegistryCache {
    categories: HashMap<String, ToolCategory>,
    tools: HashMap<String, ToolInfo>,
    last_refreshed: Option<Instant>,
    ttl: Duration,
}

impl ToolRegistryCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            categories: HashMap::new(),
            tools: HashMap::new(),
            last_refreshed: None,
            ttl,
        }
    }

    fn is_stale(&self) -> bool {
        match self.last_refreshed {
            Some(t) => t.elapsed() > self.ttl,
            None => true,
        }
    }

    /// Get or refresh the category list.
    pub async fn get_categories(
        &mut self,
        client: &McpClient,
    ) -> McpResult<&HashMap<String, ToolCategory>> {
        if self.is_stale() {
            let result = search_tools(client, "", DetailLevel::Categories).await?;
            if let Some(cats) = result.categories {
                self.categories = cats;
            }
            self.last_refreshed = Some(Instant::now());
        }
        Ok(&self.categories)
    }

    /// Get a specific tool's info, fetching from server if not cached.
    pub async fn get_tool(
        &mut self,
        client: &McpClient,
        name: &str,
    ) -> McpResult<Option<&ToolInfo>> {
        if !self.tools.contains_key(name) {
            let result = search_tools(client, name, DetailLevel::Full).await?;
            if let Some(tool_list) = result.tools {
                for tool in tool_list {
                    self.tools.insert(tool.name.clone(), tool);
                }
            }
        }
        Ok(self.tools.get(name))
    }
}

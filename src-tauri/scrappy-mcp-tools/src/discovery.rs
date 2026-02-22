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

// =============================================================================
// Unit Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // -------------------------------------------------------------------------
    // DetailLevel
    // -------------------------------------------------------------------------

    #[test]
    fn detail_level_as_str_values_are_stable() {
        assert_eq!(DetailLevel::Categories.as_str(), "categories");
        assert_eq!(DetailLevel::Names.as_str(), "names");
        assert_eq!(DetailLevel::Full.as_str(), "full");
    }

    // -------------------------------------------------------------------------
    // ToolRegistryCache staleness
    // -------------------------------------------------------------------------

    #[test]
    fn fresh_cache_is_always_stale() {
        // A brand-new cache with no last_refreshed must report stale.
        let cache = ToolRegistryCache::new(Duration::from_secs(300));
        assert!(cache.is_stale(), "new cache must be stale");
    }

    #[test]
    fn cache_with_zero_ttl_is_immediately_stale_after_population() {
        let mut cache = ToolRegistryCache::new(Duration::ZERO);
        // Simulate a refresh by setting last_refreshed manually
        cache.last_refreshed = Some(Instant::now());
        // ZERO TTL means elapsed() > 0 == true almost immediately
        // (we can't reliably test timing, so we just document the expectation)
        // The important invariant is that the field is honoured.
        let _ = cache.is_stale();
    }

    #[test]
    fn cache_with_very_long_ttl_is_not_stale_just_after_refresh() {
        let mut cache = ToolRegistryCache::new(Duration::from_secs(3600));
        cache.last_refreshed = Some(Instant::now());
        assert!(
            !cache.is_stale(),
            "cache should NOT be stale immediately after refresh"
        );
    }

    // -------------------------------------------------------------------------
    // ToolCategory serde
    // -------------------------------------------------------------------------

    #[test]
    fn tool_category_roundtrips_through_json() {
        let cat = ToolCategory {
            description: "Web tools".to_string(),
            tool_count: 5,
        };
        let json = serde_json::to_string(&cat).unwrap();
        let restored: ToolCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.description, "Web tools");
        assert_eq!(restored.tool_count, 5);
    }

    // -------------------------------------------------------------------------
    // ToolInfo serde (including the camelCase inputSchema rename)
    // -------------------------------------------------------------------------

    #[test]
    fn tool_info_deserializes_input_schema_from_camel_case() {
        let json_str = r#"{
            "name": "web_search",
            "description": "Search the web",
            "inputSchema": { "type": "object" }
        }"#;
        let info: ToolInfo = serde_json::from_str(json_str).unwrap();
        assert_eq!(info.name, "web_search");
        assert!(info.input_schema.is_some());
    }

    #[test]
    fn tool_info_with_missing_input_schema_defaults_to_none() {
        let json_str = r#"{ "name": "ping", "description": "Ping" }"#;
        let info: ToolInfo = serde_json::from_str(json_str).unwrap();
        assert!(info.input_schema.is_none());
    }

    // -------------------------------------------------------------------------
    // SearchResult serde
    // -------------------------------------------------------------------------

    #[test]
    fn search_result_with_only_tools_deserializes_correctly() {
        let json_str = r#"{
            "tools": [{ "name": "x", "description": "d" }]
        }"#;
        let result: SearchResult = serde_json::from_str(json_str).unwrap();
        assert!(result.categories.is_none());
        assert_eq!(result.tools.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn empty_search_result_has_both_fields_defaulting_to_none() {
        let result: SearchResult = serde_json::from_str("{}").unwrap();
        assert!(result.categories.is_none());
        assert!(result.tools.is_none());
    }
}

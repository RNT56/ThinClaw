use scrappy_mcp_tools::discovery::{DetailLevel, SearchResult, ToolInfo};
use scrappy_mcp_tools::skills::manager::SkillManager;
use scrappy_mcp_tools::McpClient;
use serde_json::json;
use tracing::{debug, warn};

/// Searches across Host tools, Skills, and Remote MCP tools.
pub async fn search_all_tools(
    query: &str,
    mcp_client: Option<&McpClient>,
    skill_manager: Option<&SkillManager>,
    include_host: bool,
) -> SearchResult {
    let mut all_tools = Vec::new();
    let query_lower = query.to_lowercase();

    // 1. Host Tools
    if include_host {
        let host_tools = get_host_tools_definitions();
        for tool in host_tools {
            if query_lower.is_empty()
                || tool.name.to_lowercase().contains(&query_lower)
                || tool.description.to_lowercase().contains(&query_lower)
            {
                all_tools.push(tool);
            }
        }
    }

    // 2. Skills
    if let Some(mgr) = skill_manager {
        if let Ok(skills) = mgr.list_skills() {
            for s in skills {
                if query_lower.is_empty()
                    || s.id.to_lowercase().contains(&query_lower)
                    || s.manifest.description.to_lowercase().contains(&query_lower)
                {
                    // Map skill parameters to JSON Schema
                    let mut properties = serde_json::Map::new();
                    let mut required = Vec::new();

                    for p in &s.manifest.parameters {
                        properties.insert(
                            p.name.clone(),
                            json!({
                                "type": p.param_type,
                                "description": p.description,
                            }),
                        );
                        if p.required {
                            required.push(p.name.clone());
                        }
                    }

                    let schema = json!({
                        "type": "object",
                        "properties": properties,
                        "required": required
                    });

                    all_tools.push(ToolInfo {
                        name: s.id.clone(),
                        description: format!("[Skill] {}", s.manifest.description),
                        input_schema: Some(schema),
                    });
                }
            }
        }
    }

    // 3. Remote Tools
    if let Some(client) = mcp_client {
        debug!("[discovery] searching remote tools for '{}'", query);
        match scrappy_mcp_tools::discovery::search_tools(client, query, DetailLevel::Full).await {
            Ok(remote_result) => {
                if let Some(remote_tools) = remote_result.tools {
                    all_tools.extend(remote_tools);
                }
            }
            Err(e) => {
                warn!("[discovery] failed to search remote tools: {}", e);
            }
        }
    }

    SearchResult {
        categories: None,
        tools: Some(all_tools),
    }
}

pub fn get_host_tools_definitions() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "web_search".into(),
            description: "Full text search of web content via DuckDuckGo".into(),
            input_schema: Some(json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            })),
        },
        ToolInfo {
            name: "rag_search".into(),
            description: "Search local documents and knowledge base (vector embeddings)".into(),
            input_schema: Some(json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            })),
        },
        ToolInfo {
            name: "read_file".into(),
            description: "Read file contents (sandbox restricted, read-only)".into(),
            input_schema: Some(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to file" }
                },
                "required": ["path"]
            })),
        },
    ]
}

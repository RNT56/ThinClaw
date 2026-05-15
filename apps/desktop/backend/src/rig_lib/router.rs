use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decision {
    NoTool,
    Rag,
    Web,
    RagAndWeb,
    Image,
    Clarify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStep {
    pub tool_name: String,
    pub input: Value,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPlan {
    pub decision: Decision,
    pub reason: String,
    pub steps: Vec<ToolStep>,
    pub response_style: String, // "brief", "normal", "detailed"
}

impl Default for ToolPlan {
    fn default() -> Self {
        Self {
            decision: Decision::NoTool,
            reason: "Default to chat".into(),
            steps: vec![],
            response_style: "normal".into(),
        }
    }
}

pub struct Router;

impl Router {
    pub fn plan(user_query: &str, has_attachments: bool, project_id: Option<String>) -> ToolPlan {
        // 1. Check for Attachments -> likely RAG or Image Analysis (future)
        if has_attachments {
            // content-aware routing would go here. For now, assume RAG/Context needed.
            return ToolPlan {
                decision: Decision::NoTool, // Rig handles attachments internally currently
                reason: "Attachments present, context will be passed to model.".into(),
                steps: vec![],
                response_style: "normal".into(),
            };
        }

        let query_lower = user_query.to_lowercase();

        // 2. Deterministic: Image Generation
        if query_lower.starts_with("draw")
            || query_lower.starts_with("generate image")
            || query_lower.starts_with("create picture")
            || query_lower.contains("drawing of")
        {
            return ToolPlan {
                decision: Decision::Image,
                reason: "User explicitly requested an image.".into(),
                steps: vec![ToolStep {
                    tool_name: "generate_image".into(),
                    input: serde_json::json!({ "prompt": user_query }),
                    priority: 1,
                }],
                response_style: "brief".into(),
            };
        }

        // 3. Deterministic: Web Search (Recent/Facts)
        // Keywords: price, news, latest, weather, who is X
        // "Who is Max" is dangerous if Max is the user. But "Who is <Celebrity>" needs search.
        // We'll rely on the Router/Orchestrator to filter Knowledge first?
        if query_lower.contains("price")
            || query_lower.contains("news")
            || query_lower.contains("weather")
            || query_lower.contains("latest")
            || query_lower.starts_with("search for")
        {
            return ToolPlan {
                decision: Decision::Web,
                reason: "User asked for real-time/factual info.".into(),
                steps: vec![ToolStep {
                    tool_name: "web_search".into(),
                    input: serde_json::json!({ "query": user_query }),
                    priority: 1,
                }],
                response_style: "normal".into(),
            };
        }

        // 4. Project Context RAG
        if let Some(pid) = project_id {
            let q = query_lower.as_str();
            if q.contains("code")
                || q.contains("file")
                || q.contains("how")
                || q.contains("implement")
                || q.contains("fix")
                || q.contains("where is")
                || q.contains("structure")
                || q.contains("document")
                || q.contains("doc")
                || q.contains("summary")
                || q.contains("context")
                || q.contains("what is this")
                || user_query.len() > 15
            // Check context for medium-length queries
            {
                return ToolPlan {
                    decision: Decision::Rag,
                    reason: "Project context active. Checking artifacts.".into(),
                    steps: vec![ToolStep {
                        tool_name: "rag_retrieve".into(),
                        input: serde_json::json!({ "query": user_query, "project_id": pid }),
                        priority: 1,
                    }],
                    response_style: "normal".into(),
                };
            }
        }

        // 5. Deterministic: Chat (Greetings / Short)
        let words: Vec<&str> = user_query.split_whitespace().collect();
        if words.len() < 3 {
            // "Hey", "Hello there", "Who are you?"
            return ToolPlan::default();
        }

        // 6. Fallback: Default to Chat (No LLM Classifier yet to save latency)
        // Future: Call a mini-LLM here to decide.
        ToolPlan::default()
    }
}

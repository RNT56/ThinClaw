use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Citation {
    pub source_id: String, // URL or Doc ID
    pub title: String,
    pub loc: Option<String>, // Page number or snippet
    pub confidence: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolArtifact {
    pub kind: String, // "image", "text", "json"
    pub uri: String,
    pub meta: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolResult {
    pub ok: bool,
    pub summary: String, // For UI "Searching..."
    pub data: Value,     // Raw data for LLM
    pub citations: Vec<Citation>,
    pub artifacts: Vec<ToolArtifact>,
    pub timings_ms: Option<u64>,
}

impl ToolResult {
    pub fn success(summary: String, data: Value) -> Self {
        Self {
            ok: true,
            summary,
            data,
            citations: vec![],
            artifacts: vec![],
            timings_ms: None,
        }
    }

    pub fn error(msg: String) -> Self {
        Self {
            ok: false,
            summary: msg.clone(),
            data: serde_json::json!({ "error": msg }),
            citations: vec![],
            artifacts: vec![],
            timings_ms: None,
        }
    }
}

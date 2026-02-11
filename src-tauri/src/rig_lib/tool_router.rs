use scrappy_mcp_tools::skills::manager::SkillManager;
use scrappy_mcp_tools::McpClient;
use serde_json::{json, Value};
use tracing::info;

pub struct ToolRouter<'a> {
    pub mcp_client: Option<&'a McpClient>,
    pub skill_manager: Option<&'a SkillManager>,
    pub sandbox: Option<&'a scrappy_mcp_tools::Sandbox>,
}

impl<'a> ToolRouter<'a> {
    pub async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        info!("[router] calling tool '{}' with args: {}", name, args);

        // 1. Check if it's a Skill
        if let Some(mgr) = self.skill_manager {
            if mgr.get_skill(name).is_ok() {
                info!("[router] routing '{}' to Skills System", name);
                // We need a script to run the skill via sandbox if available,
                // or run it directly if we have a runner.
                // For IPC, we'll use the sandbox.
                if let Some(sb) = self.sandbox {
                    let args_json = serde_json::to_string(&args).unwrap_or_default();
                    let script = format!(
                        "run_skill(\"{}\", '{}')",
                        name,
                        args_json.replace("'", "\\'")
                    );
                    let res = sb
                        .execute(&script)
                        .map_err(|e| format!("Skill execution failed: {:?}", e))?;
                    return Ok(json!({
                        "content": [{ "type": "text", "text": res.output }],
                        "isError": false
                    }));
                }
            }
        }

        // 2. Check if it's a Host Tool
        match name {
            "web_search" | "rag_search" | "read_file" => {
                info!("[router] routing '{}' to Host Tools", name);
                if let Some(sb) = self.sandbox {
                    let query = args
                        .get("query")
                        .or_else(|| args.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let script = format!("{}(\"{}\")", name, query.replace("\"", "\\\""));
                    let res = sb
                        .execute(&script)
                        .map_err(|e| format!("Host tool execution failed: {:?}", e))?;
                    return Ok(json!({
                        "content": [{ "type": "text", "text": res.output }],
                        "isError": false
                    }));
                }
            }
            _ => {}
        }

        // 3. Remote MCP Tools
        if let Some(client) = self.mcp_client {
            info!("[router] routing '{}' to Remote MCP", name);
            match client.call_tool_raw(name, args).await {
                Ok(res) => return Ok(res),
                Err(e) => return Err(format!("Remote MCP error: {}", e)),
            }
        }

        Err(format!(
            "Tool '{}' not found or no router configured for it",
            name
        ))
    }
}

/// Middleware to summarize/truncate long tool results
pub fn summarize_result(mut result: Value, max_chars: usize) -> Value {
    if let Some(content) = result.get_mut("content").and_then(|c| c.as_array_mut()) {
        for item in content {
            if let Some(text) = item.get_mut("text").and_then(|t| t.as_str()) {
                if text.len() > max_chars {
                    let truncated = format!(
                        "{}... [Truncated {} chars]",
                        &text[0..max_chars],
                        text.len() - max_chars
                    );
                    *item.get_mut("text").unwrap() = Value::String(truncated);
                }
            }
        }
    }
    result
}

/// Generic JSON summarizer for arbitrary structures
pub fn summarize_arbitrary_json(val: Value, max_string_len: usize, max_array_len: usize) -> Value {
    match val {
        Value::String(s) => {
            if s.len() > max_string_len {
                let truncated = format!(
                    "{}... [Truncated {} chars]",
                    &s[0..max_string_len],
                    s.len() - max_string_len
                );
                Value::String(truncated)
            } else {
                Value::String(s)
            }
        }
        Value::Array(arr) => {
            let mut new_arr = Vec::new();
            let len = arr.len();
            for (i, item) in arr.into_iter().enumerate() {
                if i >= max_array_len {
                    new_arr.push(Value::String(format!(
                        "... [{} more items truncated]",
                        len - max_array_len
                    )));
                    break;
                }
                new_arr.push(summarize_arbitrary_json(
                    item,
                    max_string_len,
                    max_array_len,
                ));
            }
            Value::Array(new_arr)
        }
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(
                    k,
                    summarize_arbitrary_json(v, max_string_len, max_array_len),
                );
            }
            Value::Object(new_map)
        }
        other => other,
    }
}

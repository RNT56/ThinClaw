use serde_json::{json, Value};
use thinclaw_desktop_tools::skills::manager::SkillManager;
use thinclaw_desktop_tools::McpClient;
use tracing::info;

/// Registry-based tool router. Host tools are auto-discovered from
/// `tool_discovery::get_host_tools_definitions()` — adding a new host tool
/// there automatically makes it routable here without any extra plumbing.
pub struct ToolRouter<'a> {
    pub mcp_client: Option<&'a McpClient>,
    pub skill_manager: Option<&'a SkillManager>,
    pub sandbox: Option<&'a thinclaw_desktop_tools::Sandbox>,
}

impl<'a> ToolRouter<'a> {
    /// Returns the set of host-tool names that are currently registered.
    /// Derived from the single source-of-truth in `tool_discovery`.
    fn host_tool_names() -> Vec<String> {
        crate::rig_lib::tool_discovery::get_host_tools_definitions()
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    pub async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        info!("[router] calling tool '{}' with args: {}", name, args);

        // 1. Check if it's a Skill
        if let Some(mgr) = self.skill_manager {
            if mgr.get_skill(name).is_ok() {
                info!("[router] routing '{}' to Skills System", name);
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

        // 2. Check if it's a Host Tool (registry-driven, no hardcoded names)
        let known_host_tools = Self::host_tool_names();
        if known_host_tools.iter().any(|t| t == name) {
            info!("[router] routing '{}' to Host Tools (registry)", name);
            if let Some(sb) = self.sandbox {
                // Build argument string from the first meaningful arg value.
                // Host tools registered in the sandbox accept a single positional
                // string argument (query or path).
                let arg_value = args
                    .as_object()
                    .and_then(|obj| {
                        obj.values()
                            .next()
                            .and_then(|v| v.as_str().map(String::from))
                    })
                    .unwrap_or_default();

                // Escape backslashes first, then double quotes, to prevent Rhai injection.
                // Order matters: escaping " before \ would break on inputs like: hello\"world
                let escaped = arg_value.replace('\\', "\\\\").replace('"', "\\\"");
                let script = format!("{}(\"{}\")", name, escaped);
                let res = sb
                    .execute(&script)
                    .map_err(|e| format!("Host tool execution failed: {:?}", e))?;
                return Ok(json!({
                    "content": [{ "type": "text", "text": res.output }],
                    "isError": false
                }));
            }
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
                    // Find a safe byte boundary to avoid panicking on multi-byte UTF-8
                    let safe_end = safe_char_boundary(text, max_chars);
                    let truncated = format!(
                        "{}... [Truncated {} chars]",
                        &text[..safe_end],
                        text.len() - safe_end
                    );
                    *item.get_mut("text").unwrap() = Value::String(truncated);
                }
            }
        }
    }
    result
}

/// Generic JSON summarizer for arbitrary structures
/// Find the largest byte index <= `target` that falls on a UTF-8 char boundary.
fn safe_char_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    // Walk backwards from target to find a valid char boundary
    let mut idx = target;
    while !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

pub fn summarize_arbitrary_json(val: Value, max_string_len: usize, max_array_len: usize) -> Value {
    match val {
        Value::String(s) => {
            if s.len() > max_string_len {
                // Find a safe byte boundary to avoid panicking on multi-byte UTF-8
                let safe_end = safe_char_boundary(&s, max_string_len);
                let truncated = format!(
                    "{}... [Truncated {} chars]",
                    &s[..safe_end],
                    s.len() - safe_end
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

// =============================================================================
// Unit Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // host_tool_names (registry-driven)
    // -------------------------------------------------------------------------

    #[test]
    fn host_tool_names_contains_known_tools() {
        let names = ToolRouter::host_tool_names();
        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"rag_search".to_string()));
        assert!(names.contains(&"read_file".to_string()));
    }

    // -------------------------------------------------------------------------
    // summarize_result
    // -------------------------------------------------------------------------

    fn make_tool_result(text: &str) -> Value {
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": false
        })
    }

    #[test]
    fn summarize_result_leaves_short_text_unchanged() {
        let result = make_tool_result("hello world");
        let out = summarize_result(result, 100);
        let text = out["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn summarize_result_leaves_exactly_limit_length_unchanged() {
        let text_at_limit = "a".repeat(50);
        let result = make_tool_result(&text_at_limit);
        let out = summarize_result(result, 50);
        // len == limit, NOT >, so no truncation should happen
        let text = out["content"][0]["text"].as_str().unwrap();
        assert_eq!(text.len(), 50);
        assert!(!text.contains("Truncated"));
    }

    #[test]
    fn summarize_result_truncates_text_exceeding_limit() {
        let long_text = "x".repeat(200);
        let result = make_tool_result(&long_text);
        let out = summarize_result(result, 50);
        let text = out["content"][0]["text"].as_str().unwrap();

        assert!(text.starts_with(&"x".repeat(50)));
        assert!(text.contains("Truncated 150 chars"));
    }

    #[test]
    fn summarize_result_truncation_message_is_accurate() {
        let long_text = "z".repeat(7500);
        let result = make_tool_result(&long_text);
        let out = summarize_result(result, 5000);
        let text = out["content"][0]["text"].as_str().unwrap();

        assert!(
            text.contains("Truncated 2500 chars"),
            "expected 2500 chars in message, got: {text}"
        );
    }

    #[test]
    fn summarize_result_handles_multiple_content_items() {
        let result = json!({
            "content": [
                { "type": "text", "text": "short" },
                { "type": "text", "text": "x".repeat(200) }
            ],
            "isError": false
        });
        let out = summarize_result(result, 50);
        let first = out["content"][0]["text"].as_str().unwrap();
        let second = out["content"][1]["text"].as_str().unwrap();

        assert_eq!(first, "short");
        assert!(
            second.contains("Truncated"),
            "second item should be truncated"
        );
    }

    #[test]
    fn summarize_result_preserves_non_text_content_items() {
        let result = json!({
            "content": [{ "type": "image", "url": "https://example.com/img.png" }],
            "isError": false
        });
        let out = summarize_result(result, 50);
        assert_eq!(out["content"][0]["url"], "https://example.com/img.png");
    }

    #[test]
    fn summarize_result_returns_value_when_content_key_missing() {
        let result = json!({ "isError": true, "error": "not found" });
        let out = summarize_result(result, 100);
        assert_eq!(out["isError"], true);
    }

    // -------------------------------------------------------------------------
    // summarize_arbitrary_json
    // -------------------------------------------------------------------------

    #[test]
    fn summarize_arbitrary_json_leaves_short_string_unchanged() {
        let val = json!("hello");
        let out = summarize_arbitrary_json(val, 100, 10);
        assert_eq!(out.as_str().unwrap(), "hello");
    }

    #[test]
    fn summarize_arbitrary_json_truncates_long_string() {
        let val = Value::String("a".repeat(200));
        let out = summarize_arbitrary_json(val, 50, 10);
        let s = out.as_str().unwrap();
        assert!(s.starts_with(&"a".repeat(50)));
        assert!(s.contains("Truncated 150 chars"));
    }

    #[test]
    fn summarize_arbitrary_json_trims_long_array() {
        let arr: Vec<Value> = (0..20).map(|i| json!(i)).collect();
        let val = Value::Array(arr);
        let out = summarize_arbitrary_json(val, 1000, 5);
        let a = out.as_array().unwrap();

        // 5 real items + 1 sentinel "… more items" message string
        assert_eq!(a.len(), 6);
        let last = a.last().unwrap().as_str().unwrap();
        assert!(last.contains("15 more items truncated"), "got: {last}");
    }

    #[test]
    fn summarize_arbitrary_json_recurses_into_object_values() {
        let val = json!({ "key": "x".repeat(200) });
        let out = summarize_arbitrary_json(val, 50, 10);
        let s = out["key"].as_str().unwrap();
        assert!(s.contains("Truncated"));
    }

    #[test]
    fn summarize_arbitrary_json_passes_through_numbers_and_bools() {
        assert_eq!(summarize_arbitrary_json(json!(42), 10, 5), json!(42));
        assert_eq!(summarize_arbitrary_json(json!(true), 10, 5), json!(true));
        assert_eq!(summarize_arbitrary_json(json!(null), 10, 5), json!(null));
    }
}

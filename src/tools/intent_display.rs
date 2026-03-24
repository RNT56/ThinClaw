//! Intent-first tool display.
//!
//! Instead of showing raw tool names and JSON arguments, this module
//! formats tool calls with a human-readable intent summary followed
//! by execution details.
//!
//! Example:
//!   "Searching the web for 'Rust async patterns'"
//!   → tool: web_search { query: "Rust async patterns" }

use serde::{Deserialize, Serialize};

/// Intent display configuration.
#[derive(Debug, Clone)]
pub struct IntentDisplayConfig {
    /// Whether intent-first display is enabled.
    pub enabled: bool,
    /// Whether to show the raw tool call after the intent.
    pub show_details: bool,
    /// Whether to show execution summaries after completion.
    pub show_exec_summary: bool,
}

impl Default for IntentDisplayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_details: false,
            show_exec_summary: true,
        }
    }
}

impl IntentDisplayConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(val) = std::env::var("INTENT_DISPLAY") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }
        if let Ok(val) = std::env::var("INTENT_SHOW_DETAILS") {
            config.show_details = val == "1" || val.eq_ignore_ascii_case("true");
        }
        if let Ok(val) = std::env::var("INTENT_SHOW_EXEC_SUMMARY") {
            config.show_exec_summary = val != "0" && !val.eq_ignore_ascii_case("false");
        }
        config
    }
}

/// Intent description for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    /// Human-readable description of what the tool is doing.
    pub intent: String,
    /// Tool name.
    pub tool_name: String,
    /// Key argument summary (e.g., "query: 'rust async'").
    pub key_args: Option<String>,
}

/// Generate an intent description from a tool call.
pub fn describe_intent(tool_name: &str, args: &serde_json::Value) -> ToolIntent {
    let intent = match tool_name {
        "web_search" | "search" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("...");
            format!("Searching the web for '{}'", query)
        }
        "fetch" | "http_fetch" | "read_url" => {
            let url = args.get("url").and_then(|u| u.as_str()).unwrap_or("...");
            format!("Fetching content from {}", url)
        }
        "shell" | "exec" | "run_command" => {
            let cmd = args
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("...");
            let preview: String = cmd.chars().take(60).collect();
            format!("Running command: {}", preview)
        }
        "read_file" | "file_read" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("...");
            format!("Reading file: {}", path)
        }
        "write_file" | "file_write" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("...");
            format!("Writing to file: {}", path)
        }
        "memory_search" | "search_memory" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("...");
            format!("Searching memory for '{}'", query)
        }
        "memory_store" | "store_memory" => "Storing information in memory".to_string(),
        "calculator" | "calc" => {
            let expr = args
                .get("expression")
                .and_then(|e| e.as_str())
                .unwrap_or("...");
            format!("Calculating: {}", expr)
        }
        "image_generate" | "generate_image" | "imagine" => {
            let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("...");
            let preview: String = prompt.chars().take(50).collect();
            format!("Generating image: {}", preview)
        }
        _ => {
            // Generic fallback: use the tool name as-is
            format!("Using tool: {}", tool_name)
        }
    };

    let key_args = extract_key_args(tool_name, args);

    ToolIntent {
        intent,
        tool_name: tool_name.to_string(),
        key_args,
    }
}

/// Extract the most important argument for display.
fn extract_key_args(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    let key = match tool_name {
        "web_search" | "search" | "memory_search" => "query",
        "fetch" | "http_fetch" => "url",
        "shell" | "exec" => "command",
        "read_file" | "write_file" => "path",
        "calculator" => "expression",
        _ => return None,
    };

    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| format!("{}: '{}'", key, s))
}

/// Format a tool execution summary.
#[derive(Debug, Clone, Serialize)]
pub struct ExecSummary {
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u64,
    pub output_preview: Option<String>,
}

impl ExecSummary {
    /// Format for display.
    pub fn display(&self, config: &IntentDisplayConfig) -> String {
        if !config.show_exec_summary {
            return String::new();
        }

        let status = if self.success { "✓" } else { "✗" };
        let duration = if self.duration_ms < 1000 {
            format!("{}ms", self.duration_ms)
        } else {
            format!("{:.1}s", self.duration_ms as f64 / 1000.0)
        };

        if let Some(ref preview) = self.output_preview {
            format!("{} {} ({}) — {}", status, self.tool_name, duration, preview)
        } else {
            format!("{} {} ({})", status, self.tool_name, duration)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_search_intent() {
        let args = serde_json::json!({"query": "Rust async patterns"});
        let intent = describe_intent("web_search", &args);
        assert!(intent.intent.contains("Searching the web"));
        assert!(intent.intent.contains("Rust async patterns"));
    }

    #[test]
    fn test_shell_intent() {
        let args = serde_json::json!({"command": "cargo build --release"});
        let intent = describe_intent("shell", &args);
        assert!(intent.intent.contains("Running command"));
        assert!(intent.intent.contains("cargo build"));
    }

    #[test]
    fn test_unknown_tool_fallback() {
        let args = serde_json::json!({"foo": "bar"});
        let intent = describe_intent("custom_tool", &args);
        assert!(intent.intent.contains("Using tool: custom_tool"));
    }

    #[test]
    fn test_key_args_extraction() {
        let args = serde_json::json!({"query": "test", "other": "ignored"});
        let key = extract_key_args("web_search", &args);
        assert_eq!(key, Some("query: 'test'".to_string()));
    }

    #[test]
    fn test_exec_summary_success() {
        let config = IntentDisplayConfig::default();
        let summary = ExecSummary {
            tool_name: "shell".to_string(),
            success: true,
            duration_ms: 150,
            output_preview: Some("OK".to_string()),
        };
        let display = summary.display(&config);
        assert!(display.contains("✓"));
        assert!(display.contains("150ms"));
    }

    #[test]
    fn test_exec_summary_long_duration() {
        let config = IntentDisplayConfig::default();
        let summary = ExecSummary {
            tool_name: "web_search".to_string(),
            success: true,
            duration_ms: 3500,
            output_preview: None,
        };
        let display = summary.display(&config);
        assert!(display.contains("3.5s"));
    }

    #[test]
    fn test_exec_summary_disabled() {
        let config = IntentDisplayConfig {
            show_exec_summary: false,
            ..Default::default()
        };
        let summary = ExecSummary {
            tool_name: "shell".to_string(),
            success: true,
            duration_ms: 100,
            output_preview: None,
        };
        assert!(summary.display(&config).is_empty());
    }
}

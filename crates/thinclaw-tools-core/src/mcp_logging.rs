//! Per-server MCP logging preference. Moved from `thinclaw-tools` so light
//! consumers (e.g. `thinclaw-gateway`) can reference it without depending on the
//! heavyweight tool runtime. `thinclaw_tools::mcp::config` re-exports it for path
//! stability.

use serde::{Deserialize, Serialize};

/// Persisted per-server logging preference.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpLoggingLevel {
    Debug,
    Info,
    #[default]
    Warning,
    Error,
}

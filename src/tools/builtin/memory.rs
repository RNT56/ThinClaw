//! Memory tools for persistent workspace memory.
//!
//! These tools allow the agent to:
//! - Search past memories, decisions, and context
//! - Read and write files in the workspace
//!
//! # Usage
//!
//! The agent should use `memory_search` before answering questions about
//! prior work, decisions, dates, people, preferences, or todos.
//!
//! Use `memory_write` to persist important facts that should be remembered
//! across sessions.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, paths};

/// Files the LLM may only APPEND to — never fully overwrite.
///
/// `IDENTITY.md` is the only truly protected file because it records the agent's
/// established name and creature. Nuking it completely would cause the agent to
/// lose its identity. All other personality files (SOUL.md, USER.md, AGENTS.md)
/// are freely rewritable so the agent can restructure and evolve them without
/// accreting stale content.
const APPEND_ONLY_IDENTITY_FILES: &[&str] = &[paths::IDENTITY];

/// Files the agent may FULLY REWRITE (replace entire content, append: false).
///
/// These personality/preference files accumulate stale sections over time if only
/// appended to. After the bootstrap ritual, the agent should use memory_write with
/// append: false to fully restructure them into clean, well-formatted markdown.
const FREELY_REWRITABLE_IDENTITY_FILES: &[&str] = &[paths::SOUL, paths::AGENTS, paths::USER];

/// Tool for searching workspace memory.
///
/// Performs hybrid search (FTS + semantic) across all memory documents.
/// The agent should call this tool before answering questions about
/// prior work, decisions, preferences, or any historical context.
use crate::workspace::SearchConfig;

/// Tool for searching workspace memory.
///
/// Performs hybrid BM25 + vector semantic search over MEMORY.md and daily logs.
/// Applies MMR re-ranking and temporal decay by default for better result quality.
pub struct MemorySearchTool {
    workspace: Arc<Workspace>,
}

impl MemorySearchTool {
    /// Create a new memory search tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search past memories, decisions, and context. MUST be called before answering \
         questions about prior work, decisions, dates, people, preferences, or todos. \
         Returns relevant snippets with relevance scores and source paths. \
         Results are MMR-diversified (no near-duplicate daily notes) and recency-weighted."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use natural language to describe what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 6, max: 20)",
                    "default": 6,
                    "minimum": 1,
                    "maximum": 20
                },
                "mmr": {
                    "type": "boolean",
                    "description": "Enable MMR diversity re-ranking (default: true). Set false only when you want raw ranked results.",
                    "default": true
                },
                "temporal_decay": {
                    "type": "boolean",
                    "description": "Downweight older notes (default: true). Set false to treat all notes equally regardless of age.",
                    "default": true
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = require_str(&params, "query")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(6)
            .min(20) as usize;

        // MMR re-ranking on by default — reduces near-duplicate daily notes.
        // Lambda 0.7 = slight relevance bias (matches openclaw recommendation).
        let use_mmr = params
            .get("mmr")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Temporal decay on by default — 30-day half-life so older notes don't
        // crowd out recent ones on equal semantic similarity.
        let use_decay = params
            .get("temporal_decay")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut config = SearchConfig::default().with_limit(limit);
        if use_mmr {
            config = config.with_mmr(0.7);
        }
        if use_decay {
            // 30-day half-life: today = 1.0×, 1 month ago = 0.5×, 3 months ago = 0.125×
            config = config.with_temporal_decay(30.0);
        }

        let results = self
            .workspace
            .search_with_config(query, config)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Search failed: {}", e)))?;

        let output = serde_json::json!({
            "query": query,
            "results": results.iter().map(|r| {
                let path = r.citation
                    .as_ref()
                    .and_then(|c| c.path.as_deref())
                    .unwrap_or("");
                serde_json::json!({
                    "content": r.content,
                    "score": r.score,
                    "path": path,
                    "document_id": r.document_id.to_string(),
                    "is_hybrid_match": r.is_hybrid(),
                })
            }).collect::<Vec<_>>(),
            "result_count": results.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory, trusted content
    }
}

/// Tool for writing to workspace memory.
///
/// Use this to persist important information that should be remembered
/// across sessions: decisions, preferences, facts, lessons learned.
pub struct MemoryWriteTool {
    workspace: Arc<Workspace>,
}

impl MemoryWriteTool {
    /// Create a new memory write tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Write to persistent memory (database-backed, NOT the local filesystem). \
         Use for facts, decisions, preferences, or lessons to remember across sessions. \
         Targets: 'memory' (MEMORY.md, long-term facts), 'daily_log' (timestamped notes), \
         'heartbeat' (HEARTBEAT.md checklist), 'SOUL.md' / 'USER.md' / 'AGENTS.md' \
         (freely rewritable — use append: false to fully restructure after bootstrap), \
         'IDENTITY.md' (append-only — preserves established name/creature), or a custom path. \
         ALWAYS write well-structured markdown: use ## headers for sections, bullet points, \
         and clear prose. Never dump raw unformatted text into identity files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to write to memory. Be concise but include relevant context."
                },
                "target": {
                    "type": "string",
                    "description": "Where to write: 'memory' for MEMORY.md, 'daily_log' for today's log, 'heartbeat' for HEARTBEAT.md checklist, or a path like 'projects/alpha/notes.md'",
                    "default": "daily_log"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content cannot be empty".to_string(),
            ));
        }

        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("daily_log");

        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // IDENTITY.md is append-only to protect the agent's established name/creature.
        if APPEND_ONLY_IDENTITY_FILES.contains(&target) {
            if !append {
                return Err(ToolError::NotAuthorized(format!(
                    "'{}' is append-only. Add an '## Update' section with your changes \
                     instead of overwriting. To fully restructure SOUL.md / AGENTS.md / \
                     USER.md, use those targets with append: false.",
                    target,
                )));
            }
            self.workspace
                .append(target, content)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            let output = serde_json::json!({
                "status": "appended",
                "path": target,
                "append": true,
                "content_length": content.len(),
                "note": "Identity file updated (append-only)",
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // SOUL.md / AGENTS.md / USER.md — freely rewritable.
        // With append: false the agent can fully restructure the file.
        if FREELY_REWRITABLE_IDENTITY_FILES.contains(&target) {
            if append {
                self.workspace
                    .append(target, content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            } else {
                self.workspace
                    .write(target, content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            }
            let output = serde_json::json!({
                "status": if append { "appended" } else { "rewritten" },
                "path": target,
                "append": append,
                "content_length": content.len(),
                "note": if append {
                    "Personality file updated (new section appended)"
                } else {
                    "Personality file fully restructured — well-formed markdown expected"
                },
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        let path = match target {
            "memory" => {
                if append {
                    self.workspace
                        .append_memory(content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(paths::MEMORY, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                paths::MEMORY.to_string()
            }
            "daily_log" => {
                self.workspace
                    .append_daily_log(content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                format!("daily/{}.md", chrono::Utc::now().format("%Y-%m-%d"))
            }
            "heartbeat" => {
                if append {
                    self.workspace
                        .append(paths::HEARTBEAT, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(paths::HEARTBEAT, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                paths::HEARTBEAT.to_string()
            }
            path => {
                // Path-form check for append-only files (IDENTITY.md).
                let normalized = path.trim_start_matches('/');
                if APPEND_ONLY_IDENTITY_FILES
                    .iter()
                    .any(|p| normalized.eq_ignore_ascii_case(p))
                {
                    if !append {
                        return Err(ToolError::NotAuthorized(format!(
                            "'{}' is append-only. Use append: true to add sections.",
                            path
                        )));
                    }
                    self.workspace
                        .append(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                    let output = serde_json::json!({
                        "status": "appended",
                        "path": path,
                        "append": true,
                        "content_length": content.len(),
                        "note": "Identity file updated (append-only)",
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }

                // Path-form check for freely rewritable personality files.
                if FREELY_REWRITABLE_IDENTITY_FILES
                    .iter()
                    .any(|p| normalized.eq_ignore_ascii_case(p))
                {
                    if append {
                        self.workspace
                            .append(path, content)
                            .await
                            .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                    } else {
                        self.workspace
                            .write(path, content)
                            .await
                            .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                    }
                    let output = serde_json::json!({
                        "status": if append { "appended" } else { "rewritten" },
                        "path": path,
                        "append": append,
                        "content_length": content.len(),
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }

                if append {
                    self.workspace
                        .append(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                path.to_string()
            }
        };

        let output = serde_json::json!({
            "status": "written",
            "path": path,
            "append": append,
            "content_length": content.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for reading workspace files, with optional line-range slicing.
///
/// Degrades gracefully when the target file doesn't exist — returns empty
/// content with `"exists": false` instead of an error. This matches openclaw's
/// `memory_get` behaviour so agents can safely probe today's daily log before
/// the first write without wrapping the call in try/catch logic.
pub struct MemoryReadTool {
    workspace: Arc<Workspace>,
}

impl MemoryReadTool {
    /// Create a new memory read tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read a file from the workspace memory (database-backed storage). \
         Use this to read files shown by memory_tree. NOT for local filesystem files \
         (use read_file for those). Works with identity files, heartbeat checklist, \
         memory, daily logs, or any custom workspace path. \
         Returns empty content (not an error) if the file does not exist yet."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (e.g., 'MEMORY.md', 'daily/2024-01-15.md', 'TOOLS.md')"
                },
                "start_line": {
                    "type": "integer",
                    "description": "1-indexed line to start reading from (optional). Useful for large files like MEMORY.md.",
                    "minimum": 1
                },
                "num_lines": {
                    "type": "integer",
                    "description": "Maximum number of lines to return (optional). Use with start_line for targeted reads.",
                    "minimum": 1
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = require_str(&params, "path")?;

        // Graceful degradation: missing file → empty content, not an error.
        // Matches openclaw memory_get: { text: "", path } on ENOENT.
        let doc = match self.workspace.read(path).await {
            Ok(doc) => doc,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
                let output = serde_json::json!({
                    "path": path,
                    "content": "",
                    "word_count": 0,
                    "exists": false,
                });
                return Ok(ToolOutput::success(output, start.elapsed()));
            }
            Err(e) => return Err(ToolError::ExecutionFailed(format!("Read failed: {}", e))),
        };

        // Optional line-range slicing.
        let content = if params.get("start_line").is_some() || params.get("num_lines").is_some() {
            let start_line = params
                .get("start_line")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as usize;
            let num_lines = params
                .get("num_lines")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            let lines: Vec<&str> = doc.content.lines().collect();
            let total_lines = lines.len();

            // Convert 1-indexed start_line to 0-indexed.
            let from = (start_line - 1).min(total_lines);
            let to = match num_lines {
                Some(n) => (from + n).min(total_lines),
                None => total_lines,
            };

            lines[from..to].join("\n")
        } else {
            doc.content.clone()
        };

        let total_lines = doc.content.lines().count();
        let output = serde_json::json!({
            "path": doc.path,
            "content": content,
            "word_count": content.split_whitespace().count(),
            "total_lines": total_lines,
            "updated_at": doc.updated_at.to_rfc3339(),
            "exists": true,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory
    }
}


/// Tool for viewing workspace structure as a tree.
///
/// Returns a hierarchical view of files and directories with configurable depth.
pub struct MemoryTreeTool {
    workspace: Arc<Workspace>,
}

impl MemoryTreeTool {
    /// Create a new memory tree tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }

    /// Recursively build tree structure.
    ///
    /// Returns a compact format where directories end with `/` and may have children.
    async fn build_tree(
        &self,
        path: &str,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        if current_depth > max_depth {
            return Ok(Vec::new());
        }

        let entries = self
            .workspace
            .list(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Tree failed: {}", e)))?;

        let mut result = Vec::new();
        for entry in entries {
            // Directories end with `/`, files don't
            let display_path = if entry.is_directory {
                format!("{}/", entry.name())
            } else {
                entry.name().to_string()
            };

            if entry.is_directory && current_depth < max_depth {
                let children =
                    Box::pin(self.build_tree(&entry.path, current_depth + 1, max_depth)).await?;
                if children.is_empty() {
                    result.push(serde_json::Value::String(display_path));
                } else {
                    result.push(serde_json::json!({ display_path: children }));
                }
            } else {
                result.push(serde_json::Value::String(display_path));
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl Tool for MemoryTreeTool {
    fn name(&self) -> &str {
        "memory_tree"
    }

    fn description(&self) -> &str {
        "View the workspace memory structure as a tree (database-backed storage). \
         Use memory_read to read files shown here, NOT read_file. \
         The workspace is separate from the local filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root path to start from (empty string for workspace root)",
                    "default": ""
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth to traverse (1 = immediate children only)",
                    "default": 1,
                    "minimum": 1,
                    "maximum": 10
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let depth = params
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 10) as usize;

        let tree = self.build_tree(path, 1, depth).await?;

        // Compact output: just the tree array
        Ok(ToolOutput::success(
            serde_json::Value::Array(tree),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

/// Tool for deleting a file from workspace memory.
///
/// Use this to clean up temporary files like BOOTSTRAP.md after setup,
/// or remove outdated notes. Identity files are protected.
pub struct MemoryDeleteTool {
    workspace: Arc<Workspace>,
    /// Optional SSE sender for broadcasting lifecycle events (e.g. BootstrapCompleted).
    sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
}

impl MemoryDeleteTool {
    /// Create a new memory delete tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace, sse_sender: None }
    }

    /// Attach an SSE sender to enable lifecycle event emission.
    pub fn with_sse_sender(
        mut self,
        sender: tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>,
    ) -> Self {
        self.sse_sender = Some(sender);
        self
    }
}

#[async_trait]
impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete a file from workspace memory (database-backed storage). \
         Cannot delete IDENTITY.md (append to it instead). \
         SOUL.md / AGENTS.md / USER.md can be fully rewritten with memory_write(append: false) \
         rather than deleted. \
         Primary use-case: memory_delete('BOOTSTRAP.md') after the identity ritual completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to delete (e.g. 'BOOTSTRAP.md', 'daily/2024-01-15.md')"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = require_str(&params, "path")?;

        // Only IDENTITY.md is delete-protected.
        // SOUL/AGENTS/USER should be restructured with memory_write(append: false) instead.
        let normalized = path.trim_start_matches('/');
        if APPEND_ONLY_IDENTITY_FILES
            .iter()
            .any(|p| normalized.eq_ignore_ascii_case(p))
        {
            return Err(ToolError::NotAuthorized(format!(
                "'{}' cannot be deleted. Use memory_write(append: true) to add sections. \
                 To restructure SOUL.md / AGENTS.md / USER.md entirely, use \
                 memory_write with append: false instead of deleting.",
                path
            )));
        }

        self.workspace
            .delete(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {}", e)))?;

        // If BOOTSTRAP.md was deleted, notify the bridge to update frontend state.
        let is_bootstrap = normalized.eq_ignore_ascii_case(crate::workspace::paths::BOOTSTRAP);
        if is_bootstrap {
            if let Some(ref tx) = self.sse_sender {
                let _ = tx.send(crate::channels::web::types::SseEvent::BootstrapCompleted);
                tracing::info!("[memory_delete] Emitted BootstrapCompleted SSE event");
            }
        }

        let output = serde_json::json!({
            "status": "deleted",
            "path": path,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    fn make_test_workspace() -> Arc<Workspace> {
        Arc::new(Workspace::new(
            "test_user",
            deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
                tokio_postgres::Config::new(),
                tokio_postgres::NoTls,
            ))
            .build()
            .unwrap(),
        ))
    }

    #[test]
    fn test_memory_search_schema() {
        let workspace = make_test_workspace();
        let tool = MemorySearchTool::new(workspace);

        assert_eq!(tool.name(), "memory_search");
        assert!(!tool.requires_sanitization());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"query".into())
        );
    }

    #[test]
    fn test_memory_write_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryWriteTool::new(workspace);

        assert_eq!(tool.name(), "memory_write");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["content"].is_object());
        assert!(schema["properties"]["target"].is_object());
        assert!(schema["properties"]["append"].is_object());
    }

    #[test]
    fn test_memory_read_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryReadTool::new(workspace);

        assert_eq!(tool.name(), "memory_read");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"path".into())
        );
    }

    #[test]
    fn test_memory_tree_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryTreeTool::new(workspace);

        assert_eq!(tool.name(), "memory_tree");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["depth"].is_object());
        assert_eq!(schema["properties"]["depth"]["default"], 1);
    }
}

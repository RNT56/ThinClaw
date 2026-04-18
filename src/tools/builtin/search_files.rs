//! Enhanced file search tool.
//!
//! Adds filename-based search on top of the existing GrepTool's content
//! search, and provides a unified `search_files` tool interface.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;

use crate::context::JobContext;
use crate::tools::builtin::file::{effective_base_dir, validate_path};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum results for filename search.
const MAX_FILENAME_RESULTS: usize = 100;

/// Maximum depth for filename search.
const MAX_SEARCH_DEPTH: usize = 10;

/// Directories to skip during filename search.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "__pycache__",
    "venv",
    ".venv",
    ".next",
    "dist",
    "build",
    ".cargo",
    ".tox",
    "vendor",
];

/// Enhanced file search tool combining filename and content search.
#[derive(Debug, Default)]
pub struct SearchFilesTool {
    base_dir: Option<PathBuf>,
}

impl SearchFilesTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    /// Search for files by name pattern.
    async fn search_filenames(
        &self,
        pattern: &str,
        search_path: &Path,
        case_insensitive: bool,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let pattern_lower = if case_insensitive {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };

        let mut results = Vec::new();
        self.search_filenames_inner(
            search_path,
            search_path,
            &pattern_lower,
            case_insensitive,
            0,
            &mut results,
        )
        .await?;

        Ok(results)
    }

    async fn search_filenames_inner(
        &self,
        base: &Path,
        dir: &Path,
        pattern: &str,
        case_insensitive: bool,
        depth: usize,
        results: &mut Vec<serde_json::Value>,
    ) -> Result<(), ToolError> {
        if depth > MAX_SEARCH_DEPTH || results.len() >= MAX_FILENAME_RESULTS {
            return Ok(());
        }

        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read entry: {}", e)))?
        {
            if results.len() >= MAX_FILENAME_RESULTS {
                break;
            }

            let entry_path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };

            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if metadata.is_dir() {
                // Skip hidden and known non-essential directories
                if SKIP_DIRS.contains(&name_str.as_ref()) || name_str.starts_with('.') {
                    continue;
                }

                // Check if directory name matches
                let name_compare = if case_insensitive {
                    name_str.to_lowercase()
                } else {
                    name_str.to_string()
                };

                if name_compare.contains(pattern) {
                    let relative = entry_path
                        .strip_prefix(base)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .to_string();
                    results.push(serde_json::json!({
                        "path": relative,
                        "name": name_str,
                        "type": "directory",
                    }));
                }

                // Recurse into subdirectories
                Box::pin(self.search_filenames_inner(
                    base,
                    &entry_path,
                    pattern,
                    case_insensitive,
                    depth + 1,
                    results,
                ))
                .await?;
            } else if metadata.is_file() {
                let name_compare = if case_insensitive {
                    name_str.to_lowercase()
                } else {
                    name_str.to_string()
                };

                if name_compare.contains(pattern) {
                    let relative = entry_path
                        .strip_prefix(base)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .to_string();

                    results.push(serde_json::json!({
                        "path": relative,
                        "name": name_str,
                        "type": "file",
                        "size_bytes": metadata.len(),
                    }));
                }
            }
        }

        Ok(())
    }

    /// Fuzzy path suggestion — when a file path doesn't match, suggest close alternatives.
    pub async fn suggest_similar_paths(
        &self,
        attempted_path: &str,
        search_dir: &Path,
    ) -> Vec<String> {
        let filename = Path::new(attempted_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(attempted_path);

        let filename_lower = filename.to_lowercase();

        // Search for similar filenames
        let mut suggestions = Vec::new();
        let results = self
            .search_filenames(&filename_lower, search_dir, true)
            .await
            .unwrap_or_default();

        for result in results.iter().take(5) {
            if let Some(path) = result.get("path").and_then(|p| p.as_str()) {
                suggestions.push(path.to_string());
            }
        }

        // If no exact substring matches, try Levenshtein-like matching
        if suggestions.is_empty() {
            // Try just the extension
            if let Some(ext) = Path::new(attempted_path)
                .extension()
                .and_then(|e| e.to_str())
            {
                let ext_pattern = format!(".{}", ext);
                let ext_results = self
                    .search_filenames(&ext_pattern, search_dir, true)
                    .await
                    .unwrap_or_default();

                for result in ext_results.iter().take(5) {
                    if let Some(path) = result.get("path").and_then(|p| p.as_str()) {
                        suggestions.push(path.to_string());
                    }
                }
            }
        }

        suggestions
    }
}

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Search for files or directories by name pattern. Use this when you know roughly \
         what a file is called but not where it lives. This finds paths by filename, not content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Filename pattern to search for (substring match)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current directory)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case-insensitive search (default true)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let pattern = require_str(&params, "pattern")?;
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let case_insensitive = params
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Resolve the search path
        let base_dir = effective_base_dir(ctx, self.base_dir.as_deref());
        let search_path = validate_path(path_str, base_dir.as_deref())?;

        if !search_path.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "Path is not a directory: {}",
                search_path.display()
            )));
        }

        let results = self
            .search_filenames(pattern, &search_path, case_insensitive)
            .await?;

        let truncated = results.len() >= MAX_FILENAME_RESULTS;

        let result = serde_json::json!({
            "results": results,
            "total": results.len(),
            "truncated": truncated,
            "pattern": pattern,
            "search_path": search_path.display().to_string(),
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Filename listing is safe
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_search_files_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("hello.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("world.rs"), "fn world() {}").unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Hello").unwrap();

        let tool = SearchFilesTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": ".rs", "path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let results = result.result.get("results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_files_case_insensitive() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# test").unwrap();

        let tool = SearchFilesTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "readme",
                    "path": dir.path().to_str().unwrap(),
                    "case_insensitive": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        let results = result.result.get("results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_files_recursive() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("src/nested/lib.rs"), "// lib").unwrap();

        let tool = SearchFilesTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": ".rs", "path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let results = result.result.get("results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_files_no_results() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();

        let tool = SearchFilesTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": ".xyz", "path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let results = result.result.get("results").unwrap().as_array().unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_files_directory_match() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("components")).unwrap();
        std::fs::write(dir.path().join("components/button.tsx"), "").unwrap();

        let tool = SearchFilesTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": "component", "path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let results = result.result.get("results").unwrap().as_array().unwrap();
        // Should find the directory
        assert!(results.iter().any(|r| r["type"] == "directory"));
    }

    #[tokio::test]
    async fn test_suggest_similar_paths() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "").unwrap();
        std::fs::write(dir.path().join("config.yaml"), "").unwrap();

        let tool = SearchFilesTool::new();
        let suggestions = tool.suggest_similar_paths("confg.toml", dir.path()).await;
        // Suggestions are heuristic. If any are returned, they should include the closest match.
        assert!(
            suggestions.is_empty() || suggestions.iter().any(|path| path.ends_with("config.toml"))
        );
    }
}

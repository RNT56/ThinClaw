//! Memory/workspace API — framework-agnostic file operations.
//!
//! Thin wrappers around `Workspace` methods. Extracted from
//! `channels/web/handlers/memory.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::workspace::Workspace;
use crate::workspace::paths;

use super::error::{ApiError, ApiResult};

fn root_list_with_home_soul(mut entries: Vec<ListEntry>) -> Vec<ListEntry> {
    let has_soul = entries.iter().any(|entry| entry.path == paths::SOUL);
    if !has_soul && crate::identity::soul_store::read_home_soul().is_ok() {
        entries.insert(
            0,
            ListEntry {
                name: paths::SOUL.to_string(),
                path: paths::SOUL.to_string(),
                is_dir: false,
                updated_at: None,
            },
        );
    }
    entries
}

/// Read a file from the workspace.
pub async fn get_file(workspace: &Arc<Workspace>, path: &str) -> ApiResult<MemoryReadResponse> {
    if path == paths::SOUL
        && let Ok(content) = crate::identity::soul_store::read_home_soul()
    {
        return Ok(MemoryReadResponse {
            path: path.to_string(),
            content,
            updated_at: None,
        });
    }

    let doc = workspace
        .read(path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MemoryReadResponse {
        path: path.to_string(),
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    })
}

/// Write content to a file in the workspace (creates or overwrites).
pub async fn write_file(workspace: &Arc<Workspace>, path: &str, content: &str) -> ApiResult<()> {
    if path == paths::SOUL {
        crate::identity::soul_store::write_home_soul(content)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(());
    }

    workspace
        .write(path, content)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// Delete a file from the workspace.
pub async fn delete_file(workspace: &Arc<Workspace>, path: &str) -> ApiResult<()> {
    workspace
        .delete(path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// List files in a workspace directory.
pub async fn list_files(
    workspace: &Arc<Workspace>,
    path: Option<&str>,
) -> ApiResult<MemoryListResponse> {
    let dir = path.unwrap_or("");
    let entries = workspace
        .list(dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    let list_entries = if dir.is_empty() {
        root_list_with_home_soul(list_entries)
    } else {
        list_entries
    };

    Ok(MemoryListResponse {
        path: dir.to_string(),
        entries: list_entries,
    })
}

/// Build a tree view of all workspace files.
pub async fn file_tree(workspace: &Arc<Workspace>) -> ApiResult<MemoryTreeResponse> {
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    if !all_paths.iter().any(|path| path == paths::SOUL)
        && crate::identity::soul_store::read_home_soul().is_ok()
    {
        entries.push(TreeEntry {
            path: paths::SOUL.to_string(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(MemoryTreeResponse { entries })
}

/// Search workspace memory (vector search).
pub async fn search(
    workspace: &Arc<Workspace>,
    query: &str,
    limit: Option<usize>,
) -> ApiResult<MemorySearchResponse> {
    let max = limit.unwrap_or(10);
    let results = workspace
        .search(query, max)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.path.clone(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(MemorySearchResponse { results: hits })
}

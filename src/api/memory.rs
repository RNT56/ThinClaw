//! Memory/workspace API — framework-agnostic file operations.
//!
//! Thin wrappers around `Workspace` methods. Extracted from
//! `channels/web/handlers/memory.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::workspace::Workspace;
use crate::workspace::paths;
use thinclaw_gateway::web::memory::{
    GatewayMemoryListSourceEntry, GatewayMemorySearchHit, list_entries_from_source,
    memory_list_response, memory_read_response, memory_search_response_from_hits,
    memory_tree_response, root_list_with_virtual_home_soul, tree_entries_from_paths,
    tree_with_virtual_home_soul,
};

use super::error::{ApiError, ApiResult};

/// Read a file from the workspace.
pub async fn get_file(workspace: &Arc<Workspace>, path: &str) -> ApiResult<MemoryReadResponse> {
    if path == paths::SOUL
        && let Ok(content) = crate::identity::soul_store::read_home_soul()
    {
        return Ok(memory_read_response(path, content, None));
    }

    let doc = workspace
        .read(path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(memory_read_response(
        path,
        doc.content,
        Some(doc.updated_at.to_rfc3339()),
    ))
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

    let list_entries: Vec<ListEntry> =
        list_entries_from_source(entries.iter().map(|entry| GatewayMemoryListSourceEntry {
            path: entry.path.clone(),
            is_directory: entry.is_directory,
            updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
        }));

    let list_entries = if dir.is_empty() {
        root_list_with_virtual_home_soul(
            list_entries,
            paths::SOUL,
            crate::identity::soul_store::read_home_soul().is_ok(),
        )
    } else {
        list_entries
    };

    Ok(memory_list_response(dir, list_entries))
}

/// Build a tree view of all workspace files.
pub async fn file_tree(workspace: &Arc<Workspace>) -> ApiResult<MemoryTreeResponse> {
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let entries = tree_with_virtual_home_soul(
        tree_entries_from_paths(&all_paths),
        paths::SOUL,
        all_paths.iter().any(|path| path == paths::SOUL),
        crate::identity::soul_store::read_home_soul().is_ok(),
    );
    Ok(memory_tree_response(entries))
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

    let hits = results
        .iter()
        .map(|r| GatewayMemorySearchHit {
            path: r.path.clone(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect::<Vec<_>>();

    Ok(memory_search_response_from_hits(hits))
}

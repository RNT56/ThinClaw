use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::workspace::paths;

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

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    #[allow(dead_code)]
    depth: Option<usize>,
}

pub(crate) async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

    Ok(Json(MemoryTreeResponse { entries }))
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    path: Option<String>,
}

pub(crate) async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    let list_entries = if path.is_empty() {
        root_list_with_home_soul(list_entries)
    } else {
        list_entries
    };

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

#[derive(Deserialize)]
pub(crate) struct ReadQuery {
    path: String,
}

pub(crate) async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    if query.path == paths::SOUL
        && let Ok(content) = crate::identity::soul_store::read_home_soul()
    {
        return Ok(Json(MemoryReadResponse {
            path: query.path,
            content,
            updated_at: None,
        }));
    }

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

pub(crate) async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    if req.path == paths::SOUL {
        crate::identity::soul_store::write_home_soul(&req.content)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(MemoryWriteResponse {
            path: req.path,
            status: "written",
        }));
    }

    workspace
        .write(&req.path, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MemoryWriteResponse {
        path: req.path,
        status: "written",
    }))
}

pub(crate) async fn memory_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryDeleteRequest>,
) -> Result<Json<MemoryDeleteResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    crate::api::memory::delete_file(workspace, &req.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MemoryDeleteResponse {
        path: req.path,
        status: "deleted",
    }))
}

pub(crate) async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.path.clone(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(Json(MemorySearchResponse { results: hits }))
}

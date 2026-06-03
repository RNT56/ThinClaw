use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::workspace::paths;
use thinclaw_gateway::web::memory::{
    GatewayMemoryListSourceEntry, GatewayMemorySearchHit, list_entries_from_source,
    memory_delete_response, memory_list_response, memory_read_response,
    memory_search_response_from_hits, memory_tree_response, memory_workspace_unavailable_error,
    memory_write_response, root_list_with_virtual_home_soul, tree_entries_from_paths,
    tree_with_virtual_home_soul,
};

pub(crate) async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let entries = tree_entries_from_paths(&all_paths);
    let entries = tree_with_virtual_home_soul(
        entries,
        paths::SOUL,
        all_paths.iter().any(|path| path == paths::SOUL),
        crate::identity::soul_store::read_home_soul().is_ok(),
    );

    Ok(Json(memory_tree_response(entries)))
}

pub(crate) async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries =
        list_entries_from_source(
            entries
                .into_iter()
                .map(|entry| GatewayMemoryListSourceEntry {
                    path: entry.path,
                    is_directory: entry.is_directory,
                    updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
                }),
        );

    let list_entries = if path.is_empty() {
        root_list_with_virtual_home_soul(
            list_entries,
            paths::SOUL,
            crate::identity::soul_store::read_home_soul().is_ok(),
        )
    } else {
        list_entries
    };

    Ok(Json(memory_list_response(path, list_entries)))
}

pub(crate) async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    if query.path == paths::SOUL
        && let Ok(content) = crate::identity::soul_store::read_home_soul()
    {
        return Ok(Json(memory_read_response(query.path, content, None)));
    }

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(memory_read_response(
        query.path,
        doc.content,
        Some(doc.updated_at.to_rfc3339()),
    )))
}

pub(crate) async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    if req.path == paths::SOUL {
        crate::identity::soul_store::write_home_soul(&req.content)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(memory_write_response(req.path)));
    }

    workspace
        .write(&req.path, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(memory_write_response(req.path)))
}

pub(crate) async fn memory_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryDeleteRequest>,
) -> Result<Json<MemoryDeleteResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    crate::api::memory::delete_file(workspace, &req.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(memory_delete_response(req.path)))
}

pub(crate) async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(memory_search_response_from_hits(
        results.into_iter().map(|result| GatewayMemorySearchHit {
            path: result.path,
            content: result.content,
            score: result.score as f64,
        }),
    )))
}

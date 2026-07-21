use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::workspace::{AuthorizedWorkspace, SearchConfig, Workspace, WorkspaceAccessRole, paths};
use thinclaw_gateway::web::identity::GatewayRequestIdentity;
use thinclaw_gateway::web::memory::{
    GatewayMemoryListSourceEntry, GatewayMemorySearchHit, list_entries_from_source,
    memory_delete_response, memory_list_response, memory_read_response,
    memory_search_response_from_hits, memory_tree_response, memory_workspace_unavailable_error,
    memory_write_response, root_list_with_virtual_home_soul, tree_entries_from_paths,
    tree_with_virtual_home_soul,
};

fn authorized_workspace(
    base: &Workspace,
    request_identity: &GatewayRequestIdentity,
    requested_scope: Option<MemoryAccessScope>,
) -> Result<AuthorizedWorkspace, (StatusCode, String)> {
    let identity = request_identity.resolved_identity(None);
    match requested_scope {
        Some(MemoryAccessScope::Conversation) => Ok(AuthorizedWorkspace::conversation(
            base, &identity, "gateway",
        )),
        Some(MemoryAccessScope::PrincipalAdmin) => {
            if request_identity.role != thinclaw_gateway::web::rbac::GatewayRole::Admin {
                return Err((
                    StatusCode::FORBIDDEN,
                    "principal_admin memory scope requires the Admin role".to_string(),
                ));
            }
            Ok(AuthorizedWorkspace::principal_admin(
                base, &identity, "gateway",
            ))
        }
        // Privilege must be explicit. An Admin token is often also used for
        // ordinary chat clients; silently switching an omitted scope to the
        // principal root makes identical memory calls behave differently by
        // credential and revives legacy-root/sibling exposure.
        None => Ok(AuthorizedWorkspace::conversation(
            base, &identity, "gateway",
        )),
    }
}

fn display_path(workspace: &AuthorizedWorkspace, canonical: &str) -> String {
    workspace
        .access()
        .display_path(canonical)
        .unwrap_or_else(|| canonical.to_string())
}

fn workspace_error(error: crate::error::WorkspaceError) -> (StatusCode, String) {
    match error {
        crate::error::WorkspaceError::AccessDenied { .. } => {
            (StatusCode::FORBIDDEN, error.to_string())
        }
        crate::error::WorkspaceError::DocumentNotFound { .. } => {
            (StatusCode::NOT_FOUND, error.to_string())
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

pub(crate) async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, query.scope)?;

    let all_paths = workspace
        .list_all()
        .await
        .map_err(workspace_error)?
        .into_iter()
        .filter_map(|path| workspace.access().display_path(&path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();

    let entries = tree_entries_from_paths(&all_paths);
    let entries = if workspace.access().role() == WorkspaceAccessRole::PrincipalAdmin {
        tree_with_virtual_home_soul(
            entries,
            paths::SOUL,
            all_paths.iter().any(|path| path == paths::SOUL),
            crate::identity::soul_store::read_home_soul().is_ok(),
        )
    } else {
        entries
    };

    Ok(Json(memory_tree_response(entries)))
}

pub(crate) async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, query.scope)?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace.list(path).await.map_err(workspace_error)?;

    let list_entries =
        list_entries_from_source(
            entries
                .into_iter()
                .map(|entry| GatewayMemoryListSourceEntry {
                    path: display_path(&workspace, &entry.path),
                    is_directory: entry.is_directory,
                    updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
                }),
        );
    let list_entries =
        if path.is_empty() && workspace.access().role() == WorkspaceAccessRole::PrincipalAdmin {
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
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, query.scope)?;

    if workspace.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && query.path.trim().trim_matches('/') == paths::SOUL
    {
        let content = crate::identity::soul_store::read_home_soul().map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read canonical home soul: {error}"),
            )
        })?;
        return Ok(Json(memory_read_response(paths::SOUL, content, None)));
    }

    let doc = workspace.read(&query.path).await.map_err(workspace_error)?;

    Ok(Json(memory_read_response(
        display_path(&workspace, &doc.path),
        doc.content,
        Some(doc.updated_at.to_rfc3339()),
    )))
}

pub(crate) async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, req.scope)?;
    let identity = request_identity.resolved_identity(None);
    let timezone_update = if workspace.access().role() == WorkspaceAccessRole::Conversation {
        crate::timezone::actor_timezone_update_for_document(&identity, &req.path, &req.content)
            .map_err(|error| (StatusCode::BAD_REQUEST, error))?
    } else {
        None
    };
    let timezone_store = match timezone_update.as_ref() {
        Some(_) => Some(state.store.as_ref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "database is required to keep USER.md timezone and routines consistent".to_string(),
            )
        })?),
        None => None,
    };

    if workspace.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && req.path.trim().trim_matches('/') == paths::SOUL
    {
        crate::identity::soul_store::write_home_soul(&req.content).map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write canonical home soul: {error}"),
            )
        })?;
        return Ok(Json(memory_write_response(paths::SOUL)));
    }

    let document = workspace
        .write(&req.path, &req.content)
        .await
        .map_err(workspace_error)?;

    if let (Some(update), Some(store)) = (timezone_update, timezone_store) {
        crate::timezone::apply_actor_timezone_change(
            store,
            &update.principal_id,
            &update.actor_id,
            update.timezone.as_deref(),
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }

    Ok(Json(memory_write_response(display_path(
        &workspace,
        &document.path,
    ))))
}

pub(crate) async fn memory_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<MemoryDeleteRequest>,
) -> Result<Json<MemoryDeleteResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, req.scope)?;

    if workspace.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && req.path.trim().trim_matches('/') == paths::SOUL
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "The canonical home SOUL.md cannot be deleted; clear or replace its content instead"
                .to_string(),
        ));
    }
    let canonical_path = workspace
        .access()
        .resolve_path(&req.path, crate::workspace::WorkspaceOperation::Delete)
        .map_err(workspace_error)?;

    workspace
        .delete(&canonical_path)
        .await
        .map_err(workspace_error)?;

    Ok(Json(memory_delete_response(display_path(
        &workspace,
        &canonical_path,
    ))))
}

pub(crate) async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let base = state
        .workspace
        .as_ref()
        .ok_or_else(memory_workspace_unavailable_error)?;
    let workspace = authorized_workspace(base, &request_identity, req.scope)?;

    if req.query.trim().is_empty() || req.query.len() > 32 * 1024 || req.query.contains('\0') {
        return Err((
            StatusCode::BAD_REQUEST,
            "memory search query is empty, malformed, or oversized".to_string(),
        ));
    }
    let limit = req.limit.unwrap_or(10).clamp(1, 100);
    let results = workspace
        .search(&req.query, SearchConfig::default().with_limit(limit))
        .await
        .map_err(workspace_error)?;

    Ok(Json(memory_search_response_from_hits(
        results.into_iter().map(|result| GatewayMemorySearchHit {
            path: display_path(&workspace, &result.path),
            content: result.content,
            score: result.score as f64,
        }),
    )))
}

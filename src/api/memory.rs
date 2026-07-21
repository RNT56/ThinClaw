//! Memory/workspace API — framework-agnostic file operations.
//!
//! Thin wrappers around `Workspace` methods. Extracted from
//! `channels/web/handlers/memory.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::identity::{ConversationKind, ResolvedIdentity};
use crate::workspace::paths;
use crate::workspace::{
    AuthorizedWorkspace, SearchConfig, Workspace, WorkspaceAccessRole, is_control_plane_path,
};
use thinclaw_gateway::web::memory::{
    GatewayMemoryListSourceEntry, GatewayMemorySearchHit, list_entries_from_source,
    memory_list_response, memory_read_response, memory_search_response_from_hits,
    memory_tree_response, root_list_with_virtual_home_soul, tree_entries_from_paths,
    tree_with_virtual_home_soul,
};

use super::error::{ApiError, ApiResult};

const MAX_MEMORY_SEARCH_QUERY_BYTES: usize = 32 * 1024;
const MAX_MEMORY_SEARCH_RESULTS: usize = 100;

fn validated_memory_search_limit(query: &str, limit: Option<usize>) -> ApiResult<usize> {
    if query.trim().is_empty()
        || query.len() > MAX_MEMORY_SEARCH_QUERY_BYTES
        || query.contains('\0')
    {
        return Err(ApiError::InvalidInput(format!(
            "memory search query must be non-empty, contain no NUL, and be at most {MAX_MEMORY_SEARCH_QUERY_BYTES} bytes"
        )));
    }
    Ok(limit.unwrap_or(10).clamp(1, MAX_MEMORY_SEARCH_RESULTS))
}

/// Caller-visible alias for the actor-private identity overlay. Root
/// `IDENTITY.md` is trusted principal control material, so projecting the
/// actor document under the same name would silently shadow one of them in
/// the desktop's composite view.
const ACTOR_IDENTITY_ALIAS: &str = "actor/IDENTITY.md";

fn identity_workspace<'a>(
    workspace: &Workspace,
    identity: &ResolvedIdentity,
    path: &'a str,
) -> ApiResult<(AuthorizedWorkspace, &'a str)> {
    let normalized = path.trim().trim_matches('/');
    if normalized == paths::ACTORS_DIR
        || normalized.starts_with(&format!("{}/", paths::ACTORS_DIR))
        || normalized == paths::CONVERSATIONS_DIR
        || normalized.starts_with(&format!("{}/", paths::CONVERSATIONS_DIR))
        || normalized == ".thinclaw"
        || normalized.starts_with(".thinclaw/")
    {
        return Err(ApiError::InvalidInput(
            "canonical actor/conversation storage paths are hidden; use a caller-relative path"
                .to_string(),
        ));
    }
    if normalized == ACTOR_IDENTITY_ALIAS {
        if identity.conversation_kind != ConversationKind::Direct {
            return Err(ApiError::InvalidInput(
                "actor/IDENTITY.md is available only in a direct actor context".to_string(),
            ));
        }
        return Ok((
            AuthorizedWorkspace::conversation(workspace, identity, "desktop"),
            paths::IDENTITY,
        ));
    }
    if normalized == "actor" || normalized.starts_with("actor/") {
        return Err(ApiError::InvalidInput(
            "actor/ is a reserved desktop alias; only actor/IDENTITY.md is supported".to_string(),
        ));
    }
    if is_control_plane_path(path) {
        Ok((
            AuthorizedWorkspace::principal_admin(workspace, identity, "desktop"),
            path,
        ))
    } else {
        Ok((
            AuthorizedWorkspace::conversation(workspace, identity, "desktop"),
            path,
        ))
    }
}

fn visible_control_path(path: &str) -> bool {
    let normalized = path.trim().trim_matches('/');
    is_control_plane_path(normalized)
        && normalized != paths::ACTORS_DIR
        && !normalized.starts_with(&format!("{}/", paths::ACTORS_DIR))
        && normalized != paths::CONVERSATIONS_DIR
        && !normalized.starts_with(&format!("{}/", paths::CONVERSATIONS_DIR))
        && normalized != ".thinclaw"
        && !normalized.starts_with(".thinclaw/")
}

async fn ensure_owner_namespace(
    workspace: &Workspace,
    identity: &ResolvedIdentity,
) -> ApiResult<()> {
    if identity.conversation_kind == ConversationKind::Direct
        && identity.actor_id == identity.principal_id
    {
        let scoped = workspace.scoped_clone(identity.principal_id.clone(), workspace.agent_id());
        scoped
            .migrate_legacy_owner_knowledge(&identity.actor_id)
            .await
            .map_err(|error| ApiError::Internal(error.to_string()))?;
    }
    Ok(())
}

/// Read from the desktop's composite memory view. Trusted control files are
/// principal-admin scoped; all other paths resolve inside the exact actor or
/// group namespace carried by `identity`.
pub async fn get_file_for_identity(
    workspace: &Arc<Workspace>,
    identity: &ResolvedIdentity,
    path: &str,
) -> ApiResult<MemoryReadResponse> {
    ensure_owner_namespace(workspace, identity).await?;
    let (scoped, resolved_path) = identity_workspace(workspace, identity, path)?;
    if scoped.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && resolved_path.trim().trim_matches('/') == paths::SOUL
    {
        let content = crate::identity::soul_store::read_home_soul()
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        return Ok(memory_read_response(paths::SOUL, content, None));
    }
    let document = scoped
        .read(resolved_path)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let display_path = if path.trim().trim_matches('/') == ACTOR_IDENTITY_ALIAS {
        ACTOR_IDENTITY_ALIAS.to_string()
    } else {
        scoped
            .access()
            .display_path(&document.path)
            .unwrap_or(document.path)
    };
    Ok(memory_read_response(
        display_path,
        document.content,
        Some(document.updated_at.to_rfc3339()),
    ))
}

pub async fn write_file_for_identity(
    workspace: &Arc<Workspace>,
    store: Option<&Arc<dyn crate::db::Database>>,
    identity: &ResolvedIdentity,
    path: &str,
    content: &str,
) -> ApiResult<()> {
    ensure_owner_namespace(workspace, identity).await?;
    let timezone_update =
        crate::timezone::actor_timezone_update_for_document(identity, path, content)
            .map_err(ApiError::InvalidInput)?;
    let timezone_store = match timezone_update.as_ref() {
        Some(_) => Some(store.ok_or_else(|| {
            ApiError::Unavailable(
                "database is required to keep USER.md timezone and routines consistent".to_string(),
            )
        })?),
        None => None,
    };
    let (scoped, resolved_path) = identity_workspace(workspace, identity, path)?;
    if scoped.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && resolved_path.trim().trim_matches('/') == paths::SOUL
    {
        crate::identity::soul_store::write_home_soul(content)
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        return Ok(());
    }
    scoped
        .write(resolved_path, content)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;

    if let (Some(update), Some(store)) = (timezone_update, timezone_store) {
        crate::timezone::apply_actor_timezone_change(
            store,
            &update.principal_id,
            &update.actor_id,
            update.timezone.as_deref(),
        )
        .await
        .map_err(ApiError::Internal)?;
    }
    Ok(())
}

pub async fn delete_file_for_identity(
    workspace: &Arc<Workspace>,
    identity: &ResolvedIdentity,
    path: &str,
) -> ApiResult<()> {
    ensure_owner_namespace(workspace, identity).await?;
    let (scoped, resolved_path) = identity_workspace(workspace, identity, path)?;
    if scoped.access().role() == WorkspaceAccessRole::PrincipalAdmin
        && resolved_path.trim().trim_matches('/') == paths::SOUL
    {
        return Err(ApiError::InvalidInput(
            "The canonical home SOUL.md cannot be deleted; clear or replace its content instead"
                .to_string(),
        ));
    }
    scoped
        .delete(resolved_path)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))
}

/// List the desktop composite view without exposing canonical actor/group
/// prefixes or sibling namespaces.
pub async fn list_files_for_identity(
    workspace: &Arc<Workspace>,
    identity: &ResolvedIdentity,
) -> ApiResult<Vec<String>> {
    ensure_owner_namespace(workspace, identity).await?;
    let control = AuthorizedWorkspace::principal_admin(workspace, identity, "desktop");
    let conversation = AuthorizedWorkspace::conversation(workspace, identity, "desktop");
    let (control_paths, conversation_paths) =
        tokio::try_join!(control.list_all(), conversation.list_all(),)
            .map_err(|error| ApiError::Internal(error.to_string()))?;

    let mut paths = std::collections::BTreeSet::new();
    paths.extend(
        control_paths
            .into_iter()
            .filter(|path| visible_control_path(path)),
    );
    paths.extend(
        conversation_paths
            .into_iter()
            .filter_map(|path| conversation.access().display_path(&path))
            .map(|path| {
                if path == paths::IDENTITY {
                    ACTOR_IDENTITY_ALIAS.to_string()
                } else {
                    path
                }
            })
            .filter(|path| !path.is_empty()),
    );
    if crate::identity::soul_store::read_home_soul().is_ok() {
        paths.insert(paths::SOUL.to_string());
    }
    Ok(paths.into_iter().collect())
}

pub async fn search_for_identity(
    workspace: &Arc<Workspace>,
    identity: &ResolvedIdentity,
    query: &str,
    limit: Option<usize>,
) -> ApiResult<MemorySearchResponse> {
    let limit = validated_memory_search_limit(query, limit)?;
    ensure_owner_namespace(workspace, identity).await?;
    let scoped = AuthorizedWorkspace::conversation(workspace, identity, "desktop");
    let results = scoped
        .search(query, SearchConfig::default().with_limit(limit))
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(memory_search_response_from_hits(results.into_iter().map(
        |result| {
            GatewayMemorySearchHit {
                path: scoped
                    .access()
                    .display_path(&result.path)
                    .unwrap_or(result.path),
                content: result.content,
                score: result.score as f64,
            }
        },
    )))
}

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
    let max = validated_memory_search_limit(query, limit)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::direct_scope_id;

    fn local_identity() -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: "local_user".to_string(),
            actor_id: "local_user".to_string(),
            conversation_scope_id: direct_scope_id("local_user", "local_user"),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: "local_user".to_string(),
            stable_external_conversation_key: "tauri://direct/local_user/test".to_string(),
        }
    }

    #[tokio::test]
    async fn desktop_composite_view_migrates_legacy_memory_and_hides_siblings() {
        let (db, _guard) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("local_user", Arc::clone(&db)));
        let legacy = workspace.scoped_clone("default", None);
        legacy.write(paths::MEMORY, "legacy memory").await.unwrap();

        workspace
            .migrate_missing_principal_scope("default")
            .await
            .unwrap();
        let identity = local_identity();
        let read = get_file_for_identity(&workspace, &identity, paths::MEMORY)
            .await
            .unwrap();
        assert_eq!(read.content, "legacy memory");

        write_file_for_identity(&workspace, None, &identity, paths::MEMORY, "actor memory")
            .await
            .unwrap();
        assert_eq!(
            workspace
                .read(&paths::actor_memory("local_user"))
                .await
                .unwrap()
                .content,
            "actor memory"
        );
        assert_eq!(
            workspace.read(paths::MEMORY).await.unwrap().content,
            "legacy memory",
            "actor writes must not mutate the compatibility root"
        );

        workspace
            .write(&paths::actor_memory("sibling"), "private sibling memory")
            .await
            .unwrap();
        workspace
            .write(paths::IDENTITY, "principal agent identity")
            .await
            .unwrap();
        let actor_identity_path = Workspace::actor_path("local_user", paths::IDENTITY);
        workspace
            .write(&actor_identity_path, "private actor identity")
            .await
            .unwrap();
        let visible = list_files_for_identity(&workspace, &identity)
            .await
            .unwrap();
        assert!(visible.iter().any(|path| path == paths::MEMORY));
        assert!(visible.iter().any(|path| path == paths::IDENTITY));
        assert!(visible.iter().any(|path| path == ACTOR_IDENTITY_ALIAS));
        assert!(!visible.iter().any(|path| path.contains("sibling")));
        assert!(!visible.iter().any(|path| path.starts_with("actors/")));
        assert!(!visible.iter().any(|path| path.starts_with(".thinclaw/")));

        assert_eq!(
            get_file_for_identity(&workspace, &identity, paths::IDENTITY)
                .await
                .unwrap()
                .content,
            "principal agent identity"
        );
        assert_eq!(
            get_file_for_identity(&workspace, &identity, ACTOR_IDENTITY_ALIAS)
                .await
                .unwrap()
                .content,
            "private actor identity"
        );
        write_file_for_identity(
            &workspace,
            None,
            &identity,
            ACTOR_IDENTITY_ALIAS,
            "updated actor identity",
        )
        .await
        .unwrap();
        assert_eq!(
            workspace.read(&actor_identity_path).await.unwrap().content,
            "updated actor identity"
        );
        assert_eq!(
            workspace.read(paths::IDENTITY).await.unwrap().content,
            "principal agent identity",
            "actor identity writes must not mutate the trusted root identity"
        );

        let sibling_path = paths::actor_memory("sibling");
        assert!(
            get_file_for_identity(&workspace, &identity, &sibling_path)
                .await
                .is_err(),
            "hidden canonical paths must not become an admin read escape hatch"
        );
        assert!(
            write_file_for_identity(
                &workspace,
                None,
                &identity,
                &sibling_path,
                "overwrite attempt",
            )
            .await
            .is_err(),
            "hidden canonical paths must not become an admin write escape hatch"
        );
    }
}

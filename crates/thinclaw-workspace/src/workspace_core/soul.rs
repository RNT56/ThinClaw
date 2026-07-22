//! Canonical home soul management and legacy workspace-soul migration.
//!
//! The canonical `SOUL.md` lives at a fixed home path (outside the DB-backed
//! workspace). This module reads/writes that file, composes a seeded soul on
//! first run, and migrates any legacy workspace-stored `SOUL.md` into the home
//! soul (main workspace) or a `SOUL.local.md` overlay (agent workspace).

use std::{fs, path::PathBuf};

use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use crate::document::paths;

const MAX_HOME_SOUL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HomeSoulStatus {
    Existing,
    CreatedFromPack,
    MigratedLegacyWorkspaceSoul,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HomeSoulEnsureResult {
    pub(super) status: HomeSoulStatus,
    pub(super) content: String,
    pub(super) path: PathBuf,
}

pub(super) fn canonical_soul_path() -> PathBuf {
    thinclaw_platform::state_paths().soul_file
}

pub(super) fn read_home_soul() -> Result<String, WorkspaceError> {
    let path = canonical_soul_path();
    thinclaw_platform::read_regular_file_bounded_single_link(&path, MAX_HOME_SOUL_BYTES)
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => WorkspaceError::DocumentNotFound {
                doc_type: paths::SOUL.to_string(),
                user_id: "home".to_string(),
            },
            _ => WorkspaceError::SearchFailed {
                reason: format!("failed to read {}: {}", path.display(), err),
            },
        })
}

pub(super) fn write_home_soul(content: &str) -> Result<(), WorkspaceError> {
    let path = canonical_soul_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkspaceError::SearchFailed {
            reason: format!("failed to create {}: {}", parent.display(), err),
        })?;
    }
    thinclaw_platform::write_private_file_atomic(&path, content.as_bytes(), true).map_err(|err| {
        WorkspaceError::SearchFailed {
            reason: format!("failed to write {}: {}", path.display(), err),
        }
    })
}

pub(super) async fn ensure_home_soul(
    workspace: &Workspace,
    seed_pack: &str,
) -> Result<HomeSoulEnsureResult, WorkspaceError> {
    let path = canonical_soul_path();
    if path.exists() {
        migrate_workspace_legacy_soul(workspace).await?;
        return Ok(HomeSoulEnsureResult {
            status: HomeSoulStatus::Existing,
            content: read_home_soul()?,
            path,
        });
    }

    if workspace.agent_id().is_none()
        && let Ok(legacy) = workspace.read(paths::SOUL).await
        && !legacy.content.trim().is_empty()
    {
        write_home_soul(&legacy.content)?;
        archive_legacy_soul(workspace, &legacy.content).await?;
        workspace.delete(paths::SOUL).await?;
        return Ok(HomeSoulEnsureResult {
            status: HomeSoulStatus::MigratedLegacyWorkspaceSoul,
            content: legacy.content,
            path,
        });
    }

    let content = thinclaw_soul::compose_seeded_soul(seed_pack).map_err(|err| {
        WorkspaceError::SearchFailed {
            reason: format!("failed to compose seeded home soul: {}", err),
        }
    })?;
    write_home_soul(&content)?;
    migrate_workspace_legacy_soul(workspace).await?;
    Ok(HomeSoulEnsureResult {
        status: HomeSoulStatus::CreatedFromPack,
        content,
        path,
    })
}

async fn migrate_workspace_legacy_soul(workspace: &Workspace) -> Result<(), WorkspaceError> {
    let Ok(legacy) = workspace.read(paths::SOUL).await else {
        return Ok(());
    };
    if legacy.content.trim().is_empty() {
        return Ok(());
    }

    if workspace.agent_id().is_some() && !workspace.exists(paths::SOUL_LOCAL).await? {
        workspace.write(paths::SOUL_LOCAL, &legacy.content).await?;
    }

    archive_legacy_soul(workspace, &legacy.content).await?;
    workspace.delete(paths::SOUL).await?;
    Ok(())
}

async fn archive_legacy_soul(workspace: &Workspace, content: &str) -> Result<(), WorkspaceError> {
    workspace.write(paths::SOUL_LEGACY, content).await?;
    Ok(())
}

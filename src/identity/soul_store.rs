use std::fs;
use std::path::PathBuf;

use crate::error::WorkspaceError;
use crate::platform::state_paths;
use crate::workspace::{Workspace, paths};

use super::soul;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSoulStatus {
    Existing,
    CreatedFromPack,
    MigratedLegacyWorkspaceSoul,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSoulEnsureResult {
    pub status: HomeSoulStatus,
    pub content: String,
    pub path: PathBuf,
}

pub fn canonical_soul_path() -> PathBuf {
    state_paths().soul_file
}

pub fn read_home_soul() -> Result<String, WorkspaceError> {
    let path = canonical_soul_path();
    fs::read_to_string(&path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => WorkspaceError::DocumentNotFound {
            doc_type: paths::SOUL.to_string(),
            user_id: "home".to_string(),
        },
        _ => WorkspaceError::SearchFailed {
            reason: format!("failed to read {}: {}", path.display(), err),
        },
    })
}

pub fn write_home_soul(content: &str) -> Result<(), WorkspaceError> {
    let path = canonical_soul_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkspaceError::SearchFailed {
            reason: format!("failed to create {}: {}", parent.display(), err),
        })?;
    }
    fs::write(&path, content).map_err(|err| WorkspaceError::SearchFailed {
        reason: format!("failed to write {}: {}", path.display(), err),
    })
}

pub async fn ensure_home_soul(
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

    let content =
        soul::compose_seeded_soul(seed_pack).map_err(|err| WorkspaceError::SearchFailed {
            reason: format!("failed to compose seeded home soul: {}", err),
        })?;
    write_home_soul(&content)?;
    migrate_workspace_legacy_soul(workspace).await?;
    Ok(HomeSoulEnsureResult {
        status: HomeSoulStatus::CreatedFromPack,
        content,
        path,
    })
}

pub async fn migrate_workspace_legacy_soul(workspace: &Workspace) -> Result<(), WorkspaceError> {
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

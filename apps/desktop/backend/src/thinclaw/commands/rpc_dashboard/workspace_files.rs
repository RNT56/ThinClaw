//! Local agent-workspace dashboard RPC commands: path resolution, Finder/
//! Explorer reveal, file listing, and file writes.

use tauri::State;
use tracing::info;

use crate::thinclaw::commands::ThinClawManager;
use crate::thinclaw::runtime_builder::get_resolved_workspace_root;

/// Return the local filesystem workspace root path.
///
/// This is the directory where the agent writes local files (write_file, shell, etc.).
/// Defaults to the same app-data agent workspace used by the ThinClaw bridge.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_workspace_path(
    manager: State<'_, ThinClawManager>,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    Ok(workspace_root_for_commands(&manager)
        .await
        .to_string_lossy()
        .to_string())
}

/// Open the local workspace directory in Finder (macOS) / Explorer (Windows).
///
/// Creates the directory if it doesn't exist yet. Returns the path that was opened.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_reveal_workspace(
    manager: State<'_, ThinClawManager>,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    let path = workspace_root_for_commands(&manager).await;
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("Failed to create workspace directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("Failed to inspect workspace directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace root must be a real directory".to_string(),
        });
    }
    let path = path
        .canonicalize()
        .map_err(|error| format!("Failed to resolve workspace directory: {error}"))?;
    let path_str = path.to_string_lossy().to_string();

    // Open in Finder (macOS) / Explorer (Windows) using OS built-ins
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Finder: {}", e))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Explorer: {}", e))?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    info!("[thinclaw-runtime] Revealed workspace: {}", path_str);
    Ok(path_str)
}

/// List all files in the agent's local `agent_workspace` directory.
///
/// Returns relative paths (from workspace root), file sizes, and modification
/// timestamps so the frontend can build a proper file browser.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_list_agent_workspace_files(
    manager: State<'_, ThinClawManager>,
) -> Result<Vec<serde_json::Value>, crate::thinclaw::bridge::BridgeError> {
    let workspace_root = workspace_root_for_commands(&manager).await;

    let root_metadata = match std::fs::symlink_metadata(&workspace_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(format!("Failed to inspect workspace: {error}").into()),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace root must be a real directory".to_string(),
        });
    }
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| format!("Failed to resolve workspace: {error}"))?;

    let mut entries = Vec::new();

    /// Directories to skip when recursively listing the workspace.
    /// These are often massive (node_modules can have 50k+ files)
    /// and walking them can cause memory corruption / OOM.
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
        ".build",
        "Pods",
    ];

    /// Hard cap on total entries to prevent runaway recursion from
    /// corrupting the allocator.
    const MAX_ENTRIES: usize = 5000;

    fn walk_dir(
        dir: &std::path::Path,
        root: &std::path::Path,
        entries: &mut Vec<serde_json::Value>,
        visited: &mut usize,
        depth: usize,
    ) {
        if depth > 6 || *visited >= MAX_ENTRIES {
            return; // Prevent runaway recursion
        }
        let read = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in read.flatten() {
            *visited = visited.saturating_add(1);
            if *visited > MAX_ENTRIES {
                return;
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
                continue;
            }
            let rel_path = path.strip_prefix(root).unwrap_or(&path);

            // Skip hidden files and common junk
            if rel_path.components().any(|component| {
                matches!(component, std::path::Component::Normal(name) if name.to_string_lossy().starts_with('.'))
            }) {
                continue;
            }
            let rel = rel_path.to_string_lossy().to_string();

            if metadata.is_dir() {
                // Skip heavy directories that would blow up memory
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&dir_name) {
                    continue;
                }
                walk_dir(&path, root, entries, visited, depth + 1);
            } else {
                let size = metadata.len();
                let modified_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                entries.push(serde_json::json!({
                    "path": rel,
                    "absolute_path": path.to_string_lossy(),
                    "size": size,
                    "modified_ms": modified_ms,
                }));
            }
        }
    }

    let mut visited = 0usize;
    walk_dir(
        &workspace_root,
        &workspace_root,
        &mut entries,
        &mut visited,
        0,
    );

    // Sort by path
    entries.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });

    Ok(entries)
}

/// Reveal a specific file in Finder (macOS) / Explorer (Windows).
///
/// Uses `open -R <path>` on macOS to select the file in a Finder window,
/// which is more user-friendly than just opening the parent folder.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_reveal_file(
    manager: State<'_, ThinClawManager>,
    path: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if path.is_empty() || path.len() > 4_096 || path.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Invalid workspace path".to_string(),
        });
    }
    let workspace_root = workspace_root_for_commands(&manager).await;
    let root_metadata = std::fs::symlink_metadata(&workspace_root)
        .map_err(|error| format!("Failed to inspect workspace: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace root must be a real directory".to_string(),
        });
    }
    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|error| format!("Failed to resolve workspace: {error}"))?;
    let requested = std::path::Path::new(&path);
    let requested_metadata = std::fs::symlink_metadata(requested)
        .map_err(|_| "Workspace file was not found".to_string())?;
    if requested_metadata.file_type().is_symlink()
        || (!requested_metadata.is_file() && !requested_metadata.is_dir())
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace path must be a real file or directory".to_string(),
        });
    }
    let p = requested
        .canonicalize()
        .map_err(|error| format!("Failed to resolve workspace path: {error}"))?;
    if p == canonical_root || !p.starts_with(&canonical_root) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Path is outside the agent workspace".to_string(),
        });
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-R") // -R = reveal (select in Finder)
        .arg(&p)
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Finder: {}", e))?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&p)
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Explorer: {}", e))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(&p))
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    Ok(())
}

/// Write content to a file in the agent's local `agent_workspace` directory.
///
/// The `relative_path` is resolved against `WORKSPACE_ROOT`. Parent directories
/// are created automatically. Path traversal (`..`) is rejected for safety.
/// Returns the absolute path of the written file.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_write_agent_workspace_file(
    manager: State<'_, ThinClawManager>,
    relative_path: String,
    content: String,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    const MAX_WORKSPACE_WRITE_BYTES: usize = 8 * 1024 * 1024;
    let relative = std::path::Path::new(&relative_path);
    if relative_path.is_empty()
        || relative_path.len() > 4_096
        || relative_path.chars().any(char::is_control)
        || relative.is_absolute()
        || relative.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
        || content.len() > MAX_WORKSPACE_WRITE_BYTES
        || content.contains('\0')
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace path or content is malformed or oversized".to_string(),
        });
    }

    let workspace_root = workspace_root_for_commands(&manager).await;
    std::fs::create_dir_all(&workspace_root)
        .map_err(|error| format!("Failed to create workspace: {error}"))?;
    let root_metadata = std::fs::symlink_metadata(&workspace_root)
        .map_err(|error| format!("Failed to inspect workspace: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Workspace root must be a real directory".to_string(),
        });
    }
    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|error| format!("Failed to resolve workspace: {error}"))?;
    let target = canonical_root.join(relative);
    ensure_real_workspace_parent(&canonical_root, relative)?;

    // Re-resolve the completed parent immediately before publishing the file.
    // The atomic writer separately rejects a final-component symlink.
    let canonical_parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Path escapes workspace root".to_string(),
        });
    }

    thinclaw_platform::write_regular_file_atomic(&target, content.as_bytes(), true)
        .map_err(|e| format!("Failed to write file: {e}"))?;

    let abs = target.to_string_lossy().to_string();
    tracing::info!(
        path = %abs,
        bytes = content.len(),
        "Wrote automation result to agent_workspace"
    );
    Ok(abs)
}

/// Create each missing relative parent one component at a time, refusing to
/// traverse an existing symlink or special file. `create_dir_all` is unsafe at
/// this boundary because one intermediate symlink can redirect creation out of
/// the workspace before a final canonicalization check notices it.
fn ensure_real_workspace_parent(
    canonical_root: &std::path::Path,
    relative: &std::path::Path,
) -> Result<(), String> {
    let mut current = canonical_root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };

    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            if matches!(component, std::path::Component::CurDir) {
                continue;
            }
            return Err("Workspace path contains an invalid parent component".to_string());
        };
        current.push(name);

        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "Workspace parent is not a real directory: {}",
                    current.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(create_error) = std::fs::create_dir(&current) {
                    if create_error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(format!(
                            "Failed to create workspace directory {}: {create_error}",
                            current.display()
                        ));
                    }
                }
                let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
                    format!(
                        "Failed to verify workspace directory {}: {error}",
                        current.display()
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Workspace parent is not a real directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect workspace directory {}: {error}",
                    current.display()
                ));
            }
        }
    }

    let resolved = current.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve workspace directory {}: {error}",
            current.display()
        )
    })?;
    if resolved != canonical_root && !resolved.starts_with(canonical_root) {
        return Err("Path escapes workspace root".to_string());
    }
    Ok(())
}

async fn workspace_root_for_commands(manager: &ThinClawManager) -> std::path::PathBuf {
    if let Some(root) = get_resolved_workspace_root().filter(|root| !root.is_empty()) {
        return std::path::PathBuf::from(root);
    }

    let cfg = manager.get_config().await;
    if let Some(root) = cfg
        .as_ref()
        .and_then(|c| c.workspace_root.as_ref())
        .filter(|root| !root.is_empty())
    {
        return std::path::PathBuf::from(root);
    }

    if let Some(base_dir) = cfg.as_ref().map(|c| c.base_dir.clone()) {
        return base_dir.join("agent_workspace");
    }

    std::env::var("HOME")
        .map(|home| {
            std::path::PathBuf::from(home)
                .join("ThinClaw")
                .join("agent_workspace")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("agent_workspace"))
}

#[cfg(test)]
mod tests {
    use super::ensure_real_workspace_parent;

    #[test]
    fn creates_only_real_relative_parent_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();

        ensure_real_workspace_parent(&root, std::path::Path::new("one/two/file.txt")).unwrap();

        assert!(root.join("one/two").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_intermediate_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        symlink(outside.path(), root.join("redirect")).unwrap();

        let error =
            ensure_real_workspace_parent(&root, std::path::Path::new("redirect/new/file.txt"))
                .unwrap_err();

        assert!(error.contains("not a real directory"));
        assert!(!outside.path().join("new").exists());
    }
}

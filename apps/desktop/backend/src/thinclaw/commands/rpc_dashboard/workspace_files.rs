//! Local agent-workspace dashboard RPC commands: path resolution, Finder/
//! Explorer reveal, file listing, and file writes.

use tauri::State;
use tracing::{info, warn};

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
) -> Result<String, String> {
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
) -> Result<String, String> {
    let path = workspace_root_for_commands(&manager).await;
    let path_str = path.to_string_lossy().to_string();

    // Ensure directory exists
    if let Err(e) = std::fs::create_dir_all(&path_str) {
        warn!(
            "[thinclaw-runtime] Could not create workspace dir {}: {}",
            path_str, e
        );
    }

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
) -> Result<Vec<serde_json::Value>, String> {
    let workspace_root = workspace_root_for_commands(&manager).await;

    if !workspace_root.exists() {
        return Ok(vec![]);
    }

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
        depth: usize,
    ) {
        if depth > 6 || entries.len() >= MAX_ENTRIES {
            return; // Prevent runaway recursion
        }
        let read = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in read.flatten() {
            if entries.len() >= MAX_ENTRIES {
                return;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            // Skip hidden files and common junk
            if rel.starts_with('.') || rel.contains("/.") || rel.ends_with(".DS_Store") {
                continue;
            }

            if path.is_dir() {
                // Skip heavy directories that would blow up memory
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&dir_name) {
                    continue;
                }
                walk_dir(&path, root, entries, depth + 1);
            } else {
                let meta = std::fs::metadata(&path);
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified_ms = meta
                    .as_ref()
                    .ok()
                    .and_then(|m| m.modified().ok())
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

    walk_dir(&workspace_root, &workspace_root, &mut entries, 0);

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
pub async fn thinclaw_reveal_file(path: String) -> Result<(), String> {
    // Security: prevent path traversal
    let p = std::path::Path::new(&path);
    if path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    // Only reveal files that exist
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-R") // -R = reveal (select in Finder)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Finder: {}", e))?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .args(["/select,", &path])
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Explorer: {}", e))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(p))
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
) -> Result<String, String> {
    // Security: prevent path traversal
    if relative_path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    let workspace_root = workspace_root_for_commands(&manager).await;

    let target = workspace_root.join(&relative_path);

    // Ensure the resolved path is still inside the workspace
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());
    // Can't canonicalize the target yet (file may not exist), but check prefix
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    // Double-check after dir creation
    let canonical_parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("Path escapes workspace root".to_string());
    }

    std::fs::write(&target, &content).map_err(|e| format!("Failed to write file: {}", e))?;

    let abs = target.to_string_lossy().to_string();
    tracing::info!(
        path = %abs,
        bytes = content.len(),
        "Wrote automation result to agent_workspace"
    );
    Ok(abs)
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

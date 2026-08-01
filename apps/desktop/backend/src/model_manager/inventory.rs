use super::*;

#[tauri::command]
#[specta::specta]
pub async fn list_models(
    app: AppHandle,
) -> Result<Vec<ModelFile>, crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;

    // Ensure category folders exist
    for category in ALLOWED_MODEL_CATEGORIES {
        let cat_dir = models_dir.join(category);
        ensure_real_directory(&cat_dir)?;
        if let Err(error) = crate::hf_hub::cleanup_stale_hf_staging_dirs(&cat_dir) {
            tracing::warn!(
                path = %cat_dir.display(),
                %error,
                "Could not clean stale HuggingFace download staging directories"
            );
        }
    }

    let mut models = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models_dir, &models_dir, &mut models, 0, &mut visited);

    // Return the complete inventory. Compatibility is explicit per entry so
    // auxiliary STT/diffusion assets are never hidden by the chat engine.
    Ok(models)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if filename.is_empty()
        || filename.len() > MAX_MODEL_PATH_BYTES
        || filename.chars().any(char::is_control)
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Download identifier is invalid".to_string(),
        });
    }
    let downloads = state.downloads.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(notify) = downloads.get(&filename) {
        notify.notify_one();
        Ok(())
    } else {
        Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Download not found".to_string(),
        })
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_model_path(app: AppHandle, path: String) -> bool {
    let Ok(models) = managed_models_dir(&app) else {
        return false;
    };
    if path.is_empty() || path.len() > 8_192 || path.chars().any(char::is_control) {
        return false;
    }
    let path = Path::new(&path);
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return false;
    }
    path.canonicalize()
        .is_ok_and(|resolved| resolved.starts_with(models))
}

#[tauri::command]
#[specta::specta]
pub async fn open_models_folder(
    app: AppHandle,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;

    // Also ensure category folders exist
    for category in ALLOWED_MODEL_CATEGORIES {
        let cat_dir = models_dir.join(category);
        ensure_real_directory(&cat_dir)?;
    }

    // Also ensure standard folders exist so users can manually drop files (Inside Diffusion for SD 1.5 logic)
    // Actually, user requested "diffusion folder will also contain the standard folder"
    let diffusion_dir = models_dir.join("Diffusion");
    let standard_dir = diffusion_dir.join("standard"); // Move standard to Diffusion/standard
    ensure_real_directory(&standard_dir)?;
    for category in ["vae", "t5", "clip", "other"] {
        ensure_real_directory(&standard_dir.join(category))?;
    }

    #[cfg(target_os = "macos")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("open").arg(&models_dir))
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("xdg-open").arg(&models_dir))
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("explorer").arg(&models_dir))
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_standard_models_folder(
    app: AppHandle,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;
    ensure_real_directory(&models_dir.join("Diffusion"))?;
    let standard_dir = models_dir.join("Diffusion").join("standard"); // Updated path
    ensure_real_directory(&standard_dir)?;

    // Ensure subfolders exist
    for category in ["vae", "t5", "clip", "other"] {
        ensure_real_directory(&standard_dir.join(category))?;
    }

    #[cfg(target_os = "macos")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("open").arg(&standard_dir))
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("xdg-open").arg(&standard_dir))
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    thinclaw_platform::spawn_reaped_std(std::process::Command::new("explorer").arg(&standard_dir))
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(
    app: AppHandle,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    install_root: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let _lifecycle_guard = crate::model_lifecycle::MODEL_LIFECYCLE_LOCK.lock().await;
    let relative = validate_model_relative(&install_root, false)?;
    let models_dir = managed_models_dir(&app)?;
    let resolved = resolve_model_install(&models_dir, &relative)?;
    crate::sidecar::deactivate_model_target_locked(
        &app,
        sidecar.inner(),
        engine_manager.inner(),
        &resolved.canonical_path,
        crate::model_lifecycle::ModelLifecycleRoles::all(),
    )
    .await?;
    delete_resolved_model_install(&resolved).map_err(Into::into)
}

pub(super) struct ResolvedModelInstall {
    pub(super) path: PathBuf,
    pub(super) canonical_path: PathBuf,
    pub(super) is_file: bool,
}

pub(super) fn resolve_model_install(
    models_dir: &Path,
    relative: &Path,
) -> Result<ResolvedModelInstall, String> {
    let mut inventory = Vec::new();
    let mut visited = 0;
    scan_models_recursive(models_dir, models_dir, &mut inventory, 0, &mut visited);
    let requested_root = inventory_relative_path(relative);
    if !inventory
        .iter()
        .any(|model| model.install_root == requested_root)
    {
        return Err("The requested path is not a declared model installation".to_string());
    }

    let canonical_models = models_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve model storage: {error}"))?;
    let file_path = models_dir.join(relative);
    let metadata =
        fs::symlink_metadata(&file_path).map_err(|_| "Managed model was not found".to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err("Managed model path is not a regular file or directory".to_string());
    }
    let canonical_target = file_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model: {error}"))?;
    if !canonical_target.starts_with(&canonical_models) || canonical_target == canonical_models {
        return Err("Managed model path escaped model storage".to_string());
    }
    Ok(ResolvedModelInstall {
        path: file_path,
        canonical_path: canonical_target,
        is_file: metadata.is_file(),
    })
}

pub(super) fn delete_resolved_model_install(resolved: &ResolvedModelInstall) -> Result<(), String> {
    let metadata = fs::symlink_metadata(&resolved.path)
        .map_err(|_| "Managed model disappeared before deletion".to_string())?;
    if metadata.file_type().is_symlink()
        || metadata.is_file() != resolved.is_file
        || !(metadata.is_file() || metadata.is_dir())
    {
        return Err("Managed model changed before deletion".to_string());
    }
    let canonical_target = resolved
        .path
        .canonicalize()
        .map_err(|error| format!("Could not re-resolve managed model: {error}"))?;
    if canonical_target != resolved.canonical_path {
        return Err("Managed model target changed before deletion".to_string());
    }
    // Delete exactly the inventory-provided install root. A legacy file does
    // not prove that its siblings belong to the same install, so parent-folder
    // inference can destroy unrelated models.
    if resolved.is_file {
        fs::remove_file(&resolved.path).map_err(|e| format!("Failed to delete file: {}", e))?;
    } else {
        fs::remove_dir_all(&resolved.path)
            .map_err(|e| format!("Failed to delete directory: {}", e))?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn delete_model_install(models_dir: &Path, relative: &Path) -> Result<(), String> {
    let resolved = resolve_model_install(models_dir, relative)?;
    delete_resolved_model_install(&resolved)
}

pub(crate) fn resolve_inventory_install_root(
    app: &AppHandle,
    install_root: &str,
) -> Result<PathBuf, String> {
    let relative = validate_model_relative(install_root, false)?;
    let models_dir = managed_models_dir(app)?;
    Ok(resolve_model_install(&models_dir, &relative)?.canonical_path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedInventoryModel {
    /// Canonical entrypoint selected from the inventory. This may be either a
    /// single model file or a repository directory.
    pub(crate) path: PathBuf,
    /// Canonical root that owns every file in this logical installation.
    pub(crate) install_root: PathBuf,
    /// Canonical companion explicitly declared by the installation manifest.
    pub(crate) companion_path: Option<PathBuf>,
}

pub(super) fn resolve_compatible_inventory_model_in(
    models_dir: &Path,
    selected_path: &Path,
    runtime: &str,
    category: &str,
) -> Result<ResolvedInventoryModel, String> {
    let canonical_models = models_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve model storage: {error}"))?;
    let metadata = fs::symlink_metadata(selected_path)
        .map_err(|_| "The selected local model was not found".to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err(
            "The selected local model must be a real regular file or directory".to_string(),
        );
    }
    let canonical_selected = selected_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve the selected local model: {error}"))?;
    if canonical_selected == canonical_models || !canonical_selected.starts_with(&canonical_models)
    {
        return Err("The selected local model is outside managed model storage".to_string());
    }

    let mut inventory = Vec::new();
    let mut visited = 0;
    scan_models_recursive(
        &canonical_models,
        &canonical_models,
        &mut inventory,
        0,
        &mut visited,
    );
    let model = inventory
        .into_iter()
        .find(|model| {
            model.category == category
                && Path::new(&model.path)
                    .canonicalize()
                    .is_ok_and(|path| path == canonical_selected)
        })
        .ok_or_else(|| {
            format!("Select a declared {category} model installation from the local inventory")
        })?;
    let selected_is_directory = metadata.is_dir();
    let (compatible, reason) = model_compatibility(
        runtime,
        &model.category,
        model.task.as_deref(),
        &model.format,
        selected_is_directory,
        model.runtime.as_deref(),
    );
    if !compatible {
        return Err(reason.unwrap_or_else(|| {
            format!("The selected model is not compatible with the active {runtime} runtime")
        }));
    }
    if model.source == "legacy" && !selected_is_directory {
        validate_legacy_single_file(&canonical_selected, &model.category, &model.format)?;
    }

    let install_root = canonical_models.join(Path::new(&model.install_root));
    let install_metadata = fs::symlink_metadata(&install_root)
        .map_err(|_| "The selected model installation is incomplete".to_string())?;
    if install_metadata.file_type().is_symlink()
        || !(install_metadata.is_file() || install_metadata.is_dir())
    {
        return Err("The selected model install root is unsafe".to_string());
    }
    let install_root = install_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve the selected model install root: {error}"))?;
    if install_root == canonical_models || !install_root.starts_with(&canonical_models) {
        return Err("The selected model install root escaped managed storage".to_string());
    }
    if install_metadata.is_file() && install_root != canonical_selected {
        return Err("The selected model does not match its declared installation".to_string());
    }
    if install_metadata.is_dir() && !canonical_selected.starts_with(&install_root) {
        return Err("The selected model escaped its declared installation".to_string());
    }

    let companion_path = model
        .companion_path
        .as_deref()
        .map(Path::new)
        .map(|path| {
            let companion_metadata = fs::symlink_metadata(path)
                .map_err(|_| "The declared model companion is missing".to_string())?;
            if companion_metadata.file_type().is_symlink()
                || !companion_metadata.is_file()
                || companion_metadata.len() == 0
            {
                return Err("The declared model companion is not a safe non-empty file".to_string());
            }
            let companion = path.canonicalize().map_err(|error| {
                format!("Could not resolve the declared model companion: {error}")
            })?;
            if install_metadata.is_file() || !companion.starts_with(&install_root) {
                return Err("The declared model companion escaped its installation".to_string());
            }
            Ok(companion)
        })
        .transpose()?;

    Ok(ResolvedInventoryModel {
        path: canonical_selected,
        install_root,
        companion_path,
    })
}

#[cfg(test)]
pub(super) fn resolve_compatible_inventory_model_path_in(
    models_dir: &Path,
    selected_path: &Path,
    runtime: &str,
    category: &str,
) -> Result<PathBuf, String> {
    resolve_compatible_inventory_model_in(models_dir, selected_path, runtime, category)
        .map(|model| model.path)
}

pub(crate) fn resolve_compatible_inventory_model(
    app: &AppHandle,
    selected_path: &str,
    runtime: &str,
    category: &str,
) -> Result<ResolvedInventoryModel, String> {
    if selected_path.is_empty()
        || selected_path.len() > MAX_MODEL_PATH_BYTES
        || selected_path.chars().any(char::is_control)
    {
        return Err("The selected local model path is invalid".to_string());
    }
    let models_dir = managed_models_dir(app)?;
    resolve_compatible_inventory_model_in(&models_dir, Path::new(selected_path), runtime, category)
}

pub(crate) fn resolve_compatible_inventory_model_path(
    app: &AppHandle,
    selected_path: &str,
    runtime: &str,
    category: &str,
) -> Result<PathBuf, String> {
    resolve_compatible_inventory_model(app, selected_path, runtime, category)
        .map(|model| model.path)
}

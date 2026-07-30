use super::*;

/// Fetch the file tree of an HF repo and parse it intelligently.
///
/// For GGUF repos (llama.cpp): extracts quantization type from filenames,
/// detects mmproj files, and sorts by size.
///
/// For MLX/vLLM repos: lists all model files (skipping README, images, etc.)
/// for a directory download.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_get_model_files_v2(
    app: AppHandle,
    repo_id: String,
    task: HfModelTask,
) -> Result<HfModelFilePlan, crate::thinclaw::bridge::BridgeError> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    build_model_file_plan(&app, &repo_id, &engine.id, task, None)
        .await
        .map_err(Into::into)
}

pub(super) fn selection_identity(
    repo_id: &str,
    revision: &str,
    task: HfModelTask,
    artifact_id: &str,
    companion_artifact_id: Option<&str>,
) -> uuid::Uuid {
    let identity = format!(
        "hf-install://{repo_id}@{revision}/{}/{artifact_id}?companion={}",
        task.as_str(),
        companion_artifact_id.unwrap_or("")
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes())
}

pub(super) fn default_destination_name(
    repo_id: &str,
    revision: &str,
    task: HfModelTask,
    artifact: &HfDownloadArtifact,
    companion_artifact_id: Option<&str>,
) -> String {
    let mut repo = repo_id.replace('/', "_");
    if repo.len() > 180 {
        repo.truncate(180);
    }
    let suffix = selection_identity(repo_id, revision, task, &artifact.id, companion_artifact_id)
        .simple()
        .to_string();
    format!("{repo}--{}", &suffix[..12])
}

pub(super) fn emit_hf_download_terminal(
    app: &AppHandle,
    download_id: &str,
    repo_id: &str,
    status: &str,
    message: Option<&str>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "download_progress",
        serde_json::json!({
            "filename": download_id,
            "download_id": download_id,
            "repo_id": repo_id,
            "status": status,
            "total": 0,
            "downloaded": 0,
            "percentage": if status == "completed" { 100.0 } else { 0.0 },
            "message": message,
        }),
    );
}

pub(super) fn validate_staged_hf_artifact(
    staging_dir: &std::path::Path,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<(), String> {
    let canonical_root = staging_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve staged HuggingFace artifact: {error}"))?;
    let requires_root_config = manifest.primary_path.is_none();
    if requires_root_config && !manifest.files.iter().any(|path| path == "config.json") {
        return Err("Staged directory artifact is missing root config.json".to_string());
    }

    for relative in &manifest.files {
        validate_hf_file_path(relative)?;
        let path = staging_dir.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| format!("Staged HuggingFace file '{relative}' is missing"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("Staged HuggingFace file '{relative}' is unsafe"));
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve staged HuggingFace file: {error}"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err("Staged HuggingFace file escaped its assigned directory".to_string());
        }

        let lower = relative.to_ascii_lowercase();
        let is_required_config = relative == "config.json"
            || (manifest.runtime == "mlx"
                && manifest.task.as_deref() == Some("diffusion")
                && relative == "model_index.json");
        if metadata.len() == 0 {
            return Err(format!("Staged managed model file '{relative}' is empty"));
        }
        if is_required_config {
            let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
                &path,
                MAX_HF_CONFIG_BYTES as u64,
            )
            .map_err(|error| {
                format!("Could not validate staged HuggingFace JSON '{relative}': {error}")
            })?;
            let config = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                format!("Staged HuggingFace JSON '{relative}' is malformed or oversized")
            })?;
            if relative == "config.json"
                && manifest.runtime == "mlx"
                && manifest.task.as_deref() == Some("embedding")
                && manifest.format == "mlx"
                && !crate::model_manager::is_supported_mlx_embedding_config(&config)
            {
                return Err(
                    "Staged MLX embedding artifact is not supported by ThinClaw's pinned mlx-embeddings 0.0.5 text-vector loader"
                        .to_string(),
                );
            }
            if relative == "config.json"
                && manifest.format == "mflux"
                && !crate::model_manager::is_supported_mflux_config(&config)
            {
                return Err(
                    "Staged MFlux artifact does not declare a supported plain FLUX.1 model"
                        .to_string(),
                );
            }
            if relative == "config.json"
                && manifest.runtime == "vllm"
                && manifest.format == "awq"
                && !is_vllm_awq_config(&config)
            {
                return Err(
                    "Staged vLLM AWQ artifact does not declare AWQ quantization".to_string()
                );
            }
        }

        if manifest.runtime == "llamacpp" && manifest.format == "gguf" {
            if !lower.ends_with(".gguf") {
                return Err("Staged llama.cpp artifact contains a non-GGUF model file".to_string());
            }
            crate::gguf::read_gguf_metadata(
                resolved
                    .to_str()
                    .ok_or_else(|| "Staged GGUF path is not valid UTF-8".to_string())?,
            )
            .map_err(|error| {
                format!("Staged GGUF file '{relative}' has invalid metadata: {error}")
            })?;
        }
    }
    if manifest.runtime == "mlx" && manifest.format == "mlx" && manifest.category == "LLM" {
        let is_vision =
            crate::model_manager::classify_mlx_vision_directory(staging_dir).map_err(|error| {
                format!("Staged MLX artifact violates its vision contract: {error}")
            })?;
        if manifest.task.as_deref() == Some("vision") && !is_vision {
            return Err(
                "Staged MLX vision artifact contains a text-only model configuration".to_string(),
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_planned_hf_files<R: tauri::Runtime>(
    app: &AppHandle<R>,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError> {
    let client = build_hf_download_client()?;
    let hf_token = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token())
        .filter(|token| {
            !token.trim().is_empty()
                && token.len() <= 16 * 1024
                && !token.chars().any(char::is_control)
        });
    download_planned_hf_files_from_app(
        app,
        &client,
        &production_hf_base_url(),
        hf_token.as_deref(),
        repo_id,
        revision,
        files,
        destination_name,
        category,
        download_id,
        cancel,
        manifest,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_planned_hf_files_from_app<R: tauri::Runtime>(
    app: &AppHandle<R>,
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    hf_token: Option<&str>,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError> {
    use tauri::Emitter;

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let progress_app = app.clone();
    download_planned_hf_files_with_http(
        &app_data,
        client,
        base_url,
        hf_token,
        move |payload| {
            let _ = progress_app.emit("download_progress", payload);
        },
        repo_id,
        revision,
        files,
        destination_name,
        category,
        download_id,
        cancel,
        manifest,
    )
    .await
}

pub(super) fn build_hf_download_client() -> Result<reqwest::Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !allowed_hf_redirect(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(2 * 60))
        .redirect(redirect_policy)
        .user_agent(concat!("ThinClawDesktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Failed to build HTTP client: {error}"))
}

#[cfg(unix)]
pub(super) fn sync_published_hf_category_with<F>(category_dir: &std::path::Path, sync: F)
where
    F: FnOnce(&std::path::Path) -> std::io::Result<()>,
{
    if let Err(error) = sync(category_dir) {
        // The no-replace rename already made the complete, validated install
        // visible. Reporting failure after that irreversible boundary makes a
        // safe retry collide with an install that actually succeeded. Preserve
        // the successful result while surfacing the reduced crash-durability.
        tracing::warn!(
            path = %category_dir.display(),
            %error,
            "HuggingFace model was published, but its parent directory could not be synced"
        );
    }
}

#[cfg(unix)]
pub(super) fn sync_published_hf_category(category_dir: &std::path::Path) {
    sync_published_hf_category_with(category_dir, |directory| {
        std::fs::File::open(directory)?.sync_all()
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_planned_hf_files_with_http<F>(
    app_data: &std::path::Path,
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    hf_token: Option<&str>,
    emit_progress: F,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError>
where
    F: Fn(serde_json::Value) + Send + Sync,
{
    use futures::StreamExt;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    validate_repo_id(repo_id)?;
    validate_hf_revision(revision, false)?;
    validate_relative_subdir(destination_name)?;
    if files.is_empty() || files.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace download must contain between 1 and {MAX_HF_DOWNLOAD_FILES} files"
        )
        .into());
    }
    if !matches!(category, "LLM" | "Embedding" | "Diffusion" | "STT" | "TTS") {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace model category is invalid".to_string(),
        });
    }
    let mut seen = std::collections::HashSet::new();
    let mut grand_total = 0_u64;
    for file in files {
        validate_hf_file_path(&file.path)?;
        if !seen.insert(file.path.as_str()) {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download contains duplicate file paths".to_string(),
            });
        }
        let size = file.expected_size.unwrap_or(0);
        if size > MAX_HF_FILE_BYTES {
            return Err(
                format!("HuggingFace file exceeds the {MAX_HF_FILE_BYTES}-byte limit").into(),
            );
        }
        grand_total = grand_total
            .checked_add(size)
            .ok_or_else(|| "HuggingFace download size overflow".to_string())?;
        if grand_total > MAX_HF_DOWNLOAD_BYTES {
            return Err(format!(
                "HuggingFace download exceeds the {MAX_HF_DOWNLOAD_BYTES}-byte limit"
            )
            .into());
        }
    }

    ensure_real_directory(app_data)?;
    let models_dir = app_data.join("models");
    ensure_real_directory(&models_dir)?;
    let category_dir = models_dir.join(category);
    ensure_real_directory(&category_dir)?;
    let destination_dir = category_dir.join(destination_name);
    match std::fs::symlink_metadata(&destination_dir) {
        Ok(_) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "The destination model directory already exists".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect model destination: {error}").into()),
    }

    let mut staging_guard = DownloadStagingGuard::create(&category_dir)?;
    let staging_dir = staging_guard.path.clone();

    let mut grand_downloaded = 0_u64;
    let mut downloaded_paths = Vec::with_capacity(files.len());
    for (file_index, planned) in files.iter().enumerate() {
        let url = hf_url_at(
            base_url,
            repo_id,
            &["resolve", revision],
            Some(&planned.path),
        )?;
        let destination = staged_file_path(&staging_dir, &planned.path)?;
        let mut request = client.get(url);
        if let Some(token) = hf_token {
            request = request.bearer_auth(token);
        }
        let response = cancellable_hf_operation(cancel, async {
            request.send().await.map_err(|error| {
                crate::rig_lib::http::transport_error("HuggingFace download request failed", error)
            })
        })
        .await?;
        if let Some(message) = hf_http_status_error(response.status()) {
            return Err(message.to_string().into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "HuggingFace download failed with HTTP {}",
                response.status()
            )
            .into());
        }
        if let (Some(expected), Some(received)) = (
            planned.expected_size.filter(|size| *size > 0),
            response.content_length(),
        ) {
            if expected != received {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: format!(
                        "HuggingFace file size changed for '{}'; refresh the model plan",
                        planned.path
                    ),
                });
            }
        }
        let file_total = planned
            .expected_size
            .filter(|size| *size > 0)
            .or_else(|| response.content_length())
            .unwrap_or(0);
        if file_total > MAX_HF_FILE_BYTES {
            return Err(
                format!("HuggingFace file exceeds the {MAX_HF_FILE_BYTES}-byte limit").into(),
            );
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut output = options
            .open(&destination)
            .map_err(|error| format!("Failed to create staged model file: {error}"))?;
        let mut hasher = Sha256::new();
        let mut file_downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        let mut last_emit_time = std::time::Instant::now();
        let mut last_overall_percentage = 0.0_f64;
        loop {
            let next_chunk = tokio::select! {
                _ = cancel.notified() => {
                    return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                        message: "HuggingFace download cancelled".to_string(),
                    });
                }
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = next_chunk else {
                break;
            };
            let chunk = chunk.map_err(|error| {
                crate::rig_lib::http::transport_error("HuggingFace download stream failed", error)
            })?;
            let chunk_size = u64::try_from(chunk.len())
                .map_err(|_| "HuggingFace download chunk size overflow".to_string())?;
            file_downloaded = file_downloaded
                .checked_add(chunk_size)
                .ok_or_else(|| "HuggingFace file size overflow".to_string())?;
            grand_downloaded = grand_downloaded
                .checked_add(chunk_size)
                .ok_or_else(|| "HuggingFace download size overflow".to_string())?;
            if file_downloaded > MAX_HF_FILE_BYTES || grand_downloaded > MAX_HF_DOWNLOAD_BYTES {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "HuggingFace download exceeded its size limit".to_string(),
                });
            }
            hasher.update(&chunk);
            output
                .write_all(&chunk)
                .map_err(|error| format!("Failed to write staged model file: {error}"))?;

            let overall_percentage = if grand_total > 0 {
                ((grand_downloaded as f64 / grand_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let file_percentage = if file_total > 0 {
                ((file_downloaded as f64 / file_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let now = std::time::Instant::now();
            if overall_percentage - last_overall_percentage >= 0.1
                || now.duration_since(last_emit_time).as_millis() > 150
            {
                last_overall_percentage = overall_percentage;
                last_emit_time = now;
                staging_guard.heartbeat();
                emit_progress(serde_json::json!({
                    "filename": download_id,
                    "download_id": download_id,
                    "repo_id": repo_id,
                    "status": "downloading",
                    "total": grand_total,
                    "downloaded": grand_downloaded,
                    "percentage": overall_percentage,
                    "current_file": planned.path,
                    "file_index": file_index,
                    "file_count": files.len(),
                    "file_percentage": file_percentage,
                }));
            }
        }
        if file_total > 0 && file_downloaded != file_total {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download length did not match the pinned plan".to_string(),
            });
        }
        if let Some(expected_hash) = &planned.sha256 {
            let actual_hash = hex::encode(hasher.finalize());
            if !actual_hash.eq_ignore_ascii_case(expected_hash) {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: format!(
                        "HuggingFace checksum verification failed for '{}'",
                        planned.path
                    ),
                });
            }
        }
        output
            .sync_all()
            .map_err(|error| format!("Failed to sync staged model file: {error}"))?;
        staging_guard.heartbeat();
        downloaded_paths.push(planned.path.clone());
    }

    validate_staged_hf_artifact(&staging_dir, manifest)?;
    crate::model_manager::write_managed_model_manifest(&staging_dir, manifest)?;
    staging_guard.prepare_publish()?;

    thinclaw_platform::rename_no_replace(&staging_dir, &destination_dir)
        .map_err(|error| format!("Failed to publish downloaded model: {error}"))?;
    staging_guard.committed = true;
    #[cfg(unix)]
    sync_published_hf_category(&category_dir);

    let total_bytes = grand_downloaded;
    emit_progress(serde_json::json!({
        "filename": download_id,
        "download_id": download_id,
        "repo_id": repo_id,
        "status": "completed",
        "total": total_bytes,
        "downloaded": total_bytes,
        "percentage": 100.0,
        "current_file": "",
        "file_index": files.len(),
        "file_count": files.len(),
    }));
    Ok(InternalDownloadResult {
        destination_dir,
        downloaded_files: downloaded_paths,
        total_bytes,
    })
}

/// Download one backend-produced, revision-pinned artifact selection.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_download_hf_selection(
    app: AppHandle,
    state: State<'_, crate::model_manager::DownloadManager>,
    request: HfDownloadSelectionRequest,
) -> Result<HfDownloadResult, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&request.repo_id)?;
    validate_hf_revision(&request.revision, false)?;
    if request.artifact_id.is_empty()
        || request.artifact_id.len() > 512
        || request.artifact_id.chars().any(char::is_control)
        || request
            .companion_artifact_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace artifact selection is invalid".to_string(),
        });
    }
    let expected_download_id =
        artifact_download_id(&request.repo_id, &request.revision, &request.artifact_id);
    let (cancel_notify, _download_guard) = state.register(&expected_download_id)?;
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    let profile = capability_profile(&engine.id, request.task).ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::Runtime {
            message: format!(
                "{} does not support HuggingFace downloads for {:?}",
                engine.display_name, request.task
            ),
        }
    })?;
    let plan = match cancellable_hf_operation(
        &cancel_notify,
        build_model_file_plan(
            &app,
            &request.repo_id,
            &engine.id,
            request.task,
            Some(&request.revision),
        ),
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => {
            emit_hf_download_terminal(
                &app,
                &expected_download_id,
                &request.repo_id,
                if error == HF_DOWNLOAD_CANCELLED {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(&error),
            );
            return Err(error.into());
        }
    };
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == request.artifact_id)
        .cloned()
        .ok_or_else(|| crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The selected HuggingFace artifact is not in the pinned model plan"
                .to_string(),
        })?;
    if artifact.download_id != expected_download_id || plan.revision != request.revision {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The pinned HuggingFace artifact identity changed; refresh the model plan"
                .to_string(),
        });
    }
    let companion = match request.companion_artifact_id.as_deref() {
        Some(companion_id) => Some(
            plan.companion_artifacts
                .iter()
                .find(|artifact| artifact.id == companion_id)
                .cloned()
                .ok_or_else(|| crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "The selected mmproj artifact is not in the pinned model plan"
                        .to_string(),
                })?,
        ),
        None => None,
    };
    if task_requires_mmproj(&engine.id, request.task) && companion.is_none() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "llama.cpp vision downloads require a selected mmproj artifact".to_string(),
        });
    }
    if companion.is_some() && artifact.layout != HfArtifactLayout::GgufVariants {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Directory model artifacts cannot have an mmproj companion".to_string(),
        });
    }

    let mut selected_files = artifact.files.clone();
    if let Some(companion) = &companion {
        selected_files.extend(companion.files.clone());
    }
    let combined_size = selected_files
        .iter()
        .try_fold(0_u64, |total, file| total.checked_add(file.size))
        .ok_or_else(|| "HuggingFace selected artifact size overflow".to_string())?;
    if combined_size > MAX_HF_DOWNLOAD_BYTES {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The selected model and companion exceed the download size limit".to_string(),
        });
    }
    let destination_name = request.destination_name.clone().unwrap_or_else(|| {
        default_destination_name(
            &request.repo_id,
            &plan.revision,
            request.task,
            &artifact,
            companion.as_ref().map(|artifact| artifact.id.as_str()),
        )
    });
    validate_relative_subdir(&destination_name)?;
    let download_id = artifact.download_id.clone();
    let _active_download = ActiveHfDownloadGuard::acquire(&download_id)?;
    let primary_path = artifact.primary_file.clone();
    let artifact_kind = match artifact.layout {
        HfArtifactLayout::Directory => "directory",
        HfArtifactLayout::GgufVariants if artifact.files.len() > 1 => "gguf_sharded",
        HfArtifactLayout::GgufVariants => "gguf_single",
    };
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: format!(
            "hf-{}",
            selection_identity(
                &request.repo_id,
                &plan.revision,
                request.task,
                &artifact.id,
                companion.as_ref().map(|artifact| artifact.id.as_str()),
            )
            .simple()
        ),
        source: "huggingface".to_string(),
        repo_id: Some(request.repo_id.clone()),
        revision: Some(plan.revision.clone()),
        category: profile.category.to_string(),
        task: Some(request.task.as_str().to_string()),
        runtime: engine.id.clone(),
        format: profile.format_tag.to_string(),
        artifact_kind: artifact_kind.to_string(),
        artifact_id: Some(artifact.id.clone()),
        companion_artifact_id: companion.as_ref().map(|artifact| artifact.id.clone()),
        companion_path: companion
            .as_ref()
            .and_then(|artifact| artifact.primary_file.clone()),
        primary_path: primary_path.clone(),
        files: selected_files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        quantization: artifact.quant_type.clone(),
    };
    crate::model_manager::validate_managed_model_manifest(&manifest)?;
    let planned_files: Vec<PlannedDownloadFile> = selected_files
        .iter()
        .map(|file| PlannedDownloadFile {
            path: file.path.clone(),
            expected_size: Some(file.size),
            sha256: file.sha256.clone(),
        })
        .collect();
    let result = download_planned_hf_files(
        &app,
        &request.repo_id,
        &plan.revision,
        &planned_files,
        &destination_name,
        profile.category,
        &download_id,
        &cancel_notify,
        &manifest,
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let message = error.to_string();
            emit_hf_download_terminal(
                &app,
                &download_id,
                &request.repo_id,
                if message == HF_DOWNLOAD_CANCELLED {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(&message),
            );
            return Err(error);
        }
    };
    let model_path = primary_path.as_deref().map_or_else(
        || result.destination_dir.clone(),
        |primary| result.destination_dir.join(primary),
    );
    let companion_path = companion
        .as_ref()
        .and_then(|artifact| artifact.primary_file.as_deref())
        .map(|relative| {
            result
                .destination_dir
                .join(relative)
                .to_string_lossy()
                .to_string()
        });
    let downloaded_files = result
        .downloaded_files
        .iter()
        .map(|path| {
            result
                .destination_dir
                .join(path)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    Ok(HfDownloadResult {
        download_id,
        repo_id: request.repo_id,
        revision: plan.revision,
        engine_id: engine.id,
        task: request.task,
        category: profile.category.to_string(),
        artifact_id: artifact.id,
        companion_artifact_id: companion.map(|artifact| artifact.id),
        destination_dir: result.destination_dir.to_string_lossy().to_string(),
        model_path: model_path.to_string_lossy().to_string(),
        companion_path,
        downloaded_files,
        total_bytes: result.total_bytes,
    })
}

/// Discover the embedding dimension of a HuggingFace model by fetching its
/// `config.json` from the API and extracting `hidden_size`, `d_model`, or
/// `embedding_dim`.
///
/// Returns `None` for GGUF single-file models or repos without a `config.json`.
/// This is used by the onboarding wizard to pre-configure the vector store
/// dimension *before* the embedding server starts, avoiding a wasteful
/// create-then-destroy cycle on first boot.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_discover_embedding_dimension(
    app: AppHandle,
    repo_id: String,
    revision: String,
) -> Result<Option<u32>, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&repo_id)?;
    validate_hf_revision(&revision, false)?;
    let client = build_hf_client(&app).await?;

    // Fetch config.json from the same immutable revision as the file plan.
    let url = hf_url(&repo_id, &["raw", &revision], Some("config.json"))?;

    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(error) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: crate::rig_lib::http::transport_error(
                    "HuggingFace embedding config request failed",
                    error,
                ),
            });
        }
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace embedding config").await?;

    let config: serde_json::Value =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_CONFIG_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace model config: {error}"))?;

    // Try the common keys in priority order:
    //   hidden_size — most common (BERT, Nomic, BGE, GTE, etc.)
    //   d_model     — used by some sentence-transformers
    //   embedding_dim — occasionally used
    let dim = config
        .get("hidden_size")
        .or_else(|| config.get("d_model"))
        .or_else(|| config.get("embedding_dim"))
        .and_then(|v| v.as_u64())
        .and_then(|dimension| u32::try_from(dimension).ok())
        .filter(|dimension| (1..=1_000_000).contains(dimension));

    Ok(dim)
}

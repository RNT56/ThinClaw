use super::*;

// Standard Assets Logic

#[derive(Serialize, Clone, Type)]
pub struct StandardAsset {
    pub(super) name: String,
    pub(super) category: String, // "vae", "t5", "clip", "other"
    pub(super) filename: String,
    pub(super) url: String,
    #[specta(type = f64)]
    pub(super) size: u64,
    #[serde(skip_serializing)]
    #[specta(skip)]
    pub(super) sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StandardAssetFileStamp {
    pub(super) size: u64,
    pub(super) modified_secs: u64,
    pub(super) modified_nanos: u32,
    pub(super) file_identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StandardAssetVerificationRecord {
    pub(super) pinned_size: u64,
    pub(super) pinned_sha256: String,
    pub(super) stamp: StandardAssetFileStamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StandardAssetVerificationManifest {
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) entries: BTreeMap<String, StandardAssetVerificationRecord>,
}

impl Default for StandardAssetVerificationManifest {
    fn default() -> Self {
        Self {
            schema_version: STANDARD_ASSET_VERIFICATION_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

pub(super) fn standard_asset_verification_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn standard_asset_cache_key(asset: &StandardAsset) -> String {
    format!("{}/{}", asset.category, asset.filename)
}

pub(super) fn standard_asset_stamp(metadata: &fs::Metadata) -> Result<StandardAssetFileStamp, String> {
    let modified = metadata
        .modified()
        .map_err(|error| format!("Could not inspect standard model asset timestamp: {error}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Standard model asset has an invalid timestamp".to_string())?;
    #[cfg(unix)]
    let file_identity = {
        use std::os::unix::fs::MetadataExt as _;
        Some(format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    };
    #[cfg(not(unix))]
    let file_identity = None;
    Ok(StandardAssetFileStamp {
        size: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
        file_identity,
    })
}

pub(super) fn inspect_existing_standard_asset(
    path: &Path,
    asset: &StandardAsset,
) -> Result<StandardAssetFileStamp, String> {
    let file = thinclaw_platform::fs::open_regular_file_nofollow(path)
        .map_err(|error| format!("Could not open standard model asset: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect standard model asset: {error}"))?;
    if metadata.len() != asset.size {
        return Err("Standard model asset size does not match its pinned metadata".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err("Standard model asset has multiple hard links".to_string());
        }
    }
    standard_asset_stamp(&metadata)
}

pub(super) fn load_standard_asset_verification_manifest(path: &Path) -> StandardAssetVerificationManifest {
    let Ok(bytes) = thinclaw_platform::read_regular_file_bounded_single_link(
        path,
        MAX_STANDARD_ASSET_VERIFICATION_BYTES,
    ) else {
        return StandardAssetVerificationManifest::default();
    };
    let Ok(manifest) = serde_json::from_slice::<StandardAssetVerificationManifest>(&bytes) else {
        return StandardAssetVerificationManifest::default();
    };
    if manifest.schema_version != STANDARD_ASSET_VERIFICATION_SCHEMA_VERSION {
        return StandardAssetVerificationManifest::default();
    }
    manifest
}

pub(super) fn record_standard_asset_verification(
    verification_path: &Path,
    asset: &StandardAsset,
    stamp: StandardAssetFileStamp,
) -> Result<(), String> {
    let _guard = standard_asset_verification_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut manifest = load_standard_asset_verification_manifest(verification_path);
    manifest.entries.insert(
        standard_asset_cache_key(asset),
        StandardAssetVerificationRecord {
            pinned_size: asset.size,
            pinned_sha256: asset.sha256.clone(),
            stamp,
        },
    );
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize standard asset verification: {error}"))?;
    if bytes.len() as u64 > MAX_STANDARD_ASSET_VERIFICATION_BYTES {
        return Err("Standard asset verification manifest is too large".to_string());
    }
    thinclaw_platform::write_private_file_atomic(verification_path, &bytes, true)
        .map_err(|error| format!("Could not save standard asset verification: {error}"))
}

pub fn get_standard_assets() -> Vec<StandardAsset> {
    vec![
        StandardAsset {
            name: "VAE (ft-mse-840000)".into(),
            category: "vae".into(),
            filename: "vae-ft-mse-840000-ema-pruned.safetensors".into(),
            url: "https://huggingface.co/stabilityai/sd-vae-ft-mse-original/resolve/629b3ad3030ce36e15e70c5db7d91df0d60c627f/vae-ft-mse-840000-ema-pruned.safetensors".into(),
            size: 334_641_190,
            sha256: "735e4c3a447a3255760d7f86845f09f937809baa529c17370d83e4c3758f3c75".into(),
        },
        StandardAsset {
            name: "T5XXL (FP16)".into(),
            category: "t5".into(),
            filename: "t5xxl_fp16.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/t5xxl_fp16.safetensors".into(),
            size: 9_787_841_024,
            sha256: "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635".into(),
        },
        StandardAsset {
            name: "CLIP L".into(),
            category: "clip".into(),
            filename: "clip_l.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/clip_l.safetensors".into(),
            size: 246_144_152,
            sha256: "660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd".into(),
        },
        StandardAsset {
            name: "CLIP G".into(),
            category: "clip".into(),
            filename: "clip_g.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/clip_g.safetensors".into(),
            size: 1_389_382_176,
            sha256: "ec310df2af79c318e24d20511b601a591ca8cd4f1fce1d8dff822a356bcdb1f4".into(),
        },
        StandardAsset {
            name: "Scheduler Config".into(),
            category: "other".into(),
            filename: "scheduler_config.json".into(),
            url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-v1-5/resolve/451f4fe16113bff5a5d2269ed5ad43b0592e9a14/scheduler/scheduler_config.json".into(),
            size: 308,
            sha256: "699cce92eb7c122e2eb7dfdea78e6187fda76a5ed4a8e42319b85610e620e091".into(),
        }
    ]
}

pub(super) fn validate_existing_standard_asset(
    path: &Path,
    asset: &StandardAsset,
) -> Result<StandardAssetFileStamp, String> {
    use sha2::{Digest as _, Sha256};
    use std::io::Read as _;

    let mut file = thinclaw_platform::fs::open_regular_file_nofollow(path)
        .map_err(|error| format!("Could not open standard model asset: {error}"))?;
    let metadata_before = file
        .metadata()
        .map_err(|error| format!("Could not inspect standard model asset: {error}"))?;
    if metadata_before.len() != asset.size {
        return Err("Standard model asset size does not match its pinned metadata".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata_before.nlink() != 1 {
            return Err("Standard model asset has multiple hard links".to_string());
        }
    }
    let stamp_before = standard_asset_stamp(&metadata_before)?;

    let mut hasher = Sha256::new();
    let mut read_total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read standard model asset: {error}"))?;
        if read == 0 {
            break;
        }
        read_total = read_total
            .checked_add(read as u64)
            .ok_or_else(|| "Standard model asset size overflow".to_string())?;
        hasher.update(&buffer[..read]);
    }
    validate_expected_download_integrity(
        read_total,
        asset.size,
        &hex::encode(hasher.finalize()),
        &asset.sha256,
    )?;
    let metadata_after = file
        .metadata()
        .map_err(|error| format!("Could not re-inspect standard model asset: {error}"))?;
    let stamp_after = standard_asset_stamp(&metadata_after)?;
    if stamp_before != stamp_after {
        return Err("Standard model asset changed while it was verified".to_string());
    }
    validate_downloaded_file(path, path, asset.size)?;
    Ok(stamp_after)
}

pub(super) fn validate_existing_standard_asset_cached(
    path: &Path,
    asset: &StandardAsset,
    verification_path: &Path,
) -> Result<(), String> {
    let stamp = inspect_existing_standard_asset(path, asset)?;
    let cached = {
        let _guard = standard_asset_verification_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        load_standard_asset_verification_manifest(verification_path)
            .entries
            .get(&standard_asset_cache_key(asset))
            .cloned()
    };
    if cached.is_some_and(|record| {
        record.pinned_size == asset.size
            && record.pinned_sha256.eq_ignore_ascii_case(&asset.sha256)
            && record.stamp == stamp
    }) {
        validate_downloaded_file(path, path, asset.size)?;
        return Ok(());
    }

    let verified_stamp = validate_existing_standard_asset(path, asset)?;
    if let Err(error) = record_standard_asset_verification(verification_path, asset, verified_stamp)
    {
        tracing::warn!("Could not cache standard asset verification: {error}");
    }
    Ok(())
}

pub(super) async fn existing_standard_asset_is_valid(
    path: PathBuf,
    asset: StandardAsset,
    verification_path: PathBuf,
) -> bool {
    tokio::task::spawn_blocking(move || {
        validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok()
    })
    .await
    .unwrap_or(false)
}

pub(super) async fn cache_downloaded_standard_asset(
    path: PathBuf,
    asset: StandardAsset,
    verification_path: PathBuf,
) {
    let result = tokio::task::spawn_blocking(move || {
        let stamp = inspect_existing_standard_asset(&path, &asset)?;
        record_standard_asset_verification(&verification_path, &asset, stamp)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!("Could not cache standard asset verification: {error}"),
        Err(error) => tracing::warn!("Standard asset verification cache task failed: {error}"),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_missing_standard_assets(
    app: AppHandle,
) -> Result<Vec<StandardAsset>, crate::thinclaw::bridge::BridgeError> {
    let models = managed_models_dir(&app)?;
    let diffusion = models.join("Diffusion");
    ensure_real_directory(&diffusion)?;
    let standard_dir = diffusion.join("standard");
    ensure_real_directory(&standard_dir)?;
    let verification_path = standard_dir.join(STANDARD_ASSET_VERIFICATION_FILENAME);

    let mut missing = Vec::new();
    let assets = get_standard_assets();

    for asset in assets {
        let category_dir = standard_dir.join(&asset.category);
        ensure_real_directory(&category_dir)?;
        let file_path = category_dir.join(&asset.filename);
        if !existing_standard_asset_is_valid(file_path, asset.clone(), verification_path.clone())
            .await
        {
            missing.push(asset);
        }
    }

    Ok(missing)
}

#[tauri::command]
#[specta::specta]
pub async fn download_standard_asset(
    app: AppHandle,
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    // Find asset
    let assets = get_standard_assets();
    let asset = assets
        .iter()
        .find(|a| a.filename == filename)
        .ok_or("Asset not found in standard list")?;

    let models = managed_models_dir(&app)?;
    let diffusion = models.join("Diffusion");
    ensure_real_directory(&diffusion)?;
    let standard = diffusion.join("standard");
    ensure_real_directory(&standard)?;
    let verification_path = standard.join(STANDARD_ASSET_VERIFICATION_FILENAME);
    let target_dir = standard.join(&asset.category);
    ensure_real_directory(&target_dir)?;

    // Check if exists
    let target_path = target_dir.join(&filename);
    match fs::symlink_metadata(&target_path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            if existing_standard_asset_is_valid(
                target_path.clone(),
                asset.clone(),
                verification_path.clone(),
            )
            .await
            {
                return Ok(target_path.to_string_lossy().to_string());
            }
            fs::remove_file(&target_path).map_err(|error| {
                format!("Could not remove invalid standard model asset: {error}")
            })?;
        }
        Ok(_) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "Standard model asset path is unsafe".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect standard model asset: {error}").into()),
    }
    let url = validate_model_download_url(&asset.url)?;
    let (notify, _download_guard) = state.register(&filename)?;
    download_model_file(
        &app,
        url,
        &target_path,
        &filename,
        notify,
        asset.size,
        &asset.sha256,
    )
    .await?;
    cache_downloaded_standard_asset(target_path.clone(), asset.clone(), verification_path).await;
    Ok(target_path.to_string_lossy().to_string())
}
#[tauri::command]
#[specta::specta]
pub async fn get_model_metadata(
    app: AppHandle,
    path: String,
) -> Result<crate::gguf::GGUFMetadata, crate::thinclaw::bridge::BridgeError> {
    if path.is_empty() || path.len() > 8_192 || path.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path is invalid".to_string(),
        });
    }
    let models = managed_models_dir(&app)?;
    let path = PathBuf::from(path);
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| "Managed model file was not found".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("gguf"))
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path is not a regular GGUF file".to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "Model metadata file must not have multiple hard links".to_string(),
            });
        }
    }
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model file: {error}"))?;
    if !resolved.starts_with(models) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path escaped managed model storage".to_string(),
        });
    }
    Ok(crate::gguf::read_gguf_metadata(
        resolved
            .to_str()
            .ok_or_else(|| "Model metadata path is not valid Unicode".to_string())?,
    )?)
}

#[derive(Serialize, Deserialize, Clone, Type)]
pub struct RemoteModelEntry {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) metadata: serde_json::Value,
    pub(super) local_version: Option<String>,
    pub(super) remote_version: Option<String>,
    #[specta(type = Option<f64>)]
    pub(super) last_checked_at: Option<i64>,
    pub(super) status: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn update_remote_model_catalog(
    pool: State<'_, sqlx::SqlitePool>,
    entries: Vec<RemoteModelEntry>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    for entry in entries {
        let metadata_json = entry.metadata.to_string();
        sqlx::query(
            "INSERT INTO models_catalog (id, name, metadata, local_version, remote_version, last_checked_at, status)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                metadata = excluded.metadata,
                remote_version = excluded.remote_version,
                last_checked_at = excluded.last_checked_at,
                status = excluded.status",
        )
        .bind(&entry.id)
        .bind(&entry.name)
        .bind(&metadata_json)
        .bind(&entry.local_version)
        .bind(&entry.remote_version)
        .bind(entry.last_checked_at)
        .bind(&entry.status)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_remote_model_catalog(
    pool: State<'_, sqlx::SqlitePool>,
) -> Result<Vec<RemoteModelEntry>, crate::thinclaw::bridge::BridgeError> {
    let rows = sqlx::query("SELECT id, name, metadata, local_version, remote_version, last_checked_at, status FROM models_catalog")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    let entries = rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            RemoteModelEntry {
                id: row.try_get("id").unwrap_or_default(),
                name: row.try_get("name").unwrap_or_default(),
                metadata: serde_json::from_str(
                    &row.try_get::<String, _>("metadata")
                        .unwrap_or_else(|_| "{}".to_string()),
                )
                .unwrap_or_default(),
                local_version: row.try_get("local_version").ok(),
                remote_version: row.try_get("remote_version").ok(),
                last_checked_at: row.try_get("last_checked_at").ok(),
                status: row.try_get("status").ok(),
            }
        })
        .collect();

    Ok(entries)
}

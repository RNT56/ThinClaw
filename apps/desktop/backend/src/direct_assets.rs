use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use std::collections::{HashMap, HashSet};
use thinclaw_runtime_contracts::{
    AssetKind, AssetNamespace, AssetOrigin, AssetRecord, AssetRef, AssetStatus, AssetVisibility,
    DirectAttachedDocument,
};

#[derive(Debug, Clone)]
pub struct NewDirectAsset {
    pub id: String,
    pub kind: AssetKind,
    pub origin: AssetOrigin,
    pub path: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub prompt: Option<String>,
    pub provider: Option<String>,
    pub style_id: Option<String>,
    pub aspect_ratio: Option<String>,
    pub resolution: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seed: Option<i64>,
    pub thumbnail_path: Option<String>,
    pub is_favorite: bool,
    pub tags: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, FromRow)]
struct DirectAssetRow {
    id: String,
    namespace: String,
    kind: String,
    origin: String,
    status: String,
    visibility: String,
    path: String,
    mime_type: Option<String>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    prompt: Option<String>,
    provider: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
}

pub struct DirectAssetStore;

impl DirectAssetStore {
    pub fn direct_ref(id: impl Into<String>) -> AssetRef {
        AssetRef {
            namespace: AssetNamespace::DirectWorkbench,
            id: id.into(),
        }
    }

    pub fn refs_for_message(
        explicit_assets: Option<Vec<AssetRef>>,
        images: Option<&[String]>,
        attached_docs: Option<&[DirectAttachedDocument]>,
    ) -> Option<Vec<AssetRef>> {
        let mut refs = Vec::new();
        let mut seen = HashSet::new();
        for asset in explicit_assets.unwrap_or_default() {
            if seen.insert((asset.namespace, asset.id.clone())) {
                refs.push(asset);
            }
        }

        if let Some(image_ids) = images {
            for image_id in image_ids {
                let reference = Self::direct_ref(image_id.clone());
                if seen.insert((reference.namespace, reference.id.clone())) {
                    refs.push(reference);
                }
            }
        }

        if let Some(docs) = attached_docs {
            for doc in docs {
                let reference = doc
                    .asset_ref
                    .clone()
                    .unwrap_or_else(|| Self::direct_ref(doc.id.clone()));
                if seen.insert((reference.namespace, reference.id.clone())) {
                    refs.push(reference);
                }
            }
        }

        if refs.is_empty() {
            None
        } else {
            Some(refs)
        }
    }

    pub async fn upsert(pool: &SqlitePool, input: NewDirectAsset) -> Result<AssetRecord, String> {
        if input.id.is_empty() || input.id.len() > 128 || input.id.chars().any(char::is_control) {
            return Err("Direct asset identifier is invalid".to_string());
        }
        if input.path.is_empty() || input.path.len() > 4096 || input.path.contains('\0') {
            return Err("Direct asset path is invalid".to_string());
        }
        if input.size_bytes.is_some_and(|size| size > i64::MAX as u64) {
            return Err("Direct asset size exceeds the database limit".to_string());
        }
        if input.sha256.as_ref().is_some_and(|digest| {
            digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("Direct asset SHA-256 digest is invalid".to_string());
        }
        for (label, value, max_bytes) in [
            ("MIME type", input.mime_type.as_deref(), 256_usize),
            ("provider", input.provider.as_deref(), 256),
            ("style", input.style_id.as_deref(), 512),
            ("aspect ratio", input.aspect_ratio.as_deref(), 64),
            ("resolution", input.resolution.as_deref(), 64),
            ("thumbnail path", input.thumbnail_path.as_deref(), 4096),
            ("tags", input.tags.as_deref(), 16 * 1024),
        ] {
            if value.is_some_and(|value| value.len() > max_bytes || value.contains('\0')) {
                return Err(format!("Direct asset {label} is invalid"));
            }
        }
        if input
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.len() > 4 * 1024 * 1024 || prompt.contains('\0'))
        {
            return Err(
                "Direct asset prompt exceeds the supported limit or contains NUL".to_string(),
            );
        }
        if input.metadata.len() > 256
            || input.metadata.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 256
                    || key.contains('\0')
                    || value.len() > 64 * 1024
                    || value.contains('\0')
            })
        {
            return Err("Direct asset metadata is invalid".to_string());
        }
        let now = Utc::now();
        let now_string = now.to_rfc3339();
        let metadata_json = if input.metadata.is_empty() {
            None
        } else {
            let encoded = serde_json::to_string(&input.metadata).map_err(|e| e.to_string())?;
            if encoded.len() > 1024 * 1024 {
                return Err("Direct asset metadata exceeds the supported size".to_string());
            }
            Some(encoded)
        };

        sqlx::query(
            r#"
            INSERT INTO direct_assets (
                id, namespace, kind, origin, status, visibility, path,
                mime_type, size_bytes, sha256, prompt, provider, style_id,
                aspect_ratio, resolution, width, height, seed, thumbnail_path,
                is_favorite, tags, metadata, created_at, updated_at
            ) VALUES (?, 'direct_workbench', ?, ?, 'ready', 'private', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace,
                kind = excluded.kind,
                origin = excluded.origin,
                status = excluded.status,
                visibility = excluded.visibility,
                path = excluded.path,
                mime_type = excluded.mime_type,
                size_bytes = excluded.size_bytes,
                sha256 = excluded.sha256,
                prompt = excluded.prompt,
                provider = excluded.provider,
                style_id = excluded.style_id,
                aspect_ratio = excluded.aspect_ratio,
                resolution = excluded.resolution,
                width = excluded.width,
                height = excluded.height,
                seed = excluded.seed,
                thumbnail_path = excluded.thumbnail_path,
                is_favorite = excluded.is_favorite,
                tags = excluded.tags,
                metadata = excluded.metadata,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&input.id)
        .bind(kind_key(input.kind))
        .bind(origin_key(input.origin))
        .bind(&input.path)
        .bind(&input.mime_type)
        .bind(input.size_bytes.and_then(|value| i64::try_from(value).ok()))
        .bind(&input.sha256)
        .bind(&input.prompt)
        .bind(&input.provider)
        .bind(&input.style_id)
        .bind(&input.aspect_ratio)
        .bind(&input.resolution)
        .bind(input.width.map(|v| v as i64))
        .bind(input.height.map(|v| v as i64))
        .bind(input.seed)
        .bind(&input.thumbnail_path)
        .bind(if input.is_favorite { 1_i64 } else { 0_i64 })
        .bind(&input.tags)
        .bind(&metadata_json)
        .bind(&now_string)
        .bind(&now_string)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to save Direct asset metadata: {}", e))?;

        Ok(AssetRecord {
            reference: Self::direct_ref(input.id),
            kind: input.kind,
            origin: input.origin,
            status: AssetStatus::Ready,
            visibility: AssetVisibility::Private,
            path: input.path,
            mime_type: input.mime_type,
            size_bytes: input.size_bytes,
            sha256: input.sha256,
            prompt: input.prompt,
            provider: input.provider,
            width: input.width,
            height: input.height,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get(pool: &SqlitePool, reference: &AssetRef) -> Result<AssetRecord, String> {
        if reference.namespace != AssetNamespace::DirectWorkbench {
            return Err("Direct asset lookup only supports direct_workbench assets".to_string());
        }

        let row = sqlx::query_as::<_, DirectAssetRow>(
            r#"
            SELECT id, namespace, kind, origin, status, visibility, path,
                   mime_type, size_bytes, sha256, prompt, provider, width, height,
                   metadata, created_at, updated_at
            FROM direct_assets
            WHERE id = ? AND namespace = 'direct_workbench' AND status != 'deleted'
            "#,
        )
        .bind(&reference.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to load Direct asset metadata: {}", e))?
        .ok_or_else(|| format!("Direct asset not found: {}", reference.id))?;

        row_to_record(row)
    }

    pub async fn path_for(pool: &SqlitePool, reference: &AssetRef) -> Result<String, String> {
        Self::get(pool, reference).await.map(|record| record.path)
    }
}

fn row_to_record(row: DirectAssetRow) -> Result<AssetRecord, String> {
    let metadata = match row.metadata.as_deref() {
        Some(metadata) => serde_json::from_str(metadata)
            .map_err(|error| format!("Direct asset metadata is corrupt: {error}"))?,
        None => HashMap::new(),
    };
    Ok(AssetRecord {
        reference: AssetRef {
            namespace: parse_namespace(&row.namespace)?,
            id: row.id,
        },
        kind: parse_kind(&row.kind)?,
        origin: parse_origin(&row.origin)?,
        status: parse_status(&row.status)?,
        visibility: parse_visibility(&row.visibility)?,
        path: row.path,
        mime_type: row.mime_type,
        size_bytes: row.size_bytes.and_then(|v| u64::try_from(v).ok()),
        sha256: row.sha256,
        prompt: row.prompt,
        provider: row.provider,
        width: row.width.and_then(|v| u32::try_from(v).ok()),
        height: row.height.and_then(|v| u32::try_from(v).ok()),
        metadata,
        created_at: parse_datetime(&row.created_at)?,
        updated_at: parse_datetime(&row.updated_at)?,
    })
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|error| format!("Direct asset timestamp is corrupt: {error}"))
}

fn kind_key(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "image",
        AssetKind::Audio => "audio",
        AssetKind::Video => "video",
        AssetKind::Document => "document",
        AssetKind::GeneratedImage => "generated_image",
        AssetKind::Other => "other",
    }
}

fn origin_key(origin: AssetOrigin) -> &'static str {
    match origin {
        AssetOrigin::Upload => "upload",
        AssetOrigin::Generated => "generated",
        AssetOrigin::DownloadedModelOutput => "downloaded_model_output",
        AssetOrigin::VoiceInput => "voice_input",
        AssetOrigin::VoiceOutput => "voice_output",
        AssetOrigin::RagDocument => "rag_document",
    }
}

fn parse_namespace(value: &str) -> Result<AssetNamespace, String> {
    match value {
        "direct_workbench" => Ok(AssetNamespace::DirectWorkbench),
        "thinclaw_agent" => Ok(AssetNamespace::ThinClawAgent),
        _ => Err(format!("Unknown direct asset namespace: {value}")),
    }
}

fn parse_kind(value: &str) -> Result<AssetKind, String> {
    match value {
        "image" => Ok(AssetKind::Image),
        "audio" => Ok(AssetKind::Audio),
        "video" => Ok(AssetKind::Video),
        "document" => Ok(AssetKind::Document),
        "generated_image" => Ok(AssetKind::GeneratedImage),
        "other" => Ok(AssetKind::Other),
        _ => Err(format!("Unknown direct asset kind: {value}")),
    }
}

fn parse_origin(value: &str) -> Result<AssetOrigin, String> {
    match value {
        "upload" => Ok(AssetOrigin::Upload),
        "generated" => Ok(AssetOrigin::Generated),
        "downloaded_model_output" => Ok(AssetOrigin::DownloadedModelOutput),
        "voice_input" => Ok(AssetOrigin::VoiceInput),
        "voice_output" => Ok(AssetOrigin::VoiceOutput),
        "rag_document" => Ok(AssetOrigin::RagDocument),
        _ => Err(format!("Unknown direct asset origin: {value}")),
    }
}

fn parse_status(value: &str) -> Result<AssetStatus, String> {
    match value {
        "ready" => Ok(AssetStatus::Ready),
        "pending" => Ok(AssetStatus::Pending),
        "deleted" => Ok(AssetStatus::Deleted),
        "error" => Ok(AssetStatus::Error),
        _ => Err(format!("Unknown direct asset status: {value}")),
    }
}

fn parse_visibility(value: &str) -> Result<AssetVisibility, String> {
    match value {
        "private" => Ok(AssetVisibility::Private),
        "shared_by_explicit_handoff" => Ok(AssetVisibility::SharedByExplicitHandoff),
        _ => Err(format!("Unknown direct asset visibility: {value}")),
    }
}

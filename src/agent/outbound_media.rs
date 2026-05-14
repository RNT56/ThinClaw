use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{LazyLock, Mutex};

use base64::Engine;
use sha2::{Digest, Sha256};
use thinclaw_media::{MediaContent, MediaLimits, MediaType};
use thinclaw_tools_core::ToolArtifact;

const GENERATED_MEDIA_TOOLS: &[&str] = &["image_generate", "comfy_run_workflow"];

#[cfg(test)]
static TEST_GENERATED_ROOTS: LazyLock<Mutex<Vec<PathBuf>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

pub(crate) async fn attachments_from_tool_result(
    tool_name: &str,
    result_json: &serde_json::Value,
    artifacts: &[ToolArtifact],
) -> Vec<MediaContent> {
    if !GENERATED_MEDIA_TOOLS.contains(&tool_name) {
        return Vec::new();
    }

    let mut attachments = attachments_from_generation_outputs(result_json).await;
    if attachments.is_empty() {
        attachments.extend(attachments_from_artifacts(artifacts));
    }
    dedupe_attachments(attachments)
}

pub(crate) fn dedupe_extend(target: &mut Vec<MediaContent>, incoming: Vec<MediaContent>) {
    let mut seen = target
        .iter()
        .map(attachment_digest)
        .collect::<HashSet<String>>();
    for attachment in incoming {
        let digest = attachment_digest(&attachment);
        if seen.insert(digest) {
            target.push(attachment);
        }
    }
}

async fn attachments_from_generation_outputs(result_json: &serde_json::Value) -> Vec<MediaContent> {
    let Some(outputs) = result_json
        .get("outputs")
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    let max_bytes = MediaLimits::from_env().default_max_bytes;
    let mut attachments = Vec::new();
    for output in outputs {
        let Some(file_path) = output.get("file_path").and_then(|value| value.as_str()) else {
            continue;
        };
        match media_from_path(output, file_path, max_bytes).await {
            Ok(Some(media)) => attachments.push(media),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(path = %file_path, error = %error, "Skipping generated media output");
            }
        }
    }
    attachments
}

async fn media_from_path(
    output: &serde_json::Value,
    file_path: &str,
    max_bytes: u64,
) -> anyhow::Result<Option<MediaContent>> {
    let path = PathBuf::from(file_path);
    let canonical = tokio::fs::canonicalize(&path).await?;
    if !is_under_approved_generated_root(&canonical).await {
        anyhow::bail!(
            "generated media path is outside approved roots: {}",
            canonical.display()
        );
    }
    let metadata = tokio::fs::metadata(&canonical).await?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Ok(None);
    }

    let bytes = tokio::fs::read(&canonical).await?;
    let filename = output
        .get("filename")
        .and_then(|value| value.as_str())
        .and_then(safe_basename)
        .or_else(|| {
            canonical
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "generated-media".to_string());
    let mime_type = output
        .get("mime_type")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });
    if MediaType::from_mime(&mime_type) == MediaType::Unknown {
        anyhow::bail!("unsupported generated media MIME type: {mime_type}");
    }

    Ok(Some(
        MediaContent::new(bytes, mime_type)
            .with_filename(filename)
            .with_source_url(canonical.to_string_lossy().to_string()),
    ))
}

fn attachments_from_artifacts(artifacts: &[ToolArtifact]) -> Vec<MediaContent> {
    artifacts
        .iter()
        .enumerate()
        .filter_map(|(idx, artifact)| match artifact {
            ToolArtifact::Image { data, mime_type } => decode_artifact(
                data,
                mime_type,
                format!("generated-{}.{}", idx + 1, extension_for_mime(mime_type)),
            ),
            ToolArtifact::Audio { data, mime_type } => decode_artifact(
                data,
                mime_type,
                format!("generated-{}.{}", idx + 1, extension_for_mime(mime_type)),
            ),
            _ => None,
        })
        .collect()
}

fn decode_artifact(data: &str, mime_type: &str, filename: String) -> Option<MediaContent> {
    if MediaType::from_mime(mime_type) == MediaType::Unknown {
        return None;
    }
    let max_bytes = MediaLimits::from_env().default_max_bytes as usize;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    if bytes.len() > max_bytes {
        return None;
    }
    Some(MediaContent::new(bytes, mime_type).with_filename(filename))
}

fn dedupe_attachments(attachments: Vec<MediaContent>) -> Vec<MediaContent> {
    let mut result = Vec::new();
    dedupe_extend(&mut result, attachments);
    result
}

fn attachment_digest(attachment: &MediaContent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(&attachment.data);
    format!("{:x}", hasher.finalize())
}

fn safe_basename(value: &str) -> Option<String> {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_string)
}

async fn is_under_approved_generated_root(path: &Path) -> bool {
    for root in approved_generated_roots() {
        if let Ok(canonical_root) = tokio::fs::canonicalize(root).await
            && path.starts_with(canonical_root)
        {
            return true;
        }
    }
    false
}

fn approved_generated_roots() -> Vec<PathBuf> {
    let mut roots = vec![thinclaw_platform::resolve_data_dir("media_cache").join("generated")];

    if let Ok(extra_roots) = std::env::var("THINCLAW_GENERATED_MEDIA_ROOTS") {
        roots.extend(
            extra_roots
                .split(',')
                .map(str::trim)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
        );
    }

    #[cfg(test)]
    roots.extend(
        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .clone(),
    );

    roots
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/wave" => "wav",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        _ if mime.starts_with("image/") => "png",
        _ if mime.starts_with("audio/") => "bin",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_tools_core::ToolArtifact;

    #[tokio::test]
    async fn generated_outputs_are_root_checked_and_deduped() {
        let generated_root = tempfile::tempdir().expect("temp generated root");
        let image_path = generated_root.path().join("image.png");
        tokio::fs::write(&image_path, b"png-bytes").await.unwrap();

        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .push(generated_root.path().to_path_buf());

        let result = serde_json::json!({
            "outputs": [
                {
                    "file_path": image_path,
                    "filename": "../rendered.png",
                    "mime_type": "image/png"
                },
                {
                    "file_path": image_path,
                    "filename": "duplicate.png",
                    "mime_type": "image/png"
                }
            ]
        });

        let attachments = attachments_from_tool_result("image_generate", &result, &[]).await;

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].data, b"png-bytes");
        assert_eq!(attachments[0].mime_type, "image/png");
        assert_eq!(attachments[0].filename.as_deref(), Some("rendered.png"));
    }

    #[tokio::test]
    async fn generated_outputs_reject_unapproved_paths_and_use_artifact_fallback() {
        let generated_root = tempfile::tempdir().expect("temp generated root");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        tokio::fs::write(outside.path(), b"outside").await.unwrap();

        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .push(generated_root.path().to_path_buf());

        let result = serde_json::json!({
            "outputs": [
                {
                    "file_path": outside.path(),
                    "filename": "outside.png",
                    "mime_type": "image/png"
                }
            ]
        });
        let artifact = ToolArtifact::Image {
            data: base64::engine::general_purpose::STANDARD.encode(b"artifact"),
            mime_type: "image/png".to_string(),
        };

        let attachments =
            attachments_from_tool_result("image_generate", &result, &[artifact]).await;

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].data, b"artifact");
        assert_eq!(attachments[0].filename.as_deref(), Some("generated-1.png"));
    }
}

//! Signal attachment IO: reading inbound attachments and staging outbound ones.

use uuid::Uuid;

use super::*;
use thinclaw_media::MediaContent;
use thinclaw_types::error::ChannelError;

/// Read Signal attachments from signal-cli's local file store.
///
/// signal-cli stores downloaded attachments at:
/// - Linux: `~/.local/share/signal-cli/attachments/<id>`
/// - macOS: `~/Library/Application Support/signal-cli/attachments/<id>`
pub(super) fn collect_signal_attachments(attachments: &[SignalAttachment]) -> Vec<MediaContent> {
    let mut result = Vec::new();

    // Resolve signal-cli attachment directory
    let attachment_dir = signal_attachment_dir();
    let Some(attachment_dir) = attachment_dir else {
        tracing::debug!("Signal: cannot resolve signal-cli attachment directory");
        return result;
    };

    for att in attachments {
        // Need an attachment ID to locate the file
        let Some(ref att_id) = att.id else {
            tracing::debug!("Signal: attachment has no id, skipping");
            continue;
        };

        // Check size before reading
        if let Some(size) = att.size
            && size > MAX_SIGNAL_ATTACHMENT_SIZE
        {
            tracing::warn!(
                id = %att_id,
                size = size,
                max = MAX_SIGNAL_ATTACHMENT_SIZE,
                "Signal: skipping oversized attachment"
            );
            continue;
        }

        // Prevent path traversal via malicious attachment IDs.
        // The att_id comes from signal-cli SSE (network input).
        if att_id.contains('/')
            || att_id.contains('\\')
            || att_id.contains("..")
            || att_id.is_empty()
        {
            tracing::warn!(
                id = %att_id,
                "Signal: rejecting attachment with suspicious path characters"
            );
            continue;
        }

        let path = attachment_dir.join(att_id);
        if !path.exists() {
            tracing::debug!(
                id = %att_id,
                path = %path.display(),
                "Signal: attachment file not found on disk"
            );
            continue;
        }

        match std::fs::read(&path) {
            Ok(data) => {
                if data.len() as u64 > MAX_SIGNAL_ATTACHMENT_SIZE {
                    tracing::warn!(
                        id = %att_id,
                        size = data.len(),
                        "Signal: attachment file exceeds size limit"
                    );
                    continue;
                }
                let mime = att
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream");
                let mut mc = MediaContent::new(data, mime);
                if let Some(ref filename) = att.filename {
                    mc = mc.with_filename(filename.clone());
                }
                tracing::debug!(
                    id = %att_id,
                    mime = %mime,
                    size = mc.size(),
                    "Signal: loaded attachment from disk"
                );
                result.push(mc);
            }
            Err(e) => {
                tracing::warn!(
                    id = %att_id,
                    error = %e,
                    "Signal: failed to read attachment file"
                );
            }
        }
    }

    result
}

pub(super) async fn write_signal_temp_attachments(
    attachments: &[MediaContent],
) -> Result<Vec<std::path::PathBuf>, ChannelError> {
    let mut paths = Vec::new();
    for attachment in attachments {
        let filename = attachment.filename.as_deref().unwrap_or("attachment");
        let safe_name = filename.replace(['/', '\\', ':'], "_");
        let path =
            std::env::temp_dir().join(format!("thinclaw-signal-{}-{safe_name}", Uuid::new_v4()));
        tokio::fs::write(&path, &attachment.data)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("failed to write Signal attachment {}: {e}", path.display()),
            })?;
        paths.push(path);
    }
    Ok(paths)
}

pub(super) async fn cleanup_signal_temp_attachments(paths: &[std::path::PathBuf]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// Resolve the signal-cli attachment directory.
pub(super) fn signal_attachment_dir() -> Option<std::path::PathBuf> {
    if let Some(override_dir) = std::env::var_os("SIGNAL_ATTACHMENTS_DIR")
        && !override_dir.is_empty()
    {
        return Some(std::path::PathBuf::from(override_dir));
    }

    let home = dirs::home_dir()?;

    // Linux: ~/.local/share/signal-cli/attachments
    let linux_path = home.join(".local/share/signal-cli/attachments");
    if linux_path.is_dir() {
        return Some(linux_path);
    }

    // macOS: ~/Library/Application Support/signal-cli/attachments
    let macos_path = home.join("Library/Application Support/signal-cli/attachments");
    if macos_path.is_dir() {
        return Some(macos_path);
    }

    #[cfg(target_os = "windows")]
    {
        let windows_paths = [
            home.join("AppData/Roaming/signal-cli/attachments"),
            home.join("AppData/Local/signal-cli/attachments"),
            home.join("scoop/persist/signal-cli/attachments"),
        ];
        for candidate in windows_paths {
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
        Some(home.join("AppData/Roaming/signal-cli/attachments"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Fallback: try the Linux path anyway (it may be created later)
        Some(linux_path)
    }
}

use std::path::Path;

use serde_json::Value;

use crate::error::WorkerError;
use crate::worker::api::{JobEventPayload, PromptResponse, WorkerHttpClient};

pub async fn post_job_event(client: &WorkerHttpClient, event_type: &str, data: &Value) {
    let payload = JobEventPayload {
        event_type: event_type.to_string(),
        data: data.clone(),
    };
    client.post_event(&payload).await;
}

pub async fn poll_for_prompt(
    client: &WorkerHttpClient,
) -> Result<Option<PromptResponse>, WorkerError> {
    client.poll_prompt().await
}

pub fn copy_auth_dir_from_mount(mount: &Path, target: &Path) -> Result<usize, WorkerError> {
    if !mount.exists() {
        return Ok(0);
    }

    std::fs::create_dir_all(target).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!("failed to create {}: {e}", target.display()),
    })?;

    copy_dir_recursive(mount, target).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!(
            "failed to copy auth from {} into {}: {e}",
            mount.display(),
            target.display()
        ),
    })
}

pub fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }

    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Skipping unreadable directory {}: {}", src.display(), e);
            return Ok(0);
        }
    };

    let mut copied = 0;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("Skipping unreadable entry in {}: {}", src.display(), e);
                continue;
            }
        };

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::debug!(
                    "Skipping entry with unreadable type {}: {}",
                    src_path.display(),
                    e
                );
                continue;
            }
        };

        if file_type.is_symlink() {
            tracing::debug!("Skipping symlink {}", src_path.display());
            continue;
        }

        if file_type.is_dir() {
            if std::fs::create_dir_all(&dst_path).is_ok() {
                copied += copy_dir_recursive(&src_path, &dst_path)?;
            }
        } else {
            match std::fs::copy(&src_path, &dst_path) {
                Ok(_) => copied += 1,
                Err(e) => {
                    tracing::debug!("Skipping unreadable file {}: {}", src_path.display(), e);
                }
            }
        }
    }

    Ok(copied)
}

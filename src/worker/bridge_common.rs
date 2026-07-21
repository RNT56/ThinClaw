use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::process::{ChildStderr, ChildStdout, Command};

use crate::error::WorkerError;
use crate::worker::api::{JobEventPayload, PromptResponse, WorkerHttpClient};

pub const MAX_BRIDGE_STDOUT_LINE_BYTES: usize = 192 * 1024;
pub const MAX_BRIDGE_STATUS_LINE_BYTES: usize = 3 * 1024;
pub const MAX_BRIDGE_SESSION_ID_BYTES: usize = 512;
const MAX_AUTH_COPY_FILES: usize = 4096;
const MAX_AUTH_COPY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUTH_COPY_DEPTH: usize = 32;

pub struct BoundedLine {
    pub text: String,
    pub truncated: bool,
}

/// A CLI child whose entire Unix process group is owned by the bridge.
///
/// `kill_on_drop` covers cancellation on every platform. The process group is
/// additionally killed on Unix so CLI-spawned shell/tool descendants cannot
/// outlive a timed-out or cancelled bridge session.
pub struct OwnedBridgeChild {
    child: thinclaw_platform::OwnedChild,
}

impl OwnedBridgeChild {
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        thinclaw_platform::OwnedChild::spawn(command).map(|child| Self { child })
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.take_stdout()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.take_stderr()
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.child.wait().await
    }

    pub async fn terminate(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.kill()).await;
    }
}

/// JoinHandle ownership that aborts instead of detaching on early return.
pub struct OwnedBridgeTask {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl OwnedBridgeTask {
    pub fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    pub async fn finish(&mut self) {
        let Some(mut handle) = self.handle.take() else {
            return;
        };
        if tokio::time::timeout(Duration::from_secs(5), &mut handle)
            .await
            .is_err()
        {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for OwnedBridgeTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Read and drain one newline-delimited record while retaining at most
/// `max_bytes`. This avoids `AsyncBufReadExt::lines()` allocating until an
/// attacker- or CLI-controlled newline eventually arrives.
pub async fn read_bounded_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<BoundedLine>>
where
    R: AsyncBufRead + Unpin,
{
    let mut retained = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut saw_input = false;
    let mut truncated = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if !saw_input {
                return Ok(None);
            }
            break;
        }
        saw_input = true;

        let newline = available.iter().position(|byte| *byte == b'\n');
        let data_len = newline.unwrap_or(available.len());
        let remaining = max_bytes.saturating_sub(retained.len());
        let keep = data_len.min(remaining);
        retained.extend_from_slice(&available[..keep]);
        if keep < data_len {
            truncated = true;
        }
        let consumed = newline.map_or(available.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if retained.last() == Some(&b'\r') {
        retained.pop();
    }
    Ok(Some(BoundedLine {
        text: String::from_utf8_lossy(&retained).into_owned(),
        truncated,
    }))
}

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
    match std::fs::symlink_metadata(mount) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => {}
        Ok(_) => {
            return Err(WorkerError::ExecutionFailed {
                reason: "auth mount is not a real directory".to_string(),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(WorkerError::ExecutionFailed {
                reason: format!("failed to inspect auth mount: {error}"),
            });
        }
    }

    ensure_real_directory(target).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!("failed to create private auth target: {e}"),
    })?;

    copy_dir_recursive(mount, target).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!(
            "failed to copy auth from {} into {}: {e}",
            mount.display(),
            target.display()
        ),
    })
}

fn ensure_real_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => Ok(()),
        Ok(_) => Err(std::io::Error::other("path is not a real directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path)?;
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(std::io::Error::other(
                    "created path is not a real directory",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
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

/// Sanitize child-process diagnostics before they are persisted as job events
/// or copied into the host's logs. CLI stderr is untrusted and can echo both
/// inherited credentials and provider tokens embedded in error messages.
pub fn sanitize_bridge_status(message: &str, exact_secrets: &[String]) -> String {
    let mut sanitized = message.to_string();
    for secret in exact_secrets {
        if secret.len() >= 8 && !secret.is_empty() {
            sanitized = sanitized.replace(secret, "[REDACTED]");
        }
    }

    let scan = thinclaw_safety::LeakDetector::new().scan(&sanitized);
    if scan.should_block {
        "[redacted sensitive child-process output]".to_string()
    } else {
        scan.redacted_content.unwrap_or(sanitized)
    }
}

/// Gather only values (never names) that a spawned bridge process could echo.
pub fn bridge_secret_values(
    extra_env: &std::collections::HashMap<String, String>,
    inherited_names: &[&str],
) -> Vec<String> {
    let mut values: Vec<String> = extra_env
        .values()
        .filter(|value| value.len() >= 8)
        .cloned()
        .collect();
    values.extend(
        inherited_names
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .filter(|value| value.len() >= 8),
    );
    values.sort();
    values.dedup();
    values
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<usize> {
    let mut budget = AuthCopyBudget::default();
    copy_dir_recursive_bounded(src, dst, 0, &mut budget)
}

#[derive(Default)]
struct AuthCopyBudget {
    files: usize,
    bytes: u64,
}

fn copy_dir_recursive_bounded(
    src: &Path,
    dst: &Path,
    depth: usize,
    budget: &mut AuthCopyBudget,
) -> std::io::Result<usize> {
    if depth > MAX_AUTH_COPY_DEPTH {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "auth directory nesting exceeds the safety limit",
        ));
    }
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
            ensure_real_directory(&dst_path)?;
            copied += copy_dir_recursive_bounded(&src_path, &dst_path, depth + 1, budget)?;
        } else if file_type.is_file() {
            let size = match entry.metadata() {
                Ok(metadata) => metadata.len(),
                Err(error) => {
                    tracing::debug!(
                        "Skipping file with unreadable metadata {}: {}",
                        src_path.display(),
                        error
                    );
                    continue;
                }
            };
            if budget.files >= MAX_AUTH_COPY_FILES
                || budget.bytes.saturating_add(size) > MAX_AUTH_COPY_BYTES
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "auth directory exceeds the file-count or byte safety limit",
                ));
            }
            let remaining = MAX_AUTH_COPY_BYTES.saturating_sub(budget.bytes);
            match thinclaw_platform::read_regular_file_bounded(&src_path, remaining).and_then(
                |bytes| {
                    thinclaw_platform::write_private_file_atomic(&dst_path, &bytes, true)?;
                    Ok(bytes.len() as u64)
                },
            ) {
                Ok(copied_bytes) => {
                    budget.files += 1;
                    budget.bytes += copied_bytes;
                    copied += 1;
                }
                Err(e) => {
                    tracing::debug!("Skipping unreadable file {}: {}", src_path.display(), e);
                }
            }
        } else {
            // Never open FIFOs, sockets, or device nodes from a host-provided
            // auth mount: doing so can block the bridge or expose host devices.
            tracing::debug!("Skipping non-regular auth entry {}", src_path.display());
        }
    }

    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn bounded_line_reader_drains_an_oversized_record() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(b"abcdef\r\nok\n").await.unwrap();
        });
        let mut reader = tokio::io::BufReader::new(reader);

        let first = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("first line");
        assert_eq!(first.text, "abcd");
        assert!(first.truncated);

        let second = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("second line");
        assert_eq!(second.text, "ok");
        assert!(!second.truncated);
        assert!(read_bounded_line(&mut reader, 4).await.unwrap().is_none());
        writer_task.await.unwrap();
    }

    #[test]
    fn bridge_status_redacts_exact_and_pattern_secrets() {
        let exact = "an-unstructured-secret-value".to_string();
        let patterned = ["sk", "-proj-", "abcdefghijklmnopqrstuvwxyz012345"].concat();
        let sanitized = sanitize_bridge_status(
            &format!("failed with {exact} and {patterned}"),
            std::slice::from_ref(&exact),
        );
        assert!(!sanitized.contains(&exact));
        assert!(!sanitized.contains(&patterned));
    }

    #[test]
    fn auth_copy_rejects_an_oversized_regular_file_before_copying() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let source_file = source.path().join("credentials.json");
        let file = std::fs::File::create(&source_file).unwrap();
        file.set_len(MAX_AUTH_COPY_BYTES + 1).unwrap();

        let error = copy_dir_recursive(source.path(), target.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(!target.path().join("credentials.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn auth_copy_does_not_follow_a_planted_destination_symlink() {
        use std::os::unix::fs::symlink;

        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("credentials.json"), b"new credentials").unwrap();
        let unrelated = target.path().join("unrelated");
        std::fs::write(&unrelated, b"keep me").unwrap();
        symlink(&unrelated, target.path().join("credentials.json")).unwrap();

        assert_eq!(copy_dir_recursive(source.path(), target.path()).unwrap(), 0);
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep me");
    }
}

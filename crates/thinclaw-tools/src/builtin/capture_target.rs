//! Safe staging and no-clobber publication for host camera/screen artifacts.

use std::path::{Path, PathBuf};

use tokio::io::AsyncReadExt;

use thinclaw_tools_core::ToolError;

const MAX_CAPTURE_PATH_BYTES: usize = 4096;
const MAX_CAPTURE_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub(super) enum CaptureFormat {
    Png,
    Jpeg,
}

impl CaptureFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    fn valid_magic(self, prefix: &[u8]) -> bool {
        match self {
            Self::Png => prefix.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Jpeg => prefix.starts_with(&[0xff, 0xd8, 0xff]),
        }
    }
}

pub(super) struct CaptureTarget {
    final_path: PathBuf,
    staging_path: PathBuf,
    _staging_dir: tempfile::TempDir,
    format: CaptureFormat,
}

impl CaptureTarget {
    pub(super) async fn prepare(
        requested_path: &Path,
        format: CaptureFormat,
    ) -> Result<Self, ToolError> {
        if requested_path.as_os_str().is_empty()
            || requested_path.as_os_str().to_string_lossy().len() > MAX_CAPTURE_PATH_BYTES
        {
            return Err(ToolError::InvalidParameters(
                "capture output path is empty or oversized".to_string(),
            ));
        }
        let file_name = requested_path
            .file_name()
            .filter(|name| *name != "." && *name != "..")
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "capture output path must end in a filename".to_string(),
                )
            })?
            .to_owned();
        let parent = requested_path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            ToolError::ExecutionFailed(format!("failed to create capture directory: {error}"))
        })?;
        let parent_metadata = tokio::fs::symlink_metadata(parent).await.map_err(|error| {
            ToolError::ExecutionFailed(format!("failed to inspect capture directory: {error}"))
        })?;
        if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
            return Err(ToolError::InvalidParameters(
                "capture output parent must be a real directory, not a symlink".to_string(),
            ));
        }
        let canonical_parent = tokio::fs::canonicalize(parent).await.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "failed to resolve capture output directory: {error}"
            ))
        })?;
        let final_path = canonical_parent.join(file_name);
        match tokio::fs::symlink_metadata(&final_path).await {
            Ok(_) => {
                return Err(ToolError::InvalidParameters(format!(
                    "capture output already exists; refusing to overwrite {}",
                    final_path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "failed to inspect capture output: {error}"
                )));
            }
        }

        let staging_dir = tempfile::Builder::new()
            .prefix(".thinclaw-capture-")
            .tempdir_in(&canonical_parent)
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to create private capture staging directory: {error}"
                ))
            })?;
        let staging_path = staging_dir
            .path()
            .join(format!("capture.{}", format.extension()));
        Ok(Self {
            final_path,
            staging_path,
            _staging_dir: staging_dir,
            format,
        })
    }

    pub(super) fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    pub(super) async fn publish(self) -> Result<(PathBuf, u64), ToolError> {
        let metadata = tokio::fs::symlink_metadata(&self.staging_path)
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("capture produced no artifact: {error}"))
            })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_CAPTURE_ARTIFACT_BYTES
        {
            return Err(ToolError::ExecutionFailed(format!(
                "capture artifact must be a regular file between 1 and {MAX_CAPTURE_ARTIFACT_BYTES} bytes"
            )));
        }

        let mut file = tokio::fs::File::open(&self.staging_path)
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("failed to inspect capture artifact: {error}"))
            })?;
        let mut prefix = [0_u8; 8];
        let prefix_len = file.read(&mut prefix).await.map_err(|error| {
            ToolError::ExecutionFailed(format!("failed to read capture artifact: {error}"))
        })?;
        if !self.format.valid_magic(&prefix[..prefix_len]) {
            return Err(ToolError::ExecutionFailed(
                "capture helper returned an unexpected file format".to_string(),
            ));
        }
        file.sync_all().await.map_err(|error| {
            ToolError::ExecutionFailed(format!("failed to sync capture artifact: {error}"))
        })?;
        drop(file);

        // Hard-link publication is atomic and fails if a file or symlink raced
        // into the destination. The staging directory lives on the same
        // filesystem as the destination by construction.
        tokio::fs::hard_link(&self.staging_path, &self.final_path)
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to publish capture without overwriting an existing path: {error}"
                ))
            })?;

        Ok((self.final_path.clone(), metadata.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publication_never_overwrites_an_existing_target() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("capture.png");
        let target = CaptureTarget::prepare(&output, CaptureFormat::Png)
            .await
            .unwrap();
        tokio::fs::write(target.staging_path(), b"\x89PNG\r\n\x1a\ncontent")
            .await
            .unwrap();
        tokio::fs::write(&output, b"existing").await.unwrap();

        assert!(target.publish().await.is_err());
        assert_eq!(tokio::fs::read(&output).await.unwrap(), b"existing");
    }
}

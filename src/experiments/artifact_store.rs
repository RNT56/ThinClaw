//! Durable artifact storage for experiment runners.
//!
//! Remote-runner trials execute inside ephemeral pods (RunPod/Vast). Any artifact
//! recorded with only a pod-local path becomes a dead reference the moment the pod
//! is torn down. This module defines a host-side [`ArtifactStore`] port and a default
//! local-filesystem implementation so the gateway host can persist the artifact bytes
//! under an operator-controlled root that survives teardown.
//!
//! The trait is deliberately minimal — `put` only. Deletion is owned by the
//! retention reaper (see `crate::api::experiments`), which prunes by path; an
//! object-store backend (e.g. `opendal`/S3) can slot in behind this same port later
//! without touching the call sites.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub const MAX_DURABLE_ARTIFACT_BYTES: usize = 512 * 1024;

/// Host-side durable storage for experiment artifacts.
///
/// Implementations write the supplied bytes to durable storage and return a stable
/// locator (absolute path or URI) that outlives the runner pod.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Persist `bytes` for the given trial/artifact and return a durable locator.
    async fn put(
        &self,
        trial_id: Uuid,
        artifact_id: Uuid,
        kind: &str,
        bytes: &[u8],
    ) -> anyhow::Result<String>;
}

/// Default [`ArtifactStore`] writing artifacts to the local filesystem under a
/// fixed root. Layout: `<root>/<trial_id>/<artifact_id>-<kind>`.
///
/// The returned locator is the absolute on-disk path, which the retention reaper
/// can later validate against the configured root before deleting.
pub struct LocalArtifactStore {
    root: PathBuf,
}

impl LocalArtifactStore {
    /// Create a store rooted at `root`. The directory tree is created lazily on
    /// the first `put`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Shared handle rooted at the operator-controlled experiments artifact dir.
    pub fn shared_default() -> Arc<dyn ArtifactStore> {
        Arc::new(Self::new(default_artifact_root()))
    }

    /// The configured root for this store.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
}

/// The default host-side durable artifact root: `<thinclaw-home>/experiments/artifacts`.
///
/// This mirrors the local-trial artifact convention in
/// `execute_local_trial` (`crate::api::experiments`) so both local and remote
/// artifacts land under the same operator-controlled, retention-managed tree.
pub fn default_artifact_root() -> PathBuf {
    thinclaw_platform::resolve_data_dir("experiments").join("artifacts")
}

/// Sanitize an artifact `kind` for use as a filename segment so a hostile or
/// unusual runner-supplied kind cannot escape the per-trial directory.
fn sanitize_kind(kind: &str) -> String {
    let cleaned: String = kind
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed.to_string()
    }
}

async fn validated_directory(path: &Path, label: &str) -> anyhow::Result<PathBuf> {
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(|error| {
        anyhow::anyhow!("failed to inspect {label} {}: {error}", path.display())
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!("{label} is not a real directory: {}", path.display());
    }
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| anyhow::anyhow!("failed to resolve {label} {}: {error}", path.display()))
}

async fn remove_temp_file(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), %error, "Failed to remove artifact temp file");
    }
}

#[async_trait]
impl ArtifactStore for LocalArtifactStore {
    async fn put(
        &self,
        trial_id: Uuid,
        artifact_id: Uuid,
        kind: &str,
        bytes: &[u8],
    ) -> anyhow::Result<String> {
        if bytes.len() > MAX_DURABLE_ARTIFACT_BYTES {
            anyhow::bail!("artifact exceeds the {MAX_DURABLE_ARTIFACT_BYTES} byte storage limit");
        }

        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to create artifact root {}: {error}",
                    self.root.display()
                )
            })?;
        let canonical_root = validated_directory(&self.root, "artifact root").await?;

        let dir = canonical_root.join(trial_id.simple().to_string());
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "failed to create artifact directory {}: {error}",
                    dir.display()
                ));
            }
        }
        let canonical_dir = validated_directory(&dir, "artifact directory").await?;
        if canonical_dir != dir {
            anyhow::bail!(
                "artifact directory resolves outside its assigned location: {}",
                dir.display()
            );
        }

        let file_name = format!("{}-{}", artifact_id.simple(), sanitize_kind(kind));
        let path = canonical_dir.join(file_name);
        let temp_path = canonical_dir.join(format!(
            ".{}-{}.tmp",
            artifact_id.simple(),
            Uuid::new_v4().simple()
        ));

        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut temp_file = options.open(&temp_path).await.map_err(|error| {
            anyhow::anyhow!(
                "failed to create artifact temp file {}: {error}",
                temp_path.display()
            )
        })?;
        if let Err(error) = temp_file.write_all(bytes).await {
            drop(temp_file);
            remove_temp_file(&temp_path).await;
            return Err(anyhow::anyhow!(
                "failed to write artifact temp file {}: {error}",
                temp_path.display()
            ));
        }
        if let Err(error) = temp_file.sync_all().await {
            drop(temp_file);
            remove_temp_file(&temp_path).await;
            return Err(anyhow::anyhow!(
                "failed to sync artifact temp file {}: {error}",
                temp_path.display()
            ));
        }
        drop(temp_file);

        // Hard-link publication is atomic and fails if the final path already
        // exists, including when an attacker pre-created a symlink there.
        if let Err(error) = tokio::fs::hard_link(&temp_path, &path).await {
            remove_temp_file(&temp_path).await;
            return Err(anyhow::anyhow!(
                "failed to publish artifact {}: {error}",
                path.display()
            ));
        }
        remove_temp_file(&temp_path).await;
        Ok(path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_store_round_trips_bytes_and_path_exists() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let store = LocalArtifactStore::new(dir.path());
        let trial_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let payload = b"benchmark-ok\nscore=1\n";

        let locator = store
            .put(trial_id, artifact_id, "run_log", payload)
            .await
            .expect("put should succeed");

        let path = std::path::Path::new(&locator);
        assert!(path.exists(), "durable artifact path should exist");
        let canonical_root = std::fs::canonicalize(dir.path()).expect("canonical root");
        assert!(
            path.starts_with(&canonical_root),
            "artifact should be written under the configured root"
        );
        let read_back = tokio::fs::read(path).await.expect("read back");
        assert_eq!(read_back, payload);
    }

    #[tokio::test]
    async fn sanitizes_hostile_kind_segment() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let store = LocalArtifactStore::new(dir.path());
        let trial_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();

        let locator = store
            .put(trial_id, artifact_id, "../../etc/passwd", b"x")
            .await
            .expect("put should succeed");

        let path = std::path::Path::new(&locator);
        let canonical_root = std::fs::canonicalize(dir.path()).expect("canonical root");
        assert!(
            path.starts_with(canonical_root.join(trial_id.simple().to_string())),
            "sanitized artifact must stay within the per-trial directory: {locator}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_precreated_trial_directory_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::TempDir::new().expect("root");
        let outside = tempfile::TempDir::new().expect("outside");
        let trial_id = Uuid::new_v4();
        symlink(
            outside.path(),
            root.path().join(trial_id.simple().to_string()),
        )
        .expect("create hostile symlink");
        let store = LocalArtifactStore::new(root.path());

        let error = store
            .put(trial_id, Uuid::new_v4(), "run_log", b"secret")
            .await
            .expect_err("symlinked trial directory must be rejected");
        assert!(error.to_string().contains("not a real directory"));
        assert!(
            std::fs::read_dir(outside.path())
                .expect("read outside")
                .next()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn never_overwrites_precreated_artifact_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::TempDir::new().expect("root");
        let outside = root.path().join("outside.txt");
        std::fs::write(&outside, "untouched").expect("seed outside file");
        let trial_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let trial_dir = root.path().join(trial_id.simple().to_string());
        std::fs::create_dir(&trial_dir).expect("create trial dir");
        let final_path = trial_dir.join(format!("{}-run_log", artifact_id.simple()));
        symlink(&outside, &final_path).expect("create hostile artifact symlink");
        let store = LocalArtifactStore::new(root.path());

        store
            .put(trial_id, artifact_id, "run_log", b"replacement")
            .await
            .expect_err("existing destination must never be overwritten");
        assert_eq!(
            std::fs::read_to_string(&outside).expect("read outside file"),
            "untouched"
        );
    }

    #[tokio::test]
    async fn rejects_oversized_artifact_at_storage_boundary() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let store = LocalArtifactStore::new(dir.path());
        let payload = vec![0_u8; MAX_DURABLE_ARTIFACT_BYTES + 1];
        store
            .put(Uuid::new_v4(), Uuid::new_v4(), "run_log", &payload)
            .await
            .expect_err("oversized payload must be rejected");
    }
}

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

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

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

#[async_trait]
impl ArtifactStore for LocalArtifactStore {
    async fn put(
        &self,
        trial_id: Uuid,
        artifact_id: Uuid,
        kind: &str,
        bytes: &[u8],
    ) -> anyhow::Result<String> {
        let dir = self.root.join(trial_id.simple().to_string());
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            anyhow::anyhow!("failed to create artifact directory {}: {e}", dir.display())
        })?;
        let file_name = format!("{}-{}", artifact_id.simple(), sanitize_kind(kind));
        let path = dir.join(file_name);
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write artifact {}: {e}", path.display()))?;
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
        assert!(
            path.starts_with(dir.path()),
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
        assert!(
            path.starts_with(dir.path().join(trial_id.simple().to_string())),
            "sanitized artifact must stay within the per-trial directory: {locator}"
        );
    }
}

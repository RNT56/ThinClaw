//! Background sync engine for periodic cloud uploads.
//!
//! Detects changed files since last sync and uploads them in batches.
//! Runs every 5 minutes when in Cloud storage mode.
//!
//! # Change Detection
//!
//! Uses a two-pass strategy:
//! 1. mtime-based filter: only consider files modified after last sync
//! 2. SHA-256 comparison: verify actual content change (avoids spurious syncs)

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use super::provider::CloudError;

// ── Types ────────────────────────────────────────────────────────────────

/// A file that has changed since last sync.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    /// Relative path from app_data_dir
    pub rel_path: String,
    /// Absolute path on disk
    pub abs_path: PathBuf,
    /// Type of change
    pub change_type: ChangeType,
    /// Current SHA-256 hash (for new/modified files)
    pub hash: Option<String>,
    /// File size in bytes
    pub size: u64,
}

/// Type of file change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
}

/// Status of the sync engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error(String),
    Disabled,
}

// ── FileTracker ──────────────────────────────────────────────────────────

/// Tracks known file hashes to detect changes.
pub struct FileTracker {
    /// Known file hashes from last sync: relative_path → SHA-256
    known_hashes: HashMap<String, String>,
    /// Timestamp of last successful sync
    last_sync: Option<DateTime<Utc>>,
}

impl FileTracker {
    pub fn new() -> Self {
        Self {
            known_hashes: HashMap::new(),
            last_sync: None,
        }
    }

    /// Load known hashes from a previous state (e.g., from manifest).
    pub fn load_from_hashes(
        hashes: HashMap<String, String>,
        last_sync: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            known_hashes: hashes,
            last_sync,
        }
    }

    /// Detect changed files by scanning a directory.
    ///
    /// Returns a list of files that are new, modified, or deleted relative
    /// to the known state.
    pub async fn detect_changes(
        &self,
        root: &Path,
        scan_dirs: &[&str],
    ) -> Result<Vec<ChangedFile>, CloudError> {
        let mut changes = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();

        for subdir in scan_dirs {
            let dir = root.join(subdir);
            if !dir.exists() {
                continue;
            }
            Self::scan_dir_recursive(
                &dir,
                root,
                &self.known_hashes,
                &mut changes,
                &mut seen_paths,
            )
            .await?;
        }

        // Detect deletions: files in known_hashes but not seen on disk
        for (rel_path, _hash) in &self.known_hashes {
            if !seen_paths.contains(rel_path) {
                let abs_path = root.join(rel_path);
                if !abs_path.exists() {
                    changes.push(ChangedFile {
                        rel_path: rel_path.clone(),
                        abs_path,
                        change_type: ChangeType::Deleted,
                        hash: None,
                        size: 0,
                    });
                }
            }
        }

        Ok(changes)
    }

    /// Update tracker state after successful sync.
    pub fn mark_synced(&mut self, changes: &[ChangedFile]) {
        for change in changes {
            match change.change_type {
                ChangeType::Added | ChangeType::Modified => {
                    if let Some(hash) = &change.hash {
                        self.known_hashes
                            .insert(change.rel_path.clone(), hash.clone());
                    }
                }
                ChangeType::Deleted => {
                    self.known_hashes.remove(&change.rel_path);
                }
            }
        }
        self.last_sync = Some(Utc::now());
    }

    /// Get the last sync timestamp.
    pub fn last_sync(&self) -> Option<DateTime<Utc>> {
        self.last_sync
    }

    // ── Internal ─────────────────────────────────────────────────────────

    async fn scan_dir_recursive(
        dir: &Path,
        root: &Path,
        known: &HashMap<String, String>,
        changes: &mut Vec<ChangedFile>,
        seen: &mut std::collections::HashSet<String>,
    ) -> Result<(), CloudError> {
        let mut entries = tokio::fs::read_dir(dir)
            .await
            .map_err(|e| CloudError::Provider(format!("read_dir '{}': {}", dir.display(), e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| CloudError::Provider(format!("next_entry: {}", e)))?
        {
            let path = entry.path();

            // Skip hidden files/dirs
            if path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }

            let meta = entry.metadata().await.map_err(|e| {
                CloudError::Provider(format!("metadata '{}': {}", path.display(), e))
            })?;

            if meta.is_dir() {
                Box::pin(Self::scan_dir_recursive(&path, root, known, changes, seen)).await?;
            } else if meta.is_file() {
                let rel_path = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                seen.insert(rel_path.clone());

                // Skip very large files (likely model files — too costly to hash/sync)
                const MAX_SYNC_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB
                if meta.len() > MAX_SYNC_FILE_SIZE {
                    debug!(
                        "[cloud/sync] Skipping large file ({} MB): {}",
                        meta.len() / (1024 * 1024),
                        rel_path
                    );
                    continue;
                }

                // Compute SHA-256 using streaming reader (constant memory)
                let hash = compute_sha256_streaming(&path).await.map_err(|e| {
                    CloudError::Provider(format!("hash '{}': {}", path.display(), e))
                })?;

                match known.get(&rel_path) {
                    None => {
                        // New file
                        changes.push(ChangedFile {
                            rel_path,
                            abs_path: path,
                            change_type: ChangeType::Added,
                            hash: Some(hash),
                            size: meta.len(),
                        });
                    }
                    Some(known_hash) if *known_hash != hash => {
                        // Modified file
                        changes.push(ChangedFile {
                            rel_path,
                            abs_path: path,
                            change_type: ChangeType::Modified,
                            hash: Some(hash),
                            size: meta.len(),
                        });
                    }
                    _ => {
                        // Unchanged — skip
                    }
                }
            }
        }

        Ok(())
    }
}

/// Streaming SHA-256: reads file in 64 KB chunks to avoid loading large files into RAM.
async fn compute_sha256_streaming(path: &Path) -> Result<String, std::io::Error> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024]; // 64 KB chunks

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

// ── SyncEngine ───────────────────────────────────────────────────────────

/// Background sync engine that periodically uploads changed files.
pub struct SyncEngine {
    /// Interval between sync cycles (default: 5 minutes)
    interval: Duration,
    /// Cancel signal
    cancel_tx: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
}

impl SyncEngine {
    /// Create a new sync engine with the given interval.
    pub fn new(interval: Duration) -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        Self {
            interval,
            cancel_tx,
            cancel_rx,
        }
    }

    /// Create with default 5-minute interval.
    pub fn default_interval() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Stop the sync loop.
    pub fn stop(&self) {
        let _ = self.cancel_tx.send(true);
        info!("[cloud/sync] Sync engine stopped");
    }

    /// Run the background sync loop.
    ///
    /// This should be spawned via `tokio::spawn`.
    /// Runs until `stop()` is called.
    pub async fn run(
        &self,
        tracker: &mut FileTracker,
        app_data_dir: &Path,
        scan_dirs: &[&str],
        // Called for each batch of changes that need to be synced.
        // The caller handles encryption + upload via CloudManager.
        mut on_changes: impl FnMut(
            Vec<ChangedFile>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), CloudError>> + Send>,
        >,
    ) {
        let mut cancel = self.cancel_rx.clone();
        let mut consecutive_failures: u32 = 0;
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;

        info!(
            "[cloud/sync] Starting sync loop (interval: {:?})",
            self.interval
        );

        loop {
            // Calculate backoff: on failures, wait longer (double each time, capped at 1 hour)
            let wait_duration = if consecutive_failures > 0 {
                let backoff_secs = self.interval.as_secs() * 2u64.pow(consecutive_failures.min(6));
                let capped = backoff_secs.min(3600); // Max 1 hour
                warn!(
                    "[cloud/sync] Backoff: waiting {}s after {} consecutive failures",
                    capped, consecutive_failures
                );
                Duration::from_secs(capped)
            } else {
                self.interval
            };

            // Wait for interval or cancellation
            tokio::select! {
                _ = tokio::time::sleep(wait_duration) => {},
                _ = cancel.changed() => {
                    if *cancel.borrow() {
                        info!("[cloud/sync] Cancelled");
                        return;
                    }
                }
            }

            // Check cancellation again
            if *self.cancel_rx.borrow() {
                return;
            }

            debug!("[cloud/sync] Starting sync cycle");

            match tracker.detect_changes(app_data_dir, scan_dirs).await {
                Ok(changes) if changes.is_empty() => {
                    debug!("[cloud/sync] No changes detected");
                    consecutive_failures = 0; // Reset on successful detection
                }
                Ok(changes) => {
                    let count = changes.len();
                    info!("[cloud/sync] {} changes detected, syncing...", count);

                    match on_changes(changes.clone()).await {
                        Ok(()) => {
                            tracker.mark_synced(&changes);
                            info!("[cloud/sync] Sync complete ({} files)", count);
                            consecutive_failures = 0;
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            warn!(
                                "[cloud/sync] Sync failed ({}/{}): {}",
                                consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                            );
                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                warn!(
                                    "[cloud/sync] {} consecutive failures reached, continuing with backoff",
                                    MAX_CONSECUTIVE_FAILURES
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    warn!(
                        "[cloud/sync] Change detection failed ({}/{}): {}",
                        consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                    );
                }
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_file_tracker_detect_new_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create a subdirectory with files
        let docs = root.join("documents");
        tokio::fs::create_dir_all(&docs).await.unwrap();
        tokio::fs::write(docs.join("a.txt"), b"hello")
            .await
            .unwrap();
        tokio::fs::write(docs.join("b.txt"), b"world")
            .await
            .unwrap();

        let tracker = FileTracker::new();
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();

        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.change_type == ChangeType::Added));
    }

    #[tokio::test]
    async fn test_file_tracker_detect_modifications() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let docs = root.join("documents");
        tokio::fs::create_dir_all(&docs).await.unwrap();
        tokio::fs::write(docs.join("a.txt"), b"original")
            .await
            .unwrap();

        // First scan — everything is new
        let mut tracker = FileTracker::new();
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        tracker.mark_synced(&changes);
        assert_eq!(changes.len(), 1);

        // Modify the file
        tokio::fs::write(docs.join("a.txt"), b"modified content")
            .await
            .unwrap();

        // Second scan — detects modification
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, ChangeType::Modified);
    }

    #[tokio::test]
    async fn test_file_tracker_detect_deletions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let docs = root.join("documents");
        tokio::fs::create_dir_all(&docs).await.unwrap();
        tokio::fs::write(docs.join("a.txt"), b"will be deleted")
            .await
            .unwrap();

        let mut tracker = FileTracker::new();
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        tracker.mark_synced(&changes);

        // Delete the file
        tokio::fs::remove_file(docs.join("a.txt")).await.unwrap();

        // Detect deletion
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, ChangeType::Deleted);
    }

    #[tokio::test]
    async fn test_file_tracker_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let docs = root.join("documents");
        tokio::fs::create_dir_all(&docs).await.unwrap();
        tokio::fs::write(docs.join("stable.txt"), b"unchanged")
            .await
            .unwrap();

        let mut tracker = FileTracker::new();
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        tracker.mark_synced(&changes);

        // No changes
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();
        assert!(changes.is_empty());
    }

    #[tokio::test]
    async fn test_file_tracker_skips_hidden_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let docs = root.join("documents");
        tokio::fs::create_dir_all(&docs).await.unwrap();
        tokio::fs::write(docs.join(".hidden"), b"secret")
            .await
            .unwrap();
        tokio::fs::write(docs.join("visible.txt"), b"public")
            .await
            .unwrap();

        let tracker = FileTracker::new();
        let changes = tracker.detect_changes(root, &["documents"]).await.unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].rel_path, "documents/visible.txt");
    }

    #[tokio::test]
    async fn test_sync_engine_stop() {
        let engine = SyncEngine::new(Duration::from_millis(50));

        // Stop immediately
        engine.stop();

        let mut tracker = FileTracker::new();
        let tmp = tempfile::tempdir().unwrap();

        // This should return immediately because cancel is already set
        tokio::time::timeout(
            Duration::from_secs(1),
            engine.run(&mut tracker, tmp.path(), &[], |_changes| {
                Box::pin(async { Ok(()) })
            }),
        )
        .await
        .expect("sync loop should have stopped");
    }
}

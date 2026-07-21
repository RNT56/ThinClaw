//! Skill hot-reload watcher.
//!
//! Watches the configured skill discovery directories for new, edited, or
//! deleted `SKILL.md` files and refreshes the in-memory registry automatically.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;

use crate::skills::registry::SkillRegistry;

const WATCHER_STOP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_WATCHED_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_WATCHED_SKILL_FILES: usize = 1_000;

/// Configuration for the skill watcher.
#[derive(Debug, Clone)]
pub struct SkillWatcherConfig {
    /// How often to poll for changes (default: 3s).
    pub poll_interval: Duration,
    /// Debounce period after a change before reloading (default: 1s).
    pub debounce: Duration,
}

impl Default for SkillWatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            debounce: Duration::from_secs(1),
        }
    }
}

/// Watches skill directories and reloads the registry on change.
pub struct SkillWatcher {
    config: SkillWatcherConfig,
    task_handle: RwLock<Option<JoinHandle<()>>>,
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
    known: Arc<RwLock<HashMap<PathBuf, SystemTime>>>,
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillWatcher {
    /// Create a new watcher for a shared skill registry.
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self {
            config: SkillWatcherConfig::default(),
            task_handle: RwLock::new(None),
            shutdown_tx: RwLock::new(None),
            known: Arc::new(RwLock::new(HashMap::new())),
            registry,
        }
    }

    /// Override the default watcher configuration.
    pub fn with_config(mut self, config: SkillWatcherConfig) -> Self {
        self.config = SkillWatcherConfig {
            poll_interval: config
                .poll_interval
                .clamp(Duration::from_millis(100), Duration::from_secs(3_600)),
            debounce: config.debounce.min(Duration::from_secs(60)),
        };
        self
    }

    /// Seed the watcher with the current set of skill files.
    pub async fn seed_from_registry(&self) {
        let snapshot = match Self::scan_registry(&self.registry).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to seed skill watcher");
                return;
            }
        };
        let count = snapshot.len();
        *self.known.write().await = snapshot;
        tracing::info!(
            known_skill_files = count,
            "Skill watcher seeded with existing skills"
        );
    }

    /// Start polling for changes.
    pub async fn start(&self) {
        self.stop().await;

        let config = self.config.clone();
        let known = Arc::clone(&self.known);
        let registry = Arc::clone(&self.registry);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        let handle = tokio::spawn(async move {
            tracing::info!(
                poll_secs = config.poll_interval.as_secs_f64(),
                "Skill hot-reload watcher started"
            );

            let mut last_reload = SystemTime::UNIX_EPOCH;

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!("Skill hot-reload watcher stopped");
                        break;
                    }
                    _ = tokio::time::sleep(config.poll_interval) => {}
                }

                let snapshot = match Self::scan_registry(&registry).await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        tracing::warn!(error = %error, "Skill watcher poll error");
                        continue;
                    }
                };

                let changed = {
                    let known_guard = known.read().await;
                    *known_guard != snapshot
                };

                if !changed {
                    continue;
                }

                let now = SystemTime::now();
                if now
                    .duration_since(last_reload)
                    .ok()
                    .is_some_and(|elapsed| elapsed < config.debounce)
                {
                    continue;
                }

                let loaded = {
                    let mut guard = registry.write().await;
                    guard.reload().await
                };
                *known.write().await = snapshot;
                last_reload = now;

                tracing::info!(
                    loaded_skills = loaded.len(),
                    skills = %loaded.join(", "),
                    "Skill watcher reloaded registry after on-disk change"
                );
            }
        });

        *self.shutdown_tx.write().await = Some(shutdown_tx);
        *self.task_handle.write().await = Some(handle);
    }

    /// Stop watching.
    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.task_handle.write().await.take() {
            drain_watcher_task(handle, "skill_watcher").await;
        }
    }

    async fn scan_registry(
        registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    ) -> Result<HashMap<PathBuf, SystemTime>, std::io::Error> {
        let dirs = {
            let guard = registry.read().await;
            guard.discovery_dirs()
        };
        scan_skill_files(&dirs).await
    }
}

async fn scan_skill_files(
    dirs: &[PathBuf],
) -> Result<HashMap<PathBuf, SystemTime>, std::io::Error> {
    let mut files = HashMap::new();

    for dir in dirs {
        match tokio::fs::symlink_metadata(dir).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "skill watcher root is not a real directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }

        let flat_skill = dir.join("SKILL.md");
        if let Some(modified) = regular_file_mtime_if_present(&flat_skill).await? {
            files.insert(flat_skill, modified);
        }
        if files.len() > MAX_WATCHED_SKILL_FILES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "skill watcher exceeds the file limit",
            ));
        }

        let mut entries = tokio::fs::read_dir(dir).await?;
        let mut scanned = 0_usize;
        while let Some(entry) = entries.next_entry().await? {
            scanned = scanned.saturating_add(1);
            if scanned > MAX_WATCHED_DIRECTORY_ENTRIES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "skill watcher directory exceeds the entry limit",
                ));
            }
            let path = entry.path();
            let meta = tokio::fs::symlink_metadata(&path).await?;
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let nested_skill = path.join("SKILL.md");
                if let Some(modified) = regular_file_mtime_if_present(&nested_skill).await? {
                    files.insert(nested_skill, modified);
                }
            }
            if files.len() > MAX_WATCHED_SKILL_FILES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "skill watcher exceeds the file limit",
                ));
            }
        }
    }

    Ok(files)
}

async fn regular_file_mtime_if_present(path: &Path) -> Result<Option<SystemTime>, std::io::Error> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            metadata.modified().map(Some)
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn drain_watcher_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(task = name, error = %error, "Watcher task exited with error");
            }
        }
        _ = tokio::time::sleep(WATCHER_STOP_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(task = name, "Watcher task did not drain before timeout; aborted");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SkillWatcherConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(3));
        assert_eq!(config.debounce, Duration::from_secs(1));
    }

    #[tokio::test]
    async fn test_stop_drains_running_watcher_promptly() {
        let registry = Arc::new(tokio::sync::RwLock::new(SkillRegistry::new(PathBuf::from(
            "/tmp/nonexistent_thinclaw_skill_watcher_stop_test",
        ))));
        let watcher = SkillWatcher::new(registry).with_config(SkillWatcherConfig {
            poll_interval: Duration::from_secs(60),
            debounce: Duration::from_millis(1),
        });

        watcher.start().await;
        tokio::time::timeout(Duration::from_millis(250), watcher.stop())
            .await
            .expect("stop should not wait for the poll interval");
    }
}

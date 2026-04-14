//! Skill hot-reload watcher.
//!
//! Watches the configured skill discovery directories for new, edited, or
//! deleted `SKILL.md` files and refreshes the in-memory registry automatically.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::skills::registry::SkillRegistry;

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
    known: Arc<RwLock<HashMap<PathBuf, SystemTime>>>,
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillWatcher {
    /// Create a new watcher for a shared skill registry.
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self {
            config: SkillWatcherConfig::default(),
            task_handle: RwLock::new(None),
            known: Arc::new(RwLock::new(HashMap::new())),
            registry,
        }
    }

    /// Override the default watcher configuration.
    pub fn with_config(mut self, config: SkillWatcherConfig) -> Self {
        self.config = config;
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
        let config = self.config.clone();
        let known = Arc::clone(&self.known);
        let registry = Arc::clone(&self.registry);

        let handle = tokio::spawn(async move {
            tracing::info!(
                poll_secs = config.poll_interval.as_secs_f64(),
                "Skill hot-reload watcher started"
            );

            let mut last_reload = SystemTime::UNIX_EPOCH;

            loop {
                tokio::time::sleep(config.poll_interval).await;

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

        *self.task_handle.write().await = Some(handle);
    }

    /// Stop watching.
    pub async fn stop(&self) {
        if let Some(handle) = self.task_handle.write().await.take() {
            handle.abort();
            tracing::info!("Skill hot-reload watcher stopped");
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
        if !tokio::fs::try_exists(dir).await.unwrap_or(false) {
            continue;
        }

        let flat_skill = dir.join("SKILL.md");
        if tokio::fs::try_exists(&flat_skill).await.unwrap_or(false) {
            files.insert(flat_skill.clone(), metadata_mtime(&flat_skill).await?);
        }

        let mut entries = tokio::fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let meta = tokio::fs::symlink_metadata(&path).await?;
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let nested_skill = path.join("SKILL.md");
                if tokio::fs::try_exists(&nested_skill).await.unwrap_or(false) {
                    files.insert(nested_skill.clone(), metadata_mtime(&nested_skill).await?);
                }
            }
        }
    }

    Ok(files)
}

async fn metadata_mtime(path: &Path) -> Result<SystemTime, std::io::Error> {
    let metadata = tokio::fs::metadata(path).await?;
    metadata.modified()
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
}

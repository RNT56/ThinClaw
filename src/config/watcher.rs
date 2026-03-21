//! Configuration file watcher for hot-reloading.
//!
//! Monitors the config file for changes and notifies subscribers
//! when the configuration has been updated on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;

/// Configuration for the file watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// How often to poll for changes (default: 2s).
    pub poll_interval: Duration,
    /// Debounce period — min time between reloads (default: 500ms).
    pub debounce: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            debounce: Duration::from_millis(500),
        }
    }
}

/// Event emitted when the config file changes.
#[derive(Debug, Clone)]
pub struct ConfigChanged {
    /// Path of the file that changed.
    pub path: PathBuf,
    /// When the change was detected.
    pub detected_at: SystemTime,
}

/// Watches a configuration file and emits events when it changes.
///
/// Uses polling with `mtime` comparison rather than OS-level file
/// notifications to avoid heavy dependencies. The 2-second default
/// poll interval is a good balance between responsiveness and overhead.
///
/// # Usage
///
/// ```rust,ignore
/// let watcher = ConfigWatcher::new("/path/to/config.toml");
/// let mut rx = watcher.subscribe();
/// watcher.start().await;
///
/// while let Ok(event) = rx.recv().await {
///     println!("Config changed: {:?}", event.path);
///     // Reload config...
/// }
/// ```
pub struct ConfigWatcher {
    path: PathBuf,
    config: WatcherConfig,
    tx: broadcast::Sender<ConfigChanged>,
    task_handle: RwLock<Option<JoinHandle<()>>>,
    /// Last known modified time.
    last_mtime: Arc<RwLock<Option<SystemTime>>>,
}

impl ConfigWatcher {
    /// Create a new watcher for the given config file path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            path: path.as_ref().to_path_buf(),
            config: WatcherConfig::default(),
            tx,
            task_handle: RwLock::new(None),
            last_mtime: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(mut self, config: WatcherConfig) -> Self {
        self.config = config;
        self
    }

    /// Subscribe to config change events.
    pub fn subscribe(&self) -> broadcast::Receiver<ConfigChanged> {
        self.tx.subscribe()
    }

    /// Start watching for changes.
    pub async fn start(&self) {
        // Read current mtime to establish baseline
        let initial_mtime = file_mtime(&self.path);
        *self.last_mtime.write().await = initial_mtime;

        let path = self.path.clone();
        let config = self.config.clone();
        let tx = self.tx.clone();
        let last_mtime = Arc::clone(&self.last_mtime);

        let handle = tokio::spawn(async move {
            tracing::info!(
                path = %path.display(),
                poll_secs = config.poll_interval.as_secs_f64(),
                "Config watcher started"
            );

            let mut last_emit = SystemTime::UNIX_EPOCH;

            loop {
                tokio::time::sleep(config.poll_interval).await;

                let current_mtime = file_mtime(&path);
                let prev_mtime = *last_mtime.read().await;

                // Check if mtime changed
                let changed = match (current_mtime, prev_mtime) {
                    (Some(current), Some(prev)) => current != prev,
                    (Some(_), None) => true, // File appeared
                    (None, Some(_)) => {
                        tracing::warn!(path = %path.display(), "Config file disappeared");
                        false
                    }
                    (None, None) => false,
                };

                if changed {
                    // Debounce
                    let now = SystemTime::now();
                    let since_last = now
                        .duration_since(last_emit)
                        .unwrap_or(Duration::from_secs(999));

                    if since_last >= config.debounce {
                        *last_mtime.write().await = current_mtime;
                        last_emit = now;

                        let event = ConfigChanged {
                            path: path.clone(),
                            detected_at: now,
                        };

                        tracing::info!(
                            path = %path.display(),
                            "Configuration file changed, notifying subscribers"
                        );

                        // Don't care if no subscribers
                        let _ = tx.send(event);
                    }
                }
            }
        });

        *self.task_handle.write().await = Some(handle);
    }

    /// Stop watching.
    pub async fn stop(&self) {
        if let Some(handle) = self.task_handle.write().await.take() {
            handle.abort();
            tracing::info!(path = %self.path.display(), "Config watcher stopped");
        }
    }

    /// Get the watched file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if the file currently exists.
    pub fn file_exists(&self) -> bool {
        self.path.exists()
    }

    /// Manually trigger a reload check.
    ///
    /// Returns `true` if the file has changed since last check.
    pub async fn check_now(&self) -> bool {
        let current_mtime = file_mtime(&self.path);
        let prev_mtime = *self.last_mtime.read().await;

        let changed = match (current_mtime, prev_mtime) {
            (Some(current), Some(prev)) => current != prev,
            (Some(_), None) => true,
            _ => false,
        };

        if changed {
            *self.last_mtime.write().await = current_mtime;

            let event = ConfigChanged {
                path: self.path.clone(),
                detected_at: SystemTime::now(),
            };
            let _ = self.tx.send(event);
        }

        changed
    }
}

/// Get file modified time, or None if the file doesn't exist.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WatcherConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(2));
        assert_eq!(config.debounce, Duration::from_millis(500));
    }

    #[test]
    fn test_watcher_nonexistent_file() {
        let watcher = ConfigWatcher::new("/tmp/nonexistent_thinclaw_config_test.toml");
        assert!(!watcher.file_exists());
    }

    #[tokio::test]
    async fn test_watcher_subscribe() {
        let watcher = ConfigWatcher::new("/tmp/test_config.toml");
        let _rx = watcher.subscribe();
        // Should not panic even with no file
    }

    #[tokio::test]
    async fn test_check_now_no_file() {
        let watcher = ConfigWatcher::new("/tmp/nonexistent_thinclaw_config_test.toml");
        let changed = watcher.check_now().await;
        assert!(!changed);
    }

    #[tokio::test]
    async fn test_check_now_detects_change() {
        let path = "/tmp/thinclaw_watcher_test.toml";
        // Create the file
        std::fs::write(path, "initial").unwrap();

        let watcher = ConfigWatcher::new(path);
        let mut rx = watcher.subscribe();

        // Establish baseline
        let _ = watcher.check_now().await;

        // Modify the file
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(path, "modified").unwrap();

        // Check
        let changed = watcher.check_now().await;
        assert!(changed, "Expected change detection");

        // Should have emitted event
        let event = rx.try_recv().unwrap();
        assert_eq!(event.path, PathBuf::from(path));

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let watcher = ConfigWatcher::new("/tmp/test.toml");
        watcher.stop().await; // Should not panic
    }
}

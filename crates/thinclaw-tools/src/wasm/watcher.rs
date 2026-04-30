//! WASM tool hot-reload watcher.
//!
//! Monitors the installed tools directory and dev build artifacts for `.wasm`
//! changes and keeps the tool registry in sync without requiring a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::wasm::ports::{RegistryUnregister, WasmToolRegistrar};
use crate::wasm::{WasmToolLoader, discover_dev_tools, discover_tools};

/// Configuration for the tool watcher.
#[derive(Debug, Clone)]
pub struct ToolWatcherConfig {
    /// How often to poll for changes (default: 3s).
    pub poll_interval: Duration,
    /// Debounce period — min time between reloads of the same tool (default: 1s).
    pub debounce: Duration,
    /// Whether to include `tools-src/*` build artifacts in the live discovery set.
    pub include_dev_artifacts: bool,
}

impl Default for ToolWatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            debounce: Duration::from_secs(1),
            include_dev_artifacts: true,
        }
    }
}

#[derive(Debug, Clone)]
struct ToolSource {
    wasm_path: PathBuf,
    capabilities_path: Option<PathBuf>,
    mtime: SystemTime,
}

#[derive(Debug, Clone)]
struct WatchedTool {
    source: ToolSource,
    last_reload: SystemTime,
}

/// Watches a tools directory and hot-reloads WASM tools.
pub struct ToolWatcher<R>
where
    R: WasmToolRegistrar + RegistryUnregister,
{
    install_dir: PathBuf,
    config: ToolWatcherConfig,
    task_handle: RwLock<Option<JoinHandle<()>>>,
    known: Arc<RwLock<HashMap<String, WatchedTool>>>,
    loader: Arc<WasmToolLoader<R>>,
    registry: Arc<R>,
}

impl<R> ToolWatcher<R>
where
    R: WasmToolRegistrar + RegistryUnregister + 'static,
{
    /// Create a new watcher for the installed tools directory.
    pub fn new(install_dir: PathBuf, loader: Arc<WasmToolLoader<R>>, registry: Arc<R>) -> Self {
        Self {
            install_dir,
            config: ToolWatcherConfig::default(),
            task_handle: RwLock::new(None),
            known: Arc::new(RwLock::new(HashMap::new())),
            loader,
            registry,
        }
    }

    /// Override the default watcher configuration.
    pub fn with_config(mut self, config: ToolWatcherConfig) -> Self {
        self.config = config;
        self
    }

    /// Seed the known tool map from the current tool sources.
    pub async fn seed_from_sources(&self) {
        let sources = match scan_current_sources(
            &self.install_dir,
            self.config.include_dev_artifacts,
        )
        .await
        {
            Ok(sources) => sources,
            Err(error) => {
                tracing::warn!(
                    dir = %self.install_dir.display(),
                    error = %error,
                    "Failed to seed tool watcher"
                );
                return;
            }
        };

        let mut known = self.known.write().await;
        let now = SystemTime::now();
        known.clear();
        for (name, source) in sources {
            known.insert(
                name,
                WatchedTool {
                    source,
                    last_reload: now,
                },
            );
        }

        tracing::info!(
            dir = %self.install_dir.display(),
            known_tools = known.len(),
            include_dev_artifacts = self.config.include_dev_artifacts,
            "Tool watcher seeded with existing tools"
        );
    }

    /// Start polling for changes.
    pub async fn start(&self) {
        let install_dir = self.install_dir.clone();
        let config = self.config.clone();
        let known = Arc::clone(&self.known);
        let loader = Arc::clone(&self.loader);
        let registry = Arc::clone(&self.registry);

        let handle = tokio::spawn(async move {
            tracing::info!(
                dir = %install_dir.display(),
                poll_secs = config.poll_interval.as_secs_f64(),
                include_dev_artifacts = config.include_dev_artifacts,
                "WASM tool hot-reload watcher started"
            );

            loop {
                tokio::time::sleep(config.poll_interval).await;

                if let Err(error) =
                    Self::poll_once(&install_dir, &config, &known, &loader, &registry).await
                {
                    tracing::warn!(error = %error, "Tool watcher poll error");
                }
            }
        });

        *self.task_handle.write().await = Some(handle);
    }

    /// Stop watching.
    pub async fn stop(&self) {
        if let Some(handle) = self.task_handle.write().await.take() {
            handle.abort();
            tracing::info!(dir = %self.install_dir.display(), "WASM tool hot-reload watcher stopped");
        }
    }

    async fn poll_once(
        install_dir: &Path,
        config: &ToolWatcherConfig,
        known: &Arc<RwLock<HashMap<String, WatchedTool>>>,
        loader: &Arc<WasmToolLoader<R>>,
        registry: &Arc<R>,
    ) -> Result<(), String> {
        let current = scan_current_sources(install_dir, config.include_dev_artifacts)
            .await
            .map_err(|error| error.to_string())?;

        let mut known_guard = known.write().await;
        let now = SystemTime::now();

        for (name, source) in &current {
            let changed = match known_guard.get(name) {
                None => true,
                Some(watched) => {
                    watched.source.mtime != source.mtime
                        || watched.source.wasm_path != source.wasm_path
                }
            };

            if !changed {
                continue;
            }

            let debounced = known_guard
                .get(name)
                .and_then(|watched| now.duration_since(watched.last_reload).ok())
                .is_some_and(|elapsed| elapsed < config.debounce);
            if debounced {
                continue;
            }

            tracing::info!(
                tool = %name,
                wasm_path = %source.wasm_path.display(),
                "WASM tool change detected, reloading"
            );

            match loader
                .load_from_files(name, &source.wasm_path, source.capabilities_path.as_deref())
                .await
            {
                Ok(()) => {
                    known_guard.insert(
                        name.clone(),
                        WatchedTool {
                            source: source.clone(),
                            last_reload: now,
                        },
                    );
                    tracing::info!(tool = %name, "WASM tool hot-reloaded successfully");
                }
                Err(error) => {
                    tracing::error!(
                        tool = %name,
                        wasm_path = %source.wasm_path.display(),
                        error = %error,
                        "Failed to hot-reload WASM tool"
                    );
                    known_guard.insert(
                        name.clone(),
                        WatchedTool {
                            source: source.clone(),
                            last_reload: now,
                        },
                    );
                }
            }
        }

        let removed: Vec<String> = known_guard
            .keys()
            .filter(|name| !current.contains_key(*name))
            .cloned()
            .collect();

        for name in removed {
            tracing::info!(tool = %name, "WASM tool source deleted, unregistering");
            registry.unregister(&name).await;
            known_guard.remove(&name);
        }

        Ok(())
    }
}

async fn scan_current_sources(
    install_dir: &Path,
    include_dev_artifacts: bool,
) -> Result<HashMap<String, ToolSource>, std::io::Error> {
    let installed = discover_tools(install_dir).await?;
    let dev = if include_dev_artifacts {
        discover_dev_tools().await?
    } else {
        HashMap::new()
    };

    let mut combined = HashMap::new();

    for (name, discovered) in installed {
        combined.insert(
            name,
            ToolSource {
                mtime: metadata_mtime(&discovered.wasm_path).await?,
                wasm_path: discovered.wasm_path,
                capabilities_path: discovered.capabilities_path,
            },
        );
    }

    for (name, discovered) in dev {
        let dev_source = ToolSource {
            mtime: metadata_mtime(&discovered.wasm_path).await?,
            wasm_path: discovered.wasm_path,
            capabilities_path: discovered.capabilities_path,
        };

        match combined.get(&name) {
            Some(existing) if existing.mtime >= dev_source.mtime => {}
            _ => {
                combined.insert(name, dev_source);
            }
        }
    }

    Ok(combined)
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
        let config = ToolWatcherConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(3));
        assert_eq!(config.debounce, Duration::from_secs(1));
        assert!(config.include_dev_artifacts);
    }
}

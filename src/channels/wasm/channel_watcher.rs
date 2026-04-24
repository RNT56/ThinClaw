//! WASM channel hot-reload watcher.
//!
//! Monitors `~/.thinclaw/channels/` for `.wasm` file changes and automatically
//! loads, reloads, or unloads WASM channels without requiring a restart.
//!
//! Uses mtime-based polling (same approach as [`crate::config::watcher`]) to
//! detect changes. This avoids the `notify` crate dependency while providing
//! reliable cross-platform file watching.
//!
//! # Events
//!
//! - **New `.wasm` file** → Load channel, call `on_start`, register with `ChannelManager`
//! - **Modified `.wasm` file** → Shutdown old channel, load new, swap in `ChannelManager`
//! - **Deleted `.wasm` file** → Shutdown channel, remove from `ChannelManager`
//!
//! # Usage
//!
//! ```rust,ignore
//! let watcher = ChannelWatcher::new(channels_dir, loader, channel_manager);
//! watcher.start().await;
//! // ... WASM files can now be added/removed at runtime
//! watcher.stop().await;
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::channels::manager::ChannelManager;
use crate::channels::wasm::loader::WasmChannelLoader;
use crate::channels::wasm::router::WasmChannelRouter;
use crate::channels::wasm::{
    RegisteredWebhookAuth, SharedWasmChannel, WasmChannelHostConfig, apply_channel_host_config,
    inject_channel_credentials_from_secrets,
};
use crate::secrets::SecretsStore;

/// Configuration for the channel watcher.
#[derive(Debug, Clone)]
pub struct ChannelWatcherConfig {
    /// How often to poll for changes (default: 3s).
    pub poll_interval: Duration,
    /// Debounce period — min time between reloads of the same file (default: 1s).
    pub debounce: Duration,
}

impl Default for ChannelWatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            debounce: Duration::from_secs(1),
        }
    }
}

/// Tracks the state of a watched `.wasm` file.
#[derive(Debug, Clone)]
struct WatchedChannel {
    /// Last known modification time.
    mtime: SystemTime,
    /// Last time we processed a change for this file.
    last_reload: SystemTime,
}

/// Watches a channels directory and hot-reloads WASM channels.
pub struct ChannelWatcher {
    /// Directory being watched.
    dir: PathBuf,
    /// Watcher configuration.
    config: ChannelWatcherConfig,
    /// Background task handle.
    task_handle: RwLock<Option<JoinHandle<()>>>,
    /// Known channels with their mtimes.
    known: Arc<RwLock<HashMap<String, WatchedChannel>>>,
    /// Channel loader for WASM modules.
    loader: Arc<WasmChannelLoader>,
    /// Channel manager for hot-add/remove.
    channel_manager: Arc<ChannelManager>,
    /// Webhook router for updating routes on channel add/remove.
    webhook_router: Option<Arc<WasmChannelRouter>>,
    /// Optional secrets store for restoring webhook secrets on hot-added channels.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// User scope for secret lookup.
    user_id: String,
    /// Host runtime values that must be re-applied to hot-loaded channels.
    host_config: WasmChannelHostConfig,
}

impl ChannelWatcher {
    /// Create a new channel watcher.
    pub fn new(
        dir: PathBuf,
        loader: Arc<WasmChannelLoader>,
        channel_manager: Arc<ChannelManager>,
    ) -> Self {
        Self {
            dir,
            config: ChannelWatcherConfig::default(),
            task_handle: RwLock::new(None),
            known: Arc::new(RwLock::new(HashMap::new())),
            loader,
            channel_manager,
            webhook_router: None,
            secrets_store: None,
            user_id: "default".to_string(),
            host_config: WasmChannelHostConfig::default(),
        }
    }

    /// Set the webhook router for updating routes on channel changes.
    pub fn with_webhook_router(mut self, router: Arc<WasmChannelRouter>) -> Self {
        self.webhook_router = Some(router);
        self
    }

    /// Set the secrets store used to restore webhook secrets for hot-added channels.
    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        self.secrets_store = Some(store);
        self.user_id = user_id.into();
        self
    }

    /// Set host runtime values that must be injected into hot-loaded channels.
    pub fn with_host_config(mut self, host_config: WasmChannelHostConfig) -> Self {
        self.host_config = host_config;
        self
    }

    /// Set custom configuration.
    pub fn with_config(mut self, config: ChannelWatcherConfig) -> Self {
        self.config = config;
        self
    }

    /// Seed the known channels from the currently loaded channels.
    ///
    /// Call this after initial channel loading to establish the baseline.
    pub async fn seed_from_dir(&self) {
        let mut known = self.known.write().await;
        if let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                    continue;
                }
                if let Some(name) = path.file_stem().and_then(|s| s.to_str())
                    && let Ok(metadata) = tokio::fs::metadata(&path).await
                    && let Ok(mtime) = metadata.modified()
                {
                    known.insert(
                        name.to_string(),
                        WatchedChannel {
                            mtime,
                            last_reload: SystemTime::UNIX_EPOCH,
                        },
                    );
                }
            }
        }
        tracing::info!(
            dir = %self.dir.display(),
            known_channels = known.len(),
            "Channel watcher seeded with existing channels"
        );
    }

    /// Start watching for changes.
    pub async fn start(&self) {
        let dir = self.dir.clone();
        let config = self.config.clone();
        let known = Arc::clone(&self.known);
        let loader = Arc::clone(&self.loader);
        let channel_manager = Arc::clone(&self.channel_manager);
        let webhook_router = self.webhook_router.clone();
        let secrets_store = self.secrets_store.clone();
        let user_id = self.user_id.clone();
        let host_config = self.host_config.clone();

        let handle = tokio::spawn(async move {
            tracing::info!(
                dir = %dir.display(),
                poll_secs = config.poll_interval.as_secs_f64(),
                "Channel hot-reload watcher started"
            );

            loop {
                tokio::time::sleep(config.poll_interval).await;

                if let Err(e) = Self::poll_once(
                    &dir,
                    &config,
                    &known,
                    &loader,
                    &channel_manager,
                    webhook_router.as_ref(),
                    secrets_store.as_deref(),
                    &user_id,
                    &host_config,
                )
                .await
                {
                    tracing::warn!(error = %e, "Channel watcher poll error");
                }
            }
        });

        *self.task_handle.write().await = Some(handle);
    }

    /// Stop watching.
    pub async fn stop(&self) {
        if let Some(handle) = self.task_handle.write().await.take() {
            handle.abort();
            tracing::info!(dir = %self.dir.display(), "Channel hot-reload watcher stopped");
        }
    }

    /// Perform a single poll cycle.
    async fn poll_once(
        dir: &Path,
        config: &ChannelWatcherConfig,
        known: &Arc<RwLock<HashMap<String, WatchedChannel>>>,
        loader: &Arc<WasmChannelLoader>,
        channel_manager: &Arc<ChannelManager>,
        webhook_router: Option<&Arc<WasmChannelRouter>>,
        secrets_store: Option<&(dyn SecretsStore + Send + Sync)>,
        user_id: &str,
        host_config: &WasmChannelHostConfig,
    ) -> Result<(), String> {
        // Scan current .wasm files
        let mut current_files: HashMap<String, SystemTime> = HashMap::new();

        if !dir.is_dir() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(dir)
            .await
            .map_err(|e| format!("read_dir failed: {}", e))?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|s| s.to_str())
                && let Ok(metadata) = tokio::fs::metadata(&path).await
                && let Ok(mtime) = metadata.modified()
            {
                current_files.insert(name.to_string(), mtime);
            }
        }

        let mut known_guard = known.write().await;
        let now = SystemTime::now();

        // Detect new and modified channels
        for (name, mtime) in &current_files {
            match known_guard.get(name) {
                None => {
                    // New channel
                    tracing::info!(channel = %name, "New WASM channel detected, loading...");
                    match Self::load_and_add(
                        dir,
                        name,
                        loader,
                        channel_manager,
                        webhook_router,
                        secrets_store,
                        user_id,
                        host_config,
                    )
                    .await
                    {
                        Ok(()) => {
                            known_guard.insert(
                                name.clone(),
                                WatchedChannel {
                                    mtime: *mtime,
                                    last_reload: now,
                                },
                            );
                            tracing::info!(channel = %name, "WASM channel hot-loaded successfully");
                        }
                        Err(e) => {
                            tracing::error!(channel = %name, error = %e, "Failed to hot-load WASM channel");
                        }
                    }
                }
                Some(watched) => {
                    if *mtime != watched.mtime {
                        // Modified — check debounce
                        let since_last = now
                            .duration_since(watched.last_reload)
                            .unwrap_or(Duration::from_secs(999));

                        if since_last >= config.debounce {
                            tracing::info!(channel = %name, "WASM channel modified, reloading...");

                            // Remove old
                            if let Err(e) = channel_manager.hot_remove(name).await {
                                tracing::warn!(channel = %name, error = %e, "Error removing old channel during reload");
                            }
                            if let Some(router) = webhook_router {
                                router.unregister(name).await;
                            }
                            loader.invalidate(name).await;

                            // Load new
                            match Self::load_and_add(
                                dir,
                                name,
                                loader,
                                channel_manager,
                                webhook_router,
                                secrets_store,
                                user_id,
                                host_config,
                            )
                            .await
                            {
                                Ok(()) => {
                                    known_guard.insert(
                                        name.clone(),
                                        WatchedChannel {
                                            mtime: *mtime,
                                            last_reload: now,
                                        },
                                    );
                                    tracing::info!(channel = %name, "WASM channel hot-reloaded successfully");
                                }
                                Err(e) => {
                                    tracing::error!(channel = %name, error = %e, "Failed to hot-reload WASM channel");
                                    // Update mtime to avoid retry loop
                                    known_guard.insert(
                                        name.clone(),
                                        WatchedChannel {
                                            mtime: *mtime,
                                            last_reload: now,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Detect deleted channels
        let removed: Vec<String> = known_guard
            .keys()
            .filter(|name| !current_files.contains_key(*name))
            .cloned()
            .collect();

        for name in removed {
            tracing::info!(channel = %name, "WASM channel file deleted, removing...");
            if let Err(e) = channel_manager.hot_remove(&name).await {
                tracing::warn!(channel = %name, error = %e, "Error removing deleted channel");
            }
            if let Some(router) = webhook_router {
                router.unregister(&name).await;
            }
            loader.invalidate(&name).await;
            known_guard.remove(&name);
            tracing::info!(channel = %name, "WASM channel hot-removed");
        }

        Ok(())
    }

    /// Load a WASM channel from disk and hot-add it to the channel manager.
    async fn load_and_add(
        dir: &Path,
        name: &str,
        loader: &Arc<WasmChannelLoader>,
        channel_manager: &Arc<ChannelManager>,
        webhook_router: Option<&Arc<WasmChannelRouter>>,
        secrets_store: Option<&(dyn SecretsStore + Send + Sync)>,
        user_id: &str,
        host_config: &WasmChannelHostConfig,
    ) -> Result<(), String> {
        let wasm_path = dir.join(format!("{}.wasm", name));
        let cap_path = dir.join(format!("{}.capabilities.json", name));
        let cap_ref = if cap_path.exists() {
            Some(cap_path.as_path())
        } else {
            None
        };

        let loaded = loader
            .load_from_files(name, &wasm_path, cap_ref)
            .await
            .map_err(|e| format!("load failed: {}", e))?;

        let secret_header = loaded.webhook_secret_header().map(str::to_string);
        let signature_secret_name = loaded.webhook_secret_name();
        let verify_token_secret_name = loaded.webhook_verify_token_secret_name();
        let secret_validation = loaded.webhook_secret_validation();
        let verify_token_param = loaded.webhook_verify_token_param().map(str::to_string);
        let channel_name = loaded.name().to_string();
        let channel_arc = Arc::new(loaded.channel);

        let signature_secret = match secrets_store {
            Some(store) => store
                .get_for_injection(
                    user_id,
                    &signature_secret_name,
                    crate::secrets::SecretAccessContext::new(
                        "wasm.channel_watcher",
                        "webhook_signature_validation",
                    ),
                )
                .await
                .ok()
                .map(|secret| secret.expose().to_string()),
            None => None,
        };

        let verify_token_secret = match (verify_token_secret_name.as_ref(), secrets_store) {
            (Some(secret_name), _) if *secret_name == signature_secret_name => {
                signature_secret.clone()
            }
            (Some(secret_name), Some(store)) => store
                .get_for_injection(
                    user_id,
                    secret_name,
                    crate::secrets::SecretAccessContext::new(
                        "wasm.channel_watcher",
                        "webhook_verify_token",
                    ),
                )
                .await
                .ok()
                .map(|secret| secret.expose().to_string()),
            _ => None,
        };

        let webhook_auth = RegisteredWebhookAuth {
            secret_header: secret_header.clone(),
            secret_validation,
            signature_secret: signature_secret.clone(),
            verify_token_param,
            verify_token_secret,
        };

        let runtime_update_count = apply_channel_host_config(
            &channel_arc,
            &channel_name,
            host_config,
            signature_secret.as_deref(),
        )
        .await;
        if runtime_update_count > 0 {
            tracing::info!(
                channel = %channel_name,
                runtime_updates = runtime_update_count,
                "Injected host runtime config into hot-loaded channel"
            );
        }

        if let Some(store) = secrets_store {
            match inject_channel_credentials_from_secrets(
                &channel_arc,
                store,
                &channel_name,
                user_id,
            )
            .await
            {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            channel = %channel_name,
                            credentials_injected = count,
                            "Injected credentials into hot-loaded channel"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        channel = %channel_name,
                        error = %error,
                        "Failed to inject credentials into hot-loaded channel"
                    );
                }
            }
        }

        if let Err(error) = channel_arc.prime_on_start_config().await {
            tracing::warn!(
                channel = %channel_name,
                error = %error,
                "Failed to prime hot-loaded channel on_start config before registration"
            );
        }

        channel_manager
            .hot_add(Box::new(SharedWasmChannel::new(Arc::clone(&channel_arc))))
            .await
            .map_err(|e| format!("hot_add failed: {}", e))?;

        if let Some(router) = webhook_router {
            let endpoints = channel_arc.endpoints().await;
            router.register(channel_arc, endpoints, webhook_auth).await;
        }

        Ok(())
    }

    /// Manually trigger a reload check.
    ///
    /// Useful for SIGHUP-triggered reloads.
    pub async fn check_now(&self) -> Result<(), String> {
        Self::poll_once(
            &self.dir,
            &self.config,
            &self.known,
            &self.loader,
            &self.channel_manager,
            self.webhook_router.as_ref(),
            self.secrets_store.as_deref(),
            &self.user_id,
            &self.host_config,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ChannelWatcherConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(3));
        assert_eq!(config.debounce, Duration::from_secs(1));
    }
}

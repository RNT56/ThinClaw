//! Compatibility adapter for the extracted WASM channel hot-reload watcher.

use std::path::PathBuf;
use std::sync::Arc;

pub use thinclaw_channels::wasm::channel_watcher::ChannelWatcherConfig;

use crate::channels::ChannelManager;
use crate::channels::wasm::{WasmChannelHostConfig, WasmChannelLoader, WasmChannelRouter};
use crate::secrets::SecretsStore;

pub struct ChannelWatcher {
    inner: thinclaw_channels::wasm::channel_watcher::ChannelWatcher,
}

impl ChannelWatcher {
    pub fn new(
        dir: PathBuf,
        loader: Arc<WasmChannelLoader>,
        channel_manager: Arc<ChannelManager>,
    ) -> Self {
        Self {
            inner: thinclaw_channels::wasm::channel_watcher::ChannelWatcher::new(
                dir,
                loader,
                channel_manager,
            ),
        }
    }

    pub fn with_webhook_router(mut self, router: Arc<WasmChannelRouter>) -> Self {
        self.inner = self.inner.with_webhook_router(router);
        self
    }

    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        self.inner = self.inner.with_secrets_store(store, user_id);
        self
    }

    pub fn with_host_config(mut self, host_config: WasmChannelHostConfig) -> Self {
        self.inner = self.inner.with_host_config(host_config.as_core());
        self
    }

    pub fn with_config(mut self, config: ChannelWatcherConfig) -> Self {
        self.inner = self.inner.with_config(config);
        self
    }

    pub async fn seed_from_dir(&self) {
        self.inner.seed_from_dir().await;
    }

    pub async fn start(&self) {
        self.inner.start().await;
    }

    pub async fn stop(&self) {
        self.inner.stop().await;
    }
}

//! WASM-extensible channel system.
//!
//! This module provides a runtime for executing WASM-based channels using a
//! Host-Managed Event Loop pattern. The host (Rust) manages infrastructure
//! (HTTP server, polling), while WASM modules define channel behavior through
//! callbacks.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────────┐
//! │                          Host-Managed Event Loop                                 │
//! │                                                                                  │
//! │   ┌─────────────┐     ┌──────────────┐     ┌──────────────┐                     │
//! │   │   HTTP      │     │   Polling    │     │   Timer      │                     │
//! │   │   Router    │     │   Scheduler  │     │   Scheduler  │                     │
//! │   └──────┬──────┘     └──────┬───────┘     └──────┬───────┘                     │
//! │          │                   │                    │                              │
//! │          └───────────────────┴────────────────────┘                              │
//! │                              │                                                   │
//! │                              ▼                                                   │
//! │                    ┌─────────────────┐                                           │
//! │                    │   Event Router  │                                           │
//! │                    └────────┬────────┘                                           │
//! │                             │                                                    │
//! │          ┌──────────────────┼──────────────────┐                                │
//! │          ▼                  ▼                  ▼                                 │
//! │   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐                           │
//! │   │ on_http_req │   │  on_poll    │   │ on_respond  │  WASM Exports             │
//! │   └─────────────┘   └─────────────┘   └─────────────┘                           │
//! │          │                  │                  │                                 │
//! │          └──────────────────┴──────────────────┘                                │
//! │                             │                                                    │
//! │                             ▼                                                    │
//! │                    ┌─────────────────┐                                           │
//! │                    │  Host Imports   │                                           │
//! │                    │  emit_message   │──────────▶ MessageStream                 │
//! │                    │  http_request   │                                           │
//! │                    │  log, etc.      │                                           │
//! │                    └─────────────────┘                                           │
//! └─────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Design Decisions
//!
//! 1. **Fresh Instance Per Callback** (NEAR Pattern) - Full isolation, no shared mutable state
//! 2. **Host Manages Infrastructure** - HTTP server, polling, timing in Rust
//! 3. **WASM Defines Behavior** - Callbacks for events, message parsing, response handling
//! 4. **Reuse Tool Runtime** - Share Wasmtime engine, extend capabilities
//!
//! # Security Model
//!
//! | Threat | Mitigation |
//! |--------|------------|
//! | Path hijacking | `allowed_paths` restricts registrable endpoints |
//! | Token exposure | Injected at host boundary, WASM never sees |
//! | State pollution | Fresh instance per callback |
//! | Workspace escape | Paths prefixed with `channels/<name>/` |
//! | Message spam | Rate limiting on `emit_message` |
//! | Resource exhaustion | Fuel metering, memory limits, callback timeout |
//! | Polling abuse | Minimum 30s interval enforced |
//!
//! # Example Usage
//!
//! ```ignore
//! use thinclaw::channels::wasm::{WasmChannelLoader, WasmChannelRuntime};
//!
//! // Create runtime (can share engine with tool runtime)
//! let runtime = WasmChannelRuntime::new(config)?;
//!
//! // Load channels from directory
//! let loader = WasmChannelLoader::new(runtime);
//! let channels = loader.load_from_dir(Path::new("~/.thinclaw/channels/")).await?;
//!
//! // Add to channel manager
//! for channel in channels {
//!     manager.add(Box::new(channel));
//! }
//! ```

mod bundled;
mod capabilities;
#[cfg(feature = "wasm-runtime")]
pub mod channel_watcher;
mod error;
#[cfg(feature = "wasm-runtime")]
mod host;
#[cfg(feature = "wasm-runtime")]
mod loader;
#[cfg(feature = "wasm-runtime")]
pub mod router;
#[cfg(feature = "wasm-runtime")]
mod runtime;
#[cfg(feature = "wasm-runtime")]
mod runtime_config;
mod schema;
pub(crate) mod telegram_html;
#[cfg(feature = "wasm-runtime")]
mod wrapper;

// Core types
pub use bundled::{available_channel_names, bundled_channel_names, install_bundled_channel};
pub use capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, PollConfig, ToolCapabilities,
};
pub use error::WasmChannelError;
#[cfg(feature = "wasm-runtime")]
pub use host::{ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MediaAttachment};
#[cfg(feature = "wasm-runtime")]
pub use loader::{
    DiscoveredChannel, LoadResults, LoadedChannel, WasmChannelLoader, default_channels_dir,
    discover_channels,
};
#[cfg(feature = "wasm-runtime")]
pub use router::{
    RegisteredEndpoint, RegisteredWebhookAuth, WasmChannelRouter, create_wasm_channel_router,
};
#[cfg(feature = "wasm-runtime")]
pub use runtime::{PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig};
#[cfg(feature = "wasm-runtime")]
pub use runtime_config::{
    WasmChannelHostConfig, apply_channel_host_config, inject_channel_credentials_from_secrets,
};
pub use schema::{
    ChannelCapabilitiesFile, ChannelConfig, SecretSetupSchema, SetupSchema, WebhookSchema,
    WebhookSecretValidation,
};
#[cfg(feature = "wasm-runtime")]
pub use thinclaw_channels::wasm::{FuelConfig, ResourceLimits};
#[cfg(feature = "wasm-runtime")]
pub use wrapper::{HttpResponse, SharedWasmChannel, WasmChannel};

#[cfg(not(feature = "wasm-runtime"))]
pub use stubs::*;

#[cfg(not(feature = "wasm-runtime"))]
pub mod router {
    pub use super::stubs::{RegisteredEndpoint, RegisteredWebhookAuth, WasmChannelRouter};

    pub fn create_wasm_channel_router(
        router: std::sync::Arc<WasmChannelRouter>,
        extension_manager: Option<std::sync::Arc<crate::extensions::ExtensionManager>>,
    ) -> axum::Router {
        super::stubs::create_wasm_channel_router(router, extension_manager)
    }
}

#[cfg(not(feature = "wasm-runtime"))]
pub mod channel_watcher {
    pub use super::stubs::{ChannelWatcher, ChannelWatcherConfig};
}

#[cfg(not(feature = "wasm-runtime"))]
mod stubs {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use async_trait::async_trait;

    use crate::channels::{
        Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
    };
    use thinclaw_types::error::ChannelError;

    use super::WasmChannelError;

    #[derive(Debug, Clone, Default)]
    pub struct FuelConfig;

    #[derive(Debug, Clone, Default)]
    pub struct ResourceLimits;

    #[derive(Debug, Clone, Default)]
    pub struct RegisteredEndpoint {
        pub path: String,
    }

    #[derive(Debug, Clone, Default)]
    pub struct RegisteredWebhookAuth {
        pub secret_header: Option<String>,
        pub secret_validation: super::WebhookSecretValidation,
        pub signature_secret: Option<String>,
        pub verify_token_param: Option<String>,
        pub verify_token_secret: Option<String>,
    }

    impl RegisteredWebhookAuth {
        pub fn has_any_secret(&self) -> bool {
            self.signature_secret.is_some() || self.verify_token_secret.is_some()
        }
    }

    #[derive(Debug, Default)]
    pub struct WasmChannelRouter;

    impl WasmChannelRouter {
        pub fn new() -> Self {
            Self
        }

        pub async fn register(
            &self,
            _channel: Arc<WasmChannel>,
            _endpoints: Vec<RegisteredEndpoint>,
            _auth: RegisteredWebhookAuth,
        ) {
        }

        pub async fn update_webhook_auth(&self, _channel_name: &str, _auth: RegisteredWebhookAuth) {
        }

        pub async fn get_channel_for_path(&self, _path: &str) -> Option<Arc<WasmChannel>> {
            None
        }
    }

    pub fn create_wasm_channel_router(
        _router: Arc<WasmChannelRouter>,
        _extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    ) -> axum::Router {
        axum::Router::new()
    }

    #[derive(Debug, Clone, Default)]
    pub struct WasmChannelRuntimeConfig;

    #[derive(Debug, Clone, Default)]
    pub struct WasmChannelRuntime;

    impl WasmChannelRuntime {
        pub fn new(_config: WasmChannelRuntimeConfig) -> Result<Self, WasmChannelError> {
            Err(WasmChannelError::Config(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }
    }

    #[derive(Debug)]
    pub struct WasmChannelLoader;

    impl WasmChannelLoader {
        pub fn new(
            _runtime: Arc<WasmChannelRuntime>,
            _pairing_store: Arc<crate::pairing::PairingStore>,
        ) -> Self {
            Self
        }

        pub async fn load_from_dir(&self, _dir: &Path) -> Result<LoadResults, WasmChannelError> {
            Err(WasmChannelError::Config(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }

        pub async fn load_from_files(
            &self,
            name: &str,
            _wasm_path: &Path,
            _capabilities_path: Option<&Path>,
        ) -> Result<LoadedChannel, WasmChannelError> {
            Err(WasmChannelError::Config(format!(
                "WASM channel runtime is not available for '{}'",
                name
            )))
        }
    }

    #[derive(Debug, Default)]
    pub struct LoadResults {
        pub loaded: Vec<LoadedChannel>,
        pub errors: Vec<(PathBuf, WasmChannelError)>,
    }

    #[derive(Debug)]
    pub struct DiscoveredChannel {
        pub name: String,
        pub wasm_path: PathBuf,
        pub capabilities_path: Option<PathBuf>,
    }

    pub async fn discover_channels(
        _dir: &Path,
    ) -> Result<std::collections::HashMap<String, DiscoveredChannel>, std::io::Error> {
        Ok(std::collections::HashMap::new())
    }

    pub fn default_channels_dir() -> PathBuf {
        crate::platform::state_paths().channels_dir
    }

    #[derive(Debug)]
    pub struct LoadedChannel {
        name: String,
        pub channel: WasmChannel,
    }

    impl LoadedChannel {
        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn webhook_secret_name(&self) -> String {
            format!("{}_webhook_secret", self.name)
        }

        pub fn webhook_verify_token_secret_name(&self) -> Option<String> {
            None
        }

        pub fn webhook_secret_header(&self) -> Option<&str> {
            None
        }

        pub fn webhook_secret_validation(&self) -> super::WebhookSecretValidation {
            super::WebhookSecretValidation::default()
        }

        pub fn webhook_verify_token_param(&self) -> Option<&str> {
            None
        }
    }

    #[derive(Debug)]
    pub struct WasmChannel {
        name: String,
    }

    impl WasmChannel {
        pub fn name(&self) -> &str {
            &self.name
        }

        pub async fn endpoints(&self) -> Vec<RegisteredEndpoint> {
            Vec::new()
        }

        pub async fn prime_on_start_config(&self) -> Result<serde_json::Value, WasmChannelError> {
            Err(WasmChannelError::Config(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }

        pub async fn refresh_on_start_config(&self) -> Result<serde_json::Value, WasmChannelError> {
            Err(WasmChannelError::Config(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }
    }

    pub type HttpResponse = serde_json::Value;

    #[derive(Debug, Clone)]
    pub struct SharedWasmChannel {
        inner: Arc<WasmChannel>,
    }

    impl SharedWasmChannel {
        pub fn new(inner: Arc<WasmChannel>) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl Channel for SharedWasmChannel {
        fn name(&self) -> &str {
            self.inner.name()
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Err(ChannelError::Configuration(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Err(ChannelError::Configuration(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }

        async fn send_status(
            &self,
            _status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        fn stream_mode(&self) -> StreamMode {
            StreamMode::None
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Err(ChannelError::Configuration(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct WasmChannelHostConfig {
        pub tunnel_url: Option<String>,
        pub telegram_tunnel_url: Option<String>,
        pub telegram_owner_id: Option<i64>,
        pub telegram_stream_mode: Option<String>,
        pub telegram_transport_mode: String,
        pub telegram_host_webhook_capable: bool,
        pub telegram_host_transport_reason: Option<String>,
        pub discord_stream_mode: Option<String>,
    }

    impl WasmChannelHostConfig {
        pub fn from_config(config: &crate::config::Config) -> Self {
            Self {
                tunnel_url: config.tunnel.public_url.clone(),
                telegram_tunnel_url: None,
                telegram_owner_id: config.channels.telegram_owner_id,
                telegram_stream_mode: config.channels.telegram_stream_mode.clone(),
                telegram_transport_mode: config.channels.telegram_transport_mode.clone(),
                telegram_host_webhook_capable: false,
                telegram_host_transport_reason: Some(
                    "ThinClaw was built without the wasm-runtime feature".to_string(),
                ),
                discord_stream_mode: config.channels.discord_stream_mode.clone(),
            }
        }
    }

    pub async fn apply_channel_host_config(
        _channel: &Arc<WasmChannel>,
        _channel_name: &str,
        _host_config: &WasmChannelHostConfig,
        _webhook_secret: Option<&str>,
    ) -> usize {
        0
    }

    pub async fn inject_channel_credentials_from_secrets(
        _channel: &Arc<WasmChannel>,
        _secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
        _channel_name: &str,
        _user_id: &str,
    ) -> Result<usize, WasmChannelError> {
        Ok(0)
    }

    #[derive(Debug, Clone, Default)]
    pub struct ChannelWatcherConfig;

    pub struct ChannelWatcher;

    impl ChannelWatcher {
        pub fn new(
            _channels_dir: PathBuf,
            _loader: Arc<WasmChannelLoader>,
            _manager: Arc<crate::channels::ChannelManager>,
        ) -> Self {
            Self
        }

        pub fn with_webhook_router(self, _router: Arc<WasmChannelRouter>) -> Self {
            self
        }

        pub fn with_host_config(self, _host_config: WasmChannelHostConfig) -> Self {
            self
        }

        pub fn with_secrets_store(
            self,
            _secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
            _user_id: &str,
        ) -> Self {
            self
        }

        pub async fn seed_from_dir(&self) {}

        pub async fn start(&self) {}
    }
}

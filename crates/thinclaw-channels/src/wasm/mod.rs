//! Root-independent WASM channel runtime pieces.

pub mod bundled;
pub mod capabilities;
#[cfg(feature = "wasm-runtime")]
pub mod channel_watcher;
pub mod error;
#[cfg(feature = "wasm-runtime")]
pub mod host;
#[cfg(feature = "wasm-runtime")]
pub mod limits;
#[cfg(feature = "wasm-runtime")]
pub mod loader;
#[cfg(feature = "wasm-runtime")]
pub mod router;
#[cfg(feature = "wasm-runtime")]
pub mod runtime;
#[cfg(feature = "wasm-runtime")]
pub mod runtime_config;
pub mod schema;
pub mod telegram_html;
#[cfg(feature = "wasm-runtime")]
pub mod wrapper;

pub use bundled::{available_channel_names, bundled_channel_names, install_bundled_channel};
pub use capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, PollConfig, ToolCapabilities,
};
pub use error::WasmChannelError;
#[cfg(feature = "wasm-runtime")]
pub use host::{ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MediaAttachment};
#[cfg(feature = "wasm-runtime")]
pub use limits::{FuelConfig, ResourceLimits, WasmResourceLimiter};
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
pub use wrapper::{HttpResponse, SharedWasmChannel, WasmChannel};

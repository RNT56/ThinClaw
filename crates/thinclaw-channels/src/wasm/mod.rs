//! Root-independent WASM channel runtime pieces.

pub mod bundled;
pub mod capabilities;
pub mod channel_watcher;
pub mod error;
pub mod host;
pub mod limits;
pub mod loader;
pub mod router;
pub mod runtime;
pub mod runtime_config;
pub mod schema;
pub mod telegram_html;
pub mod wrapper;

pub use bundled::{available_channel_names, bundled_channel_names, install_bundled_channel};
pub use capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, PollConfig, ToolCapabilities,
};
pub use error::WasmChannelError;
pub use host::{ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MediaAttachment};
pub use limits::{FuelConfig, ResourceLimits, WasmResourceLimiter};
pub use loader::{
    DiscoveredChannel, LoadResults, LoadedChannel, WasmChannelLoader, default_channels_dir,
    discover_channels,
};
pub use router::{
    RegisteredEndpoint, RegisteredWebhookAuth, WasmChannelRouter, create_wasm_channel_router,
};
pub use runtime::{PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig};
pub use runtime_config::{
    WasmChannelHostConfig, apply_channel_host_config, inject_channel_credentials_from_secrets,
};
pub use schema::{
    ChannelCapabilitiesFile, ChannelConfig, SecretSetupSchema, SetupSchema, WebhookSchema,
    WebhookSecretValidation,
};
pub use wrapper::{HttpResponse, SharedWasmChannel, WasmChannel};

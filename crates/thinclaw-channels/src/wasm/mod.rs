//! Root-independent WASM channel runtime pieces.

pub mod bundled;
pub mod capabilities;
pub mod error;
pub mod limits;
pub mod runtime;
pub mod schema;
pub mod telegram_html;

pub use bundled::{available_channel_names, bundled_channel_names, install_bundled_channel};
pub use capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, PollConfig, ToolCapabilities,
};
pub use error::WasmChannelError;
pub use limits::{FuelConfig, ResourceLimits};
pub use runtime::{PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig};
pub use schema::{
    ChannelCapabilitiesFile, ChannelConfig, SecretSetupSchema, SetupSchema, WebhookSchema,
    WebhookSecretValidation,
};

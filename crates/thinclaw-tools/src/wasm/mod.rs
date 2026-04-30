//! Root-independent WASM tool runtime and configuration types.

pub mod allowlist;
pub mod capabilities;
pub mod capabilities_schema;
pub mod error;
pub mod host;
pub mod limits;
pub mod rate_limiter;
pub mod runtime;

pub use allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WorkspaceCapability, WorkspaceReader,
};
pub use capabilities_schema::{
    AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema, RateLimitSchema,
    ValidationEndpointSchema,
};
pub use error::{TrapCode, TrapInfo, WasmError};
pub use host::{HostState, LogEntry, LogLevel};
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
    WasmResourceLimiter,
};
pub use rate_limiter::{
    LimitType, RateLimitError, RateLimitResult, RateLimiter, WasmDenyReason, WasmRateLimitConfig,
    WasmRateLimiter,
};
pub use runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmRuntimeConfig, WasmToolRuntime};

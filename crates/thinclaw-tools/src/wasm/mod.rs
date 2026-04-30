//! Root-independent WASM tool runtime and configuration types.

pub mod allowlist;
pub mod capabilities;
pub mod capabilities_schema;
pub mod credential_injector;
pub mod error;
pub mod host;
pub mod limits;
pub mod oauth;
pub mod rate_limiter;
pub mod runtime;
pub mod storage;

pub use allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WorkspaceCapability, WorkspaceReader,
};
pub use capabilities_schema::{
    AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema, RateLimitSchema,
    ValidationEndpointSchema,
};
pub use credential_injector::{
    CredentialInjector, InjectedCredentials, InjectionError, SharedCredentialRegistry,
    inject_credential,
};
pub use error::{TrapCode, TrapInfo, WasmError};
pub use host::{HostState, LogEntry, LogLevel};
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
    WasmResourceLimiter,
};
pub use oauth::{
    GOOGLE_OAUTH_TOKEN, LEGACY_GMAIL_OAUTH_TOKEN, OAuthCredentials, OAuthPkcePair,
    OAuthRefreshConfig, ResolvedOAuthConfig, WasmOAuthTokenExchange, WasmToolAuthCheck,
    WasmToolAuthMode, WasmToolAuthStatus, WasmToolAuthorizationRequest, WasmToolOAuthError,
    WasmToolOAuthFlow, build_authorization_url, builtin_credentials, canonical_secret_name,
    is_google_secret_name, refresh_secret_name, resolve_oauth_refresh_config, scopes_secret_name,
    shared_auth_provider,
};
pub use rate_limiter::{
    LimitType, RateLimitError, RateLimitResult, RateLimiter, WasmDenyReason, WasmRateLimitConfig,
    WasmRateLimiter,
};
pub use runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmRuntimeConfig, WasmToolRuntime};
#[cfg(feature = "libsql")]
pub use storage::LibSqlWasmToolStore;
#[cfg(feature = "postgres")]
pub use storage::PostgresWasmToolStore;
pub use storage::{
    StoreToolParams, StoredCapabilities, StoredWasmTool, StoredWasmToolWithBinary, ToolStatus,
    TrustLevel, WasmStorageError, WasmToolStore, compute_binary_hash, verify_binary_integrity,
};

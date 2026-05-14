//! WASM sandbox for untrusted tool execution.

mod allowlist;
mod capabilities;
mod capabilities_schema;
mod credential_injector;
mod error;
mod host;
#[cfg(feature = "wasm-runtime")]
mod limits;
#[cfg(feature = "wasm-runtime")]
mod loader;
mod oauth;
mod rate_limiter;
#[cfg(feature = "wasm-runtime")]
mod runtime;
mod storage;
#[cfg(feature = "wasm-runtime")]
mod watcher;
#[cfg(feature = "wasm-runtime")]
mod wrapper;

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
};
pub use error::{TrapCode, TrapInfo, WasmError};
pub use host::{HostState, LogEntry, LogLevel};
#[cfg(feature = "wasm-runtime")]
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
    WasmResourceLimiter,
};
#[cfg(feature = "wasm-runtime")]
pub use loader::{
    DiscoveredTool, LoadResults, WasmLoadError, WasmToolLoader, discover_dev_tools, discover_tools,
    load_dev_tools, resolve_wasm_target_dir, wasm_artifact_path,
};
pub use oauth::{
    GOOGLE_OAUTH_TOKEN, LEGACY_GMAIL_OAUTH_TOKEN, OAuthPkcePair, OAuthRefreshConfig,
    ResolvedOAuthConfig, WasmOAuthTokenExchange, WasmToolAuthCheck, WasmToolAuthMode,
    WasmToolAuthStatus, WasmToolAuthorizationRequest, WasmToolOAuthError, WasmToolOAuthFlow,
    build_authorization_url, canonical_secret_name, is_google_secret_name, refresh_secret_name,
    resolve_oauth_refresh_config, scopes_secret_name, shared_auth_provider,
};
pub use rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};
pub use rate_limiter::{WasmDenyReason, WasmRateLimitConfig, WasmRateLimiter};
#[cfg(feature = "wasm-runtime")]
pub use runtime::{PreparedModule, WasmRuntimeConfig, WasmToolRuntime};
#[cfg(feature = "libsql")]
pub use storage::LibSqlWasmToolStore;
#[cfg(feature = "postgres")]
pub use storage::PostgresWasmToolStore;
pub use storage::{
    StoreToolParams, StoredCapabilities, StoredWasmTool, StoredWasmToolWithBinary, ToolStatus,
    TrustLevel, WasmStorageError, WasmToolStore, compute_binary_hash, verify_binary_integrity,
};
#[cfg(feature = "wasm-runtime")]
pub use watcher::{ToolWatcher, ToolWatcherConfig};
#[cfg(feature = "wasm-runtime")]
pub use wrapper::{SafetyLeakScanner, WasmToolWrapper};

#[cfg(not(feature = "wasm-runtime"))]
pub use stubs::*;

#[cfg(not(feature = "wasm-runtime"))]
mod stubs {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use async_trait::async_trait;

    use crate::context::JobContext;
    use crate::tools::tool::{Tool, ToolError, ToolOutput};

    #[derive(Debug, Clone, Default)]
    pub struct FuelConfig {
        pub enabled: bool,
        pub fuel_limit: u64,
    }

    #[derive(Debug, Clone)]
    pub struct ResourceLimits {
        pub max_memory_bytes: usize,
        pub max_table_elements: u32,
        pub timeout: std::time::Duration,
    }

    impl Default for ResourceLimits {
        fn default() -> Self {
            Self {
                max_memory_bytes: 0,
                max_table_elements: 0,
                timeout: std::time::Duration::from_secs(0),
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct PreparedModule {
        pub name: String,
    }

    #[derive(Debug, Clone)]
    pub struct WasmToolRuntime;

    impl WasmToolRuntime {
        pub fn new(_config: WasmRuntimeConfig) -> Result<Self, super::WasmError> {
            Err(super::WasmError::ConfigError(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }

        pub async fn prepare(
            &self,
            name: &str,
            _bytes: &[u8],
            _limits: Option<ResourceLimits>,
        ) -> Result<PreparedModule, super::WasmError> {
            Err(super::WasmError::ConfigError(format!(
                "WASM runtime is not available for tool '{}'",
                name
            )))
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct WasmRuntimeConfig {
        pub default_limits: ResourceLimits,
        pub fuel_config: FuelConfig,
        pub cache_enabled: bool,
    }

    pub struct WasmToolLoader;

    impl WasmToolLoader {
        pub fn new(
            _runtime: Arc<WasmToolRuntime>,
            _registry: Arc<crate::tools::ToolRegistry>,
        ) -> Self {
            Self
        }

        pub fn with_tool_invoker(
            self,
            _invoker: Arc<crate::tools::execution::HostMediatedToolInvoker>,
        ) -> Self {
            self
        }

        pub fn with_secrets_store(
            self,
            _secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        ) -> Self {
            self
        }

        pub async fn load_from_files(
            &self,
            name: &str,
            _wasm_path: &Path,
            _capabilities_path: Option<&Path>,
        ) -> Result<(), super::WasmError> {
            Err(super::WasmError::ConfigError(format!(
                "WASM runtime is not available for tool '{}'",
                name
            )))
        }
    }

    #[derive(Debug, Default)]
    pub struct LoadResults {
        pub loaded: Vec<String>,
        pub errors: Vec<(PathBuf, super::WasmError)>,
    }

    pub async fn load_dev_tools(
        _loader: &WasmToolLoader,
        _dir: &Path,
    ) -> Result<LoadResults, super::WasmError> {
        Err(super::WasmError::ConfigError(
            "ThinClaw was built without the wasm-runtime feature".to_string(),
        ))
    }

    #[derive(Debug, Default, Clone)]
    pub struct DiscoveredTool {
        pub name: String,
        pub wasm_path: PathBuf,
        pub capabilities_path: Option<PathBuf>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("ThinClaw was built without the wasm-runtime feature")]
    pub struct WasmLoadError;

    pub async fn discover_tools(
        _dir: &Path,
    ) -> Result<std::collections::HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(std::collections::HashMap::new())
    }

    pub async fn discover_dev_tools()
    -> Result<std::collections::HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(std::collections::HashMap::new())
    }

    pub fn resolve_wasm_target_dir(crate_dir: &Path) -> PathBuf {
        std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate_dir.join("target"))
    }

    pub fn wasm_artifact_path(crate_dir: &Path, binary_name: &str) -> PathBuf {
        resolve_wasm_target_dir(crate_dir)
            .join("wasm32-wasip2")
            .join("release")
            .join(format!("{binary_name}.wasm"))
    }

    pub struct ToolWatcher;

    #[derive(Debug, Clone, Default)]
    pub struct ToolWatcherConfig;

    impl ToolWatcher {
        pub fn new(
            _install_dir: PathBuf,
            _loader: Arc<WasmToolLoader>,
            _registry: Arc<crate::tools::ToolRegistry>,
        ) -> Self {
            Self
        }

        pub async fn seed_from_sources(&self) {}

        pub async fn start(&self) {}
    }

    #[derive(Debug, Clone)]
    pub struct WasmToolWrapper {
        name: String,
    }

    impl WasmToolWrapper {
        pub fn new(
            _runtime: Arc<WasmToolRuntime>,
            prepared: PreparedModule,
            _capabilities: super::Capabilities,
        ) -> Self {
            Self {
                name: prepared.name,
            }
        }
    }

    #[async_trait]
    impl Tool for WasmToolWrapper {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "WASM tool runtime unavailable"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::ExecutionFailed(
                "ThinClaw was built without the wasm-runtime feature".to_string(),
            ))
        }
    }

    #[derive(Debug, Clone)]
    pub struct SafetyLeakScanner;
}

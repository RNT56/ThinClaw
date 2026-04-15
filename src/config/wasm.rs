use std::path::PathBuf;
use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// WASM sandbox configuration.
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Whether WASM tool execution is enabled.
    pub enabled: bool,
    /// Directory containing installed WASM tools (default: ~/.thinclaw/tools/).
    pub tools_dir: PathBuf,
    /// Default memory limit in bytes (default: 10 MB).
    pub default_memory_limit: u64,
    /// Default execution timeout in seconds (default: 60).
    pub default_timeout_secs: u64,
    /// Default fuel limit for CPU metering (default: 10M).
    pub default_fuel_limit: u64,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: default_tools_dir(),
            default_memory_limit: 10 * 1024 * 1024, // 10 MB
            default_timeout_secs: 60,
            default_fuel_limit: 10_000_000,
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

/// Get the default tools directory (~/.thinclaw/tools/).
fn default_tools_dir() -> PathBuf {
    crate::platform::state_paths().tools_dir
}

impl WasmConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let db = &settings.wasm;
        Ok(Self {
            enabled: parse_bool_env("WASM_ENABLED", db.enabled)?,
            tools_dir: optional_env("WASM_TOOLS_DIR")?
                .map(PathBuf::from)
                .or_else(|| db.tools_dir.clone())
                .unwrap_or_else(default_tools_dir),
            default_memory_limit: parse_optional_env(
                "WASM_DEFAULT_MEMORY_LIMIT",
                db.default_memory_limit,
            )?,
            default_timeout_secs: parse_optional_env(
                "WASM_DEFAULT_TIMEOUT_SECS",
                db.default_timeout_secs,
            )?,
            default_fuel_limit: parse_optional_env(
                "WASM_DEFAULT_FUEL_LIMIT",
                db.default_fuel_limit,
            )?,
            cache_compiled: parse_bool_env("WASM_CACHE_COMPILED", db.cache_compiled)?,
            cache_dir: optional_env("WASM_CACHE_DIR")?
                .map(PathBuf::from)
                .or_else(|| db.cache_dir.clone()),
        })
    }

    /// Convert to WasmRuntimeConfig.
    pub fn to_runtime_config(&self) -> crate::tools::wasm::WasmRuntimeConfig {
        use crate::tools::wasm::{FuelConfig, ResourceLimits, WasmRuntimeConfig};

        WasmRuntimeConfig {
            default_limits: ResourceLimits {
                memory_bytes: self.default_memory_limit,
                fuel: self.default_fuel_limit,
                timeout: Duration::from_secs(self.default_timeout_secs),
            },
            fuel_config: FuelConfig {
                initial_fuel: self.default_fuel_limit,
                enabled: true,
            },
            cache_compiled: self.cache_compiled,
            cache_dir: self.cache_dir.clone(),
            optimization_level: wasmtime::OptLevel::Speed,
        }
    }
}

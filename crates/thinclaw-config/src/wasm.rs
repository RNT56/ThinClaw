//! WASM tool runtime configuration.

use std::path::PathBuf;

use thinclaw_platform::state_paths;
use thinclaw_settings::Settings;
use thinclaw_types::error::ConfigError;

use crate::helpers::{optional_env, parse_bool_env, parse_optional_env};

/// WASM sandbox configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            default_memory_limit: 10 * 1024 * 1024,
            default_timeout_secs: 60,
            default_fuel_limit: 10_000_000,
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

fn default_tools_dir() -> PathBuf {
    state_paths().tools_dir
}

impl WasmConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    #[test]
    fn resolve_defaults_from_settings() {
        let _guard = lock_env();
        unsafe {
            std::env::remove_var("WASM_ENABLED");
            std::env::remove_var("WASM_TOOLS_DIR");
            std::env::remove_var("WASM_DEFAULT_MEMORY_LIMIT");
            std::env::remove_var("WASM_DEFAULT_TIMEOUT_SECS");
            std::env::remove_var("WASM_DEFAULT_FUEL_LIMIT");
            std::env::remove_var("WASM_CACHE_COMPILED");
            std::env::remove_var("WASM_CACHE_DIR");
        }

        let cfg = WasmConfig::resolve(&Settings::default()).expect("wasm config");
        assert_eq!(cfg, WasmConfig::default());
    }

    #[test]
    fn resolve_env_overrides_limits_and_cache() {
        let _guard = lock_env();
        unsafe {
            std::env::set_var("WASM_DEFAULT_MEMORY_LIMIT", "1024");
            std::env::set_var("WASM_DEFAULT_TIMEOUT_SECS", "5");
            std::env::set_var("WASM_DEFAULT_FUEL_LIMIT", "12345");
            std::env::set_var("WASM_CACHE_COMPILED", "false");
            std::env::set_var("WASM_CACHE_DIR", "/tmp/wasm-cache");
        }

        let cfg = WasmConfig::resolve(&Settings::default()).expect("wasm config");
        assert_eq!(cfg.default_memory_limit, 1024);
        assert_eq!(cfg.default_timeout_secs, 5);
        assert_eq!(cfg.default_fuel_limit, 12345);
        assert!(!cfg.cache_compiled);
        assert_eq!(cfg.cache_dir, Some(PathBuf::from("/tmp/wasm-cache")));

        unsafe {
            std::env::remove_var("WASM_DEFAULT_MEMORY_LIMIT");
            std::env::remove_var("WASM_DEFAULT_TIMEOUT_SECS");
            std::env::remove_var("WASM_DEFAULT_FUEL_LIMIT");
            std::env::remove_var("WASM_CACHE_COMPILED");
            std::env::remove_var("WASM_CACHE_DIR");
        }
    }
}

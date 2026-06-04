pub use thinclaw_config::wasm::WasmConfig;

#[cfg(feature = "wasm-runtime")]
pub(crate) trait WasmConfigExt {
    fn to_runtime_config(&self) -> crate::tools::wasm::WasmRuntimeConfig;
}

#[cfg(feature = "wasm-runtime")]
impl WasmConfigExt for WasmConfig {
    fn to_runtime_config(&self) -> crate::tools::wasm::WasmRuntimeConfig {
        use std::time::Duration;

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

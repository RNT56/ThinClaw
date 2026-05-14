use std::time::Duration;
use wasmtime::ResourceLimiter;

/// Default memory limit: 10 MB.
pub const DEFAULT_MEMORY_LIMIT: u64 = 10 * 1024 * 1024;

/// Default fuel limit: 10 million instructions.
pub const DEFAULT_FUEL_LIMIT: u64 = 10_000_000;

/// Default execution timeout: 60 seconds.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Resource limits for a single WASM channel execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub memory_bytes: u64,
    pub fuel: u64,
    pub timeout: Duration,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_LIMIT,
            fuel: DEFAULT_FUEL_LIMIT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl ResourceLimits {
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.memory_bytes = bytes;
        self
    }

    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Configuration for fuel metering.
#[derive(Debug, Clone)]
pub struct FuelConfig {
    pub initial_fuel: u64,
    pub enabled: bool,
}

impl Default for FuelConfig {
    fn default() -> Self {
        Self {
            initial_fuel: DEFAULT_FUEL_LIMIT,
            enabled: true,
        }
    }
}

impl FuelConfig {
    pub fn disabled() -> Self {
        Self {
            initial_fuel: 0,
            enabled: false,
        }
    }

    pub fn with_limit(fuel: u64) -> Self {
        Self {
            initial_fuel: fuel,
            enabled: true,
        }
    }
}

pub struct WasmResourceLimiter {
    memory_limit: u64,
    memory_used: u64,
    max_tables: u32,
    max_instances: u32,
}

impl WasmResourceLimiter {
    pub fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            max_tables: 10,
            max_instances: 10,
        }
    }

    pub fn memory_used(&self) -> u64 {
        self.memory_used
    }

    pub fn memory_limit(&self) -> u64 {
        self.memory_limit
    }
}

impl ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let desired_u64 = desired as u64;
        if desired_u64 > self.memory_limit {
            tracing::warn!(
                current = current,
                desired = desired,
                limit = self.memory_limit,
                "WASM memory growth denied: would exceed limit"
            );
            return Ok(false);
        }
        self.memory_used = desired_u64;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        if desired > 10_000 {
            tracing::warn!(
                current = current,
                desired = desired,
                "WASM table growth denied: too large"
            );
            return Ok(false);
        }
        Ok(true)
    }

    fn instances(&self) -> usize {
        self.max_instances as usize
    }

    fn tables(&self) -> usize {
        self.max_tables as usize
    }

    fn memories(&self) -> usize {
        self.max_instances as usize
    }
}

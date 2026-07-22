//! WASM channel runtime for managing compiled channel components.
//!
//! Similar to tool runtime, follows the principle: compile once at registration,
//! instantiate fresh per callback execution.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio::sync::RwLock;
use wasmtime::{Config, Engine, OptLevel};

use crate::wasm::error::WasmChannelError;
use crate::wasm::limits::{FuelConfig, ResourceLimits};

/// How often the background thread advances the engine epoch. The epoch trap is
/// a backup timeout for guests that ignore fuel (or run with fuel disabled).
pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);

/// Epoch ticks granted beyond the callback timeout. The outer async timeout is
/// the primary deadline; the epoch trap fires a little later, only to stop a
/// guest still spinning on a blocking thread after its result was abandoned.
const EPOCH_DEADLINE_MARGIN_TICKS: u64 = 8;
const MAX_WASM_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREPARED_MODULES: usize = 128;
const MAX_PARALLEL_COMPILATIONS: usize = 4;
const MAX_WASM_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_WASM_FUEL: u64 = 1_000_000_000;
const MAX_CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);

fn validate_resource_limits(limits: &ResourceLimits) -> Result<(), WasmChannelError> {
    if limits.memory_bytes == 0 || limits.memory_bytes > MAX_WASM_MEMORY_BYTES {
        return Err(WasmChannelError::Config(format!(
            "WASM memory limit must be between 1 and {MAX_WASM_MEMORY_BYTES} bytes"
        )));
    }
    if limits.fuel == 0 || limits.fuel > MAX_WASM_FUEL {
        return Err(WasmChannelError::Config(format!(
            "WASM fuel limit must be between 1 and {MAX_WASM_FUEL}"
        )));
    }
    if limits.timeout.is_zero() || limits.timeout > MAX_CALLBACK_TIMEOUT {
        return Err(WasmChannelError::Config(
            "WASM timeout must be greater than zero and at most 120 seconds".to_string(),
        ));
    }
    Ok(())
}

/// Epoch-tick budget for a callback whose wall-clock timeout is `timeout`.
/// Always at least one tick so a zero/absurd timeout can never disable the trap.
pub(crate) fn epoch_deadline_ticks(timeout: Duration) -> u64 {
    let tick_ms = EPOCH_TICK_INTERVAL.as_millis().max(1) as u64;
    let timeout_ms = timeout.as_millis() as u64;
    (timeout_ms / tick_ms)
        .saturating_add(EPOCH_DEADLINE_MARGIN_TICKS)
        .max(1)
}

/// Configuration for the WASM channel runtime.
#[derive(Debug, Clone)]
pub struct WasmChannelRuntimeConfig {
    /// Default resource limits for channels.
    pub default_limits: ResourceLimits,
    /// Fuel configuration.
    pub fuel_config: FuelConfig,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
    /// Cranelift optimization level.
    pub optimization_level: OptLevel,
    /// Default callback timeout.
    pub callback_timeout: Duration,
}

impl Default for WasmChannelRuntimeConfig {
    fn default() -> Self {
        Self {
            default_limits: ResourceLimits {
                // Channels may need more memory for message buffering
                memory_bytes: 50 * 1024 * 1024, // 50 MB
                fuel: 10_000_000,
                timeout: Duration::from_secs(60),
            },
            fuel_config: FuelConfig::default(),
            cache_compiled: true,
            cache_dir: None,
            optimization_level: OptLevel::Speed,
            callback_timeout: Duration::from_secs(30),
        }
    }
}

impl WasmChannelRuntimeConfig {
    /// Create a minimal config for testing.
    pub fn for_testing() -> Self {
        Self {
            default_limits: ResourceLimits {
                memory_bytes: 5 * 1024 * 1024, // 5 MB
                fuel: 1_000_000,
                timeout: Duration::from_secs(5),
            },
            fuel_config: FuelConfig::with_limit(1_000_000),
            cache_compiled: false,
            cache_dir: None,
            optimization_level: OptLevel::None, // Faster compilation for tests
            callback_timeout: Duration::from_secs(5),
        }
    }
}

/// A compiled WASM channel component ready for instantiation.
///
/// Stores the pre-compiled `Component` directly so instantiation
/// doesn't require recompilation.
pub struct PreparedChannelModule {
    /// Channel name.
    pub name: String,
    /// Channel description.
    pub description: String,
    /// Pre-compiled component (cheaply cloneable via internal Arc).
    pub(crate) component: Option<wasmtime::component::Component>,
    /// Resource limits for this channel.
    pub limits: ResourceLimits,
    source_digest: [u8; 32],
}

impl PreparedChannelModule {
    /// Get the pre-compiled component for instantiation.
    pub fn component(&self) -> Option<&wasmtime::component::Component> {
        self.component.as_ref()
    }

    /// Create a PreparedChannelModule for testing purposes.
    ///
    /// Creates a module with no actual WASM component, suitable for testing
    /// channel infrastructure without requiring a real WASM component.
    pub fn for_testing(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            component: None,
            limits: ResourceLimits::default(),
            source_digest: [0; 32],
        }
    }
}

impl std::fmt::Debug for PreparedChannelModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedChannelModule")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("has_component", &self.component.is_some())
            .field("limits", &self.limits)
            .field(
                "source_digest_prefix",
                &&hex::encode(&self.source_digest[..6]),
            )
            .finish()
    }
}

/// WASM channel runtime.
///
/// Manages the Wasmtime engine and a cache of prepared channel modules.
pub struct WasmChannelRuntime {
    /// Wasmtime engine with configured settings.
    engine: Engine,
    /// Runtime configuration.
    config: WasmChannelRuntimeConfig,
    /// Cache of prepared modules by name.
    modules: RwLock<HashMap<String, Arc<PreparedChannelModule>>>,
    compilation_slots: Arc<tokio::sync::Semaphore>,
    epoch_running: Arc<AtomicBool>,
    epoch_thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl WasmChannelRuntime {
    /// Create a new runtime with the given configuration.
    pub fn new(config: WasmChannelRuntimeConfig) -> Result<Self, WasmChannelError> {
        validate_resource_limits(&config.default_limits)?;
        if config.callback_timeout.is_zero() || config.callback_timeout > MAX_CALLBACK_TIMEOUT {
            return Err(WasmChannelError::Config(
                "callback timeout must be greater than zero and at most 120 seconds".to_string(),
            ));
        }
        if config.fuel_config.enabled
            && (config.fuel_config.initial_fuel == 0
                || config.fuel_config.initial_fuel > MAX_WASM_FUEL)
        {
            return Err(WasmChannelError::Config(format!(
                "initial fuel must be between 1 and {MAX_WASM_FUEL}"
            )));
        }
        let mut wasmtime_config = Config::new();

        // Enable fuel consumption for CPU limiting
        if config.fuel_config.enabled {
            wasmtime_config.consume_fuel(true);
        }

        // Enable epoch interruption as a backup timeout mechanism
        wasmtime_config.epoch_interruption(true);

        // Enable component model (WASI Preview 2)
        wasmtime_config.wasm_component_model(true);

        // Threads are disabled by default (wasmtime 'threads' feature not compiled in).

        // Set optimization level
        wasmtime_config.cranelift_opt_level(config.optimization_level);

        // Disable debug info in production
        wasmtime_config.debug_info(false);

        // Note: Wasmtime compilation caching (via CacheConfig) can be enabled
        // if the "cache" feature is added to the wasmtime dependency for faster
        // subsequent startups. Skipped here to minimize feature surface.

        let engine = Engine::new(&wasmtime_config).map_err(|e| {
            WasmChannelError::Config(format!("Failed to create Wasmtime engine: {}", e))
        })?;

        // Advance the engine epoch on a background thread. Without this,
        // `epoch_deadline_trap()` never fires, so the epoch timeout is inert and
        // a guest that ignores fuel (or runs with fuel disabled) can spin
        // forever on a blocking thread. The runtime is a process-lifetime
        // singleton, so one ticker thread is created for the whole process.
        let ticker_engine = engine.clone();
        let epoch_running = Arc::new(AtomicBool::new(true));
        let ticker_running = Arc::clone(&epoch_running);
        let epoch_thread = std::thread::Builder::new()
            .name("wasm-channel-epoch-ticker".into())
            .spawn(move || {
                while ticker_running.load(Ordering::Acquire) {
                    std::thread::park_timeout(EPOCH_TICK_INTERVAL);
                    if !ticker_running.load(Ordering::Acquire) {
                        break;
                    }
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to spawn epoch ticker thread: {}", e))
            })?;

        Ok(Self {
            engine,
            config,
            modules: RwLock::new(HashMap::new()),
            compilation_slots: Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL_COMPILATIONS)),
            epoch_running,
            epoch_thread: std::sync::Mutex::new(Some(epoch_thread)),
        })
    }

    /// Get the Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Get the runtime configuration.
    pub fn config(&self) -> &WasmChannelRuntimeConfig {
        &self.config
    }

    /// Prepare a WASM channel component for execution.
    ///
    /// This validates and compiles the component.
    /// The compiled component is cached for fast instantiation.
    pub async fn prepare(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        limits: Option<ResourceLimits>,
        description: Option<String>,
    ) -> Result<Arc<PreparedChannelModule>, WasmChannelError> {
        if !crate::wasm::capabilities::is_valid_channel_name(name) {
            return Err(WasmChannelError::InvalidName(name.to_string()));
        }
        if wasm_bytes.is_empty() || wasm_bytes.len() > MAX_WASM_MODULE_BYTES {
            return Err(WasmChannelError::Compilation(format!(
                "WASM component must be between 1 and {MAX_WASM_MODULE_BYTES} bytes"
            )));
        }
        if description
            .as_deref()
            .is_some_and(|value| value.len() > 4096)
        {
            return Err(WasmChannelError::Config(
                "channel description exceeds 4096 bytes".to_string(),
            ));
        }
        let limits = limits.unwrap_or_else(|| self.config.default_limits.clone());
        validate_resource_limits(&limits)?;
        let source_digest: [u8; 32] = Sha256::digest(wasm_bytes).into();

        // Check if already prepared
        if let Some(module) = self.modules.read().await.get(name)
            && module.source_digest == source_digest
        {
            return Ok(Arc::clone(module));
        }

        let _compilation_permit = Arc::clone(&self.compilation_slots)
            .acquire_owned()
            .await
            .map_err(|_| WasmChannelError::Config("compilation limiter is closed".to_string()))?;
        // A previous waiter may have populated the cache while this task was
        // queued for a compilation slot.
        if let Some(module) = self.modules.read().await.get(name)
            && module.source_digest == source_digest
        {
            return Ok(Arc::clone(module));
        }

        let name = name.to_string();
        let wasm_bytes = wasm_bytes.to_vec();
        let engine = self.engine.clone();
        let desc = description.unwrap_or_else(|| format!("WASM channel: {}", name));

        // Compile in blocking task (Wasmtime compilation is synchronous)
        let prepared = tokio::task::spawn_blocking(move || {
            // Validate and compile the component
            let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
                .map_err(|e| WasmChannelError::Compilation(e.to_string()))?;

            Ok::<_, WasmChannelError>(PreparedChannelModule {
                name: name.clone(),
                description: desc,
                component: Some(component),
                limits,
                source_digest,
            })
        })
        .await
        .map_err(|e| {
            WasmChannelError::Compilation(format!("Preparation task panicked: {}", e))
        })??;

        let prepared = Arc::new(prepared);

        // Cache the prepared module
        if self.config.cache_compiled {
            let mut modules = self.modules.write().await;
            if !modules.contains_key(&prepared.name) && modules.len() >= MAX_PREPARED_MODULES {
                return Err(WasmChannelError::Config(format!(
                    "prepared channel cache exceeds the {MAX_PREPARED_MODULES}-module limit"
                )));
            }
            modules.insert(prepared.name.clone(), Arc::clone(&prepared));
        }

        tracing::info!(
            name = %prepared.name,
            "Prepared WASM channel for execution"
        );

        Ok(prepared)
    }

    /// Get a prepared module by name.
    pub async fn get(&self, name: &str) -> Option<Arc<PreparedChannelModule>> {
        self.modules.read().await.get(name).cloned()
    }

    /// Remove a prepared module from the cache.
    pub async fn remove(&self, name: &str) -> Option<Arc<PreparedChannelModule>> {
        self.modules.write().await.remove(name)
    }

    /// List all prepared module names.
    pub async fn list(&self) -> Vec<String> {
        self.modules.read().await.keys().cloned().collect()
    }

    /// Clear all cached modules.
    pub async fn clear(&self) {
        self.modules.write().await.clear();
    }
}

impl Drop for WasmChannelRuntime {
    fn drop(&mut self) {
        self.epoch_running.store(false, Ordering::Release);
        let mut guard = self
            .epoch_thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handle) = guard.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

impl std::fmt::Debug for WasmChannelRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmChannelRuntime")
            .field("config", &self.config)
            .field("modules", &"<RwLock<HashMap>>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::wasm::runtime::{
        WasmChannelRuntime, WasmChannelRuntimeConfig, epoch_deadline_ticks,
    };
    use std::time::Duration;

    #[test]
    fn epoch_deadline_scales_with_timeout_and_never_zero() {
        // 30s / 500ms = 60 ticks, plus the 8-tick margin.
        assert_eq!(epoch_deadline_ticks(Duration::from_secs(30)), 68);
        // A zero/absurd timeout must still leave the trap armed (>= 1 tick).
        assert!(epoch_deadline_ticks(Duration::ZERO) >= 1);
        // The deadline exceeds the timeout so the outer async timeout fires first.
        let ticks = epoch_deadline_ticks(Duration::from_secs(5));
        assert!(ticks * 500 > 5_000, "deadline must exceed the timeout");
    }

    #[test]
    fn test_runtime_config_default() {
        let config = WasmChannelRuntimeConfig::default();
        assert!(config.cache_compiled);
        assert!(config.fuel_config.enabled);
        // Channels get more memory than tools
        assert_eq!(config.default_limits.memory_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn test_runtime_config_for_testing() {
        let config = WasmChannelRuntimeConfig::for_testing();
        assert!(!config.cache_compiled);
        assert_eq!(config.default_limits.memory_bytes, 5 * 1024 * 1024);
    }

    #[test]
    fn test_runtime_creation() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = WasmChannelRuntime::new(config).unwrap();
        assert!(runtime.config().fuel_config.enabled);
    }

    #[tokio::test]
    async fn test_module_cache_operations() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = WasmChannelRuntime::new(config).unwrap();

        // Initially empty
        assert!(runtime.list().await.is_empty());
        assert!(runtime.get("test").await.is_none());
    }
}

use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::events::{StatusReporter, ToolEvent};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Script runtime error: {0}")]
    Runtime(String),
    #[error("Script compilation error: {0}")]
    Compilation(String),
    #[error("Forbidden pattern in script: '{0}'")]
    ForbiddenPattern(String),
    #[error("Execution timeout after {0}s")]
    Timeout(u64),
    #[error("Result too large: {size} bytes (max {max} bytes)")]
    ResultTooLarge { size: usize, max: usize },
    #[error("System error: {0}")]
    System(String),
}

impl SandboxError {
    /// Format as an LLM-friendly error message that the agent can use to
    /// self-correct its script.
    pub fn to_llm_feedback(&self) -> String {
        match self {
            Self::Runtime(msg) => format!(
                "Tool Execution Error:\n{}\n\nHint: Check your variable names, property access, and tool arguments. Rewrite the code to fix this.",
                msg
            ),
            Self::Compilation(msg) => format!(
                "Script Compilation Error:\n{}\n\nHint: The function or tool you called does not exist. Use `search_tools` to discover available tools.",
                msg
            ),
            Self::ForbiddenPattern(pat) => format!(
                "Security Violation: Script contains forbidden pattern '{}'. Only MCP tool functions are available in the sandbox.",
                pat
            ),
            Self::Timeout(secs) => format!(
                "Execution timed out after {}s. Simplify your script or reduce the number of tool calls.",
                secs
            ),
            Self::ResultTooLarge { size, max } => format!(
                "Result too large ({} bytes, max {} bytes). Filter or summarize the data before returning.",
                size, max
            ),
            Self::System(msg) => format!("Internal system error: {}", msg),
        }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Max script operations (prevents infinite loops)
    pub max_operations: u64,
    /// Max execution time in seconds
    pub timeout_seconds: u64,
    /// Max result JSON size in bytes
    pub max_result_size: usize,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            timeout_seconds: 30,
            max_result_size: 1_048_576, // 1 MB
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SandboxResult {
    /// JSON-serialized output of the script
    pub output: String,
    /// Wall-clock execution time
    pub execution_time_ms: u128,
    /// Number of Rhai operations consumed
    pub operations_used: u64,
}

// ---------------------------------------------------------------------------
// Forbidden patterns
// ---------------------------------------------------------------------------

const FORBIDDEN_PATTERNS: &[&str] = &["std::fs", "std::net", "std::process", "unsafe", "extern"];
const MAX_SANDBOX_SCRIPT_BYTES: usize = 256 * 1024;
const MAX_SANDBOX_TIMEOUT_SECONDS: u64 = 5 * 60;

fn monotonic_millis() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    u64::try_from(ORIGIN.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Sandbox {
    engine: Arc<Engine>,
    config: SandboxConfig,
    reporter: Arc<dyn StatusReporter>,
    deadline_millis: Arc<AtomicU64>,
    execution_lock: Arc<Mutex<()>>,
}

impl Sandbox {
    pub fn new(config: SandboxConfig, reporter: Arc<dyn StatusReporter>) -> Self {
        let config = SandboxConfig {
            max_operations: config.max_operations.max(1),
            timeout_seconds: config.timeout_seconds.clamp(1, MAX_SANDBOX_TIMEOUT_SECONDS),
            max_result_size: config.max_result_size.max(1),
        };
        let mut engine = Engine::new();
        let deadline_millis = Arc::new(AtomicU64::new(u64::MAX));

        // Apply resource limits
        engine.set_max_operations(config.max_operations);
        engine.set_max_call_levels(32);
        engine.set_max_variables(256);
        engine.set_max_functions(256);
        engine.set_max_modules(64);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_string_size(config.max_result_size.saturating_mul(2).max(1_024));
        engine.set_max_array_size(10_000);
        engine.set_max_map_size(10_000);
        let progress_deadline = deadline_millis.clone();
        engine.on_progress(move |_| {
            (monotonic_millis() >= progress_deadline.load(Ordering::Relaxed))
                .then(|| rhai::Dynamic::from("sandbox execution deadline exceeded"))
        });

        // Register built-in utility functions
        engine.register_fn("json_stringify", |map: rhai::Map| -> String {
            serde_json::to_string(&map).unwrap_or_default()
        });

        engine.register_fn("parse_json", |json: String| -> rhai::Dynamic {
            match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(v) => rhai::serde::to_dynamic(v).unwrap_or_default(),
                Err(_) => rhai::Dynamic::UNIT,
            }
        });

        engine.register_fn("timestamp_now", || -> String {
            chrono::Local::now().to_rfc3339()
        });

        Self {
            engine: Arc::new(engine),
            config,
            reporter,
            deadline_millis,
            execution_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Get mutable access to the underlying Rhai engine so the caller can
    /// register host-provided tool functions using Rhai's public API.
    ///
    /// # Example
    /// ```ignore
    /// sandbox.engine_mut().register_fn("web_search", move |query: String| -> Dynamic {
    ///     // ... call local search ...
    /// });
    /// ```
    pub fn engine_mut(&mut self) -> &mut Engine {
        Arc::get_mut(&mut self.engine)
            .expect("sandbox engine cannot be modified after the sandbox has been cloned")
    }

    /// Execute a script in the sandbox.
    pub fn execute(&self, script: &str) -> Result<SandboxResult, SandboxError> {
        // 1. Validate
        Self::validate_script(script)?;
        let _execution_guard = self
            .execution_lock
            .lock()
            .map_err(|_| SandboxError::System("sandbox execution lock is poisoned".to_string()))?;
        self.deadline_millis.store(
            monotonic_millis().saturating_add(self.config.timeout_seconds.saturating_mul(1_000)),
            Ordering::Relaxed,
        );

        // 2. Report start
        let reporter = self.reporter.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                reporter
                    .report(ToolEvent::Status {
                        msg: "Executing script...".into(),
                        icon: None,
                    })
                    .await;
            });
        }

        // 3. Execute with timeout
        let start = Instant::now();
        let mut scope = Scope::new();

        let result = self
            .engine
            .eval_with_scope::<Dynamic>(&mut scope, script)
            .map_err(|e| Self::map_rhai_error(*e, self.config.timeout_seconds));

        self.deadline_millis.store(u64::MAX, Ordering::Relaxed);
        let result = result?;

        let elapsed = start.elapsed();
        if elapsed > Duration::from_secs(self.config.timeout_seconds) {
            return Err(SandboxError::Timeout(self.config.timeout_seconds));
        }

        // 5. Serialize result
        let json_result = if result.is_unit() {
            "null".to_string()
        } else if result.is_string() {
            result.into_string().unwrap_or_default()
        } else {
            // Attempt JSON serialization via Rhai map/array
            format!("{}", result)
        };

        // 6. Check size
        if json_result.len() > self.config.max_result_size {
            return Err(SandboxError::ResultTooLarge {
                size: json_result.len(),
                max: self.config.max_result_size,
            });
        }

        Ok(SandboxResult {
            output: json_result,
            execution_time_ms: elapsed.as_millis(),
            operations_used: 0, // Rhai doesn't expose this directly yet
        })
    }

    /// Execute without blocking an async runtime worker. The Rhai progress
    /// callback interrupts CPU-bound scripts, while this outer deadline also
    /// releases the caller if a synchronous host function stalls.
    pub async fn execute_async(&self, script: String) -> Result<SandboxResult, SandboxError> {
        Self::validate_script(&script)?;
        let timeout_seconds = self.config.timeout_seconds;
        let sandbox = self.clone();
        let task = tokio::task::spawn_blocking(move || sandbox.execute(&script));
        match tokio::time::timeout(Duration::from_secs(timeout_seconds), task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(SandboxError::System(format!(
                "sandbox worker failed: {error}"
            ))),
            Err(_) => Err(SandboxError::Timeout(timeout_seconds)),
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn validate_script(script: &str) -> Result<(), SandboxError> {
        if script.len() > MAX_SANDBOX_SCRIPT_BYTES || script.contains('\0') {
            return Err(SandboxError::Compilation(
                "script is malformed or exceeds the 256 KiB limit".to_string(),
            ));
        }
        for pattern in FORBIDDEN_PATTERNS {
            if script.contains(pattern) {
                return Err(SandboxError::ForbiddenPattern(pattern.to_string()));
            }
        }
        Ok(())
    }

    fn map_rhai_error(err: EvalAltResult, timeout_seconds: u64) -> SandboxError {
        match err {
            EvalAltResult::ErrorRuntime(msg, _pos) => SandboxError::Runtime(msg.to_string()),
            EvalAltResult::ErrorFunctionNotFound(sig, _pos) => {
                SandboxError::Compilation(format!("Unknown function: {}", sig))
            }
            EvalAltResult::ErrorTooManyOperations(_pos) => SandboxError::Timeout(timeout_seconds),
            EvalAltResult::ErrorTerminated(_, _) => SandboxError::Timeout(timeout_seconds),
            EvalAltResult::ErrorParsing(parse_err, _pos) => {
                SandboxError::Compilation(format!("Parse error: {}", parse_err))
            }
            other => SandboxError::Runtime(other.to_string()),
        }
    }
}

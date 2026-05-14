//! Multi-engine inference abstraction.
//!
//! Each ThinClaw Desktop build targets **one** inference engine per platform
//! (determined by Cargo feature flags at compile time). This module provides
//! the `InferenceEngine` trait that all engines implement, plus the
//! `get_active_engine_info` Tauri command that tells the frontend which
//! engine is active.

use async_trait::async_trait;
use serde::Serialize;
use specta::Type;
use tauri::Emitter;

// Conditionally compile engine implementations
#[cfg(feature = "llamacpp")]
pub mod engine_llamacpp;

#[cfg(feature = "mlx")]
pub mod engine_mlx;

#[cfg(feature = "vllm")]
pub mod engine_vllm;

#[cfg(feature = "ollama")]
pub mod engine_ollama;

// ---------------------------------------------------------------------------
// InferenceEngine trait — the abstraction all engines implement
// ---------------------------------------------------------------------------

/// Trait that every inference engine backend must implement.
///
/// All engines expose an **OpenAI-compatible HTTP API** on a local port,
/// so the rest of the stack (`chat.rs`, `rig_lib`, Orchestrator) is
/// engine-agnostic.
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    /// Start the engine serving the given model.
    /// Returns the `(port, api_token)` the engine is listening on.
    async fn start(
        &self,
        model_path: &str,
        context_size: u32,
        options: EngineStartOptions,
    ) -> Result<(u16, String), String>;

    /// Stop the engine and free GPU/RAM.
    async fn stop(&self) -> Result<(), String>;

    /// Returns `true` if the engine's HTTP endpoint is accepting requests.
    async fn is_ready(&self) -> bool;

    /// The base URL for OpenAI-compatible API calls (e.g. `http://127.0.0.1:{port}/v1`).
    fn base_url(&self) -> Option<String>;

    /// The model identifier that the engine's server expects in request bodies.
    ///
    /// For `mlx_lm.server` this must match the `--model` argument (a local path
    /// or HF repo ID); for llama-server it's typically ignored.  If `None`,
    /// the caller should fall back to `"default"`.
    fn model_id(&self) -> Option<String> {
        None
    }

    /// The effective context window size for the currently loaded model.
    ///
    /// This is `min(user_requested_context, model_max_context)`.  Engines that
    /// don't track this should return `None`, and callers fall back to a safe
    /// default (e.g. 4096).
    fn max_context(&self) -> Option<u32> {
        None
    }

    /// Human-readable engine name for UI display.
    fn display_name(&self) -> &'static str;

    /// Engine identifier string (matches the Cargo feature name).
    fn engine_id(&self) -> &'static str;

    /// Returns `true` if this engine consumes single-file models (GGUF).
    /// Returns `false` if it expects a model directory (MLX safetensors, vLLM).
    fn uses_single_file_model(&self) -> bool;

    /// The HuggingFace tag used to filter compatible models in HF Hub search.
    fn hf_search_tag(&self) -> &'static str;
}

/// Read the model's native maximum context window from its `config.json`.
///
/// Checks these fields in order:
///   1. `max_position_embeddings` (root level — Llama, Qwen, Mistral, Phi, …)
///   2. `text_config.max_position_embeddings` (Gemma 3 multimodal wrapper)
///   3. `max_seq_len` / `max_sequence_length` / `n_ctx` / `context_length` (alternate names)
///
/// Returns `None` if the file doesn't exist or none of the fields are found.
pub fn read_model_max_context(model_path: &str) -> Option<u32> {
    let config_path = std::path::Path::new(model_path).join("config.json");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Try root-level fields first
    let root_fields = [
        "max_position_embeddings",
        "max_seq_len",
        "max_sequence_length",
        "n_ctx",
        "context_length",
        "seq_length",
    ];
    for field in &root_fields {
        if let Some(v) = json.get(field).and_then(|v| v.as_u64()) {
            return Some(v as u32);
        }
    }

    // Try nested text_config (Gemma 3, etc.)
    if let Some(tc) = json.get("text_config") {
        for field in &root_fields {
            if let Some(v) = tc.get(field).and_then(|v| v.as_u64()) {
                return Some(v as u32);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Options passed to `InferenceEngine::start()` beyond model path / context.
#[derive(Debug, Clone, Default)]
pub struct EngineStartOptions {
    pub n_gpu_layers: i32,
    pub template: Option<String>,
    pub mmproj: Option<String>,
    pub expose_network: bool,
    pub mlock: bool,
    pub quantize_kv: bool,
}

/// Information about the active inference engine, exposed to the frontend.
#[derive(Debug, Clone, Serialize, Type)]
pub struct EngineInfo {
    /// Engine identifier: `"llamacpp"`, `"mlx"`, `"vllm"`, `"ollama"`, or `"none"`.
    pub id: String,
    /// Human-readable name, e.g. `"llama.cpp (Metal)"`.
    pub display_name: String,
    /// Whether this engine is currently available and functional.
    pub available: bool,
    /// Whether first-launch setup is needed (e.g. MLX venv bootstrap).
    pub requires_setup: bool,
    /// Short description.
    pub description: String,
    /// HF tag used for model discovery filtering.
    pub hf_tag: String,
    /// Whether this engine uses single-file models (true) or directories (false).
    pub single_file_model: bool,
}

// ---------------------------------------------------------------------------
// Tauri command: get_active_engine_info
// ---------------------------------------------------------------------------

/// Returns information about the single inference engine compiled into this build.
///
/// The frontend uses this to:
/// - Filter HF Hub search results by the correct tag
/// - Know whether to show single-file (GGUF quant picker) or directory download UI
/// - Display the engine name in the status bar
#[tauri::command]
#[specta::specta]
pub fn get_active_engine_info() -> EngineInfo {
    // Exactly one of these feature flags is expected to be active per build.
    // Priority: mlx > vllm > llamacpp > ollama > none

    #[cfg(feature = "mlx")]
    {
        return EngineInfo {
            id: "mlx".into(),
            display_name: "MLX (Apple Silicon)".into(),
            available: true,
            requires_setup: false, // will be checked at runtime by engine_mlx
            description: "Apple's MLX framework — best performance on Apple Silicon".into(),
            hf_tag: "mlx".into(),
            single_file_model: false,
        };
    }

    #[cfg(feature = "vllm")]
    {
        return EngineInfo {
            id: "vllm".into(),
            display_name: "vLLM (CUDA)".into(),
            available: true,
            requires_setup: false,
            description: "High-throughput inference — requires NVIDIA GPU with CUDA".into(),
            hf_tag: "awq".into(),
            single_file_model: false,
        };
    }

    #[cfg(feature = "llamacpp")]
    {
        return EngineInfo {
            id: "llamacpp".into(),
            display_name: "llama.cpp".into(),
            available: true,
            requires_setup: false,
            description: "Fast local inference via llama.cpp (Metal/CUDA/CPU)".into(),
            hf_tag: "gguf".into(),
            single_file_model: true,
        };
    }

    #[cfg(feature = "ollama")]
    {
        return EngineInfo {
            id: "ollama".into(),
            display_name: "Ollama".into(),
            available: true,
            requires_setup: false,
            description: "Community model runner — install from ollama.ai".into(),
            hf_tag: "gguf".into(), // Ollama uses GGUF internally
            single_file_model: true,
        };
    }

    // No engine feature enabled — cloud-only build
    #[allow(unreachable_code)]
    EngineInfo {
        id: "none".into(),
        display_name: "Cloud Only".into(),
        available: true,
        requires_setup: false,
        description: "No local inference — use cloud providers only".into(),
        hf_tag: "".into(),
        single_file_model: false,
    }
}

// ---------------------------------------------------------------------------
// EngineManager — Tauri managed state holding the active engine instance
// ---------------------------------------------------------------------------

use std::path::PathBuf;

/// Managed state that holds the active inference engine instance.
///
/// Registered as `app.manage(EngineManager::new(app_data_dir))` in `lib.rs`.
pub struct EngineManager {
    pub engine: tokio::sync::Mutex<Option<Box<dyn InferenceEngine>>>,
    pub app_data_dir: PathBuf,
}

impl EngineManager {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let engine: Option<Box<dyn InferenceEngine>> = Self::create_engine(&app_data_dir);

        Self {
            engine: tokio::sync::Mutex::new(engine),
            app_data_dir,
        }
    }

    /// Create the engine instance based on compile-time feature flags.
    #[allow(unused_variables)]
    fn create_engine(app_data_dir: &PathBuf) -> Option<Box<dyn InferenceEngine>> {
        #[cfg(feature = "mlx")]
        {
            let engine = engine_mlx::MlxEngine::new();
            engine.set_app_data_dir(app_data_dir.clone());
            // Resolve the bundled `uv` sidecar binary path.
            // In dev: backend/bin/uv-{target}
            // In production: resolved by Tauri sidecar mechanism
            let uv_path = Self::resolve_uv_path();
            if let Some(path) = uv_path {
                engine.set_uv_path(path);
            }
            return Some(Box::new(engine));
        }

        #[cfg(feature = "vllm")]
        {
            let engine = engine_vllm::VllmEngine::new();
            engine.set_app_data_dir(app_data_dir.clone());
            let uv_path = Self::resolve_uv_path();
            if let Some(path) = uv_path {
                engine.set_uv_path(path);
            }
            return Some(Box::new(engine));
        }

        #[cfg(feature = "llamacpp")]
        {
            let engine = engine_llamacpp::LlamaCppEngine::new();
            return Some(Box::new(engine));
        }

        #[cfg(feature = "ollama")]
        {
            let engine = engine_ollama::OllamaEngine::new();
            return Some(Box::new(engine));
        }

        #[allow(unreachable_code)]
        None
    }

    /// Resolve the path to the `uv` binary.
    ///
    /// Search order:
    /// 1. `backend/bin/uv-{target-triple}` (dev builds — compile-time CARGO_MANIFEST_DIR)
    /// 2. Next to the app executable (production Tauri bundles)
    /// 3. `uv` on system PATH
    /// 4. `~/.thinclaw-desktop/uv` (auto-downloaded fallback)
    /// 5. `~/.scrappy/uv` (legacy readable fallback)
    ///
    /// If none found, returns `None` — the engine will auto-download in `bootstrap()`.
    #[allow(dead_code)]
    fn resolve_uv_path() -> Option<PathBuf> {
        let target_triple = Self::current_target_triple()?;
        let binary_name = format!("uv-{}", target_triple);

        // 1. Check compile-time manifest dir (dev builds: CARGO_MANIFEST_DIR = backend/)
        {
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let dev_path = PathBuf::from(manifest_dir).join("bin").join(&binary_name);
            if dev_path.exists() {
                println!("[engine] Found uv sidecar at {:?}", dev_path);
                return Some(dev_path);
            }
        }

        // 2. Check relative to the current exe (production builds)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let prod_path = exe_dir.join(&binary_name);
                if prod_path.exists() {
                    println!("[engine] Found uv sidecar at {:?}", prod_path);
                    return Some(prod_path);
                }
            }
        }

        // 3. Check if uv is on PATH
        if let Ok(output) = std::process::Command::new("which").arg("uv").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    println!("[engine] Found system uv at {}", path);
                    return Some(PathBuf::from(path));
                }
            }
        }

        // 4. Check ~/.thinclaw-desktop/uv (auto-download location)
        if let Ok(home) = std::env::var("HOME") {
            let home = PathBuf::from(home);
            let local_uv = home.join(".thinclaw-desktop").join("uv");
            if local_uv.exists() {
                println!("[engine] Found local uv at {:?}", local_uv);
                return Some(local_uv);
            }

            let legacy_uv = home.join(".scrappy").join("uv");
            if legacy_uv.exists() {
                println!("[engine] Found legacy local uv at {:?}", legacy_uv);
                return Some(legacy_uv);
            }
        }

        println!("[engine] uv binary not found — will auto-download during bootstrap");
        None
    }

    /// Get the current target triple string.
    #[allow(dead_code)]
    fn current_target_triple() -> Option<&'static str> {
        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            Some("aarch64-apple-darwin")
        } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
            Some("x86_64-apple-darwin")
        } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            Some("x86_64-unknown-linux-gnu")
        } else {
            None
        }
    }
}

/// Setup status returned to the frontend.
#[derive(Debug, Clone, Serialize, Type)]
pub struct EngineSetupStatus {
    /// Whether the engine needs first-launch setup (Python bootstrap).
    pub needs_setup: bool,
    /// Whether setup is currently in progress.
    pub setup_in_progress: bool,
    /// Human-readable status message.
    pub message: String,
}

/// Returns whether the active engine needs first-launch setup.
///
/// - `llamacpp`: never needs setup (bundled sidecar)
/// - `ollama`: never needs setup (external daemon)
/// - `mlx` / `vllm`: need setup if the Python venv hasn't been bootstrapped yet
#[tauri::command]
#[specta::specta]
pub fn get_engine_setup_status(
    #[allow(unused_variables)] engine_manager: tauri::State<'_, EngineManager>,
) -> EngineSetupStatus {
    let info = get_active_engine_info();

    // Only MLX and vLLM need bootstrap
    let needs_setup = match info.id.as_str() {
        #[cfg(feature = "mlx")]
        "mlx" => {
            // Check if mlx-env exists
            !engine_manager
                .app_data_dir
                .join("mlx-env")
                .join("bin")
                .join("python3")
                .exists()
        }
        #[cfg(feature = "vllm")]
        "vllm" => !engine_manager
            .app_data_dir
            .join("vllm-env")
            .join("bin")
            .join("python3")
            .exists(),
        _ => false,
    };

    EngineSetupStatus {
        needs_setup,
        setup_in_progress: false, // simplified — real progress tracked via events
        message: if needs_setup {
            format!(
                "{} requires first-launch setup (~2 minutes)",
                info.display_name
            )
        } else {
            format!("{} is ready", info.display_name)
        },
    }
}

/// Trigger first-launch bootstrap for the active engine (MLX/vLLM).
///
/// Emits `engine_setup_progress` events:
/// `{ stage: "creating_venv" | "installing" | "complete" | "error", message: String }`
#[tauri::command]
#[specta::specta]
pub async fn setup_engine(
    app: tauri::AppHandle,
    _engine_manager: tauri::State<'_, EngineManager>,
) -> Result<(), String> {
    let info = get_active_engine_info();

    #[derive(Clone, serde::Serialize)]
    struct SetupProgress {
        stage: String,
        message: String,
    }

    #[allow(unused)]
    let emit = |stage: &str, msg: &str| {
        let _ = app.emit(
            "engine_setup_progress",
            SetupProgress {
                stage: stage.to_string(),
                message: msg.to_string(),
            },
        );
    };

    match info.id.as_str() {
        #[cfg(feature = "mlx")]
        "mlx" => {
            emit("creating_venv", "Setting up MLX environment...");

            // Create a temporary engine for bootstrap (the managed one is behind tokio::Mutex)
            let engine = engine_mlx::MlxEngine::new();
            engine.set_app_data_dir(_engine_manager.app_data_dir.clone());

            emit(
                "installing",
                "Installing mlx-openai-server (this may take 2-3 minutes)...",
            );
            engine.bootstrap().await?;

            emit("complete", "MLX setup complete!");
            Ok(())
        }
        #[cfg(feature = "vllm")]
        "vllm" => {
            emit("creating_venv", "Setting up vLLM environment...");

            let engine = engine_vllm::VllmEngine::new();
            engine.set_app_data_dir(_engine_manager.app_data_dir.clone());

            emit(
                "installing",
                "Installing vLLM (this may take 5-10 minutes)...",
            );
            engine.bootstrap().await?;

            emit("complete", "vLLM setup complete!");
            Ok(())
        }
        _ => {
            // llamacpp and ollama don't need setup
            Ok(())
        }
    }
}

/// Start the active engine with the given model.
///
/// This is the new engine-aware entry point. For llamacpp builds, the existing
/// `start_chat_server` in sidecar.rs still works — this command is for MLX/vLLM/Ollama.
#[tauri::command]
#[specta::specta]
pub async fn start_engine(
    engine_manager: tauri::State<'_, EngineManager>,
    model_path: String,
    context_size: u32,
) -> Result<EngineStartResult, String> {
    let mut guard = engine_manager.engine.lock().await;
    let engine = guard.as_mut().ok_or("No engine configured")?;

    let options = EngineStartOptions::default();
    let (port, token) = engine.start(&model_path, context_size, options).await?;

    Ok(EngineStartResult { port, token })
}

/// Result of starting an engine.
#[derive(Debug, Clone, Serialize, Type)]
pub struct EngineStartResult {
    pub port: u16,
    pub token: String,
}

/// Stop the active engine.
#[tauri::command]
#[specta::specta]
pub async fn stop_engine(engine_manager: tauri::State<'_, EngineManager>) -> Result<(), String> {
    let mut guard = engine_manager.engine.lock().await;
    if let Some(engine) = guard.as_mut() {
        engine.stop().await?;
    }
    Ok(())
}

/// Check if the active engine is ready (health check).
#[tauri::command]
#[specta::specta]
pub async fn is_engine_ready(
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<bool, String> {
    let guard = engine_manager.engine.lock().await;
    if let Some(engine) = guard.as_ref() {
        Ok(engine.is_ready().await)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_active_engine_returns_valid_info() {
        let info = get_active_engine_info();
        assert!(!info.id.is_empty(), "engine id must not be empty");
        assert!(
            !info.display_name.is_empty(),
            "display_name must not be empty"
        );
        assert!(!info.hf_tag.is_empty(), "hf_tag must not be empty");

        // Feature-specific assertions. When multiple features are compiled
        // together, the first one wins (mlx > llamacpp > vllm > ollama).
        #[cfg(feature = "mlx")]
        {
            assert_eq!(info.id, "mlx");
            assert_eq!(info.hf_tag, "mlx");
            assert!(!info.single_file_model);
        }

        #[cfg(all(feature = "llamacpp", not(feature = "mlx")))]
        {
            assert_eq!(info.id, "llamacpp");
            assert_eq!(info.hf_tag, "gguf");
            assert!(info.single_file_model);
        }
    }

    #[test]
    fn engine_info_serializes() {
        let info = get_active_engine_info();
        let json = serde_json::to_string(&info).expect("EngineInfo should serialize");
        assert!(json.contains(&info.id));
    }

    #[test]
    fn read_max_context_root_level() {
        let dir = std::env::temp_dir().join("scrappy_test_ctx_root");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"max_position_embeddings": 131072}"#,
        )
        .unwrap();
        assert_eq!(read_model_max_context(dir.to_str().unwrap()), Some(131072));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_max_context_nested_text_config() {
        // Gemma 3 VLMs put max_position_embeddings inside text_config
        let dir = std::env::temp_dir().join("scrappy_test_ctx_nested");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "gemma3", "text_config": {"max_position_embeddings": 8192}}"#,
        )
        .unwrap();
        assert_eq!(read_model_max_context(dir.to_str().unwrap()), Some(8192));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_max_context_alternate_field_names() {
        let dir = std::env::temp_dir().join("scrappy_test_ctx_alt");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("config.json"), r#"{"n_ctx": 4096}"#).unwrap();
        assert_eq!(read_model_max_context(dir.to_str().unwrap()), Some(4096));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_max_context_missing_config() {
        assert_eq!(read_model_max_context("/nonexistent/path/to/model"), None);
    }
}

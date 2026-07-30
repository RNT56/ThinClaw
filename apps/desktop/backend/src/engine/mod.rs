//! Multi-engine inference abstraction.
//!
//! Each ThinClaw Desktop build targets **one** inference engine per platform
//! (determined by Cargo feature flags at compile time). This module provides
//! the `InferenceEngine` trait that all engines implement, plus the
//! `direct_runtime_get_active_engine_info` Tauri command that tells the frontend which
//! engine is active.

use async_trait::async_trait;
use serde::Serialize;
use specta::Type;
#[cfg(any(feature = "mlx", feature = "vllm"))]
use std::collections::VecDeque;
#[cfg(any(feature = "mlx", feature = "vllm"))]
use std::sync::Arc;
use tauri::Emitter;

/// Engine dependency pins are sourced from `apps/desktop/engine-manifest.json`
/// by `build.rs`. Python transitive dependencies are locked with hashes in
/// `runtime/mlx/requirements.lock`; these aliases exist only for diagnostics
/// and focused contract tests.
#[cfg(any(feature = "mlx", feature = "vllm", test))]
pub(crate) const UV_VERSION: &str = env!("THINCLAW_UV_VERSION");
#[cfg(any(feature = "mlx", test))]
pub(crate) const MLX_OPENAI_SERVER_VERSION: &str = env!("THINCLAW_MLX_SERVER_VERSION");
#[cfg(any(feature = "mlx", test))]
pub(crate) const MLX_MINIMUM_MACOS: &str = env!("THINCLAW_MLX_MINIMUM_MACOS");
#[cfg(any(feature = "vllm", test))]
pub(crate) const VLLM_VERSION: &str = env!("THINCLAW_VLLM_VERSION");
#[cfg(any(feature = "vllm", test))]
pub(crate) const VLLM_TORCH_BACKEND: &str = env!("THINCLAW_VLLM_TORCH_BACKEND");
#[cfg(any(feature = "vllm", test))]
pub(crate) const VLLM_MINIMUM_GLIBC: &str = env!("THINCLAW_VLLM_MINIMUM_GLIBC");
#[cfg(any(feature = "vllm", test))]
pub(crate) const VLLM_MINIMUM_COMPUTE_CAPABILITY: &str =
    env!("THINCLAW_VLLM_MINIMUM_COMPUTE_CAPABILITY");
#[cfg(any(feature = "mlx", feature = "vllm", test))]
pub(crate) const PYTHON_VERSION: &str = env!("THINCLAW_PYTHON_VERSION");
#[cfg(feature = "mlx")]
pub(crate) const PYTHON_ABI: &str = env!("THINCLAW_PYTHON_ABI");

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

    /// Credential required by the local endpoint. Public runtime snapshots
    /// redact this; internal runtime wiring retains it.
    fn api_key(&self) -> Option<String> {
        None
    }

    /// The model identifier that the engine's server expects in request bodies.
    ///
    /// For `mlx-openai-server` this must match the model argument (a local path
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
    let content =
        thinclaw_platform::read_regular_file_bounded(&config_path, 4 * 1024 * 1024).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&content).ok()?;

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
            return u32::try_from(v).ok().filter(|value| *value > 0);
        }
    }

    // Try nested text_config (Gemma 3, etc.)
    if let Some(tc) = json.get("text_config") {
        for field in &root_fields {
            if let Some(v) = tc.get(field).and_then(|v| v.as_u64()) {
                return u32::try_from(v).ok().filter(|value| *value > 0);
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

/// Bounded in-memory diagnostics for long-running inference servers. Keeping
/// output out of persistent files avoids leaking ephemeral API credentials,
/// while draining both pipes prevents child processes from blocking.
#[derive(Clone, Default)]
#[cfg(any(feature = "mlx", feature = "vllm"))]
pub(crate) struct RuntimeDiagnostics {
    lines: Arc<tokio::sync::Mutex<VecDeque<String>>>,
}

#[cfg(any(feature = "mlx", feature = "vllm"))]
impl RuntimeDiagnostics {
    const MAX_LINES: usize = 80;
    const MAX_LINE_BYTES: usize = 4 * 1024;
    const MAX_SUMMARY_BYTES: usize = 16 * 1024;

    pub async fn reset(&self) {
        self.lines.lock().await.clear();
    }

    pub fn capture<R>(&self, pipe: R, stream: &'static str, redactions: Vec<String>)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let lines = self.lines.clone();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(pipe);
            loop {
                let line =
                    match thinclaw_platform::read_bounded_line(&mut reader, Self::MAX_LINE_BYTES)
                        .await
                    {
                        Ok(Some(line)) => line,
                        Ok(None) => break,
                        Err(error) => {
                            let mut guard = lines.lock().await;
                            guard.push_back(format!("{stream}: output read failed: {error}"));
                            break;
                        }
                    };
                let mut text = line.into_lossy_text();
                for secret in &redactions {
                    if !secret.is_empty() {
                        text = text.replace(secret, "[REDACTED]");
                    }
                }
                let text = text
                    .chars()
                    .filter(|character| !character.is_control() || *character == '\t')
                    .collect::<String>();
                let mut guard = lines.lock().await;
                if guard.len() == Self::MAX_LINES {
                    guard.pop_front();
                }
                guard.push_back(format!("{stream}: {text}"));
            }
        });
    }

    pub async fn summary(&self) -> String {
        let lines = self.lines.lock().await;
        if lines.is_empty() {
            "No server diagnostics were emitted.".to_string()
        } else {
            let combined = lines.iter().cloned().collect::<Vec<_>>().join("\n");
            if combined.len() <= Self::MAX_SUMMARY_BYTES {
                combined
            } else {
                let mut start = combined.len() - Self::MAX_SUMMARY_BYTES;
                while !combined.is_char_boundary(start) {
                    start += 1;
                }
                format!("… earlier diagnostics omitted …\n{}", &combined[start..])
            }
        }
    }
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
// Tauri command: direct_runtime_get_active_engine_info
// ---------------------------------------------------------------------------

/// Returns information about the single inference engine compiled into this build.
///
/// The frontend uses this to:
/// - Filter HF Hub search results by the correct tag
/// - Know whether to show single-file (GGUF quant picker) or directory download UI
/// - Display the engine name in the status bar
#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_active_engine_info() -> EngineInfo {
    // Exactly one of these feature flags is expected to be active per build.
    // Priority: mlx > vllm > llamacpp > ollama > none

    #[cfg(feature = "mlx")]
    {
        return EngineInfo {
            id: "mlx".into(),
            display_name: "MLX (Apple Silicon)".into(),
            available: engine_supported_on_host("mlx"),
            requires_setup: true,
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
            available: engine_supported_on_host("vllm"),
            requires_setup: true,
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
            description: "Community model runner — install from ollama.com".into(),
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

/// Return model identifiers currently installed in the local Ollama daemon.
///
/// The command intentionally accepts no endpoint, token, or headers. Ollama
/// discovery is limited to the unauthenticated loopback `/api/tags` endpoint
/// and applies strict time, response-size, count, and identifier bounds.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_list_ollama_models(
) -> Result<Vec<String>, crate::thinclaw::bridge::BridgeError> {
    #[cfg(feature = "ollama")]
    {
        return engine_ollama::list_installed_models()
            .await
            .map_err(Into::into);
    }

    #[cfg(not(feature = "ollama"))]
    {
        Err("This desktop build does not use the Ollama runtime".into())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_snapshot(
    sidecar: tauri::State<'_, crate::sidecar::SidecarManager>,
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<LocalRuntimeSnapshot, crate::thinclaw::bridge::BridgeError> {
    Ok(local_runtime_snapshot(&sidecar, &engine_manager)
        .await
        .redacted_for_public_clients())
}

// ---------------------------------------------------------------------------
// EngineManager — Tauri managed state holding the active engine instance
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use thinclaw_runtime_contracts::{
    LocalRuntimeEndpoint, LocalRuntimeKind, LocalRuntimeSnapshot, RuntimeCapability,
    RuntimeExposurePolicy, RuntimeReadiness,
};

/// Managed state that holds the active inference engine instance.
///
/// Registered as `app.manage(EngineManager::new(app_data_dir))` in `lib.rs`.
pub struct EngineManager {
    pub engine: tokio::sync::Mutex<Option<Box<dyn InferenceEngine>>>,
    /// Canonical local model artifact currently served by `engine`.
    active_model_path: tokio::sync::Mutex<Option<PathBuf>>,
    pub app_data_dir: PathBuf,
    provisioning: tokio::sync::RwLock<ProvisioningRecord>,
    provisioning_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
struct ProvisioningRecord {
    state: EngineProvisioningState,
    message: String,
    error: Option<String>,
}

fn runtime_kind_from_engine_id(engine_id: &str) -> LocalRuntimeKind {
    match engine_id {
        "llamacpp" => LocalRuntimeKind::LlamaCpp,
        "mlx" => LocalRuntimeKind::Mlx,
        "vllm" => LocalRuntimeKind::Vllm,
        "ollama" => LocalRuntimeKind::Ollama,
        _ => LocalRuntimeKind::None,
    }
}

fn push_capability(capabilities: &mut Vec<RuntimeCapability>, capability: RuntimeCapability) {
    if !capabilities.contains(&capability) {
        capabilities.push(capability);
    }
}

fn sidecar_active_capabilities(sidecar: &crate::sidecar::SidecarManager) -> Vec<RuntimeCapability> {
    let mut capabilities = Vec::new();
    if sidecar.get_embedding_config().is_some() {
        push_capability(&mut capabilities, RuntimeCapability::Embedding);
    }
    if sidecar.is_stt_active() {
        push_capability(&mut capabilities, RuntimeCapability::Stt);
    }
    if sidecar.is_tts_configured() {
        push_capability(&mut capabilities, RuntimeCapability::Tts);
    }
    if sidecar.is_image_configured() {
        push_capability(&mut capabilities, RuntimeCapability::Diffusion);
    }
    capabilities
}

fn active_capabilities_for_runtime(
    engine_id: &str,
    sidecar: &crate::sidecar::SidecarManager,
) -> Vec<RuntimeCapability> {
    let mut capabilities = vec![RuntimeCapability::Chat];
    for capability in sidecar_active_capabilities(sidecar) {
        push_capability(&mut capabilities, capability);
    }

    // MLX auxiliary services are launched through SidecarManager, so their
    // active state is represented above. vLLM and Ollama expose chat only.
    match engine_id {
        "llamacpp" | "mlx" | "vllm" | "ollama" => capabilities,
        _ => Vec::new(),
    }
}

fn supported_capabilities_for_runtime(engine_id: &str) -> Vec<RuntimeCapability> {
    match engine_id {
        "llamacpp" => vec![
            RuntimeCapability::Chat,
            RuntimeCapability::Embedding,
            RuntimeCapability::Stt,
            RuntimeCapability::Tts,
            RuntimeCapability::Diffusion,
        ],
        "mlx" => vec![
            RuntimeCapability::Chat,
            RuntimeCapability::Embedding,
            RuntimeCapability::Stt,
            RuntimeCapability::Diffusion,
        ],
        "vllm" | "ollama" => vec![RuntimeCapability::Chat],
        _ => Vec::new(),
    }
}

#[allow(unused_variables)]
fn engine_needs_setup(info: &EngineInfo, engine_manager: &EngineManager) -> bool {
    match info.id.as_str() {
        #[cfg(feature = "mlx")]
        "mlx" => {
            let engine = engine_mlx::MlxEngine::new();
            engine.set_app_data_dir(engine_manager.app_data_dir.clone());
            !engine.is_bootstrapped()
        }
        #[cfg(feature = "vllm")]
        "vllm" => {
            let engine = engine_vllm::VllmEngine::new();
            engine.set_app_data_dir(engine_manager.app_data_dir.clone());
            !engine.is_bootstrapped()
        }
        _ => false,
    }
}

/// Build the shared local runtime snapshot consumed by Direct Workbench and
/// the ThinClaw runtime bridge.
pub async fn local_runtime_snapshot(
    sidecar: &crate::sidecar::SidecarManager,
    engine_manager: &EngineManager,
) -> LocalRuntimeSnapshot {
    let info = direct_runtime_get_active_engine_info();
    let kind = runtime_kind_from_engine_id(&info.id);

    if let Some((port, token, context_size, model_family)) = sidecar.get_chat_config() {
        return LocalRuntimeSnapshot {
            kind,
            display_name: info.display_name,
            readiness: RuntimeReadiness::Ready,
            endpoint: Some(LocalRuntimeEndpoint {
                base_url: format!("http://127.0.0.1:{port}/v1"),
                api_key: if token.is_empty() { None } else { Some(token) },
                model_id: Some("default".to_string()),
                context_size: Some(context_size),
                model_family: Some(model_family),
            }),
            capabilities: active_capabilities_for_runtime(&info.id, sidecar),
            supported_capabilities: supported_capabilities_for_runtime(&info.id),
            exposure_policy: RuntimeExposurePolicy::SharedWhenEnabled,
            unavailable_reason: None,
        };
    }

    let guard = engine_manager.engine.lock().await;
    if let Some(engine) = guard.as_ref() {
        if engine.is_ready().await {
            if let Some(base_url) = engine.base_url() {
                let engine_id = engine.engine_id();
                return LocalRuntimeSnapshot {
                    kind: runtime_kind_from_engine_id(engine_id),
                    display_name: engine.display_name().to_string(),
                    readiness: RuntimeReadiness::Ready,
                    endpoint: Some(LocalRuntimeEndpoint {
                        base_url,
                        api_key: engine.api_key(),
                        model_id: engine.model_id(),
                        context_size: engine.max_context(),
                        model_family: None,
                    }),
                    capabilities: active_capabilities_for_runtime(engine_id, sidecar),
                    supported_capabilities: supported_capabilities_for_runtime(engine_id),
                    exposure_policy: RuntimeExposurePolicy::SharedWhenEnabled,
                    unavailable_reason: None,
                };
            }
        } else if let Some(base_url) = engine.base_url() {
            let engine_id = engine.engine_id();
            let readiness = if engine_id == "ollama" {
                RuntimeReadiness::Unavailable
            } else {
                RuntimeReadiness::Starting
            };
            return LocalRuntimeSnapshot {
                kind: runtime_kind_from_engine_id(engine_id),
                display_name: engine.display_name().to_string(),
                readiness,
                endpoint: None,
                capabilities: Vec::new(),
                supported_capabilities: supported_capabilities_for_runtime(engine_id),
                exposure_policy: RuntimeExposurePolicy::SharedWhenEnabled,
                unavailable_reason: Some(if engine_id == "ollama" {
                    "Ollama daemon is not running. Start it with `ollama serve`.".to_string()
                } else {
                    format!("Local runtime endpoint {base_url} is not ready yet")
                }),
            };
        }
    }
    drop(guard);

    let live_provisioning = engine_manager.provisioning.read().await.clone();
    let provisioning = if matches!(
        live_provisioning.state,
        EngineProvisioningState::Installing | EngineProvisioningState::Broken
    ) {
        live_provisioning
    } else {
        derived_provisioning_state(&info, engine_manager)
    };
    let readiness = match provisioning.state {
        EngineProvisioningState::Installing | EngineProvisioningState::Checking => {
            RuntimeReadiness::Starting
        }
        EngineProvisioningState::NeedsSetup | EngineProvisioningState::Broken => {
            RuntimeReadiness::SetupRequired
        }
        EngineProvisioningState::Unsupported => RuntimeReadiness::Unavailable,
        EngineProvisioningState::Ready => RuntimeReadiness::Unavailable,
    };

    LocalRuntimeSnapshot {
        kind,
        display_name: info.display_name,
        readiness,
        endpoint: None,
        capabilities: Vec::new(),
        supported_capabilities: supported_capabilities_for_runtime(&info.id),
        exposure_policy: RuntimeExposurePolicy::SharedWhenEnabled,
        unavailable_reason: Some(if provisioning.state != EngineProvisioningState::Ready {
            provisioning
                .error
                .unwrap_or_else(|| provisioning.message.clone())
        } else {
            "No local chat runtime endpoint is running".to_string()
        }),
    }
}

/// Convert a runtime snapshot into the legacy local LLM tuple consumed by
/// ThinClaw Desktop's config writer.
///
/// The tuple shape predates `LocalRuntimeSnapshot` and stores only
/// `(port, api_key, context_size, model_family)`. Keep this adapter at the
/// boundary so newer runtime selection still flows through the shared snapshot.
pub fn local_runtime_snapshot_to_local_llm(
    snapshot: &LocalRuntimeSnapshot,
) -> Option<(u16, String, u32, String)> {
    let endpoint = snapshot.endpoint.as_ref()?;
    let parsed = reqwest::Url::parse(&endpoint.base_url).ok()?;
    let port = parsed.port_or_known_default()?;
    Some((
        port,
        endpoint.api_key.clone().unwrap_or_default(),
        endpoint.context_size.unwrap_or(16_384),
        endpoint
            .model_family
            .clone()
            .unwrap_or_else(|| "chatml".to_string()),
    ))
}

impl EngineManager {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let engine: Option<Box<dyn InferenceEngine>> = Self::create_engine(&app_data_dir);

        Self {
            engine: tokio::sync::Mutex::new(engine),
            active_model_path: tokio::sync::Mutex::new(None),
            app_data_dir,
            provisioning: tokio::sync::RwLock::new(ProvisioningRecord {
                state: EngineProvisioningState::Checking,
                message: "Checking local inference runtime...".to_string(),
                error: None,
            }),
            provisioning_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) async fn stop_active_engine_locked(&self) -> Result<(), String> {
        let mut engine = self.engine.lock().await;
        let result = match engine.as_mut() {
            Some(engine) => engine.stop().await,
            None => Ok(()),
        };
        // A failed process-tree kill can still consume the engine handle and
        // clear its endpoint state. Never retain a stale target after stop was
        // attempted.
        *self.active_model_path.lock().await = None;
        result
    }

    /// Replace the currently served model under the caller-held global model
    /// lifecycle lock. MLX and vLLM deliberately reject a second `start`, so
    /// every entry point must use this stop-then-start boundary rather than
    /// relying on individual frontend callers to orchestrate a restart.
    async fn replace_active_engine_locked(
        &self,
        model_path: &str,
        canonical_model_path: Option<PathBuf>,
        context_size: u32,
        options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        let mut engine = self.engine.lock().await;
        let engine = engine.as_mut().ok_or("No engine configured")?;
        let stop_result = engine.stop().await;
        *self.active_model_path.lock().await = None;
        stop_result?;

        let endpoint = engine.start(model_path, context_size, options).await?;
        *self.active_model_path.lock().await = canonical_model_path;
        Ok(endpoint)
    }

    /// Stop the engine only when its backend-owned canonical model path is the
    /// selected install root or a file within that root. The caller holds the
    /// shared model lifecycle lock.
    pub(crate) async fn stop_if_using_install_locked(
        &self,
        install_root: &Path,
    ) -> Result<bool, String> {
        let uses_install = self
            .active_model_path
            .lock()
            .await
            .as_deref()
            .is_some_and(|path| {
                crate::model_lifecycle::model_path_uses_install(path, install_root)
            });
        if !uses_install {
            return Ok(false);
        }
        self.stop_active_engine_locked().await?;
        Ok(true)
    }

    /// Create the engine instance based on compile-time feature flags.
    #[allow(unused_variables)]
    fn create_engine(app_data_dir: &Path) -> Option<Box<dyn InferenceEngine>> {
        #[cfg(feature = "mlx")]
        {
            let engine = engine_mlx::MlxEngine::new();
            engine.set_app_data_dir(app_data_dir.to_path_buf());
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
            engine.set_app_data_dir(app_data_dir.to_path_buf());
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
    /// 1. Stable installed name next to the app executable (`uv` / `uv.exe`)
    /// 2. Target-suffixed source asset in explicit development builds
    /// 3. System PATH only when explicitly opted in for development
    ///
    /// Every result is canonicalized and checked as an executable regular file.
    /// If none is found, setup fails closed instead of executing an unverified
    /// binary. Tauri removes the target suffix when it packages an external
    /// binary, so source and installed names intentionally differ.
    #[allow(dead_code)]
    fn resolve_uv_path() -> Option<PathBuf> {
        fn checked_candidate(path: PathBuf) -> Option<PathBuf> {
            let path = path.canonicalize().ok()?;
            let metadata = std::fs::metadata(&path).ok()?;
            if !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            Some(path)
        }

        // 1. Tauri packages `bin/uv-{target}` as `uv` (or `uv.exe`) beside
        // the application executable.
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let prod_path = exe_dir.join(if cfg!(windows) { "uv.exe" } else { "uv" });
                if let Some(path) = checked_candidate(prod_path) {
                    tracing::info!("Using packaged uv sidecar");
                    return Some(path);
                }
            }
        }

        // 2. A compile-time source path is acceptable only for a debug build.
        // Never let a release binary silently depend on the checkout that
        // happened to compile it.
        if cfg!(debug_assertions) {
            let target_triple = Self::current_target_triple()?;
            let binary_name = format!(
                "uv-{target_triple}{}",
                if cfg!(windows) { ".exe" } else { "" }
            );
            let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("bin")
                .join(binary_name);
            if let Some(path) = checked_candidate(dev_path) {
                tracing::info!("Using development uv sidecar");
                return Some(path);
            }
        }

        // 3. System uv is an explicit builder/developer override, never a
        // production fallback.
        if std::env::var("THINCLAW_ALLOW_SYSTEM_UV").as_deref() == Ok("1") {
            let name = if cfg!(windows) { "uv.exe" } else { "uv" };
            if let Some(path) =
                thinclaw_platform::find_executable_in_path(name).and_then(checked_candidate)
            {
                tracing::info!("Using explicitly allowed uv from PATH");
                return Some(path);
            }
        }

        tracing::warn!("uv binary is unavailable; local Python engine setup will fail closed");
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

/// Durable backend-owned provisioning phase for the compiled local engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineProvisioningState {
    Checking,
    Unsupported,
    NeedsSetup,
    Installing,
    Ready,
    Broken,
}

/// Setup status returned to the frontend.
#[derive(Debug, Clone, Serialize, Type)]
pub struct EngineSetupStatus {
    /// Machine-readable provisioning state.
    pub state: EngineProvisioningState,
    /// Whether the engine needs first-launch setup (Python bootstrap).
    pub needs_setup: bool,
    /// Whether setup is currently in progress.
    pub setup_in_progress: bool,
    /// Human-readable status message.
    pub message: String,
    /// Last bounded setup failure, if any.
    pub error: Option<String>,
}

fn engine_supported_on_host(engine_id: &str) -> bool {
    match engine_id {
        "mlx" => cfg!(all(target_os = "macos", target_arch = "aarch64")),
        "vllm" => cfg!(all(target_os = "linux", target_arch = "x86_64")),
        "llamacpp" | "ollama" | "none" => true,
        _ => false,
    }
}

fn derived_provisioning_state(
    info: &EngineInfo,
    engine_manager: &EngineManager,
) -> ProvisioningRecord {
    if !engine_supported_on_host(&info.id) {
        return ProvisioningRecord {
            state: EngineProvisioningState::Unsupported,
            message: format!("{} is unsupported on this host", info.display_name),
            error: Some(
                "Install a ThinClaw build containing an engine supported by this operating system and architecture"
                    .to_string(),
            ),
        };
    }
    if engine_needs_setup(info, engine_manager) {
        ProvisioningRecord {
            state: EngineProvisioningState::NeedsSetup,
            message: format!("{} requires local runtime provisioning", info.display_name),
            error: None,
        }
    } else {
        ProvisioningRecord {
            state: EngineProvisioningState::Ready,
            message: format!("{} runtime is installed and validated", info.display_name),
            error: None,
        }
    }
}

/// Return the backend-owned provisioning status. Filesystem validation remains
/// authoritative across restarts; the in-memory record supplies live progress
/// and the last actionable failure.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_get_engine_setup_status(
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<EngineSetupStatus, crate::thinclaw::bridge::BridgeError> {
    let info = direct_runtime_get_active_engine_info();
    let live = engine_manager.provisioning.read().await.clone();
    let record = if live.state == EngineProvisioningState::Installing {
        live
    } else {
        let derived = derived_provisioning_state(&info, &engine_manager);
        if live.state == EngineProvisioningState::Broken
            && derived.state != EngineProvisioningState::Ready
        {
            live
        } else {
            derived
        }
    };
    Ok(EngineSetupStatus {
        state: record.state,
        needs_setup: matches!(
            record.state,
            EngineProvisioningState::NeedsSetup | EngineProvisioningState::Broken
        ),
        setup_in_progress: record.state == EngineProvisioningState::Installing,
        message: record.message,
        error: record.error,
    })
}

#[derive(Clone, serde::Serialize)]
struct SetupProgress {
    stage: String,
    message: String,
}

fn emit_setup_progress(app: &tauri::AppHandle, stage: &str, message: &str) {
    let _ = app.emit(
        "engine_setup_progress",
        SetupProgress {
            stage: stage.to_string(),
            message: message.to_string(),
        },
    );
}

async fn update_provisioning(
    engine_manager: &EngineManager,
    state: EngineProvisioningState,
    message: impl Into<String>,
    error: Option<String>,
) {
    *engine_manager.provisioning.write().await = ProvisioningRecord {
        state,
        message: message.into(),
        error,
    };
}

async fn provision_active_engine(
    app: &tauri::AppHandle,
    engine_manager: &EngineManager,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let _provisioning_guard = engine_manager.provisioning_lock.lock().await;
    let info = direct_runtime_get_active_engine_info();

    if !engine_supported_on_host(&info.id) {
        let message = format!("{} is unsupported on this host", info.display_name);
        update_provisioning(
            engine_manager,
            EngineProvisioningState::Unsupported,
            &message,
            Some(message.clone()),
        )
        .await;
        emit_setup_progress(app, "error", &message);
        return Err(message.into());
    }

    {
        let engine = engine_manager.engine.lock().await;
        if engine
            .as_ref()
            .and_then(|engine| engine.base_url())
            .is_some()
        {
            return Err("Stop the local inference engine before repairing its runtime".into());
        }
    }

    if !engine_needs_setup(&info, engine_manager) {
        let message = format!("{} runtime is already ready", info.display_name);
        update_provisioning(
            engine_manager,
            EngineProvisioningState::Ready,
            &message,
            None,
        )
        .await;
        emit_setup_progress(app, "complete", &message);
        return Ok(());
    }

    let installing_message = match info.id.as_str() {
        "mlx" => "Installing the locked MLX runtime and Python environment...",
        "vllm" => "Installing the validated vLLM and CUDA runtime environment...",
        _ => "Validating the bundled local inference runtime...",
    };
    update_provisioning(
        engine_manager,
        EngineProvisioningState::Installing,
        installing_message,
        None,
    )
    .await;
    emit_setup_progress(app, "creating_venv", installing_message);

    let result: Result<(), String> = match info.id.as_str() {
        #[cfg(feature = "mlx")]
        "mlx" => {
            let engine = engine_mlx::MlxEngine::new();
            engine.set_app_data_dir(engine_manager.app_data_dir.clone());
            if let Some(path) = EngineManager::resolve_uv_path() {
                engine.set_uv_path(path);
            }
            emit_setup_progress(
                app,
                "installing",
                "Installing and validating the hash-locked MLX service stack...",
            );
            engine.bootstrap().await
        }
        #[cfg(feature = "vllm")]
        "vllm" => {
            let engine = engine_vllm::VllmEngine::new();
            engine.set_app_data_dir(engine_manager.app_data_dir.clone());
            if let Some(path) = EngineManager::resolve_uv_path() {
                engine.set_uv_path(path);
            }
            emit_setup_progress(
                app,
                "installing",
                "Installing and validating the vLLM CUDA service stack...",
            );
            engine.bootstrap().await
        }
        _ => Ok(()),
    };

    match result {
        Ok(()) if !engine_needs_setup(&info, engine_manager) => {
            let message = format!("{} runtime is ready", info.display_name);
            update_provisioning(
                engine_manager,
                EngineProvisioningState::Ready,
                &message,
                None,
            )
            .await;
            emit_setup_progress(app, "complete", &message);
            Ok(())
        }
        Ok(()) => {
            let message =
                "Provisioning finished but the runtime did not pass validation".to_string();
            update_provisioning(
                engine_manager,
                EngineProvisioningState::Broken,
                &message,
                Some(message.clone()),
            )
            .await;
            emit_setup_progress(app, "error", &message);
            Err(message.into())
        }
        Err(error) => {
            let error = error.chars().take(2_048).collect::<String>();
            let message = format!("{} setup failed", info.display_name);
            update_provisioning(
                engine_manager,
                EngineProvisioningState::Broken,
                &message,
                Some(error.clone()),
            )
            .await;
            emit_setup_progress(app, "error", &error);
            Err(error.into())
        }
    }
}

/// Trigger first-launch bootstrap or repair for the active engine.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_setup_engine(
    app: tauri::AppHandle,
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    provision_active_engine(&app, &engine_manager).await
}

/// Ensure the active engine is provisioned without requiring its inference
/// process to be running. Safe to call before every automatic start.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_ensure_engine_ready(
    app: tauri::AppHandle,
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    provision_active_engine(&app, &engine_manager).await
}

/// Start the active engine with the given model.
///
/// This is the new engine-aware entry point. For llamacpp builds, the existing
/// `direct_runtime_start_chat_server` in sidecar.rs still works — this command is for MLX/vLLM/Ollama.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_engine(
    app: tauri::AppHandle,
    engine_manager: tauri::State<'_, EngineManager>,
    model_path: String,
    context_size: u32,
) -> Result<EngineStartResult, crate::thinclaw::bridge::BridgeError> {
    let _lifecycle_guard = crate::model_lifecycle::MODEL_LIFECYCLE_LOCK.lock().await;
    let _provisioning_guard = engine_manager.provisioning_lock.lock().await;
    let info = direct_runtime_get_active_engine_info();
    if !engine_supported_on_host(&info.id) {
        return Err(format!("{} is unsupported on this host", info.display_name).into());
    }
    if engine_needs_setup(&info, &engine_manager) {
        return Err(format!(
            "{} runtime requires provisioning before inference can start",
            info.display_name
        )
        .into());
    }
    let (canonical_model_path, resolved_model_path) = if info.id == "ollama" {
        (None, model_path)
    } else {
        let path = crate::model_manager::resolve_compatible_inventory_model_path(
            &app,
            &model_path,
            &info.id,
            "LLM",
        )?;
        let resolved = path
            .to_str()
            .ok_or_else(|| "The selected local model path is not valid UTF-8".to_string())?
            .to_string();
        (Some(path), resolved)
    };

    let options = EngineStartOptions::default();
    let (port, _token) = engine_manager
        .replace_active_engine_locked(
            &resolved_model_path,
            canonical_model_path,
            context_size,
            options,
        )
        .await?;

    // Endpoint credentials stay in the backend runtime snapshot. They are not
    // renderer state and must not cross the Tauri IPC boundary.
    Ok(EngineStartResult {
        port,
        token: String::new(),
    })
}

/// Result of starting an engine.
#[derive(Clone, Serialize, Type)]
pub struct EngineStartResult {
    pub port: u16,
    pub token: String,
}

impl std::fmt::Debug for EngineStartResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EngineStartResult")
            .field("port", &self.port)
            .field("token", &crate::debug_redaction::Redacted)
            .finish()
    }
}

/// Stop the active engine.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_stop_engine(
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let _lifecycle_guard = crate::model_lifecycle::MODEL_LIFECYCLE_LOCK.lock().await;
    Ok(engine_manager.stop_active_engine_locked().await?)
}

/// Check if the active engine is ready (health check).
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_is_engine_ready(
    sidecar: tauri::State<'_, crate::sidecar::SidecarManager>,
    engine_manager: tauri::State<'_, EngineManager>,
) -> Result<bool, crate::thinclaw::bridge::BridgeError> {
    Ok(local_runtime_snapshot(&sidecar, &engine_manager)
        .await
        .endpoint
        .is_some())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct RecordingEngine {
        events: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
        fail_stop: bool,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for RecordingEngine {
        async fn start(
            &self,
            model_path: &str,
            _context_size: u32,
            _options: EngineStartOptions,
        ) -> Result<(u16, String), String> {
            self.events
                .lock()
                .expect("events")
                .push(format!("start:{model_path}"));
            if self.fail_start {
                Err("start failed".to_string())
            } else {
                Ok((54_321, "secret".to_string()))
            }
        }

        async fn stop(&self) -> Result<(), String> {
            self.events.lock().expect("events").push("stop".to_string());
            if self.fail_stop {
                Err("stop failed".to_string())
            } else {
                Ok(())
            }
        }

        async fn is_ready(&self) -> bool {
            false
        }

        fn base_url(&self) -> Option<String> {
            None
        }

        fn display_name(&self) -> &'static str {
            "recording"
        }

        fn engine_id(&self) -> &'static str {
            "recording"
        }

        fn uses_single_file_model(&self) -> bool {
            false
        }

        fn hf_search_tag(&self) -> &'static str {
            "recording"
        }
    }

    async fn recording_manager(
        events: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
        fail_stop: bool,
    ) -> EngineManager {
        let temp = tempfile::tempdir().expect("tempdir");
        let manager = EngineManager::new(temp.path().to_path_buf());
        *manager.engine.lock().await = Some(Box::new(RecordingEngine {
            events,
            fail_start,
            fail_stop,
        }));
        manager
    }

    #[tokio::test]
    async fn engine_start_replaces_the_existing_process_and_target() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let manager = recording_manager(events.clone(), false, false).await;
        let old_target = PathBuf::from("/managed/model-a");
        let new_target = PathBuf::from("/managed/model-b");
        *manager.active_model_path.lock().await = Some(old_target);

        let endpoint = manager
            .replace_active_engine_locked(
                "/managed/model-b",
                Some(new_target.clone()),
                8_192,
                EngineStartOptions::default(),
            )
            .await
            .expect("replace engine");

        assert_eq!(endpoint.0, 54_321);
        assert_eq!(
            *events.lock().expect("events"),
            vec!["stop", "start:/managed/model-b"]
        );
        assert_eq!(
            manager.active_model_path.lock().await.as_ref(),
            Some(&new_target)
        );
    }

    #[tokio::test]
    async fn failed_engine_replacement_does_not_retain_the_old_target() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let manager = recording_manager(events.clone(), true, false).await;
        *manager.active_model_path.lock().await = Some(PathBuf::from("/managed/model-a"));

        assert!(manager
            .replace_active_engine_locked(
                "/managed/model-b",
                Some(PathBuf::from("/managed/model-b")),
                8_192,
                EngineStartOptions::default(),
            )
            .await
            .is_err());
        assert_eq!(
            *events.lock().expect("events"),
            vec!["stop", "start:/managed/model-b"]
        );
        assert!(manager.active_model_path.lock().await.is_none());
    }

    #[tokio::test]
    async fn failed_engine_stop_clears_the_target_and_prevents_replacement_start() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let manager = recording_manager(events.clone(), false, true).await;
        *manager.active_model_path.lock().await = Some(PathBuf::from("/managed/model-a"));

        assert!(manager
            .replace_active_engine_locked(
                "/managed/model-b",
                Some(PathBuf::from("/managed/model-b")),
                8_192,
                EngineStartOptions::default(),
            )
            .await
            .is_err());
        assert_eq!(*events.lock().expect("events"), vec!["stop"]);
        assert!(manager.active_model_path.lock().await.is_none());
    }

    #[tokio::test]
    async fn engine_target_stop_preserves_a_different_active_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let install_a = temp.path().join("model-a");
        let install_b = temp.path().join("model-b");
        std::fs::create_dir(&install_a).expect("install a");
        std::fs::create_dir(&install_b).expect("install b");
        let install_a = install_a.canonicalize().expect("canonical a");
        let install_b = install_b.canonicalize().expect("canonical b");
        let manager = EngineManager::new(temp.path().to_path_buf());
        *manager.active_model_path.lock().await = Some(install_b.clone());

        assert!(!manager
            .stop_if_using_install_locked(&install_a)
            .await
            .expect("unrelated target"));
        assert_eq!(
            manager.active_model_path.lock().await.as_ref(),
            Some(&install_b),
        );
        assert!(manager
            .stop_if_using_install_locked(&install_b)
            .await
            .expect("matching target"));
        assert!(manager.active_model_path.lock().await.is_none());
    }

    #[test]
    fn get_active_engine_returns_valid_info() {
        let info = direct_runtime_get_active_engine_info();
        assert!(!info.id.is_empty(), "engine id must not be empty");
        assert!(
            !info.display_name.is_empty(),
            "display_name must not be empty"
        );
        if info.id == "none" {
            assert!(
                info.hf_tag.is_empty(),
                "cloud-only runtime must not advertise a Hugging Face format"
            );
        } else {
            assert!(
                !info.hf_tag.is_empty(),
                "local runtime hf_tag must not be empty"
            );
        }

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
        let info = direct_runtime_get_active_engine_info();
        let json = serde_json::to_string(&info).expect("EngineInfo should serialize");
        assert!(json.contains(&info.id));
    }

    #[test]
    fn engine_dependency_pins_and_provisioning_scripts_stay_aligned() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../engine-manifest.json")).unwrap();
        let mlx_lock = include_str!("../../../runtime/mlx/requirements.lock");
        let vllm_lock = include_str!("../../../runtime/vllm/requirements.lock");
        let uv_script = include_str!("../../../scripts/setup_uv.sh");
        let llama_script = include_str!("../../../scripts/setup_llama.sh");
        assert_eq!(manifest["uv"]["version"], UV_VERSION);
        assert_eq!(
            manifest["engines"]["mlx"]["version"],
            MLX_OPENAI_SERVER_VERSION
        );
        assert_eq!(
            manifest["engines"]["mlx"]["minimumMacosVersion"],
            MLX_MINIMUM_MACOS
        );
        assert_eq!(manifest["engines"]["vllm"]["version"], VLLM_VERSION);
        assert_eq!(
            manifest["engines"]["vllm"]["torchBackend"],
            VLLM_TORCH_BACKEND
        );
        assert_eq!(
            manifest["engines"]["vllm"]["minimumGlibcVersion"],
            VLLM_MINIMUM_GLIBC
        );
        assert_eq!(
            manifest["engines"]["vllm"]["minimumComputeCapability"],
            VLLM_MINIMUM_COMPUTE_CAPABILITY
        );
        assert_eq!(manifest["python"]["version"], PYTHON_VERSION);
        assert_eq!(
            manifest["engines"]["llamacpp"]["version"],
            env!("THINCLAW_LLAMA_CPP_VERSION")
        );
        assert!(uv_script.contains("engine-manifest.json"));
        assert!(llama_script.contains("engine-manifest.json"));
        for (distribution, version) in manifest["engines"]["mlx"]["resolvedPackages"]
            .as_object()
            .unwrap()
        {
            assert!(
                mlx_lock.contains(&format!("{distribution}=={}", version.as_str().unwrap())),
                "{distribution} is missing from the MLX lock"
            );
        }
        assert!(
            vllm_lock.contains(&format!("vllm=={VLLM_VERSION}")),
            "vLLM is missing from its lock"
        );
        assert!(
            vllm_lock.contains("torch==2.11.0+cu129"),
            "reviewed CUDA PyTorch build is missing from the vLLM lock"
        );
    }

    #[test]
    fn runtime_kind_mapping_matches_contract_wire_variants() {
        assert_eq!(
            runtime_kind_from_engine_id("llamacpp"),
            LocalRuntimeKind::LlamaCpp
        );
        assert_eq!(runtime_kind_from_engine_id("mlx"), LocalRuntimeKind::Mlx);
        assert_eq!(runtime_kind_from_engine_id("vllm"), LocalRuntimeKind::Vllm);
        assert_eq!(
            runtime_kind_from_engine_id("ollama"),
            LocalRuntimeKind::Ollama
        );
        assert_eq!(
            runtime_kind_from_engine_id("unsupported"),
            LocalRuntimeKind::None
        );
    }

    #[test]
    fn supported_capabilities_are_stable_per_runtime_family() {
        assert_eq!(
            supported_capabilities_for_runtime("llamacpp"),
            vec![
                RuntimeCapability::Chat,
                RuntimeCapability::Embedding,
                RuntimeCapability::Stt,
                RuntimeCapability::Tts,
                RuntimeCapability::Diffusion,
            ]
        );
        assert_eq!(
            supported_capabilities_for_runtime("mlx"),
            vec![
                RuntimeCapability::Chat,
                RuntimeCapability::Embedding,
                RuntimeCapability::Stt,
                RuntimeCapability::Diffusion,
            ]
        );
        assert_eq!(
            supported_capabilities_for_runtime("vllm"),
            vec![RuntimeCapability::Chat]
        );
        assert_eq!(
            supported_capabilities_for_runtime("ollama"),
            vec![RuntimeCapability::Chat]
        );
        assert!(supported_capabilities_for_runtime("none").is_empty());
    }

    #[test]
    fn runtime_snapshot_converts_to_legacy_local_llm_config() {
        let snapshot = LocalRuntimeSnapshot {
            kind: LocalRuntimeKind::Mlx,
            display_name: "MLX".into(),
            readiness: RuntimeReadiness::Ready,
            endpoint: Some(LocalRuntimeEndpoint {
                base_url: "http://127.0.0.1:8765/v1".into(),
                api_key: Some("token".into()),
                model_id: Some("mlx-model".into()),
                context_size: Some(65_536),
                model_family: None,
            }),
            capabilities: vec![RuntimeCapability::Chat],
            supported_capabilities: vec![RuntimeCapability::Chat],
            exposure_policy: RuntimeExposurePolicy::SharedWhenEnabled,
            unavailable_reason: None,
        };

        assert_eq!(
            local_runtime_snapshot_to_local_llm(&snapshot),
            Some((8765, "token".into(), 65_536, "chatml".into()))
        );
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

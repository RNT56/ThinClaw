//! MLX inference engine implementation (macOS Apple Silicon only).
//!
//! Uses `uv` (bundled Tauri sidecar) to bootstrap an isolated Python
//! environment with `mlx_lm` installed, then spawns `mlx_lm.server`
//! as an OpenAI-compatible HTTP server on a dynamic port.
//!
//! ## First-launch bootstrap:
//! 1. Creates `~/.scrappy/mlx-env/` via `uv venv`
//! 2. Installs `mlx_lm` via `uv pip install`
//! 3. Subsequent starts skip steps 1-2
//!
//! ## On model start:
//! 1. Spawns `python -m mlx_lm.server --model <path> --port <port>`
//! 2. Polls `/health` until ready
//! 3. Returns `(port, "")` — MLX server has no auth token

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;

use super::{EngineStartOptions, InferenceEngine};

/// MLX engine — spawns `mlx_lm.server` as an OpenAI-compatible server.
pub struct MlxEngine {
    port: Mutex<Option<u16>>,
    process: Mutex<Option<tokio::process::Child>>,
    /// Path to the mlx virtualenv (set during bootstrap)
    venv_path: Mutex<Option<PathBuf>>,
    /// Path to the bundled `uv` sidecar binary
    uv_path: Mutex<Option<PathBuf>>,
    /// The model path/ID that was passed to `start()` — needed for request bodies
    loaded_model: Mutex<Option<String>>,
    /// Effective context window: min(user_requested, model_max_from_config)
    effective_context: Mutex<Option<u32>>,
}

impl MlxEngine {
    pub fn new() -> Self {
        Self {
            port: Mutex::new(None),
            process: Mutex::new(None),
            venv_path: Mutex::new(None),
            uv_path: Mutex::new(None),
            loaded_model: Mutex::new(None),
            effective_context: Mutex::new(None),
        }
    }

    /// Set the app data directory so we know where to create the venv.
    pub fn set_app_data_dir(&self, dir: PathBuf) {
        let venv = dir.join("mlx-env");
        *self.venv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(venv);
    }

    /// Set the path to the bundled `uv` sidecar.
    ///
    /// Call this from `lib.rs` during app setup, passing the path resolved
    /// by `tauri::Manager::path().resolve("bin/uv", BaseDirectory::Resource)`.
    pub fn set_uv_path(&self, path: PathBuf) {
        *self.uv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }

    /// Get the path to the uv binary.
    /// Falls back to checking $PATH, then ~/.scrappy/uv.
    fn uv_bin(&self) -> Option<String> {
        // 1. Use the explicitly set sidecar path (set by EngineManager::create_engine)
        if let Some(p) = self
            .uv_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            if p.exists() {
                return Some(p.to_string_lossy().into_owned());
            }
        }

        // 2. Check system PATH
        if let Ok(output) = std::process::Command::new("which").arg("uv").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }

        // 3. Check auto-download location
        if let Ok(home) = std::env::var("HOME") {
            let local_uv = PathBuf::from(home).join(".scrappy").join("uv");
            if local_uv.exists() {
                return Some(local_uv.to_string_lossy().into_owned());
            }
        }

        None // Caller should auto-download
    }

    fn get_venv_path(&self) -> Option<PathBuf> {
        self.venv_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn get_python_path(&self) -> Option<PathBuf> {
        self.get_venv_path().map(|v| v.join("bin").join("python3"))
    }

    /// Check if the MLX environment is already bootstrapped.
    ///
    /// Checks for a `.mlx-openai-server` marker file that is written after
    /// a successful `mlx-openai-server` installation. This ensures venvs
    /// bootstrapped with the old `mlx_lm` are re-bootstrapped correctly.
    pub fn is_bootstrapped(&self) -> bool {
        self.get_venv_path()
            .map(|v| v.join(".mlx-openai-server").exists())
            .unwrap_or(false)
    }

    /// Check if a model directory contains a vision-capable (VLM) model.
    ///
    /// Reads `config.json` for vision-related keys and verifies that vision
    /// weights actually exist in the safetensors files. Both checks must pass.
    ///
    /// Config conventions checked:
    /// - `vision_config` (LLaVA, Qwen-VL, Gemma 3)
    /// - `vision_feature_layer` + `image_token_index` (Ministral 3 / Pixtral)
    /// - Architecture name containing "ConditionalGeneration" (HF convention for VLMs)
    ///
    /// Weight verification prevents crashes when an MLX conversion carries a
    /// multimodal config.json but stripped the vision encoder during conversion.
    fn is_vision_model(model_path: &str) -> bool {
        let base = std::path::Path::new(model_path);
        let config = base.join("config.json");

        let config_indicates_vision = std::fs::read_to_string(&config)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .map(|j| {
                // Standard VLM config key (LLaVA, Qwen-VL, Gemma 3, etc.)
                if j.get("vision_config").is_some() {
                    return true;
                }
                // Mistral 3 / Pixtral style: vision_feature_layer + image_token_index
                if j.get("vision_feature_layer").is_some() || j.get("image_token_index").is_some() {
                    return true;
                }
                // HuggingFace architecture convention: "ForConditionalGeneration" = multimodal
                if let Some(archs) = j.get("architectures").and_then(|a| a.as_array()) {
                    for arch in archs {
                        if let Some(name) = arch.as_str() {
                            if name.contains("ConditionalGeneration")
                                || name.contains("VisionModel")
                                || name.contains("ForCausalImageTextToText")
                            {
                                return true;
                            }
                        }
                    }
                }
                false
            })
            .unwrap_or(false);

        if !config_indicates_vision {
            return false;
        }

        // Config says vision — now verify that vision weights actually exist.
        // Some MLX conversions strip the vision encoder during conversion
        // (e.g. mlx-community text-only conversions of multimodal source models).
        if Self::has_vision_weights(base) {
            return true;
        }

        // Weights are missing — fall back to text-only mode.
        println!(
            "[mlx] config.json indicates a VLM but no vision_tower / vision_model weights \
             found in safetensors — this MLX conversion appears text-only. \
             Launching as type=lm instead of multimodal."
        );
        false
    }

    /// Check whether the model directory contains vision encoder weights.
    /// Looks at `model.safetensors.index.json` (multi-shard) first, then
    /// falls back to scanning `*.safetensors` file names for vision-related
    /// patterns (for single-file models that lack an index).
    fn has_vision_weights(model_dir: &std::path::Path) -> bool {
        // 1. Multi-shard: check the weight_map in the index file
        let index = model_dir.join("model.safetensors.index.json");
        if let Ok(content) = std::fs::read_to_string(&index) {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(weight_map) = j.get("weight_map").and_then(|w| w.as_object()) {
                    let has_vision = weight_map.keys().any(|k| {
                        k.starts_with("vision_tower.")
                            || k.starts_with("vision_model.")
                            || k.starts_with("multi_modal_projector.")
                    });
                    return has_vision;
                }
            }
        }

        // 2. Single-file: if a file named *vision* or *mmproj* exists among safetensors
        if let Ok(entries) = std::fs::read_dir(model_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".safetensors")
                    && (name.contains("vision") || name.contains("mmproj"))
                {
                    return true;
                }
            }
        }

        // 3. Fallback: if there's no index and no vision-named files,
        //    assume text-only (we can't cheaply inspect safetensors internals)
        false
    }

    /// Check if a Python binary is version 3.11+.
    ///
    /// Runs `python3 --version` and parses the output. Returns `false` if the
    /// version is below 3.11 or if the check fails for any reason.
    async fn check_python_version(python_path: &std::path::Path) -> bool {
        let output = tokio::process::Command::new(python_path)
            .args(["--version"])
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                // Output is like "Python 3.12.4"
                let version_str = String::from_utf8_lossy(&out.stdout);
                if let Some(ver) = version_str.strip_prefix("Python ") {
                    let parts: Vec<&str> = ver.trim().split('.').collect();
                    if parts.len() >= 2 {
                        let minor: u32 = parts[1].parse().unwrap_or(0);
                        return minor >= 11;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Bootstrap the MLX environment using `uv`.
    ///
    /// This is called on first launch. It:
    /// 1. Auto-downloads `uv` if not present
    /// 2. Creates a venv with `uv venv <path>` (skipped if venv already exists)
    /// 3. Installs `mlx-openai-server` (unified text+vision+audio server)
    /// 4. Writes a `.mlx-openai-server` marker file
    /// 5. Subsequent starts skip all steps
    pub async fn bootstrap(&self) -> Result<(), String> {
        let venv = self
            .get_venv_path()
            .ok_or("MLX venv path not set — call set_app_data_dir first")?;

        if self.is_bootstrapped() {
            println!("[mlx] Environment already bootstrapped at {:?}", venv);
            return Ok(());
        }

        // Resolve or auto-download uv
        let uv_bin = match self.uv_bin() {
            Some(path) => {
                println!("[mlx] Using uv at: {}", path);
                path
            }
            None => {
                println!("[mlx] uv not found — auto-downloading...");
                self.auto_download_uv().await?
            }
        };

        // Step 1: Create venv with Python 3.12+
        // mlx-openai-server requires Python >=3.11. We pin 3.12 so `uv` will
        // auto-download it if the system Python is too old (e.g. 3.10).
        let python = self.get_python_path().ok_or("Python path not available")?;

        let needs_new_venv = if python.exists() {
            // Check if existing venv has a compatible Python version
            let version_ok = Self::check_python_version(&python).await;
            if !version_ok {
                println!("[mlx] Existing venv has Python <3.11, recreating with Python 3.12...");
                // Remove old incompatible venv
                let _ = std::fs::remove_dir_all(&venv);
                true
            } else {
                println!(
                    "[mlx] Reusing existing venv at {:?} (upgrading packages)",
                    venv
                );
                false
            }
        } else {
            true
        };

        if needs_new_venv {
            println!("[mlx] Creating virtualenv at {:?} with Python 3.12", venv);
            let output = tokio::process::Command::new(&uv_bin)
                .args(["venv", "--python", "3.12", &venv.to_string_lossy()])
                .output()
                .await
                .map_err(|e| format!("Failed to run uv venv: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("uv venv failed: {}", stderr));
            }
        }

        // Step 2: Install MLX service stack (unified venv for all MLX services)
        //   - mlx-openai-server: Chat/VLM LLM server (text + vision + audio)
        //   - mlx-embeddings:    Embedding model support (replaces llama-server --embedding)
        //   - mflux:             Image generation (replaces sd-server / sd.cpp)
        //   - mlx-whisper:       Speech-to-text (replaces whisper-server / whisper.cpp)
        // Using --upgrade ensures we get the latest version even if upgrading.
        println!("[mlx] Installing MLX service stack (mlx-openai-server, mlx-embeddings, mflux, mlx-whisper)...");

        let output = tokio::process::Command::new(&uv_bin)
            .args([
                "pip",
                "install",
                "--upgrade",
                "--python",
                &python.to_string_lossy(),
                "mlx-openai-server",
                "mlx-embeddings",
                "mflux",
                "mlx-whisper",
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to install MLX service stack: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("MLX service stack install failed: {}", stderr));
        }

        // Step 2b: Patch mlx-vlm handler for attention_mask→mask bug.
        // mlx-vlm's stream_generate() expects a "mask" key but the HuggingFace
        // processor returns "attention_mask".  Without this patch, Gemma 3 (and
        // potentially other VLMs) crash with:
        //   expand_dims(): incompatible function arguments … NoneType, int
        // We insert a key-rename after the torch→mlx conversion block.
        Self::apply_vlm_attention_mask_patch(&venv);
        Self::apply_vlm_content_normalization_patch(&venv);

        // Step 3: Write marker file so is_bootstrapped() returns true next time
        let marker = venv.join(".mlx-openai-server");
        std::fs::write(&marker, "installed")
            .map_err(|e| format!("Failed to write marker file: {}", e))?;

        println!("[mlx] Bootstrap complete.");
        Ok(())
    }

    /// Patch `app/handler/mlx_vlm.py` inside the venv to fix the
    /// `attention_mask` → `mask` key mismatch that causes VLMs to crash.
    ///
    /// This is a workaround for <https://github.com/Blaizzy/mlx-vlm/issues/XXX>.
    /// The patch is idempotent — it does nothing if already applied.
    fn apply_vlm_attention_mask_patch(venv: &std::path::Path) {
        // The handler lives under site-packages/app/handler/mlx_vlm.py
        let handler = venv
            .join("lib")
            .join("python3.12")
            .join("site-packages")
            .join("app")
            .join("handler")
            .join("mlx_vlm.py");

        if !handler.exists() {
            println!("[mlx] Patch: mlx_vlm.py handler not found, skipping");
            return;
        }

        let Ok(source) = std::fs::read_to_string(&handler) else {
            println!("[mlx] Patch: could not read mlx_vlm.py");
            return;
        };

        // Already patched?
        if source.contains("PATCH (scrappy)") {
            println!("[mlx] Patch: attention_mask fix already applied");
            return;
        }

        // The pattern we look for after the torch→mlx conversion block.
        // We insert our rename right after it.
        let needle = "                    vision_inputs[key] = mx.array(value)\n";
        let patch = r#"                    vision_inputs[key] = mx.array(value)

            # PATCH (scrappy): mlx-vlm's stream_generate expects "mask" but the
            # HuggingFace processor returns "attention_mask". Without this
            # rename, Gemma 3 crashes with expand_dims(NoneType, int).
            if "attention_mask" in vision_inputs and "mask" not in vision_inputs:
                vision_inputs["mask"] = vision_inputs.pop("attention_mask")
"#;

        let patched = source.replace(needle, patch);
        if patched == source {
            println!("[mlx] Patch: could not locate insertion point, skipping");
            return;
        }

        match std::fs::write(&handler, patched) {
            Ok(_) => println!("[mlx] Patch: applied attention_mask→mask fix to mlx_vlm.py"),
            Err(e) => println!("[mlx] Patch: failed to write: {}", e),
        }
    }

    /// Patch `app/handler/mlx_vlm.py` to normalize list content in
    /// system/assistant messages to plain strings.
    ///
    /// When IronClaw (or any OpenAI-compatible client) sends multipart content
    /// format for non-user messages, the VLM handler passes
    /// `ChatCompletionContentPartText` Pydantic objects through as-is. Downstream
    /// code then crashes with `'ChatCompletionContentPartText' object is not
    /// subscriptable` because it expects plain strings or dicts.
    ///
    /// This patch normalizes list content to a joined text string for system
    /// and assistant roles.
    fn apply_vlm_content_normalization_patch(venv: &std::path::Path) {
        let handler = venv
            .join("lib")
            .join("python3.12")
            .join("site-packages")
            .join("app")
            .join("handler")
            .join("mlx_vlm.py");

        if !handler.exists() {
            return;
        }

        let Ok(source) = std::fs::read_to_string(&handler) else {
            return;
        };

        // Already patched?
        if source.contains("PATCH (scrappy): normalize list content") {
            println!("[mlx] Patch: content normalization already applied");
            return;
        }

        // The pattern: system/assistant messages pass content through as-is.
        // We replace the block to normalize list content to plain text.
        let needle = r#"            if message.role in ["system", "assistant"]:
                chat_messages.append({"role": message.role, "content": message.content})
                continue"#;

        let patch = r#"            if message.role in ["system", "assistant"]:
                # PATCH (scrappy): normalize list content to plain text.
                # IronClaw sends multipart content format (list of ContentPart
                # objects) for all roles. Without normalization, Pydantic objects
                # cause "'ChatCompletionContentPartText' object is not subscriptable".
                msg_content = message.content
                if isinstance(msg_content, list):
                    text_parts = []
                    for part in msg_content:
                        if hasattr(part, 'text'):
                            text_parts.append(part.text)
                        elif isinstance(part, dict) and 'text' in part:
                            text_parts.append(part['text'])
                        elif isinstance(part, str):
                            text_parts.append(part)
                    msg_content = "\n".join(text_parts) if text_parts else str(message.content)
                chat_messages.append({"role": message.role, "content": msg_content})
                continue"#;

        let patched = source.replace(needle, patch);
        if patched == source {
            println!(
                "[mlx] Patch: could not locate content normalization insertion point, skipping"
            );
            return;
        }

        match std::fs::write(&handler, patched) {
            Ok(_) => println!("[mlx] Patch: applied content normalization fix to mlx_vlm.py"),
            Err(e) => println!("[mlx] Patch: failed to write: {}", e),
        }
    }

    /// Find a free port to use.
    fn find_free_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind port: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get local addr: {}", e))?
            .port();
        Ok(port)
    }

    /// Auto-download `uv` from GitHub releases into `~/.scrappy/uv`.
    ///
    /// This is called during `bootstrap()` if no `uv` binary was found.
    /// The download is a one-time operation — subsequent bootstraps find
    /// the cached binary via `uv_bin()`.
    async fn auto_download_uv(&self) -> Result<String, String> {
        let uv_version = "0.4.30"; // Pinned version — matches scripts/setup_uv.sh

        let (platform, asset) = if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            ("aarch64-apple-darwin", "uv-aarch64-apple-darwin.tar.gz")
        } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
            ("x86_64-apple-darwin", "uv-x86_64-apple-darwin.tar.gz")
        } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            (
                "x86_64-unknown-linux-gnu",
                "uv-x86_64-unknown-linux-gnu.tar.gz",
            )
        } else {
            return Err("Unsupported platform for uv auto-download".into());
        };

        let url = format!(
            "https://github.com/astral-sh/uv/releases/download/{}/{}",
            uv_version, asset
        );

        let dest_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "HOME not set")?
            .join(".scrappy");

        std::fs::create_dir_all(&dest_dir)
            .map_err(|e| format!("Failed to create ~/.scrappy: {}", e))?;

        let dest_path = dest_dir.join("uv");
        let archive_path = dest_dir.join(asset);

        println!("[mlx] Downloading uv {} for {}...", uv_version, platform);

        // Download the archive
        let response = reqwest::get(&url)
            .await
            .map_err(|e| format!("Failed to download uv: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Failed to download uv: HTTP {}", response.status()));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read uv download: {}", e))?;

        std::fs::write(&archive_path, &bytes)
            .map_err(|e| format!("Failed to write uv archive: {}", e))?;

        // Extract the archive using tar (available on macOS and Linux)
        let temp_dir = dest_dir.join("uv-temp");
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let output = tokio::process::Command::new("tar")
            .args([
                "-xzf",
                &archive_path.to_string_lossy(),
                "-C",
                &temp_dir.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to extract uv archive: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("tar extraction failed: {}", stderr));
        }

        // Find the uv binary in the extracted contents
        let uv_extracted = Self::find_file_recursive(&temp_dir, "uv")
            .ok_or("uv binary not found in extracted archive")?;

        std::fs::copy(&uv_extracted, &dest_path)
            .map_err(|e| format!("Failed to copy uv binary: {}", e))?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&dest_path, perms)
                .map_err(|e| format!("Failed to chmod uv: {}", e))?;
        }

        // Cleanup
        let _ = std::fs::remove_file(&archive_path);
        let _ = std::fs::remove_dir_all(&temp_dir);

        println!("[mlx] uv {} installed at {:?}", uv_version, dest_path);
        Ok(dest_path.to_string_lossy().into_owned())
    }

    /// Recursively find a file by name in a directory.
    fn find_file_recursive(dir: &PathBuf, name: &str) -> Option<PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.file_name().map(|n| n == name).unwrap_or(false) {
                    return Some(path);
                }
                if path.is_dir() {
                    if let Some(found) = Self::find_file_recursive(&path, name) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}

impl Default for MlxEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InferenceEngine for MlxEngine {
    async fn start(
        &self,
        model_path: &str,
        context_size: u32,
        _options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        let venv = self
            .get_venv_path()
            .ok_or("MLX venv path not set — call set_app_data_dir first")?;
        let python = self
            .get_python_path()
            .ok_or("MLX environment not bootstrapped. Run bootstrap() first.")?;

        if !python.exists() {
            return Err("MLX environment not found. Please set up MLX first.".into());
        }

        // Apply any pending patches to the MLX server code.
        // These are idempotent — they no-op if already applied.
        Self::apply_vlm_attention_mask_patch(&venv);
        Self::apply_vlm_content_normalization_patch(&venv);

        // Read the model's native context window from config.json
        let model_max = super::read_model_max_context(model_path);
        let effective = match model_max {
            Some(model_limit) => {
                let eff = context_size.min(model_limit);
                println!(
                    "[mlx] Model max context: {} (from config.json), user requested: {}, effective: {}",
                    model_limit, context_size, eff
                );
                eff
            }
            None => {
                println!(
                    "[mlx] No max_position_embeddings in config.json, using user setting: {}",
                    context_size
                );
                context_size
            }
        };

        let port = Self::find_free_port()?;

        // Detect vision models to select the correct model type for mlx-openai-server.
        // Checks both config.json and actual safetensors weights to ensure the vision
        // encoder was included in this conversion (some MLX conversions strip it).
        let is_vlm = Self::is_vision_model(model_path);
        let model_type = if is_vlm { "multimodal" } else { "lm" };

        println!(
            "[mlx] Starting mlx-openai-server on port {} with model {} (type={})",
            port, model_path, model_type
        );

        let port_str = port.to_string();

        // mlx-openai-server installs a CLI entry point in the venv's bin/ dir.
        // We invoke it directly rather than using `python -m`.
        let server_bin = venv.join("bin").join("mlx-openai-server");
        if !server_bin.exists() {
            return Err(format!(
                "mlx-openai-server binary not found at {:?} — try re-bootstrapping",
                server_bin
            ));
        }

        let child = tokio::process::Command::new(&server_bin)
            .args([
                "launch",
                "--model-path",
                model_path,
                "--model-type",
                model_type,
                "--port",
                &port_str,
                "--host",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn mlx-openai-server: {}", e))?;

        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
        *self.process.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        *self.loaded_model.lock().unwrap_or_else(|e| e.into_inner()) = Some(model_path.to_string());
        *self
            .effective_context
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(effective);

        // Poll for readiness (up to 60 seconds for model loading)
        let client = reqwest::Client::new();
        let start = std::time::Instant::now();

        loop {
            if start.elapsed().as_secs() > 60 {
                self.stop().await.ok();
                return Err("MLX server startup timeout (60s)".into());
            }

            // Check if process is still alive — extract dead child if exited
            let dead_child = {
                let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(ref mut child) = *guard {
                    match child.try_wait() {
                        Ok(Some(status)) => Some((guard.take().unwrap(), status)),
                        Ok(None) => None,
                        Err(e) => return Err(format!("Failed to check MLX process: {}", e)),
                    }
                } else {
                    None
                }
            };
            // guard dropped here — safe to .await below

            if let Some((mut child, status)) = dead_child {
                let stderr_msg = if let Some(mut stderr) = child.stderr.take() {
                    let mut buf = String::new();
                    use tokio::io::AsyncReadExt;
                    let _ = stderr.read_to_string(&mut buf).await;
                    if buf.trim().is_empty() {
                        String::from("(no stderr output)")
                    } else {
                        let trimmed = if buf.len() > 500 {
                            let mut start = buf.len() - 500;
                            while !buf.is_char_boundary(start) && start < buf.len() {
                                start += 1;
                            }
                            format!("...{}", &buf[start..])
                        } else {
                            buf
                        };
                        trimmed.trim().to_string()
                    }
                } else {
                    String::from("(stderr not available)")
                };
                println!(
                    "[mlx] Server crashed during startup. stderr:\n{}",
                    stderr_msg
                );
                return Err(format!(
                    "MLX server exited during startup (code {:?}):\n{}",
                    status.code(),
                    stderr_msg
                ));
            }

            match client
                .get(format!("http://127.0.0.1:{}/v1/models", port))
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                Ok(res) if res.status().is_success() => {
                    println!("[mlx] Server is ready on port {}", port);
                    return Ok((port, String::new())); // MLX server has no auth token
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
            }
        }
    }

    async fn stop(&self) -> Result<(), String> {
        let child = {
            let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(mut child) = child {
            child.kill().await.ok();
            println!("[mlx] Server stopped.");
        }
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        let port = match *self.port.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(p) => p,
            None => return false,
        };

        reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/v1/models", port))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn base_url(&self) -> Option<String> {
        self.port
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|p| format!("http://127.0.0.1:{}/v1", p))
    }

    fn model_id(&self) -> Option<String> {
        self.loaded_model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn max_context(&self) -> Option<u32> {
        *self
            .effective_context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn display_name(&self) -> &'static str {
        "MLX (Apple Silicon)"
    }

    fn engine_id(&self) -> &'static str {
        "mlx"
    }

    fn uses_single_file_model(&self) -> bool {
        false // MLX uses model directories
    }

    fn hf_search_tag(&self) -> &'static str {
        "mlx"
    }
}

// Cleanup on drop
impl Drop for MlxEngine {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(ref mut child) = *guard {
                // Blocking kill — we're in drop so can't use async
                let _ = child.start_kill();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mlx_engine_defaults() {
        let engine = MlxEngine::new();
        assert_eq!(engine.engine_id(), "mlx");
        assert_eq!(engine.hf_search_tag(), "mlx");
        assert!(!engine.uses_single_file_model());
        assert!(engine.base_url().is_none()); // no port set
    }

    #[test]
    fn venv_path_setup() {
        let engine = MlxEngine::new();
        assert!(!engine.is_bootstrapped());

        engine.set_app_data_dir(PathBuf::from("/tmp/test_scrappy"));
        assert_eq!(
            engine.get_venv_path(),
            Some(PathBuf::from("/tmp/test_scrappy/mlx-env"))
        );
        assert_eq!(
            engine.get_python_path(),
            Some(PathBuf::from("/tmp/test_scrappy/mlx-env/bin/python3"))
        );
    }

    #[test]
    fn is_vision_model_with_vision_config() {
        let dir = std::env::temp_dir().join("scrappy_test_vision");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "mistral3", "vision_config": {"image_size": 512}}"#,
        )
        .unwrap();
        // Need vision weights too
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"vision_tower.layer.weight": "model.safetensors", "language_model.layer.weight": "model.safetensors"}}"#,
        )
        .unwrap();
        assert!(MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_text_only() {
        let dir = std::env::temp_dir().join("scrappy_test_text");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "llama", "max_position_embeddings": 4096}"#,
        )
        .unwrap();
        assert!(!MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_missing_config() {
        assert!(!MlxEngine::is_vision_model("/nonexistent/path/to/model"));
    }

    #[test]
    fn is_vision_model_empty_vision_config() {
        // Some VLMs have an empty vision_config object — should detect if weights present
        let dir = std::env::temp_dir().join("scrappy_test_empty_vc");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "gemma3", "vision_config": {}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"vision_model.encoder.weight": "model.safetensors"}}"#,
        )
        .unwrap();
        assert!(MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_nested_text_config() {
        // Gemma 3 VLMs wrap text_config + vision_config at root level
        let dir = std::env::temp_dir().join("scrappy_test_nested");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "gemma3", "text_config": {"max_position_embeddings": 8192}, "vision_config": {"image_size": 896}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"vision_tower.encoder.weight": "model.safetensors", "multi_modal_projector.linear.weight": "model.safetensors"}}"#,
        )
        .unwrap();
        assert!(MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_malformed_json() {
        let dir = std::env::temp_dir().join("scrappy_test_malformed");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("config.json"), "not valid json {{").unwrap();
        assert!(!MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_ministral3_style() {
        // Ministral-3 with BOTH config + vision weights → true
        let dir = std::env::temp_dir().join("scrappy_test_ministral3");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "mistral3", "architectures": ["Mistral3ForConditionalGeneration"], "vision_feature_layer": -1, "image_token_index": 10, "text_config": {"max_position_embeddings": 262144}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"vision_tower.vision_model.transformer.layers.0.attention.k_proj.weight": "model.safetensors", "language_model.model.layers.0.mlp.gate_proj.weight": "model.safetensors"}}"#,
        )
        .unwrap();
        assert!(MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_config_vlm_but_weights_text_only() {
        // mlx-community/Ministral-3-3B-Instruct-2512: config says VLM but weights
        // only contain language_model.* — should return false to prevent crash
        let dir = std::env::temp_dir().join("scrappy_test_vlm_no_weights");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"model_type": "mistral3", "architectures": ["Mistral3ForConditionalGeneration"], "vision_feature_layer": -1, "image_token_index": 10}"#,
        )
        .unwrap();
        // Weights only have language_model — NO vision_tower
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"language_model.model.embed_tokens.weight": "model-00001.safetensors", "language_model.model.layers.0.mlp.gate_proj.weight": "model-00001.safetensors", "language_model.model.norm.weight": "model-00002.safetensors"}}"#,
        )
        .unwrap();
        assert!(!MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_vision_model_conditional_generation_arch() {
        // HF convention: "ForConditionalGeneration" suffix means multimodal
        let dir = std::env::temp_dir().join("scrappy_test_condgen");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("config.json"),
            r#"{"architectures": ["LlavaForConditionalGeneration"], "model_type": "llava"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map": {"vision_tower.encoder.weight": "model.safetensors", "multi_modal_projector.linear.weight": "model.safetensors"}}"#,
        )
        .unwrap();
        assert!(MlxEngine::is_vision_model(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base_url_with_port() {
        let engine = MlxEngine::new();
        assert!(engine.base_url().is_none());

        // Manually set a port to test base_url formatting
        *engine.port.lock().unwrap() = Some(8765);
        assert_eq!(engine.base_url(), Some("http://127.0.0.1:8765/v1".into()));
    }

    #[test]
    fn not_bootstrapped_without_app_dir() {
        let engine = MlxEngine::new();
        assert!(!engine.is_bootstrapped());
        assert!(engine.get_venv_path().is_none());
        assert!(engine.get_python_path().is_none());
    }

    #[test]
    fn bootstrapped_requires_marker_file() {
        let dir = std::env::temp_dir().join("scrappy_test_bootstrap_marker");
        let venv = dir.join("mlx-env");
        let _ = std::fs::create_dir_all(&venv);

        let engine = MlxEngine::new();
        engine.set_app_data_dir(dir.clone());

        // Venv dir exists but no marker → not bootstrapped
        assert!(!engine.is_bootstrapped());

        // Write marker → bootstrapped
        std::fs::write(venv.join(".mlx-openai-server"), "installed").unwrap();
        assert!(engine.is_bootstrapped());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

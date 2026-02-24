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
    pub fn is_bootstrapped(&self) -> bool {
        self.get_python_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Bootstrap the MLX environment using `uv`.
    ///
    /// This is called on first launch. It:
    /// 1. Auto-downloads `uv` if not present
    /// 2. Creates a venv with `uv venv <path>`
    /// 3. Installs `mlx_lm` with `uv pip install --python <venv/bin/python> mlx_lm`
    /// 4. Subsequent starts skip all steps
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

        println!("[mlx] Creating virtualenv at {:?}", venv);

        // Step 1: Create venv
        let output = tokio::process::Command::new(&uv_bin)
            .args(["venv", &venv.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to run uv venv: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("uv venv failed: {}", stderr));
        }

        // Step 2: Install mlx_lm
        println!("[mlx] Installing mlx_lm...");
        let python = self
            .get_python_path()
            .ok_or("Python path not available after venv creation")?;

        let output = tokio::process::Command::new(&uv_bin)
            .args([
                "pip",
                "install",
                "--python",
                &python.to_string_lossy(),
                "mlx_lm",
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to install mlx_lm: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mlx_lm install failed: {}", stderr));
        }

        println!("[mlx] Bootstrap complete.");
        Ok(())
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
        let python = self
            .get_python_path()
            .ok_or("MLX environment not bootstrapped. Run bootstrap() first.")?;

        if !python.exists() {
            return Err("MLX environment not found. Please set up MLX first.".into());
        }

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

        // Max output tokens: default is 512 in mlx_lm.server which is too low
        // for chat. Set it to half the effective context, capped at 8192.
        let max_output_tokens = (effective / 2).min(8192);

        println!(
            "[mlx] Starting mlx_lm.server on port {} with model {} (max_tokens={})",
            port, model_path, max_output_tokens
        );

        let max_output_str = max_output_tokens.to_string();
        let port_str = port.to_string();

        let child = tokio::process::Command::new(&python)
            .args([
                "-m",
                "mlx_lm.server",
                "--model",
                model_path,
                "--port",
                &port_str,
                "--host",
                "127.0.0.1",
                "--max-tokens",
                &max_output_str,
                "--use-default-chat-template",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn mlx_lm.server: {}", e))?;

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

            // Check if process is still alive
            {
                let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(ref mut child) = *guard {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            return Err(format!(
                                "MLX server exited during startup with code {:?}",
                                status.code()
                            ));
                        }
                        Ok(None) => {} // Still running
                        Err(e) => return Err(format!("Failed to check MLX process: {}", e)),
                    }
                }
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
}

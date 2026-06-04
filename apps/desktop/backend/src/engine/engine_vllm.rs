//! vLLM inference engine implementation (Linux CUDA only).
//!
//! Uses `uv` to bootstrap an isolated Python environment with `vllm`
//! installed, then spawns `vllm serve` as an OpenAI-compatible HTTP
//! server on a dynamic port.
//!
//! ## Prerequisites:
//! - NVIDIA GPU with CUDA toolkit installed
//! - `nvidia-smi` must be available on PATH
//!
//! ## First-launch bootstrap:
//! 1. Creates `vllm-env/` under the ThinClaw Desktop app data directory via `uv venv`
//! 2. Installs `vllm` via `uv pip install`
//! 3. Subsequent starts skip steps 1-2

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;

use super::{EngineStartOptions, InferenceEngine};

/// vLLM engine — spawns `vllm serve` as an OpenAI-compatible server.
pub struct VllmEngine {
    port: Mutex<Option<u16>>,
    process: Mutex<Option<tokio::process::Child>>,
    venv_path: Mutex<Option<PathBuf>>,
    loaded_model: Mutex<Option<String>>,
    /// Path to the bundled `uv` sidecar binary
    uv_path: Mutex<Option<PathBuf>>,
}

impl VllmEngine {
    pub fn new() -> Self {
        Self {
            port: Mutex::new(None),
            process: Mutex::new(None),
            venv_path: Mutex::new(None),
            loaded_model: Mutex::new(None),
            uv_path: Mutex::new(None),
        }
    }

    pub fn set_app_data_dir(&self, dir: PathBuf) {
        let venv = dir.join("vllm-env");
        *self.venv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(venv);
    }

    /// Set the path to the bundled `uv` sidecar.
    pub fn set_uv_path(&self, path: PathBuf) {
        *self.uv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }

    /// Get the path to the uv binary.
    /// Falls back to checking $PATH if the bundled path isn't set.
    fn uv_bin(&self) -> String {
        if let Some(p) = self
            .uv_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
        "uv".to_string()
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

    pub fn is_bootstrapped(&self) -> bool {
        self.get_python_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Check if CUDA is available by probing `nvidia-smi`.
    pub fn has_cuda() -> bool {
        std::process::Command::new("nvidia-smi")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Bootstrap the vLLM environment using `uv`.
    pub async fn bootstrap(&self) -> Result<(), String> {
        let uv_bin = self.uv_bin();
        if !Self::has_cuda() {
            return Err(
                "CUDA not detected. vLLM requires an NVIDIA GPU with CUDA installed.".into(),
            );
        }

        let venv = self.get_venv_path().ok_or("vLLM venv path not set")?;

        if self.is_bootstrapped() {
            println!("[vllm] Environment already bootstrapped at {:?}", venv);
            return Ok(());
        }

        // Step 1: Create venv
        println!("[vllm] Creating virtualenv at {:?}", venv);
        let output = tokio::process::Command::new(&uv_bin)
            .args(["venv", &venv.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to run uv venv: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "uv venv failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Step 2: Install vllm
        println!("[vllm] Installing vllm (this may take several minutes)...");
        let python = self.get_python_path().ok_or("Python path not available")?;

        let output = tokio::process::Command::new(&uv_bin)
            .args([
                "pip",
                "install",
                "--python",
                &python.to_string_lossy(),
                "vllm",
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to install vllm: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "vllm install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        println!("[vllm] Bootstrap complete.");
        Ok(())
    }

    fn find_free_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind port: {}", e))?;
        listener
            .local_addr()
            .map_err(|e| format!("Failed to get addr: {}", e))
            .map(|a| a.port())
    }
}

impl Default for VllmEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InferenceEngine for VllmEngine {
    async fn start(
        &self,
        model_path: &str,
        _context_size: u32,
        _options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        let python = self
            .get_python_path()
            .ok_or("vLLM environment not bootstrapped.")?;

        if !python.exists() {
            return Err("vLLM environment not found. Please set up vLLM first.".into());
        }

        let port = Self::find_free_port()?;

        println!(
            "[vllm] Starting vllm serve on port {} with model {}",
            port, model_path
        );

        let child = tokio::process::Command::new(&python)
            .args([
                "-m",
                "vllm.entrypoints.openai.api_server",
                "--model",
                model_path,
                "--port",
                &port.to_string(),
                "--host",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn vllm: {}", e))?;

        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
        *self.process.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        *self.loaded_model.lock().unwrap_or_else(|e| e.into_inner()) = Some(model_path.to_string());

        // Poll for readiness (up to 120 seconds — vLLM model loading can be slow)
        let client = reqwest::Client::new();
        let start = std::time::Instant::now();

        loop {
            if start.elapsed().as_secs() > 120 {
                self.stop().await.ok();
                return Err("vLLM server startup timeout (120s)".into());
            }

            {
                let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(ref mut child) = *guard {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            return Err(format!(
                                "vLLM exited during startup with code {:?}",
                                status.code()
                            ));
                        }
                        Ok(None) => {}
                        Err(e) => return Err(format!("Failed to check vLLM process: {}", e)),
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
                    println!("[vllm] Server is ready on port {}", port);
                    return Ok((port, String::new()));
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
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
            println!("[vllm] Server stopped.");
        }
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.loaded_model.lock().unwrap_or_else(|e| e.into_inner()) = None;
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

    fn display_name(&self) -> &'static str {
        "vLLM (CUDA)"
    }
    fn engine_id(&self) -> &'static str {
        "vllm"
    }
    fn uses_single_file_model(&self) -> bool {
        false
    }
    fn hf_search_tag(&self) -> &'static str {
        "awq"
    }
}

impl Drop for VllmEngine {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.start_kill();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vllm_engine_defaults() {
        let engine = VllmEngine::new();
        assert_eq!(engine.engine_id(), "vllm");
        assert_eq!(engine.hf_search_tag(), "awq");
        assert!(!engine.uses_single_file_model());
    }
}

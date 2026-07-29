//! vLLM inference engine implementation (Linux CUDA only).
//!
//! The Python environment is built in a private staging directory, pinned to a
//! reviewed vLLM release, and atomically swapped into place. The serving process
//! runs in a descendant-owned boundary and exposes an authenticated loopback API.

use async_trait::async_trait;
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use thinclaw_platform::{
    bounded_command_output, find_executable_in_path, rename_no_replace, OwnedChild,
};
use tokio::process::Command;

use super::{
    EngineStartOptions, InferenceEngine, RuntimeDiagnostics, PYTHON_VERSION, UV_VERSION,
    VLLM_MINIMUM_COMPUTE_CAPABILITY, VLLM_MINIMUM_GLIBC, VLLM_TORCH_BACKEND, VLLM_VERSION,
};

const BOOTSTRAP_MARKER: &str = ".thinclaw-vllm-bootstrap";
const SERVED_MODEL_NAME: &str = "thinclaw-local";
const MAX_CONTEXT_SIZE: u32 = 1_048_576;
const UTILITY_OUTPUT_LIMIT: usize = 1024 * 1024;
const CUDA_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const VENV_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const REQUIREMENTS_LOCK: &[u8] = include_bytes!("../../../runtime/vllm/requirements.lock");

static BOOTSTRAP_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// vLLM engine — spawns `vllm serve` as an OpenAI-compatible server.
pub struct VllmEngine {
    lifecycle: tokio::sync::Mutex<()>,
    port: Mutex<Option<u16>>,
    process: Mutex<Option<OwnedChild>>,
    venv_path: Mutex<Option<PathBuf>>,
    served_model: Mutex<Option<String>>,
    effective_context: Mutex<Option<u32>>,
    api_token: Mutex<Option<String>>,
    /// Path to the bundled `uv` sidecar binary.
    uv_path: Mutex<Option<PathBuf>>,
    diagnostics: RuntimeDiagnostics,
}

impl VllmEngine {
    pub fn new() -> Self {
        Self {
            lifecycle: tokio::sync::Mutex::new(()),
            port: Mutex::new(None),
            process: Mutex::new(None),
            venv_path: Mutex::new(None),
            served_model: Mutex::new(None),
            effective_context: Mutex::new(None),
            api_token: Mutex::new(None),
            uv_path: Mutex::new(None),
            diagnostics: RuntimeDiagnostics::default(),
        }
    }

    pub fn set_app_data_dir(&self, dir: PathBuf) {
        *self.venv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.join("vllm-env"));
    }

    pub fn set_uv_path(&self, path: PathBuf) {
        *self.uv_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }

    fn uv_bin(&self) -> Result<PathBuf, String> {
        if let Some(path) = self
            .uv_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("Configured uv binary is unavailable: {error}"))?;
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                return path
                    .canonicalize()
                    .map_err(|error| format!("Could not resolve uv binary: {error}"));
            }
            return Err("Configured uv path is not a regular file".to_string());
        }
        if std::env::var("THINCLAW_ALLOW_SYSTEM_UV").as_deref() == Ok("1") {
            return find_executable_in_path("uv")
                .and_then(|path| path.canonicalize().ok())
                .filter(|path| path.is_file())
                .ok_or_else(|| "The explicitly allowed system uv is unavailable".to_string());
        }
        Err("The packaged uv runtime asset is unavailable".to_string())
    }

    fn get_venv_path(&self) -> Option<PathBuf> {
        self.venv_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn python_path_for(venv: &Path) -> PathBuf {
        if cfg!(windows) {
            venv.join("Scripts").join("python.exe")
        } else {
            venv.join("bin").join("python3")
        }
    }

    fn server_path_for(venv: &Path) -> PathBuf {
        if cfg!(windows) {
            venv.join("Scripts").join("vllm.exe")
        } else {
            venv.join("bin").join("vllm")
        }
    }

    fn expected_marker() -> String {
        let lock_hash = hex::encode(Sha256::digest(REQUIREMENTS_LOCK));
        format!(
            "vllm={VLLM_VERSION}\npython={PYTHON_VERSION}\ntorch-backend={VLLM_TORCH_BACKEND}\nlock-sha256={lock_hash}\nrelocatable=1\n"
        )
    }

    fn regular_resolved_file(path: &Path) -> bool {
        path.canonicalize()
            .ok()
            .and_then(|resolved| std::fs::metadata(resolved).ok())
            .is_some_and(|metadata| metadata.is_file())
    }

    fn environment_is_complete(venv: &Path) -> bool {
        let marker = venv.join(BOOTSTRAP_MARKER);
        let marker_ok =
            std::fs::symlink_metadata(&marker).is_ok_and(|metadata| {
                metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 512
            }) && thinclaw_platform::read_regular_file_bounded_single_link(&marker, 512)
                .is_ok_and(|value| value == Self::expected_marker().as_bytes());
        marker_ok
            && Self::regular_resolved_file(&Self::python_path_for(venv))
            && Self::regular_resolved_file(&Self::server_path_for(venv))
    }

    pub fn is_bootstrapped(&self) -> bool {
        self.get_venv_path()
            .as_deref()
            .is_some_and(Self::environment_is_complete)
    }

    fn hardened_uv_command(uv: &Path, data_root: &Path) -> Command {
        let mut command = Command::new(uv);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if name.starts_with("UV_")
                || name.starts_with("PIP_")
                || matches!(
                    name.as_str(),
                    "PYTHONPATH" | "PYTHONHOME" | "VIRTUAL_ENV" | "CONDA_PREFIX"
                )
            {
                command.env_remove(key);
            }
        }
        command
            .env("UV_NO_CONFIG", "1")
            .env("UV_CACHE_DIR", data_root.join("runtime-cache").join("uv"))
            .env("UV_PYTHON_INSTALL_DIR", data_root.join("python"))
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1");
        command
    }

    fn clean_process_detail(bytes: &[u8]) -> String {
        let value = String::from_utf8_lossy(bytes);
        let mut cleaned = String::new();
        for character in value.chars() {
            if cleaned.len() >= 2_048 {
                break;
            }
            if character == '\n' || character == '\t' || !character.is_control() {
                cleaned.push(character);
            }
        }
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() {
            "command returned a non-zero status".to_string()
        } else {
            cleaned
        }
    }

    async fn run_uv(
        uv: &Path,
        data_root: &Path,
        operation: &'static str,
        args: &[std::ffi::OsString],
        timeout: Duration,
    ) -> Result<(), String> {
        let mut command = Self::hardened_uv_command(uv, data_root);
        command.args(args);
        let output = bounded_command_output(
            &mut command,
            timeout,
            UTILITY_OUTPUT_LIMIT,
            UTILITY_OUTPUT_LIMIT,
        )
        .await
        .map_err(|error| format!("uv {operation} failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "uv {operation} failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        Ok(())
    }

    async fn verify_uv(uv: &Path, data_root: &Path) -> Result<(), String> {
        let mut command = Self::hardened_uv_command(uv, data_root);
        command.arg("--version");
        let output =
            bounded_command_output(&mut command, Duration::from_secs(10), 16 * 1024, 16 * 1024)
                .await
                .map_err(|error| format!("Could not inspect packaged uv: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Packaged uv version check failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        let actual = String::from_utf8_lossy(&output.stdout);
        if !actual.trim().starts_with(&format!("uv {UV_VERSION}")) {
            return Err(format!(
                "Packaged uv version mismatch: expected {UV_VERSION}, got {}",
                actual.trim()
            ));
        }
        Ok(())
    }

    fn version_at_least(actual: &str, minimum: &str) -> bool {
        let parse = |value: &str| {
            value
                .split('.')
                .take(2)
                .map(|part| {
                    part.chars()
                        .take_while(|character| character.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        parse(actual) >= parse(minimum)
    }

    async fn verify_glibc() -> Result<(), String> {
        if !cfg!(target_os = "linux") {
            return Err("vLLM is supported only on Linux x86_64 hosts".to_string());
        }
        let ldd = find_executable_in_path("ldd")
            .ok_or_else(|| "Could not inspect glibc because ldd is unavailable".to_string())?;
        let mut command = Command::new(ldd);
        command.arg("--version");
        let output = bounded_command_output(&mut command, CUDA_PROBE_TIMEOUT, 16 * 1024, 16 * 1024)
            .await
            .map_err(|error| format!("glibc version check failed: {error}"))?;
        let detail = format!(
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = detail
            .split_whitespace()
            .rev()
            .find(|value| {
                value.split_once('.').is_some_and(|(major, minor)| {
                    major.chars().all(|c| c.is_ascii_digit())
                        && minor
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .next()
                            .is_some()
                })
            })
            .ok_or_else(|| "Could not parse the host glibc version".to_string())?;
        if !Self::version_at_least(actual, VLLM_MINIMUM_GLIBC) {
            return Err(format!(
                "vLLM requires glibc {VLLM_MINIMUM_GLIBC} or newer; this host reports {actual}"
            ));
        }
        Ok(())
    }

    /// Validate that at least one visible NVIDIA GPU satisfies the reviewed
    /// vLLM wheel's minimum compute capability.
    async fn verify_cuda() -> Result<(), String> {
        let executable = find_executable_in_path("nvidia-smi").ok_or_else(|| {
            "nvidia-smi is unavailable; install a working NVIDIA driver".to_string()
        })?;
        let mut command = Command::new(executable);
        command
            .arg("--query-gpu=name,compute_cap,driver_version,memory.total")
            .arg("--format=csv,noheader,nounits");
        let output = bounded_command_output(&mut command, CUDA_PROBE_TIMEOUT, 64 * 1024, 16 * 1024)
            .await
            .map_err(|error| format!("NVIDIA GPU probe failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "NVIDIA GPU probe failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        let rows = String::from_utf8_lossy(&output.stdout);
        let compatible = rows.lines().any(|line| {
            line.split(',').nth(1).is_some_and(|capability| {
                Self::version_at_least(capability.trim(), VLLM_MINIMUM_COMPUTE_CAPABILITY)
            })
        });
        if !compatible {
            return Err(format!(
                "vLLM requires an NVIDIA GPU with compute capability {VLLM_MINIMUM_COMPUTE_CAPABILITY} or newer; detected: {}",
                rows.trim()
            ));
        }
        Ok(())
    }

    async fn validate_python_environment(python: &Path) -> Result<(), String> {
        let validation = format!(
            r#"import importlib.metadata
import platform
import torch

if platform.python_version() != "{PYTHON_VERSION}":
    raise RuntimeError(f"python={{platform.python_version()}}, expected {PYTHON_VERSION}")
actual = importlib.metadata.version("vllm")
if actual != "{VLLM_VERSION}":
    raise RuntimeError(f"vllm={{actual}}, expected {VLLM_VERSION}")
if not torch.cuda.is_available():
    raise RuntimeError("PyTorch cannot access CUDA")
probe = torch.empty(1, device="cuda")
del probe
print("thinclaw-vllm-runtime-ok")
"#
        );
        let mut command = Command::new(python);
        command
            .arg("-c")
            .arg(validation)
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env_remove("VIRTUAL_ENV")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1");
        let output = bounded_command_output(
            &mut command,
            Duration::from_secs(3 * 60),
            UTILITY_OUTPUT_LIMIT,
            UTILITY_OUTPUT_LIMIT,
        )
        .await
        .map_err(|error| format!("vLLM runtime validation failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "vLLM runtime validation failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        if !String::from_utf8_lossy(&output.stdout).contains("thinclaw-vllm-runtime-ok") {
            return Err("vLLM runtime validation returned an unexpected result".to_string());
        }
        Ok(())
    }

    async fn validate_activated_environment(venv: &Path) -> Result<(), String> {
        let server = Self::server_path_for(venv);
        let mut command = Command::new(server);
        command
            .arg("--help")
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env_remove("VIRTUAL_ENV")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1");
        let output = bounded_command_output(
            &mut command,
            Duration::from_secs(60),
            UTILITY_OUTPUT_LIMIT,
            UTILITY_OUTPUT_LIMIT,
        )
        .await
        .map_err(|error| format!("Activated vLLM executable validation failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Activated vLLM executable validation failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        Ok(())
    }

    async fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
        thinclaw_platform::write_private_file_atomic_async(
            path.to_path_buf(),
            contents.to_vec(),
            true,
        )
        .await
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
    }

    /// Build a complete vLLM environment off to the side and atomically swap it
    /// into place only after every step succeeds.
    pub async fn bootstrap(&self) -> Result<(), String> {
        let _bootstrap_guard = BOOTSTRAP_LOCK.lock().await;
        Self::verify_glibc().await?;
        Self::verify_cuda().await?;
        if self.is_bootstrapped() {
            return Ok(());
        }

        let uv = self.uv_bin()?;
        let final_venv = self
            .get_venv_path()
            .ok_or_else(|| "vLLM venv path not set".to_string())?;
        let parent = final_venv
            .parent()
            .ok_or_else(|| "vLLM venv has no parent directory".to_string())?
            .to_path_buf();
        tokio::fs::create_dir_all(&parent)
            .await
            .map_err(|error| format!("Could not create vLLM data directory: {error}"))?;
        let parent_metadata = tokio::fs::symlink_metadata(&parent)
            .await
            .map_err(|error| format!("Could not inspect vLLM data directory: {error}"))?;
        if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
            return Err("vLLM data directory must be a real directory".to_string());
        }
        #[cfg(unix)]
        tokio::fs::set_permissions(&parent, {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(0o700)
        })
        .await
        .map_err(|error| format!("Could not protect vLLM data directory: {error}"))?;
        tokio::fs::create_dir_all(parent.join("runtime-cache").join("uv"))
            .await
            .map_err(|error| format!("Could not create vLLM runtime cache: {error}"))?;
        tokio::fs::create_dir_all(parent.join("python"))
            .await
            .map_err(|error| format!("Could not create vLLM Python directory: {error}"))?;
        Self::verify_uv(&uv, &parent).await?;

        let staging = tempfile::Builder::new()
            .prefix(".vllm-bootstrap-")
            .tempdir_in(&parent)
            .map_err(|error| format!("Could not create vLLM staging directory: {error}"))?;
        let staged_venv = staging.path().join("venv");
        Self::run_uv(
            &uv,
            &parent,
            "venv creation",
            &[
                "venv".into(),
                "--python".into(),
                PYTHON_VERSION.into(),
                "--python-preference".into(),
                "only-managed".into(),
                "--relocatable".into(),
                staged_venv.as_os_str().to_os_string(),
            ],
            VENV_TIMEOUT,
        )
        .await?;

        let staged_python = Self::python_path_for(&staged_venv);
        let staged_requirements = staging.path().join("requirements.lock");
        Self::write_private_file(&staged_requirements, REQUIREMENTS_LOCK).await?;
        Self::run_uv(
            &uv,
            &parent,
            "package installation",
            &[
                "pip".into(),
                "install".into(),
                "--require-hashes".into(),
                "--only-binary".into(),
                ":all:".into(),
                "--torch-backend".into(),
                VLLM_TORCH_BACKEND.into(),
                "--python".into(),
                staged_python.as_os_str().to_os_string(),
                "--requirements".into(),
                staged_requirements.as_os_str().to_os_string(),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        Self::validate_python_environment(&staged_python).await?;
        Self::write_private_file(
            &staged_venv.join(BOOTSTRAP_MARKER),
            Self::expected_marker().as_bytes(),
        )
        .await?;
        if !Self::environment_is_complete(&staged_venv) {
            return Err("Staged vLLM environment did not pass validation".to_string());
        }

        let backup = staging.path().join("previous-venv");
        let had_previous = match tokio::fs::symlink_metadata(&final_venv).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("Existing vLLM environment is not a real directory".to_string());
                }
                rename_no_replace(&final_venv, &backup)
                    .map_err(|error| format!("Could not stage old vLLM environment: {error}"))?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("Could not inspect vLLM environment: {error}")),
        };
        if let Err(error) = rename_no_replace(&staged_venv, &final_venv) {
            if had_previous {
                let _ = rename_no_replace(&backup, &final_venv);
            }
            return Err(format!(
                "Could not atomically activate vLLM environment: {error}"
            ));
        }
        if let Err(error) = Self::validate_activated_environment(&final_venv).await {
            let failed = staging.path().join("failed-venv");
            rename_no_replace(&final_venv, &failed).map_err(|rollback_error| {
                format!("{error}; could not stage failed vLLM environment: {rollback_error}")
            })?;
            if had_previous {
                rename_no_replace(&backup, &final_venv).map_err(|rollback_error| {
                    format!(
                        "{error}; could not restore previous vLLM environment: {rollback_error}"
                    )
                })?;
            }
            return Err(error);
        }
        if had_previous {
            if let Err(error) = tokio::fs::remove_dir_all(&backup).await {
                tracing::warn!(%error, "Could not remove previous vLLM environment");
            }
        }
        Ok(())
    }

    fn find_free_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("Failed to reserve a loopback port: {error}"))?;
        listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("Failed to inspect reserved port: {error}"))
    }

    fn validate_model_directory(model_path: &str) -> Result<PathBuf, String> {
        if model_path.is_empty() || model_path.len() > 4096 {
            return Err("Model path is empty or too long".to_string());
        }
        let path = PathBuf::from(model_path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect model directory: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("vLLM model path must be a real local directory".to_string());
        }
        let config = path.join("config.json");
        let config_metadata = std::fs::symlink_metadata(&config)
            .map_err(|error| format!("Model config.json is unavailable: {error}"))?;
        if config_metadata.file_type().is_symlink()
            || !config_metadata.is_file()
            || config_metadata.len() > 1024 * 1024
        {
            return Err("Model config.json must be a bounded regular file".to_string());
        }
        path.canonicalize()
            .map_err(|error| format!("Could not resolve model directory: {error}"))
    }

    fn generate_api_token() -> String {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn local_client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| format!("Could not build local vLLM client: {error}"))
    }

    fn clear_runtime_state(&self) {
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.served_model.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .effective_context
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self.api_token.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        context_size: u32,
        _options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        if context_size == 0 || context_size > MAX_CONTEXT_SIZE {
            return Err(format!(
                "Context size must be between 1 and {MAX_CONTEXT_SIZE}"
            ));
        }
        if self
            .process
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            return Err("vLLM is already running; stop it before starting another model".into());
        }
        if !self.is_bootstrapped() {
            return Err("vLLM environment is missing or does not match the pinned version".into());
        }

        let model_path = Self::validate_model_directory(model_path)?;
        let model_path_string = model_path
            .to_str()
            .ok_or_else(|| "vLLM model path is not valid UTF-8".to_string())?
            .to_string();
        let model_limit = super::read_model_max_context(&model_path_string);
        let effective_context = model_limit
            .map(|limit| context_size.min(limit))
            .unwrap_or(context_size);
        let venv = self
            .get_venv_path()
            .ok_or_else(|| "vLLM environment not configured".to_string())?;
        let server = Self::server_path_for(&venv);
        let port = Self::find_free_port()?;
        let token = Self::generate_api_token();
        let client = Self::local_client()?;

        self.diagnostics.reset().await;
        let mut command = Command::new(&server);
        command
            .args([
                "serve",
                &model_path_string,
                "--served-model-name",
                SERVED_MODEL_NAME,
                "--port",
                &port.to_string(),
                "--host",
                "127.0.0.1",
                "--api-key",
                &token,
                "--max-model-len",
                &effective_context.to_string(),
                "--disable-log-requests",
            ])
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env("PYTHONNOUSERSITE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Keep ownership local until readiness. Cancellation of this future
        // drops `OwnedChild`, killing the complete process tree without
        // publishing a half-started runtime snapshot.
        let mut child = OwnedChild::spawn(&mut command)
            .map_err(|error| format!("Failed to spawn vLLM: {error}"))?;
        if let Some(stdout) = child.take_stdout() {
            self.diagnostics
                .capture(stdout, "stdout", vec![token.clone()]);
        }
        if let Some(stderr) = child.take_stderr() {
            self.diagnostics
                .capture(stderr, "stderr", vec![token.clone()]);
        }
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let diagnostics = self.diagnostics.summary().await;
                return Err(format!(
                    "vLLM server startup exceeded its {STARTUP_TIMEOUT:?} deadline.\n{diagnostics}"
                ));
            }

            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("Failed to inspect vLLM process: {error}"))?
            {
                let diagnostics = self.diagnostics.summary().await;
                return Err(format!(
                    "vLLM exited during startup with code {:?}.\n{diagnostics}",
                    status.code(),
                ));
            }

            if client
                .get(format!("http://127.0.0.1:{port}/v1/models"))
                .bearer_auth(&token)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                *self.port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
                *self.process.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                *self.served_model.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(SERVED_MODEL_NAME.to_string());
                *self
                    .effective_context
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(effective_context);
                *self.api_token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
                return Ok((port, token));
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
    }

    async fn stop(&self) -> Result<(), String> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        let child = self
            .process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let result = if let Some(mut child) = child {
            child
                .kill()
                .await
                .map_err(|error| format!("Failed to stop vLLM process tree: {error}"))
        } else {
            Ok(())
        };
        self.clear_runtime_state();
        result
    }

    async fn is_ready(&self) -> bool {
        let alive = {
            let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        };
        if !alive {
            self.process
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            self.clear_runtime_state();
            return false;
        }
        let Some(port) = *self.port.lock().unwrap_or_else(|e| e.into_inner()) else {
            return false;
        };
        let Some(token) = self
            .api_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return false;
        };
        let Ok(client) = Self::local_client() else {
            return false;
        };
        client
            .get(format!("http://127.0.0.1:{port}/v1/models"))
            .bearer_auth(token)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    fn base_url(&self) -> Option<String> {
        self.port
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|port| format!("http://127.0.0.1:{port}/v1"))
    }

    fn api_key(&self) -> Option<String> {
        self.api_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn model_id(&self) -> Option<String> {
        self.served_model
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vllm_engine_defaults() {
        let engine = VllmEngine::new();
        assert_eq!(engine.engine_id(), "vllm");
        assert_eq!(engine.hf_search_tag(), "awq");
        assert!(!engine.uses_single_file_model());
        assert!(engine.api_key().is_none());
    }

    #[test]
    fn bootstrap_requires_the_exact_version_marker() {
        let temp = tempfile::tempdir().unwrap();
        let engine = VllmEngine::new();
        engine.set_app_data_dir(temp.path().to_path_buf());
        let venv = temp.path().join("vllm-env");
        let python = VllmEngine::python_path_for(&venv);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, b"python").unwrap();
        std::fs::write(VllmEngine::server_path_for(&venv), b"vllm").unwrap();
        std::fs::write(venv.join(BOOTSTRAP_MARKER), b"vllm=old\n").unwrap();
        assert!(!engine.is_bootstrapped());
        std::fs::write(venv.join(BOOTSTRAP_MARKER), VllmEngine::expected_marker()).unwrap();
        assert!(engine.is_bootstrapped());
    }

    #[test]
    fn generated_api_tokens_are_high_entropy_and_unique() {
        let first = VllmEngine::generate_api_token();
        let second = VllmEngine::generate_api_token();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn host_version_comparison_enforces_reviewed_minimums() {
        assert!(VllmEngine::version_at_least("2.31", "2.31"));
        assert!(VllmEngine::version_at_least("2.39", "2.31"));
        assert!(VllmEngine::version_at_least("8.0", "7.5"));
        assert!(!VllmEngine::version_at_least("2.30", "2.31"));
        assert!(!VllmEngine::version_at_least("7.0", "7.5"));
    }
}

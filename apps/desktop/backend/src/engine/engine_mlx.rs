//! MLX inference engine implementation (macOS Apple Silicon only).
//!
//! The Python environment is assembled in a private staging directory from a
//! pinned server release, validated, and atomically activated. The model server
//! runs inside a descendant-owned process boundary and is protected by a
//! per-launch bearer token injected through a small, validated `sitecustomize`
//! shim because upstream `mlx-openai-server` does not implement authentication.

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
    EngineStartOptions, InferenceEngine, RuntimeDiagnostics, MLX_MINIMUM_MACOS,
    MLX_OPENAI_SERVER_VERSION, PYTHON_ABI, PYTHON_VERSION, UV_VERSION,
};

const BOOTSTRAP_MARKER: &str = ".thinclaw-mlx-bootstrap";
const SERVED_MODEL_NAME: &str = "thinclaw-local";
const MAX_CONTEXT_SIZE: u32 = 1_048_576;
const MAX_MODEL_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_MODEL_DIRECTORY_ENTRIES: usize = 4096;
const UTILITY_OUTPUT_LIMIT: usize = 1024 * 1024;
const VENV_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const REQUIREMENTS_LOCK: &[u8] = include_bytes!("../../../runtime/mlx/requirements.lock");
const RUNTIME_PACKAGES_JSON: &str = env!("THINCLAW_MLX_RUNTIME_PACKAGES");

// `mlx-openai-server` 1.8.1 intentionally accepts any non-empty OpenAI client
// key and has no server-side API-key option. Python imports `sitecustomize`
// before the console entry point, so this private venv shim adds authentication
// without editing vendor source. The exact bytes are checked before every use.
const AUTH_SHIM: &str = r#"# ThinClaw MLX authentication shim v1
import hmac
import os

from fastapi import FastAPI
from fastapi.responses import JSONResponse

_thinclaw_original_fastapi_init = FastAPI.__init__


def _thinclaw_fastapi_init(self, *args, **kwargs):
    _thinclaw_original_fastapi_init(self, *args, **kwargs)
    token = os.environ.get("THINCLAW_MLX_API_KEY", "")
    if len(token) < 32:
        raise RuntimeError("ThinClaw MLX API credential is unavailable")
    expected = "Bearer " + token

    @self.middleware("http")
    async def _thinclaw_require_bearer(request, call_next):
        supplied = request.headers.get("authorization", "")
        if not hmac.compare_digest(supplied, expected):
            return JSONResponse(status_code=401, content={"error": "Unauthorized"})
        return await call_next(request)


FastAPI.__init__ = _thinclaw_fastapi_init
"#;

static BOOTSTRAP_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// MLX engine backed by the OpenAI-compatible `mlx-openai-server` process.
pub struct MlxEngine {
    lifecycle: tokio::sync::Mutex<()>,
    port: Mutex<Option<u16>>,
    process: Mutex<Option<OwnedChild>>,
    runtime_dir: Mutex<Option<tempfile::TempDir>>,
    venv_path: Mutex<Option<PathBuf>>,
    uv_path: Mutex<Option<PathBuf>>,
    served_model: Mutex<Option<String>>,
    effective_context: Mutex<Option<u32>>,
    api_token: Mutex<Option<String>>,
    diagnostics: RuntimeDiagnostics,
}

impl MlxEngine {
    pub fn new() -> Self {
        Self {
            lifecycle: tokio::sync::Mutex::new(()),
            port: Mutex::new(None),
            process: Mutex::new(None),
            runtime_dir: Mutex::new(None),
            venv_path: Mutex::new(None),
            uv_path: Mutex::new(None),
            served_model: Mutex::new(None),
            effective_context: Mutex::new(None),
            api_token: Mutex::new(None),
            diagnostics: RuntimeDiagnostics::default(),
        }
    }

    pub fn set_app_data_dir(&self, dir: PathBuf) {
        *self
            .venv_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(dir.join("mlx-env"));
    }

    pub fn set_uv_path(&self, path: PathBuf) {
        *self
            .uv_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(path);
    }

    fn uv_bin(&self) -> Result<PathBuf, String> {
        if let Some(path) = self
            .uv_path
            .lock()
            .unwrap_or_else(|error| error.into_inner())
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
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn python_path_for(venv: &Path) -> PathBuf {
        venv.join("bin").join("python3")
    }

    fn server_path_for(venv: &Path) -> PathBuf {
        venv.join("bin").join("mlx-openai-server")
    }

    fn sitecustomize_path_for(venv: &Path) -> PathBuf {
        venv.join("lib")
            .join(format!("python{PYTHON_ABI}"))
            .join("site-packages")
            .join("sitecustomize.py")
    }

    #[cfg(test)]
    fn get_python_path(&self) -> Option<PathBuf> {
        self.get_venv_path()
            .map(|venv| Self::python_path_for(&venv))
    }

    fn expected_marker() -> String {
        let lock_hash = hex::encode(Sha256::digest(REQUIREMENTS_LOCK));
        format!(
            "mlx-openai-server={MLX_OPENAI_SERVER_VERSION}\npython={PYTHON_VERSION}\nlock-sha256={lock_hash}\nrelocatable=1\nauth-shim=1\n"
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
        let shim = Self::sitecustomize_path_for(venv);
        let marker_ok =
            std::fs::symlink_metadata(&marker).is_ok_and(|metadata| {
                metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 256
            }) && thinclaw_platform::read_regular_file_bounded_single_link(&marker, 256)
                .is_ok_and(|value| value == Self::expected_marker().as_bytes());
        let shim_ok = std::fs::symlink_metadata(&shim).is_ok_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == AUTH_SHIM.len() as u64
        }) && thinclaw_platform::read_regular_file_bounded_single_link(
            &shim,
            AUTH_SHIM.len() as u64,
        )
        .is_ok_and(|value| value == AUTH_SHIM.as_bytes());

        marker_ok
            && shim_ok
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
            if matches!(character, '\n' | '\t') || !character.is_control() {
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

    async fn verify_host() -> Result<(), String> {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return Err("MLX is supported only on Apple Silicon macOS".to_string());
        }
        let executable = PathBuf::from("/usr/bin/sw_vers");
        if !Self::regular_resolved_file(&executable) {
            return Err("Could not inspect the macOS version".to_string());
        }
        let mut command = Command::new(executable);
        command.arg("-productVersion");
        let output = bounded_command_output(&mut command, Duration::from_secs(10), 4096, 4096)
            .await
            .map_err(|error| format!("macOS version check failed: {error}"))?;
        let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let major = actual
            .split('.')
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| format!("Could not parse macOS version: {actual}"))?;
        let minimum_major = MLX_MINIMUM_MACOS
            .split('.')
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| "Invalid MLX minimum macOS version in this build".to_string())?;
        if major < minimum_major {
            return Err(format!(
                "MLX requires macOS {MLX_MINIMUM_MACOS} or newer; this host reports {actual}"
            ));
        }
        Ok(())
    }

    async fn validate_python_environment(python: &Path) -> Result<(), String> {
        let validation = format!(
            r#"import importlib
import importlib.metadata
import platform
from fastapi import FastAPI

expected = {packages}
if platform.python_version() != "{python_version}":
    raise RuntimeError(f"python={{platform.python_version()}}, expected {python_version}")
auth_probe = FastAPI()
if not auth_probe.user_middleware:
    raise RuntimeError("ThinClaw MLX authentication middleware is inactive")
modules = {{
    "mlx": "mlx.core",
    "mlx-lm": "mlx_lm",
    "mlx-vlm": "mlx_vlm",
    "mlx-embeddings": "mlx_embeddings",
    "mlx-openai-server": "app",
    "mflux": "mflux",
    "mlx-whisper": "mlx_whisper",
    "av": "av",
}}
for distribution, wanted in expected.items():
    actual = importlib.metadata.version(distribution)
    if actual != wanted:
        raise RuntimeError(f"{{distribution}}={{actual}}, expected {{wanted}}")
    importlib.import_module(modules[distribution])
print("thinclaw-mlx-runtime-ok")
"#,
            packages = RUNTIME_PACKAGES_JSON,
            python_version = PYTHON_VERSION,
        );
        let mut command = Command::new(python);
        command
            .arg("-c")
            .arg(validation)
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env_remove("VIRTUAL_ENV")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("THINCLAW_MLX_API_KEY", "0".repeat(64));
        let output = bounded_command_output(
            &mut command,
            Duration::from_secs(3 * 60),
            UTILITY_OUTPUT_LIMIT,
            UTILITY_OUTPUT_LIMIT,
        )
        .await
        .map_err(|error| format!("MLX runtime validation failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "MLX runtime validation failed: {}",
                Self::clean_process_detail(&output.stderr)
            ));
        }
        if !String::from_utf8_lossy(&output.stdout).contains("thinclaw-mlx-runtime-ok") {
            return Err("MLX runtime validation returned an unexpected result".to_string());
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
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("THINCLAW_MLX_API_KEY", "0".repeat(64));
        let output = bounded_command_output(
            &mut command,
            Duration::from_secs(60),
            UTILITY_OUTPUT_LIMIT,
            UTILITY_OUTPUT_LIMIT,
        )
        .await
        .map_err(|error| format!("Activated MLX executable validation failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Activated MLX executable validation failed: {}",
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

    /// Build a complete environment in staging and activate it only after the
    /// package, executable, marker, and authentication shim all validate.
    pub async fn bootstrap(&self) -> Result<(), String> {
        let _bootstrap_guard = BOOTSTRAP_LOCK.lock().await;
        Self::verify_host().await?;
        if self.is_bootstrapped() {
            return Ok(());
        }

        let uv = self.uv_bin()?;
        let final_venv = self
            .get_venv_path()
            .ok_or_else(|| "MLX venv path is not configured".to_string())?;
        let parent = final_venv
            .parent()
            .ok_or_else(|| "MLX venv has no parent directory".to_string())?
            .to_path_buf();
        tokio::fs::create_dir_all(&parent)
            .await
            .map_err(|error| format!("Could not create MLX data directory: {error}"))?;
        let parent_metadata = tokio::fs::symlink_metadata(&parent)
            .await
            .map_err(|error| format!("Could not inspect MLX data directory: {error}"))?;
        if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
            return Err("MLX data directory must be a real directory".to_string());
        }
        #[cfg(unix)]
        tokio::fs::set_permissions(&parent, {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(0o700)
        })
        .await
        .map_err(|error| format!("Could not protect MLX data directory: {error}"))?;
        tokio::fs::create_dir_all(parent.join("runtime-cache").join("uv"))
            .await
            .map_err(|error| format!("Could not create MLX runtime cache: {error}"))?;
        tokio::fs::create_dir_all(parent.join("python"))
            .await
            .map_err(|error| format!("Could not create MLX Python directory: {error}"))?;
        Self::verify_uv(&uv, &parent).await?;

        let staging = tempfile::Builder::new()
            .prefix(".mlx-bootstrap-")
            .tempdir_in(&parent)
            .map_err(|error| format!("Could not create MLX staging directory: {error}"))?;
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
                "--python".into(),
                staged_python.as_os_str().to_os_string(),
                "--requirements".into(),
                staged_requirements.as_os_str().to_os_string(),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        let shim_path = Self::sitecustomize_path_for(&staged_venv);
        let shim_parent = shim_path
            .parent()
            .ok_or_else(|| "MLX authentication shim has no parent directory".to_string())?;
        tokio::fs::create_dir_all(shim_parent)
            .await
            .map_err(|error| format!("Could not create MLX site-packages directory: {error}"))?;
        Self::write_private_file(&shim_path, AUTH_SHIM.as_bytes()).await?;
        Self::validate_python_environment(&staged_python).await?;
        Self::write_private_file(
            &staged_venv.join(BOOTSTRAP_MARKER),
            Self::expected_marker().as_bytes(),
        )
        .await?;

        if !Self::environment_is_complete(&staged_venv) {
            return Err("Staged MLX environment did not pass validation".to_string());
        }

        let backup = staging.path().join("previous-venv");
        let had_previous = match tokio::fs::symlink_metadata(&final_venv).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("Existing MLX environment is not a real directory".to_string());
                }
                rename_no_replace(&final_venv, &backup)
                    .map_err(|error| format!("Could not stage old MLX environment: {error}"))?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("Could not inspect MLX environment: {error}")),
        };
        if let Err(error) = rename_no_replace(&staged_venv, &final_venv) {
            if had_previous {
                let _ = rename_no_replace(&backup, &final_venv);
            }
            return Err(format!(
                "Could not atomically activate MLX environment: {error}"
            ));
        }
        if let Err(error) = Self::validate_activated_environment(&final_venv).await {
            let failed = staging.path().join("failed-venv");
            rename_no_replace(&final_venv, &failed).map_err(|rollback_error| {
                format!("{error}; could not stage failed MLX environment: {rollback_error}")
            })?;
            if had_previous {
                rename_no_replace(&backup, &final_venv).map_err(|rollback_error| {
                    format!("{error}; could not restore previous MLX environment: {rollback_error}")
                })?;
            }
            return Err(error);
        }
        if had_previous {
            if let Err(error) = tokio::fs::remove_dir_all(&backup).await {
                tracing::warn!(%error, "Could not remove previous MLX environment");
            }
        }
        Ok(())
    }

    fn read_bounded_regular_file(path: &Path, limit: usize) -> Option<Vec<u8>> {
        thinclaw_platform::read_regular_file_bounded_single_link(path, limit as u64).ok()
    }

    fn validate_model_directory(model_path: &str) -> Result<PathBuf, String> {
        if model_path.is_empty() || model_path.len() > 4096 {
            return Err("MLX model path is empty or too long".to_string());
        }
        let path = PathBuf::from(model_path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect MLX model directory: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("MLX model path must be a real local directory".to_string());
        }

        let config =
            Self::read_bounded_regular_file(&path.join("config.json"), MAX_MODEL_CONFIG_BYTES)
                .ok_or_else(|| "MLX config.json must be a bounded regular file".to_string())?;
        let parsed: serde_json::Value = serde_json::from_slice(&config)
            .map_err(|error| format!("MLX config.json is invalid: {error}"))?;
        if !parsed.is_object() {
            return Err("MLX config.json must contain a JSON object".to_string());
        }

        let mut entries = 0_usize;
        let mut has_weights = false;
        for entry in std::fs::read_dir(&path)
            .map_err(|error| format!("Could not inspect MLX model files: {error}"))?
        {
            entries = entries.saturating_add(1);
            if entries > MAX_MODEL_DIRECTORY_ENTRIES {
                return Err(format!(
                    "MLX model directory exceeds the {MAX_MODEL_DIRECTORY_ENTRIES}-entry limit"
                ));
            }
            let entry = entry.map_err(|error| format!("Could not inspect model entry: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Could not inspect model entry type: {error}"))?;
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy().to_ascii_lowercase();
            has_weights |= name.ends_with(".safetensors") || name.ends_with(".npz");
        }
        if !has_weights {
            return Err("MLX model directory does not contain regular model weights".to_string());
        }

        path.canonicalize()
            .map_err(|error| format!("Could not resolve MLX model directory: {error}"))
    }

    fn find_free_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("Failed to reserve an MLX loopback port: {error}"))?;
        listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("Failed to inspect MLX loopback port: {error}"))
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
            .map_err(|error| format!("Could not build local MLX client: {error}"))
    }

    fn clear_runtime_state(&self) {
        *self.port.lock().unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .served_model
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .effective_context
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .api_token
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .runtime_dir
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
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
            return Err("MLX is already running; stop it before loading another model".into());
        }
        if !self.is_bootstrapped() {
            return Err("MLX environment is missing or does not match the pinned version".into());
        }

        let model_path = Self::validate_model_directory(model_path)?;
        let model_path_string = model_path
            .to_str()
            .ok_or_else(|| "MLX model path is not valid UTF-8".to_string())?
            .to_string();
        let model_limit = super::read_model_max_context(&model_path_string);
        let effective_context = model_limit
            .map(|limit| context_size.min(limit))
            .unwrap_or(context_size);
        let model_type = match crate::model_manager::classify_mlx_vision_directory(&model_path)? {
            true => "multimodal",
            false => "lm",
        };

        let venv = self
            .get_venv_path()
            .ok_or_else(|| "MLX environment is not configured".to_string())?;
        let server = Self::server_path_for(&venv);
        if !Self::regular_resolved_file(&server) {
            return Err("MLX server executable is unavailable".to_string());
        }
        let parent = venv
            .parent()
            .ok_or_else(|| "MLX environment has no parent directory".to_string())?;
        let runtime_dir = tempfile::Builder::new()
            .prefix(".mlx-runtime-")
            .tempdir_in(parent)
            .map_err(|error| format!("Could not create private MLX runtime directory: {error}"))?;
        let prompt_cache = runtime_dir.path().join("prompt-cache");
        std::fs::create_dir(&prompt_cache)
            .map_err(|error| format!("Could not create private MLX prompt cache: {error}"))?;

        let port = Self::find_free_port()?;
        let token = Self::generate_api_token();
        let client = Self::local_client()?;
        self.diagnostics.reset().await;
        let mut command = Command::new(&server);
        command
            .args([
                "launch",
                "--model-path",
                &model_path_string,
                "--model-type",
                model_type,
                "--served-model-name",
                SERVED_MODEL_NAME,
                "--port",
                &port.to_string(),
                "--host",
                "127.0.0.1",
                "--context-length",
                &effective_context.to_string(),
                "--queue-size",
                "8",
                "--decode-concurrency",
                "4",
                "--prompt-concurrency",
                "1",
                "--prefill-step-size",
                "512",
                "--max-tokens",
                "8192",
                "--prompt-cache-size",
                "1",
                "--max-bytes",
                "2147483648",
                "--prompt-cache-dir",
                &prompt_cache.to_string_lossy(),
                "--no-log-file",
                "--log-level",
                "ERROR",
            ])
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env_remove("VIRTUAL_ENV")
            .env("PYTHONNOUSERSITE", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .env("THINCLAW_MLX_API_KEY", &token)
            .current_dir(runtime_dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Keep the process and its private cache local until authenticated
        // readiness succeeds. Cancellation drops both leases automatically.
        let mut child = OwnedChild::spawn(&mut command)
            .map_err(|error| format!("Failed to spawn MLX server: {error}"))?;
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
                    "MLX server startup exceeded its {STARTUP_TIMEOUT:?} deadline.\n{diagnostics}"
                ));
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("Failed to inspect MLX process: {error}"))?
            {
                let diagnostics = self.diagnostics.summary().await;
                return Err(format!(
                    "MLX server exited during startup with code {:?}.\n{diagnostics}",
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
                *self.port.lock().unwrap_or_else(|error| error.into_inner()) = Some(port);
                *self
                    .process
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(child);
                *self
                    .runtime_dir
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(runtime_dir);
                *self
                    .served_model
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) =
                    Some(SERVED_MODEL_NAME.to_string());
                *self
                    .effective_context
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(effective_context);
                *self
                    .api_token
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(token.clone());
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
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let result = if let Some(mut child) = child {
            child
                .kill()
                .await
                .map_err(|error| format!("Failed to stop MLX process tree: {error}"))
        } else {
            Ok(())
        };
        self.clear_runtime_state();
        result
    }

    async fn is_ready(&self) -> bool {
        let alive = {
            let mut guard = self
                .process
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match guard.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        };
        if !alive {
            self.process
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            self.clear_runtime_state();
            return false;
        }
        let Some(port) = *self.port.lock().unwrap_or_else(|error| error.into_inner()) else {
            return false;
        };
        let Some(token) = self
            .api_token
            .lock()
            .unwrap_or_else(|error| error.into_inner())
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
            .unwrap_or_else(|error| error.into_inner())
            .map(|port| format!("http://127.0.0.1:{port}/v1"))
    }

    fn api_key(&self) -> Option<String> {
        self.api_token
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn model_id(&self) -> Option<String> {
        self.served_model
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn max_context(&self) -> Option<u32> {
        *self
            .effective_context
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn display_name(&self) -> &'static str {
        "MLX (Apple Silicon)"
    }

    fn engine_id(&self) -> &'static str {
        "mlx"
    }

    fn uses_single_file_model(&self) -> bool {
        false
    }

    fn hf_search_tag(&self) -> &'static str {
        "mlx"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_dir(config: &str, weight_map: Option<&str>) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), config).unwrap();
        std::fs::write(directory.path().join("model.safetensors"), [0_u8; 8]).unwrap();
        if let Some(weight_map) = weight_map {
            std::fs::write(
                directory.path().join("model.safetensors.index.json"),
                weight_map,
            )
            .unwrap();
        }
        directory
    }

    #[test]
    fn mlx_engine_defaults() {
        let engine = MlxEngine::new();
        assert_eq!(engine.engine_id(), "mlx");
        assert_eq!(engine.hf_search_tag(), "mlx");
        assert!(!engine.uses_single_file_model());
        assert!(engine.base_url().is_none());
        assert!(engine.api_key().is_none());
    }

    #[test]
    fn venv_path_setup() {
        let directory = tempfile::tempdir().unwrap();
        let engine = MlxEngine::new();
        engine.set_app_data_dir(directory.path().to_path_buf());
        assert_eq!(
            engine.get_venv_path(),
            Some(directory.path().join("mlx-env"))
        );
        assert_eq!(
            engine.get_python_path(),
            Some(directory.path().join("mlx-env/bin/python3"))
        );
    }

    #[test]
    fn vision_requires_config_and_matching_weights() {
        let vision = model_dir(
            r#"{"architectures":["LlavaForConditionalGeneration"],"vision_config":{}}"#,
            Some(r#"{"weight_map":{"vision_tower.layer.weight":"model.safetensors"}}"#),
        );
        assert_eq!(
            crate::model_manager::classify_mlx_vision_directory(vision.path())
                .expect("valid vision directory"),
            true
        );

        let text_only = model_dir(
            r#"{"architectures":["LlamaForCausalLM"],"max_position_embeddings":4096}"#,
            None,
        );
        assert_eq!(
            crate::model_manager::classify_mlx_vision_directory(text_only.path())
                .expect("valid text-only directory"),
            false
        );

        let misleading = model_dir(
            r#"{"architectures":["MistralForConditionalGeneration"],"vision_config":{}}"#,
            Some(r#"{"weight_map":{"language_model.layer.weight":"model.safetensors"}}"#),
        );
        assert!(crate::model_manager::classify_mlx_vision_directory(misleading.path()).is_err());
    }

    #[test]
    fn single_file_safetensors_header_detects_vision_weights() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("config.json"),
            r#"{"vision_config":{}}"#,
        )
        .unwrap();
        let header =
            br#"{"vision_model.layer.weight":{"dtype":"F16","shape":[1],"data_offsets":[0,2]}}"#;
        let mut file = Vec::new();
        file.extend_from_slice(&(header.len() as u64).to_le_bytes());
        file.extend_from_slice(header);
        file.extend_from_slice(&[0, 0]);
        let path = directory.path().join("model.safetensors");
        std::fs::write(&path, file).unwrap();
        assert_eq!(
            crate::model_manager::classify_mlx_vision_directory(directory.path())
                .expect("valid single-file vision directory"),
            true
        );
    }

    #[test]
    fn model_validation_rejects_missing_weights_and_symlinked_config() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), b"{}").unwrap();
        assert!(MlxEngine::validate_model_directory(directory.path().to_str().unwrap()).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = directory.path().join("real-config.json");
            std::fs::write(&target, b"{}").unwrap();
            std::fs::remove_file(directory.path().join("config.json")).unwrap();
            symlink(&target, directory.path().join("config.json")).unwrap();
            std::fs::write(directory.path().join("weights.npz"), b"weights").unwrap();
            assert!(
                MlxEngine::validate_model_directory(directory.path().to_str().unwrap()).is_err()
            );
        }
    }

    #[test]
    fn bootstrap_requires_exact_marker_shim_and_executables() {
        let directory = tempfile::tempdir().unwrap();
        let engine = MlxEngine::new();
        engine.set_app_data_dir(directory.path().to_path_buf());
        let venv = directory.path().join("mlx-env");
        let python = MlxEngine::python_path_for(&venv);
        let server = MlxEngine::server_path_for(&venv);
        let shim = MlxEngine::sitecustomize_path_for(&venv);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&python, b"python").unwrap();
        std::fs::write(&server, b"server").unwrap();
        std::fs::write(&shim, AUTH_SHIM).unwrap();
        std::fs::write(venv.join(BOOTSTRAP_MARKER), b"old").unwrap();
        assert!(!engine.is_bootstrapped());
        std::fs::write(venv.join(BOOTSTRAP_MARKER), MlxEngine::expected_marker()).unwrap();
        assert!(engine.is_bootstrapped());
    }

    #[test]
    fn generated_api_tokens_are_high_entropy_and_unique() {
        let first = MlxEngine::generate_api_token();
        let second = MlxEngine::generate_api_token();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}

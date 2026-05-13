use std::path::PathBuf;
use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

#[derive(Debug, Clone)]
pub struct ComfyUiConfig {
    pub enabled: bool,
    pub mode: String,
    pub host: String,
    pub port: u16,
    pub workspace_dir: PathBuf,
    pub output_dir: PathBuf,
    pub default_workflow: String,
    pub default_aspect_ratio: String,
    pub cloud_api_key_secret: String,
    pub allow_lifecycle_management: bool,
    pub allow_untrusted_workflows: bool,
    pub request_timeout: Duration,
    pub max_output_bytes: u64,
    pub max_concurrent_jobs: usize,
}

impl Default for ComfyUiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "local_existing".to_string(),
            host: "http://127.0.0.1:8188".to_string(),
            port: 8188,
            workspace_dir: crate::platform::resolve_data_dir("comfyui"),
            output_dir: crate::platform::resolve_data_dir("media_cache").join("generated"),
            default_workflow: "sdxl_txt2img".to_string(),
            default_aspect_ratio: "square".to_string(),
            cloud_api_key_secret: "comfy_cloud_api_key".to_string(),
            allow_lifecycle_management: false,
            allow_untrusted_workflows: false,
            request_timeout: Duration::from_secs(600),
            max_output_bytes: 100 * 1024 * 1024,
            max_concurrent_jobs: 1,
        }
    }
}

impl ComfyUiConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("COMFYUI_ENABLED", settings.comfyui.enabled)?,
            mode: optional_env("COMFYUI_MODE")?.unwrap_or_else(|| settings.comfyui.mode.clone()),
            host: optional_env("COMFYUI_HOST")?.unwrap_or_else(|| settings.comfyui.host.clone()),
            port: parse_optional_env("COMFYUI_PORT", settings.comfyui.port)?,
            workspace_dir: optional_env("COMFYUI_WORKSPACE_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&settings.comfyui.workspace_dir)),
            output_dir: optional_env("COMFYUI_OUTPUT_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&settings.comfyui.output_dir)),
            default_workflow: optional_env("COMFYUI_DEFAULT_WORKFLOW")?
                .unwrap_or_else(|| settings.comfyui.default_workflow.clone()),
            default_aspect_ratio: optional_env("COMFYUI_DEFAULT_ASPECT_RATIO")?
                .unwrap_or_else(|| settings.comfyui.default_aspect_ratio.clone()),
            cloud_api_key_secret: optional_env("COMFYUI_CLOUD_API_KEY_SECRET")?
                .unwrap_or_else(|| settings.comfyui.cloud_api_key_secret.clone()),
            allow_lifecycle_management: parse_bool_env(
                "COMFYUI_ALLOW_LIFECYCLE_MANAGEMENT",
                settings.comfyui.allow_lifecycle_management,
            )?,
            allow_untrusted_workflows: parse_bool_env(
                "COMFYUI_ALLOW_UNTRUSTED_WORKFLOWS",
                settings.comfyui.allow_untrusted_workflows,
            )?,
            request_timeout: Duration::from_secs(parse_optional_env(
                "COMFYUI_REQUEST_TIMEOUT_SECS",
                settings.comfyui.request_timeout_secs,
            )?),
            max_output_bytes: parse_optional_env(
                "COMFYUI_MAX_OUTPUT_BYTES",
                settings.comfyui.max_output_bytes,
            )?,
            max_concurrent_jobs: parse_optional_env(
                "COMFYUI_MAX_CONCURRENT_JOBS",
                settings.comfyui.max_concurrent_jobs,
            )?
            .max(1),
        })
    }
}

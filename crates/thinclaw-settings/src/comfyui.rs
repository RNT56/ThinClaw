use super::*;

fn default_comfyui_mode() -> String {
    "local_existing".to_string()
}

fn default_comfyui_host() -> String {
    "http://127.0.0.1:8188".to_string()
}

fn default_comfyui_port() -> u16 {
    8188
}

fn default_comfyui_workspace_dir() -> String {
    thinclaw_platform::resolve_data_dir("comfyui")
        .to_string_lossy()
        .to_string()
}

fn default_comfyui_output_dir() -> String {
    thinclaw_platform::resolve_data_dir("media_cache")
        .join("generated")
        .to_string_lossy()
        .to_string()
}

fn default_comfyui_workflow() -> String {
    "sdxl_txt2img".to_string()
}

fn default_comfyui_aspect_ratio() -> String {
    "square".to_string()
}

fn default_comfyui_cloud_secret() -> String {
    "comfy_cloud_api_key".to_string()
}

fn default_comfyui_request_timeout_secs() -> u64 {
    600
}

fn default_comfyui_max_output_bytes() -> u64 {
    100 * 1024 * 1024
}

fn default_comfyui_max_concurrent_jobs() -> usize {
    1
}

/// ComfyUI media generation settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyUiSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_comfyui_mode")]
    pub mode: String,
    #[serde(default = "default_comfyui_host")]
    pub host: String,
    #[serde(default = "default_comfyui_port")]
    pub port: u16,
    #[serde(default = "default_comfyui_workspace_dir")]
    pub workspace_dir: String,
    #[serde(default = "default_comfyui_output_dir")]
    pub output_dir: String,
    #[serde(default = "default_comfyui_workflow")]
    pub default_workflow: String,
    #[serde(default = "default_comfyui_aspect_ratio")]
    pub default_aspect_ratio: String,
    #[serde(default = "default_comfyui_cloud_secret")]
    pub cloud_api_key_secret: String,
    #[serde(default)]
    pub allow_lifecycle_management: bool,
    #[serde(default)]
    pub allow_untrusted_workflows: bool,
    #[serde(default = "default_comfyui_request_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_comfyui_max_output_bytes")]
    pub max_output_bytes: u64,
    #[serde(default = "default_comfyui_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
}

impl Default for ComfyUiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_comfyui_mode(),
            host: default_comfyui_host(),
            port: default_comfyui_port(),
            workspace_dir: default_comfyui_workspace_dir(),
            output_dir: default_comfyui_output_dir(),
            default_workflow: default_comfyui_workflow(),
            default_aspect_ratio: default_comfyui_aspect_ratio(),
            cloud_api_key_secret: default_comfyui_cloud_secret(),
            allow_lifecycle_management: false,
            allow_untrusted_workflows: false,
            request_timeout_secs: default_comfyui_request_timeout_secs(),
            max_output_bytes: default_comfyui_max_output_bytes(),
            max_concurrent_jobs: default_comfyui_max_concurrent_jobs(),
        }
    }
}

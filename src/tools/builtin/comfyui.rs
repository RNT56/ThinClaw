//! ComfyUI media generation tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::config::ComfyUiConfig;
use crate::context::JobContext;
use crate::secrets::{SecretAccessContext, SecretsStore};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolError, ToolMetadata, ToolOutput,
    ToolRateLimitConfig, ToolSideEffectLevel, require_str,
};

#[derive(Clone)]
struct ComfyToolState {
    config: ComfyUiConfig,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    semaphore: Arc<Semaphore>,
}

impl ComfyToolState {
    fn new(config: ComfyUiConfig, secrets: Option<Arc<dyn SecretsStore + Send + Sync>>) -> Self {
        let permits = config.max_concurrent_jobs.max(1);
        Self {
            config,
            secrets,
            semaphore: Arc::new(Semaphore::new(permits)),
        }
    }

    async fn client(&self) -> Result<thinclaw_media::ComfyUiClient, ToolError> {
        let mode = parse_mode(&self.config.mode)?;
        let api_key = if mode.is_cloud() {
            Some(self.resolve_cloud_api_key().await?)
        } else {
            None
        };
        thinclaw_media::ComfyUiClient::new(thinclaw_media::ComfyUiConfig {
            mode,
            host: self.config.host.clone(),
            api_key,
            output_dir: self.config.output_dir.clone(),
            request_timeout: self.config.request_timeout,
            max_output_bytes: self.config.max_output_bytes,
        })
        .map_err(tool_external)
    }

    async fn resolve_cloud_api_key(&self) -> Result<String, ToolError> {
        if let Some(secrets) = &self.secrets
            && let Ok(secret) = secrets
                .get_for_injection(
                    "default",
                    &self.config.cloud_api_key_secret,
                    SecretAccessContext::new("builtin.comfyui", "comfy_cloud_request")
                        .target("cloud.comfy.org", "/api"),
                )
                .await
        {
            return Ok(secret.expose().to_string());
        }
        std::env::var("COMFY_CLOUD_API_KEY").map_err(|_| {
            ToolError::ExecutionFailed(format!(
                "Comfy Cloud mode requires secret '{}' or COMFY_CLOUD_API_KEY",
                self.config.cloud_api_key_secret
            ))
        })
    }

    fn default_workflow_name(&self) -> String {
        self.config.default_workflow.clone()
    }
}

pub struct ImageGenerateTool {
    state: ComfyToolState,
}

impl ImageGenerateTool {
    pub fn new(
        config: ComfyUiConfig,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state: ComfyToolState::new(config, secrets),
        }
    }
}

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate an image with ComfyUI. Use for prompt-to-image requests. Outputs image files and renderable image artifacts."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Image prompt to generate."},
                "aspect_ratio": {
                    "type": "string",
                    "enum": ["square", "landscape", "portrait", "wide", "tall", "1:1", "16:9", "9:16"],
                    "default": self.state.config.default_aspect_ratio
                },
                "negative_prompt": {"type": "string", "description": "Optional negative prompt."},
                "seed": {"type": "integer", "description": "Optional deterministic seed."},
                "workflow": {"type": "string", "description": "Bundled workflow name or approved workflow path."},
                "width": {"type": "integer", "minimum": 64, "maximum": 4096},
                "height": {"type": "integer", "minimum": 64, "maximum": 4096},
                "steps": {"type": "integer", "minimum": 1, "maximum": 150},
                "cfg": {"type": "number", "minimum": 0.0, "maximum": 30.0},
                "model": {"type": "string", "description": "Optional checkpoint/model filename to inject."}
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        if !self.state.config.enabled {
            return Err(ToolError::ExecutionFailed(
                "ComfyUI image generation is disabled. Set comfyui.enabled=true or COMFYUI_ENABLED=true.".to_string(),
            ));
        }
        let _permit =
            self.state.semaphore.acquire().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("ComfyUI semaphore closed: {e}"))
            })?;
        let start = Instant::now();
        let prompt = require_str(&params, "prompt")?;
        if prompt.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "prompt cannot be empty".to_string(),
            ));
        }

        let workflow_name = params
            .get("workflow")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.state.default_workflow_name());
        let workflow =
            load_workflow(&workflow_name, self.state.config.allow_untrusted_workflows).await?;
        let aspect_ratio = params
            .get("aspect_ratio")
            .and_then(Value::as_str)
            .unwrap_or(&self.state.config.default_aspect_ratio)
            .parse::<thinclaw_media::ComfyAspectRatio>()
            .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;

        let client = self.state.client().await?;
        let generation = client
            .generate(thinclaw_media::ComfyGenerateRequest {
                prompt: prompt.to_string(),
                negative_prompt: params
                    .get("negative_prompt")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                aspect_ratio,
                width: optional_u32(&params, "width")?,
                height: optional_u32(&params, "height")?,
                seed: params.get("seed").and_then(Value::as_i64),
                steps: optional_u32(&params, "steps")?,
                cfg: params.get("cfg").and_then(Value::as_f64),
                model: params
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                workflow,
                workflow_name: workflow_name.clone(),
                input_image: None,
                mask_image: None,
                wait_for_completion: true,
                use_websocket: true,
            })
            .await
            .map_err(tool_external)?;

        Ok(generation_output(generation, start.elapsed()).await?)
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        self.state.config.request_timeout + Duration::from_secs(15)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(8, 40))
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            authoritative_source: false,
            live_data: true,
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: ToolApprovalClass::Conditional,
            parallel_safe: false,
            route_intents: Vec::new(),
        }
    }
}

pub struct ComfyHealthTool {
    state: ComfyToolState,
}

impl ComfyHealthTool {
    pub fn new(
        config: ComfyUiConfig,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state: ComfyToolState::new(config, secrets),
        }
    }
}

#[async_trait]
impl Tool for ComfyHealthTool {
    fn name(&self) -> &str {
        "comfy_health"
    }

    fn description(&self) -> &str {
        "Check configured ComfyUI server health and object-info availability."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "required": []})
    }

    async fn execute(&self, _params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let client = self.state.client().await?;
        let health = client.health().await;
        Ok(ToolOutput::success(json!(health), start.elapsed()))
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::read_only()
    }
}

pub struct ComfyCheckDepsTool {
    state: ComfyToolState,
}

impl ComfyCheckDepsTool {
    pub fn new(
        config: ComfyUiConfig,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state: ComfyToolState::new(config, secrets),
        }
    }
}

#[async_trait]
impl Tool for ComfyCheckDepsTool {
    fn name(&self) -> &str {
        "comfy_check_deps"
    }

    fn description(&self) -> &str {
        "Check a ComfyUI workflow for missing custom nodes and model references."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "workflow": {"type": "string", "description": "Bundled workflow name or approved workflow JSON path."}
            },
            "required": ["workflow"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let workflow_name = require_str(&params, "workflow")?;
        let workflow =
            load_workflow(workflow_name, self.state.config.allow_untrusted_workflows).await?;
        let client = self.state.client().await?;
        let report = client
            .check_dependencies(&workflow)
            .await
            .map_err(tool_external)?;
        Ok(ToolOutput::success(json!(report), start.elapsed()))
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::read_only()
    }
}

pub struct ComfyRunWorkflowTool {
    state: ComfyToolState,
}

impl ComfyRunWorkflowTool {
    pub fn new(
        config: ComfyUiConfig,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state: ComfyToolState::new(config, secrets),
        }
    }
}

#[async_trait]
impl Tool for ComfyRunWorkflowTool {
    fn name(&self) -> &str {
        "comfy_run_workflow"
    }

    fn description(&self) -> &str {
        "Run a bundled or explicitly approved ComfyUI workflow, including img2img, upscale, or custom inpaint/video workflows."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "workflow": {"type": "string", "description": "Bundled workflow name or approved workflow JSON path."},
                "prompt": {"type": "string"},
                "negative_prompt": {"type": "string"},
                "aspect_ratio": {"type": "string", "default": self.state.config.default_aspect_ratio},
                "seed": {"type": "integer"},
                "width": {"type": "integer", "minimum": 64, "maximum": 4096},
                "height": {"type": "integer", "minimum": 64, "maximum": 4096},
                "steps": {"type": "integer", "minimum": 1, "maximum": 150},
                "cfg": {"type": "number", "minimum": 0.0, "maximum": 30.0},
                "model": {"type": "string"},
                "input_image": {"type": "string", "description": "Local input image path for img2img/upscale."},
                "mask_image": {"type": "string", "description": "Local mask image path for inpaint."},
                "wait": {"type": "boolean", "default": true}
            },
            "required": ["workflow", "prompt"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        if !self.state.config.enabled {
            return Err(ToolError::ExecutionFailed(
                "ComfyUI workflow execution is disabled.".to_string(),
            ));
        }
        let _permit =
            self.state.semaphore.acquire().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("ComfyUI semaphore closed: {e}"))
            })?;
        let start = Instant::now();
        let workflow_name = require_str(&params, "workflow")?;
        let prompt = require_str(&params, "prompt")?;
        let workflow =
            load_workflow(workflow_name, self.state.config.allow_untrusted_workflows).await?;
        let aspect_ratio = params
            .get("aspect_ratio")
            .and_then(Value::as_str)
            .unwrap_or(&self.state.config.default_aspect_ratio)
            .parse::<thinclaw_media::ComfyAspectRatio>()
            .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;
        let client = self.state.client().await?;
        let generation = client
            .generate(thinclaw_media::ComfyGenerateRequest {
                prompt: prompt.to_string(),
                negative_prompt: params
                    .get("negative_prompt")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                aspect_ratio,
                width: optional_u32(&params, "width")?,
                height: optional_u32(&params, "height")?,
                seed: params.get("seed").and_then(Value::as_i64),
                steps: optional_u32(&params, "steps")?,
                cfg: params.get("cfg").and_then(Value::as_f64),
                model: params
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                workflow,
                workflow_name: workflow_name.to_string(),
                input_image: params
                    .get("input_image")
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
                mask_image: params
                    .get("mask_image")
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
                wait_for_completion: params.get("wait").and_then(Value::as_bool).unwrap_or(true),
                use_websocket: true,
            })
            .await
            .map_err(tool_external)?;

        Ok(generation_output(generation, start.elapsed()).await?)
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        self.state.config.request_timeout + Duration::from_secs(15)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(6, 30))
    }
}

pub struct ComfyManageTool {
    state: ComfyToolState,
}

impl ComfyManageTool {
    pub fn new(
        config: ComfyUiConfig,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state: ComfyToolState::new(config, secrets),
        }
    }
}

#[async_trait]
impl Tool for ComfyManageTool {
    fn name(&self) -> &str {
        "comfy_manage"
    }

    fn description(&self) -> &str {
        "Explicitly manage local ComfyUI lifecycle: hardware_check, install_cli, install_comfyui, launch, stop, download_model, install_node."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["hardware_check", "install_cli", "install_comfyui", "launch", "stop", "download_model", "install_node"]
                },
                "gpu": {"type": "string", "enum": ["nvidia", "amd", "m-series", "cpu"]},
                "model_url": {"type": "string"},
                "model_type": {"type": "string"},
                "node": {"type": "string"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        if !self.state.config.allow_lifecycle_management {
            return Err(ToolError::ExecutionFailed(
                "ComfyUI lifecycle management is disabled. Set comfyui.allow_lifecycle_management=true.".to_string(),
            ));
        }
        let start = Instant::now();
        let action = require_str(&params, "action")?;
        let result = match action {
            "hardware_check" => hardware_check(),
            "install_cli" => {
                run_command(
                    "python3",
                    &["-m", "pip", "install", "--user", "comfy-cli"],
                    None,
                )
                .await?
            }
            "install_comfyui" => {
                let gpu = params.get("gpu").and_then(Value::as_str).unwrap_or("cpu");
                let flag = match gpu {
                    "nvidia" => "--nvidia",
                    "amd" => "--amd",
                    "m-series" => "--m-series",
                    "cpu" => "--cpu",
                    other => {
                        return Err(ToolError::InvalidParameters(format!(
                            "invalid gpu '{other}'"
                        )));
                    }
                };
                run_command(
                    "comfy",
                    &["--skip-prompt", "install", flag],
                    Some(&self.state.config.workspace_dir),
                )
                .await?
            }
            "launch" => {
                let port = self.state.config.port.to_string();
                run_command(
                    "comfy",
                    &["launch", "--background", "--", "--port", &port],
                    Some(&self.state.config.workspace_dir),
                )
                .await?
            }
            "stop" => {
                run_command("comfy", &["stop"], Some(&self.state.config.workspace_dir)).await?
            }
            "download_model" => {
                let url = require_str(&params, "model_url")?;
                let model_type = params
                    .get("model_type")
                    .and_then(Value::as_str)
                    .unwrap_or("checkpoints");
                run_command(
                    "comfy",
                    &["model", "download", "--url", url, "--type", model_type],
                    Some(&self.state.config.workspace_dir),
                )
                .await?
            }
            "install_node" => {
                let node = require_str(&params, "node")?;
                run_command(
                    "comfy",
                    &["node", "install", node],
                    Some(&self.state.config.workspace_dir),
                )
                .await?
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown action '{other}'"
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(1200)
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            authoritative_source: false,
            live_data: true,
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: ToolApprovalClass::Always,
            parallel_safe: false,
            route_intents: Vec::new(),
        }
    }
}

async fn generation_output(
    generation: thinclaw_media::ComfyGeneration,
    duration: Duration,
) -> Result<ToolOutput, ToolError> {
    let mut artifacts = Vec::new();
    for output in &generation.outputs {
        if output.media_type == "image" {
            let bytes = tokio::fs::read(&output.file_path).await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to read generated image {}: {e}",
                    output.file_path.display()
                ))
            })?;
            artifacts.push(crate::tools::tool::ToolArtifact::Image {
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type: output.mime_type.clone(),
            });
        } else {
            artifacts.push(crate::tools::tool::ToolArtifact::ResourceLink {
                uri: output.file_path.to_string_lossy().to_string(),
                name: Some(output.filename.clone()),
                title: Some(output.filename.clone()),
                mime_type: Some(output.mime_type.clone()),
                description: Some(format!("ComfyUI {} output", output.media_type)),
            });
        }
    }

    Ok(ToolOutput::success(json!(generation), duration).with_artifacts(artifacts))
}

async fn load_workflow(name_or_path: &str, allow_untrusted: bool) -> Result<Value, ToolError> {
    if let Some(workflow) = thinclaw_media::bundled_workflow(name_or_path) {
        return Ok(workflow);
    }
    if !allow_untrusted {
        return Err(ToolError::InvalidParameters(format!(
            "workflow '{name_or_path}' is not bundled and untrusted workflow paths are disabled"
        )));
    }
    let path = Path::new(name_or_path);
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        ToolError::InvalidParameters(format!("failed to read workflow {}: {e}", path.display()))
    })?;
    serde_json::from_str(&content).map_err(|e| {
        ToolError::InvalidParameters(format!("failed to parse workflow {}: {e}", path.display()))
    })
}

fn parse_mode(mode: &str) -> Result<thinclaw_media::ComfyUiMode, ToolError> {
    match mode {
        "local_existing" => Ok(thinclaw_media::ComfyUiMode::LocalExisting),
        "local_managed" => Ok(thinclaw_media::ComfyUiMode::LocalManaged),
        "cloud" => Ok(thinclaw_media::ComfyUiMode::Cloud),
        other => Err(ToolError::InvalidParameters(format!(
            "invalid ComfyUI mode '{other}'"
        ))),
    }
}

fn optional_u32(params: &Value, key: &str) -> Result<Option<u32>, ToolError> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                ToolError::InvalidParameters(format!("{key} is too large for a 32-bit integer"))
            })
        })
        .transpose()
}

fn tool_external(error: impl std::fmt::Display) -> ToolError {
    ToolError::ExternalService(error.to_string())
}

fn hardware_check() -> Value {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();
    let total_memory_gib = system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let cpu_count = system.cpus().len();
    let os = sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_string());
    let arch = std::env::consts::ARCH;
    let verdict = if cfg!(target_os = "macos") && total_memory_gib >= 16.0 {
        "ok_m_series_or_cpu"
    } else if total_memory_gib >= 8.0 {
        "ok_if_gpu_available"
    } else {
        "cloud_recommended"
    };
    json!({
        "os": os,
        "arch": arch,
        "cpu_count": cpu_count,
        "total_memory_gib": (total_memory_gib * 10.0).round() / 10.0,
        "verdict": verdict,
        "notes": [
            "Use nvidia-smi or rocm-smi manually to confirm discrete GPU VRAM.",
            "ComfyUI local generation is most reliable with at least 8GB VRAM or Apple Silicon with 16GB+ unified memory."
        ]
    })
}

async fn run_command(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Value, ToolError> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        tokio::fs::create_dir_all(cwd).await.map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "failed to create ComfyUI workspace {}: {e}",
                cwd.display()
            ))
        })?;
        command.current_dir(cwd);
    }
    let output = command.output().await.map_err(|e| {
        ToolError::ExecutionFailed(format!("failed to run {program} {}: {e}", args.join(" ")))
    })?;
    Ok(json!({
        "program": program,
        "args": args,
        "success": output.status.success(),
        "status": output.status.code(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_generate_schema_has_prompt() {
        let tool = ImageGenerateTool::new(ComfyUiConfig::default(), None);
        let schema = tool.parameters_schema();
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("prompt"))
        );
    }

    #[tokio::test]
    async fn rejects_untrusted_workflow_by_default() {
        let err = load_workflow("/tmp/not-approved.json", false)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("untrusted workflow paths are disabled")
        );
    }
}

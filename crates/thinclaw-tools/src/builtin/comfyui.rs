//! Root-independent ComfyUI host tool wrappers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolArtifact, ToolError, ToolMetadata,
    ToolOutput, ToolRateLimitConfig, ToolSideEffectLevel, require_str,
};
use thinclaw_types::JobContext;

#[cfg(test)]
use crate::ports::ToolOperationScope;
use crate::ports::{ComfyUiToolHostPort, ToolComfyActionRequest, tool_scope_from_job_context};

pub const IMAGE_GENERATE_DESCRIPTION: &str = "Generate an image with ComfyUI. Use for prompt-to-image requests. Outputs image files and renderable image artifacts.";
pub const COMFY_HEALTH_DESCRIPTION: &str =
    "Check configured ComfyUI server health and object-info availability.";
pub const COMFY_CHECK_DEPS_DESCRIPTION: &str =
    "Check a ComfyUI workflow for missing custom nodes and model references.";
pub const COMFY_RUN_WORKFLOW_DESCRIPTION: &str = "Run a bundled or explicitly approved ComfyUI workflow, including img2img, upscale, or custom inpaint/video workflows.";
pub const COMFY_MANAGE_DESCRIPTION: &str = "Explicitly manage local ComfyUI lifecycle: hardware_check, install_cli, install_comfyui, launch, stop, download_model, install_node.";

#[derive(Debug, Clone, PartialEq)]
pub enum ComfyWorkflowJsonSource {
    Bundled(Value),
    ApprovedPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfyGenerationImageBytes {
    pub file_path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComfyGenerationRequestKind {
    ImageGenerate,
    WorkflowRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfyManageCommand {
    pub program: String,
    pub args: Vec<String>,
    pub use_workspace_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComfyManageOperation {
    HardwareCheck,
    Command(ComfyManageCommand),
}

async fn execute_comfy_action<F, Fut>(
    host: &Arc<dyn ComfyUiToolHostPort>,
    params: Value,
    ctx: &JobContext,
    action: F,
) -> Result<ToolOutput, ToolError>
where
    F: FnOnce(Arc<dyn ComfyUiToolHostPort>, ToolComfyActionRequest) -> Fut,
    Fut: std::future::Future<
            Output = Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError>,
        >,
{
    let start = Instant::now();
    let request = ToolComfyActionRequest {
        scope: tool_scope_from_job_context(ctx),
        params,
    };
    let result = action(Arc::clone(host), request)
        .await
        .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
    Ok(ToolOutput::success(result.output, start.elapsed()).with_artifacts(result.artifacts))
}

pub fn image_generate_schema(default_aspect_ratio: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string", "description": "Image prompt to generate."},
            "aspect_ratio": {
                "type": "string",
                "enum": ["square", "landscape", "portrait", "wide", "tall", "1:1", "16:9", "9:16"],
                "default": default_aspect_ratio
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

pub fn comfy_health_schema() -> Value {
    json!({"type": "object", "properties": {}, "required": []})
}

pub fn comfy_check_deps_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workflow": {"type": "string", "description": "Bundled workflow name or approved workflow JSON path."}
        },
        "required": ["workflow"]
    })
}

pub fn comfy_run_workflow_schema(default_aspect_ratio: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "workflow": {"type": "string", "description": "Bundled workflow name or approved workflow JSON path."},
            "prompt": {"type": "string"},
            "negative_prompt": {"type": "string"},
            "aspect_ratio": {"type": "string", "default": default_aspect_ratio},
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

pub fn comfy_manage_schema() -> Value {
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

pub fn resolve_workflow_json_source(
    name_or_path: &str,
    allow_untrusted: bool,
) -> Result<ComfyWorkflowJsonSource, ToolError> {
    if let Some(workflow) = thinclaw_media::bundled_workflow(name_or_path) {
        return Ok(ComfyWorkflowJsonSource::Bundled(workflow));
    }
    if !allow_untrusted {
        return Err(ToolError::InvalidParameters(format!(
            "workflow '{name_or_path}' is not bundled and untrusted workflow paths are disabled"
        )));
    }
    Ok(ComfyWorkflowJsonSource::ApprovedPath)
}

pub fn parse_workflow_json(name_or_path: &str, content: &str) -> Result<Value, ToolError> {
    serde_json::from_str(content).map_err(|e| {
        ToolError::InvalidParameters(format!("failed to parse workflow {name_or_path}: {e}"))
    })
}

pub fn parse_comfy_mode(mode: &str) -> Result<thinclaw_media::ComfyUiMode, ToolError> {
    match mode {
        "local_existing" => Ok(thinclaw_media::ComfyUiMode::LocalExisting),
        "local_managed" => Ok(thinclaw_media::ComfyUiMode::LocalManaged),
        "cloud" => Ok(thinclaw_media::ComfyUiMode::Cloud),
        other => Err(ToolError::InvalidParameters(format!(
            "invalid ComfyUI mode '{other}'"
        ))),
    }
}

pub fn optional_u32_param(params: &Value, key: &str) -> Result<Option<u32>, ToolError> {
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

pub fn tool_external(error: impl std::fmt::Display) -> ToolError {
    ToolError::ExternalService(error.to_string())
}

pub fn comfy_install_gpu_flag(gpu: &str) -> Result<&'static str, ToolError> {
    match gpu {
        "nvidia" => Ok("--nvidia"),
        "amd" => Ok("--amd"),
        "m-series" => Ok("--m-series"),
        "cpu" => Ok("--cpu"),
        other => Err(ToolError::InvalidParameters(format!(
            "invalid gpu '{other}'"
        ))),
    }
}

pub fn validate_manage_action(action: &str) -> Result<(), ToolError> {
    match action {
        "hardware_check" | "install_cli" | "install_comfyui" | "launch" | "stop"
        | "download_model" | "install_node" => Ok(()),
        other => Err(ToolError::InvalidParameters(format!(
            "unknown action '{other}'"
        ))),
    }
}

pub fn validate_manage_params(params: &Value) -> Result<(), ToolError> {
    let action = require_str(params, "action")?;
    validate_manage_action(action)
}

pub fn validate_image_generate_params(params: &Value) -> Result<(), ToolError> {
    let prompt = require_str(params, "prompt")?;
    if prompt.trim().is_empty() {
        return Err(ToolError::InvalidParameters(
            "prompt cannot be empty".to_string(),
        ));
    }
    Ok(())
}

pub fn comfy_workflow_name_or_default(params: &Value, default_workflow: &str) -> String {
    params
        .get("workflow")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_workflow.to_string())
}

pub fn require_workflow_name(params: &Value) -> Result<&str, ToolError> {
    require_str(params, "workflow")
}

pub fn validate_workflow_name_params(params: &Value) -> Result<(), ToolError> {
    let _ = require_workflow_name(params)?;
    Ok(())
}

pub fn validate_workflow_run_params(params: &Value) -> Result<(), ToolError> {
    let _ = require_workflow_name(params)?;
    let _ = require_str(params, "prompt")?;
    Ok(())
}

pub fn comfy_generate_request(
    params: &Value,
    workflow: Value,
    workflow_name: impl Into<String>,
    default_aspect_ratio: &str,
    kind: ComfyGenerationRequestKind,
) -> Result<thinclaw_media::ComfyGenerateRequest, ToolError> {
    match kind {
        ComfyGenerationRequestKind::ImageGenerate => validate_image_generate_params(params)?,
        ComfyGenerationRequestKind::WorkflowRun => validate_workflow_run_params(params)?,
    }

    let prompt = require_str(params, "prompt")?;
    let aspect_ratio = params
        .get("aspect_ratio")
        .and_then(Value::as_str)
        .unwrap_or(default_aspect_ratio)
        .parse::<thinclaw_media::ComfyAspectRatio>()
        .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;
    let accepts_input_images = matches!(kind, ComfyGenerationRequestKind::WorkflowRun);

    Ok(thinclaw_media::ComfyGenerateRequest {
        prompt: prompt.to_string(),
        negative_prompt: params
            .get("negative_prompt")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        aspect_ratio,
        width: optional_u32_param(params, "width")?,
        height: optional_u32_param(params, "height")?,
        seed: params.get("seed").and_then(Value::as_i64),
        steps: optional_u32_param(params, "steps")?,
        cfg: params.get("cfg").and_then(Value::as_f64),
        model: params
            .get("model")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        workflow,
        workflow_name: workflow_name.into(),
        input_image: accepts_input_images
            .then(|| {
                params
                    .get("input_image")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .flatten(),
        mask_image: accepts_input_images
            .then(|| {
                params
                    .get("mask_image")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .flatten(),
        wait_for_completion: if accepts_input_images {
            params.get("wait").and_then(Value::as_bool).unwrap_or(true)
        } else {
            true
        },
        use_websocket: true,
    })
}

pub fn comfy_manage_operation(
    params: &Value,
    port: u16,
) -> Result<ComfyManageOperation, ToolError> {
    let action = require_str(params, "action")?;
    let command = match action {
        "hardware_check" => return Ok(ComfyManageOperation::HardwareCheck),
        "install_cli" => ComfyManageCommand {
            program: "python3".to_string(),
            args: vec![
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--user".to_string(),
                "comfy-cli".to_string(),
            ],
            use_workspace_dir: false,
        },
        "install_comfyui" => {
            let gpu = params.get("gpu").and_then(Value::as_str).unwrap_or("cpu");
            ComfyManageCommand {
                program: "comfy".to_string(),
                args: vec![
                    "--skip-prompt".to_string(),
                    "install".to_string(),
                    comfy_install_gpu_flag(gpu)?.to_string(),
                ],
                use_workspace_dir: true,
            }
        }
        "launch" => ComfyManageCommand {
            program: "comfy".to_string(),
            args: vec![
                "launch".to_string(),
                "--background".to_string(),
                "--".to_string(),
                "--port".to_string(),
                port.to_string(),
            ],
            use_workspace_dir: true,
        },
        "stop" => ComfyManageCommand {
            program: "comfy".to_string(),
            args: vec!["stop".to_string()],
            use_workspace_dir: true,
        },
        "download_model" => {
            let url = require_str(params, "model_url")?;
            let model_type = params
                .get("model_type")
                .and_then(Value::as_str)
                .unwrap_or("checkpoints");
            ComfyManageCommand {
                program: "comfy".to_string(),
                args: vec![
                    "model".to_string(),
                    "download".to_string(),
                    "--url".to_string(),
                    url.to_string(),
                    "--type".to_string(),
                    model_type.to_string(),
                ],
                use_workspace_dir: true,
            }
        }
        "install_node" => {
            let node = require_str(params, "node")?;
            ComfyManageCommand {
                program: "comfy".to_string(),
                args: vec!["node".to_string(), "install".to_string(), node.to_string()],
                use_workspace_dir: true,
            }
        }
        other => {
            return Err(ToolError::InvalidParameters(format!(
                "unknown action '{other}'"
            )));
        }
    };

    Ok(ComfyManageOperation::Command(command))
}

pub fn comfy_hardware_check() -> Value {
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

pub fn comfy_generation_output(
    generation: thinclaw_media::ComfyGeneration,
    duration: Duration,
    image_bytes: Vec<ComfyGenerationImageBytes>,
) -> Result<ToolOutput, ToolError> {
    let mut images_by_path: HashMap<PathBuf, Vec<u8>> = image_bytes
        .into_iter()
        .map(|image| (image.file_path, image.bytes))
        .collect();
    let mut artifacts = Vec::new();
    for output in &generation.outputs {
        if output.media_type == "image" {
            let bytes = images_by_path.remove(&output.file_path).ok_or_else(|| {
                ToolError::ExecutionFailed(format!(
                    "missing generated image bytes {}",
                    output.file_path.display()
                ))
            })?;
            artifacts.push(ToolArtifact::Image {
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type: output.mime_type.clone(),
            });
        } else {
            artifacts.push(ToolArtifact::ResourceLink {
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

pub struct ImageGenerateHostTool {
    host: Arc<dyn ComfyUiToolHostPort>,
    default_aspect_ratio: String,
    request_timeout: Duration,
}

impl ImageGenerateHostTool {
    pub fn new(
        host: Arc<dyn ComfyUiToolHostPort>,
        default_aspect_ratio: impl Into<String>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            host,
            default_aspect_ratio: default_aspect_ratio.into(),
            request_timeout,
        }
    }
}

#[async_trait]
impl Tool for ImageGenerateHostTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        IMAGE_GENERATE_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        image_generate_schema(&self.default_aspect_ratio)
    }

    async fn execute(&self, params: Value, ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        validate_image_generate_params(&params)?;
        execute_comfy_action(&self.host, params, ctx, |host, request| async move {
            host.image_generate_action(request).await
        })
        .await
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        self.request_timeout + Duration::from_secs(15)
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

macro_rules! comfy_host_tool {
    (
        $tool:ident,
        $name:literal,
        $description:expr,
        $schema:expr,
        $validate:expr,
        $method:ident,
        $approval:expr,
        $metadata:expr
    ) => {
        pub struct $tool {
            host: Arc<dyn ComfyUiToolHostPort>,
        }

        impl $tool {
            pub fn new(host: Arc<dyn ComfyUiToolHostPort>) -> Self {
                Self { host }
            }
        }

        #[async_trait]
        impl Tool for $tool {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $description
            }

            fn parameters_schema(&self) -> Value {
                ($schema)()
            }

            async fn execute(
                &self,
                params: Value,
                ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                $validate(&params)?;
                execute_comfy_action(&self.host, params, ctx, |host, request| async move {
                    host.$method(request).await
                })
                .await
            }

            fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
                $approval
            }

            fn metadata(&self) -> ToolMetadata {
                $metadata
            }
        }
    };
}

fn validate_noop(params: &Value) -> Result<(), ToolError> {
    let _ = params;
    Ok(())
}

fn validate_workflow_name(params: &Value) -> Result<(), ToolError> {
    validate_workflow_name_params(params)
}

fn validate_workflow_run(params: &Value) -> Result<(), ToolError> {
    validate_workflow_run_params(params)
}

fn validate_manage(params: &Value) -> Result<(), ToolError> {
    validate_manage_params(params)
}

comfy_host_tool!(
    ComfyHealthHostTool,
    "comfy_health",
    COMFY_HEALTH_DESCRIPTION,
    comfy_health_schema,
    validate_noop,
    comfy_health_action,
    ApprovalRequirement::Never,
    ToolMetadata::read_only()
);

comfy_host_tool!(
    ComfyCheckDepsHostTool,
    "comfy_check_deps",
    COMFY_CHECK_DEPS_DESCRIPTION,
    comfy_check_deps_schema,
    validate_workflow_name,
    comfy_check_deps_action,
    ApprovalRequirement::Never,
    ToolMetadata::read_only()
);

pub struct ComfyRunWorkflowHostTool {
    host: Arc<dyn ComfyUiToolHostPort>,
    default_aspect_ratio: String,
    request_timeout: Duration,
}

impl ComfyRunWorkflowHostTool {
    pub fn new(
        host: Arc<dyn ComfyUiToolHostPort>,
        default_aspect_ratio: impl Into<String>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            host,
            default_aspect_ratio: default_aspect_ratio.into(),
            request_timeout,
        }
    }
}

#[async_trait]
impl Tool for ComfyRunWorkflowHostTool {
    fn name(&self) -> &str {
        "comfy_run_workflow"
    }

    fn description(&self) -> &str {
        COMFY_RUN_WORKFLOW_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        comfy_run_workflow_schema(&self.default_aspect_ratio)
    }

    async fn execute(&self, params: Value, ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        validate_workflow_run(&params)?;
        execute_comfy_action(&self.host, params, ctx, |host, request| async move {
            host.comfy_run_workflow_action(request).await
        })
        .await
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        self.request_timeout + Duration::from_secs(15)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(6, 30))
    }
}

comfy_host_tool!(
    ComfyManageHostTool,
    "comfy_manage",
    COMFY_MANAGE_DESCRIPTION,
    comfy_manage_schema,
    validate_manage,
    comfy_manage_action,
    ApprovalRequirement::Always,
    ToolMetadata {
        authoritative_source: false,
        live_data: true,
        side_effect_level: ToolSideEffectLevel::Write,
        approval_class: ToolApprovalClass::Always,
        parallel_safe: false,
        route_intents: Vec::new(),
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    struct StubComfyHost;

    impl StubComfyHost {
        fn output(
            action: &str,
            request: ToolComfyActionRequest,
        ) -> crate::ports::ToolComfyActionResult {
            crate::ports::ToolComfyActionResult {
                output: json!({
                    "action": action,
                    "principal_id": request.scope.principal_id,
                    "actor_id": request.scope.actor_id,
                    "params": request.params,
                }),
                artifacts: vec![thinclaw_tools_core::ToolArtifact::Text {
                    text: action.to_string(),
                }],
            }
        }
    }

    #[async_trait]
    impl ComfyUiToolHostPort for StubComfyHost {
        async fn comfy_status(
            &self,
            _scope: ToolOperationScope,
        ) -> Result<crate::ports::ToolComfyStatus, crate::ports::ToolHostError> {
            Err(crate::ports::ToolHostError::Unavailable {
                service: "comfy_status_structured".to_string(),
            })
        }

        async fn run_comfy_workflow(
            &self,
            _request: crate::ports::ToolComfyWorkflowRequest,
        ) -> Result<crate::ports::ToolComfyWorkflowResult, crate::ports::ToolHostError> {
            Err(crate::ports::ToolHostError::Unavailable {
                service: "comfy_workflow_structured".to_string(),
            })
        }

        async fn image_generate_action(
            &self,
            request: ToolComfyActionRequest,
        ) -> Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError> {
            Ok(Self::output("image_generate", request))
        }

        async fn comfy_health_action(
            &self,
            request: ToolComfyActionRequest,
        ) -> Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError> {
            Ok(Self::output("comfy_health", request))
        }

        async fn comfy_check_deps_action(
            &self,
            request: ToolComfyActionRequest,
        ) -> Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError> {
            Ok(Self::output("comfy_check_deps", request))
        }

        async fn comfy_run_workflow_action(
            &self,
            request: ToolComfyActionRequest,
        ) -> Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError> {
            Ok(Self::output("comfy_run_workflow", request))
        }

        async fn comfy_manage_action(
            &self,
            request: ToolComfyActionRequest,
        ) -> Result<crate::ports::ToolComfyActionResult, crate::ports::ToolHostError> {
            Ok(Self::output("comfy_manage", request))
        }
    }

    fn host() -> Arc<dyn ComfyUiToolHostPort> {
        Arc::new(StubComfyHost)
    }

    #[test]
    fn comfyui_policy_helpers_parse_and_validate_params() {
        assert_eq!(
            parse_comfy_mode("cloud").unwrap(),
            thinclaw_media::ComfyUiMode::Cloud
        );
        assert!(
            parse_comfy_mode("remote")
                .unwrap_err()
                .to_string()
                .contains("invalid ComfyUI mode")
        );

        assert_eq!(
            optional_u32_param(&json!({ "width": 1024 }), "width").unwrap(),
            Some(1024)
        );
        assert_eq!(optional_u32_param(&json!({}), "width").unwrap(), None);
        assert!(
            optional_u32_param(&json!({ "width": u64::MAX }), "width")
                .unwrap_err()
                .to_string()
                .contains("too large")
        );

        assert_eq!(comfy_install_gpu_flag("m-series").unwrap(), "--m-series");
        assert!(
            comfy_install_gpu_flag("intel")
                .unwrap_err()
                .to_string()
                .contains("invalid gpu")
        );
        assert!(validate_manage_action("hardware_check").is_ok());
        assert!(
            validate_manage_action("reboot")
                .unwrap_err()
                .to_string()
                .contains("unknown action")
        );
    }

    #[test]
    fn comfyui_workflow_helpers_resolve_and_parse_json() {
        match resolve_workflow_json_source("default", false).unwrap() {
            ComfyWorkflowJsonSource::Bundled(workflow) => assert!(workflow.is_object()),
            ComfyWorkflowJsonSource::ApprovedPath => panic!("default workflow should be bundled"),
        }
        assert_eq!(
            resolve_workflow_json_source("/tmp/custom.json", true).unwrap(),
            ComfyWorkflowJsonSource::ApprovedPath
        );
        assert!(
            resolve_workflow_json_source("/tmp/custom.json", false)
                .unwrap_err()
                .to_string()
                .contains("untrusted workflow paths are disabled")
        );

        let parsed = parse_workflow_json(
            "workflow.json",
            r#"{"1":{"class_type":"CheckpointLoaderSimple"}}"#,
        )
        .unwrap();
        assert_eq!(parsed["1"]["class_type"], "CheckpointLoaderSimple");
        assert!(
            parse_workflow_json("workflow.json", "{")
                .unwrap_err()
                .to_string()
                .contains("failed to parse workflow workflow.json")
        );
    }

    #[test]
    fn comfyui_generate_request_maps_image_and_workflow_params() {
        let image_request = comfy_generate_request(
            &json!({
                "prompt": "cat",
                "negative_prompt": "blur",
                "aspect_ratio": "wide",
                "seed": 9,
                "steps": 12,
                "cfg": 7.5,
                "model": "model.safetensors",
                "input_image": "/tmp/ignored.png",
                "wait": false
            }),
            json!({ "workflow": "image" }),
            "default",
            "square",
            ComfyGenerationRequestKind::ImageGenerate,
        )
        .unwrap();

        assert_eq!(image_request.prompt, "cat");
        assert_eq!(image_request.negative_prompt.as_deref(), Some("blur"));
        assert_eq!(
            image_request.aspect_ratio,
            thinclaw_media::ComfyAspectRatio::Wide
        );
        assert_eq!(image_request.seed, Some(9));
        assert_eq!(image_request.steps, Some(12));
        assert_eq!(image_request.cfg, Some(7.5));
        assert_eq!(image_request.model.as_deref(), Some("model.safetensors"));
        assert_eq!(image_request.workflow_name, "default");
        assert_eq!(image_request.input_image, None);
        assert!(image_request.wait_for_completion);
        assert!(image_request.use_websocket);

        let workflow_request = comfy_generate_request(
            &json!({
                "workflow": "custom",
                "prompt": "dog",
                "input_image": "/tmp/input.png",
                "mask_image": "/tmp/mask.png",
                "wait": false
            }),
            json!({ "workflow": "run" }),
            "custom",
            "portrait",
            ComfyGenerationRequestKind::WorkflowRun,
        )
        .unwrap();

        assert_eq!(
            workflow_request.aspect_ratio,
            thinclaw_media::ComfyAspectRatio::Portrait
        );
        assert_eq!(
            workflow_request.input_image.as_deref(),
            Some(std::path::Path::new("/tmp/input.png"))
        );
        assert_eq!(
            workflow_request.mask_image.as_deref(),
            Some(std::path::Path::new("/tmp/mask.png"))
        );
        assert!(!workflow_request.wait_for_completion);

        assert!(
            comfy_generate_request(
                &json!({ "prompt": "   " }),
                json!({}),
                "default",
                "square",
                ComfyGenerationRequestKind::ImageGenerate,
            )
            .unwrap_err()
            .to_string()
            .contains("prompt cannot be empty")
        );
    }

    #[test]
    fn comfyui_manage_operation_maps_actions_to_command_specs() {
        assert_eq!(
            comfy_manage_operation(&json!({ "action": "hardware_check" }), 8188).unwrap(),
            ComfyManageOperation::HardwareCheck
        );

        let install = comfy_manage_operation(
            &json!({ "action": "install_comfyui", "gpu": "m-series" }),
            8188,
        )
        .unwrap();
        assert_eq!(
            install,
            ComfyManageOperation::Command(ComfyManageCommand {
                program: "comfy".to_string(),
                args: vec![
                    "--skip-prompt".to_string(),
                    "install".to_string(),
                    "--m-series".to_string(),
                ],
                use_workspace_dir: true,
            })
        );

        let launch = comfy_manage_operation(&json!({ "action": "launch" }), 9191).unwrap();
        match launch {
            ComfyManageOperation::Command(command) => {
                assert_eq!(command.program, "comfy");
                assert_eq!(
                    command.args,
                    ["launch", "--background", "--", "--port", "9191"]
                );
                assert!(command.use_workspace_dir);
            }
            ComfyManageOperation::HardwareCheck => panic!("expected command"),
        }

        assert!(
            comfy_manage_operation(&json!({ "action": "download_model" }), 8188)
                .unwrap_err()
                .to_string()
                .contains("model_url")
        );
    }

    #[test]
    fn comfyui_generation_output_maps_artifacts_from_supplied_bytes() {
        let image_path = PathBuf::from("/tmp/comfy-output.png");
        let video_path = PathBuf::from("/tmp/comfy-output.mp4");
        let generation = thinclaw_media::ComfyGeneration {
            prompt_id: "prompt-1".to_string(),
            client_id: "client-1".to_string(),
            workflow_name: "workflow".to_string(),
            seed: 42,
            width: 1024,
            height: 1024,
            outputs: vec![
                thinclaw_media::ComfySavedOutput {
                    file_path: image_path.clone(),
                    filename: "comfy-output.png".to_string(),
                    mime_type: "image/png".to_string(),
                    size_bytes: 3,
                    media_type: "image".to_string(),
                },
                thinclaw_media::ComfySavedOutput {
                    file_path: video_path.clone(),
                    filename: "comfy-output.mp4".to_string(),
                    mime_type: "video/mp4".to_string(),
                    size_bytes: 5,
                    media_type: "video".to_string(),
                },
            ],
        };

        let output = comfy_generation_output(
            generation,
            Duration::from_millis(7),
            vec![ComfyGenerationImageBytes {
                file_path: image_path,
                bytes: b"img".to_vec(),
            }],
        )
        .unwrap();

        assert_eq!(output.result["prompt_id"], "prompt-1");
        assert_eq!(output.artifacts.len(), 2);
        match &output.artifacts[0] {
            ToolArtifact::Image { data, mime_type } => {
                assert_eq!(data, "aW1n");
                assert_eq!(mime_type, "image/png");
            }
            other => panic!("expected image artifact, got {other:?}"),
        }
        match &output.artifacts[1] {
            ToolArtifact::ResourceLink {
                uri,
                name,
                mime_type,
                description,
                ..
            } => {
                assert_eq!(uri, video_path.to_string_lossy().as_ref());
                assert_eq!(name.as_deref(), Some("comfy-output.mp4"));
                assert_eq!(mime_type.as_deref(), Some("video/mp4"));
                assert_eq!(description.as_deref(), Some("ComfyUI video output"));
            }
            other => panic!("expected resource link artifact, got {other:?}"),
        }
    }

    #[test]
    fn comfyui_generation_output_requires_image_bytes_for_images() {
        let image_path = PathBuf::from("/tmp/missing.png");
        let generation = thinclaw_media::ComfyGeneration {
            prompt_id: "prompt-1".to_string(),
            client_id: "client-1".to_string(),
            workflow_name: "workflow".to_string(),
            seed: 42,
            width: 1024,
            height: 1024,
            outputs: vec![thinclaw_media::ComfySavedOutput {
                file_path: image_path,
                filename: "missing.png".to_string(),
                mime_type: "image/png".to_string(),
                size_bytes: 3,
                media_type: "image".to_string(),
            }],
        };

        assert!(
            comfy_generation_output(generation, Duration::from_millis(7), Vec::new())
                .unwrap_err()
                .to_string()
                .contains("missing generated image bytes")
        );
    }

    #[tokio::test]
    async fn comfy_host_tools_delegate_outputs_and_artifacts() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "comfy", "test");
        let host = host();

        let image = ImageGenerateHostTool::new(Arc::clone(&host), "square", Duration::from_secs(5));
        assert_eq!(
            image.parameters_schema()["properties"]["aspect_ratio"]["default"],
            "square"
        );
        assert_eq!(image.rate_limit_config().unwrap().requests_per_minute, 8);
        let output = image
            .execute(json!({ "prompt": "cat" }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["action"], "image_generate");
        assert_eq!(output.artifacts.len(), 1);

        let health = ComfyHealthHostTool::new(Arc::clone(&host));
        assert_eq!(health.metadata(), ToolMetadata::read_only());
        assert_eq!(
            health.execute(json!({}), &ctx).await.unwrap().result["action"],
            "comfy_health"
        );

        let deps = ComfyCheckDepsHostTool::new(Arc::clone(&host));
        assert_eq!(
            deps.execute(json!({ "workflow": "sdxl" }), &ctx)
                .await
                .unwrap()
                .result["action"],
            "comfy_check_deps"
        );

        let run = ComfyRunWorkflowHostTool::new(Arc::clone(&host), "wide", Duration::from_secs(5));
        assert_eq!(
            run.requires_approval(&json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        assert_eq!(
            run.execute(json!({ "workflow": "sdxl", "prompt": "cat" }), &ctx)
                .await
                .unwrap()
                .result["action"],
            "comfy_run_workflow"
        );

        let manage = ComfyManageHostTool::new(host);
        assert_eq!(
            manage.requires_approval(&json!({})),
            ApprovalRequirement::Always
        );
        assert_eq!(
            manage
                .execute(json!({ "action": "hardware_check" }), &ctx)
                .await
                .unwrap()
                .result["action"],
            "comfy_manage"
        );
    }
}

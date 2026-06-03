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
        let prompt = require_str(&params, "prompt")?;
        if prompt.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "prompt cannot be empty".to_string(),
            ));
        }
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
    let _ = require_str(params, "workflow")?;
    Ok(())
}

fn validate_workflow_run(params: &Value) -> Result<(), ToolError> {
    let _ = require_str(params, "workflow")?;
    let _ = require_str(params, "prompt")?;
    Ok(())
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

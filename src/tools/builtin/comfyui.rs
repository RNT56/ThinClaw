//! ComfyUI media generation tools.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::config::ComfyUiConfig;
use crate::context::JobContext;
use crate::secrets::{SecretAccessContext, SecretsStore};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolError, ToolMetadata, ToolOutput,
    ToolRateLimitConfig, ToolSideEffectLevel,
};
use thinclaw_tools::builtin::comfyui::{
    COMFY_CHECK_DEPS_DESCRIPTION, COMFY_HEALTH_DESCRIPTION, COMFY_MANAGE_DESCRIPTION,
    COMFY_RUN_WORKFLOW_DESCRIPTION, ComfyGenerationImageBytes, ComfyGenerationRequestKind,
    ComfyManageOperation, ComfyWorkflowJsonSource, IMAGE_GENERATE_DESCRIPTION,
    comfy_check_deps_schema, comfy_generate_request, comfy_generation_output, comfy_hardware_check,
    comfy_health_schema, comfy_manage_operation, comfy_manage_schema, comfy_run_workflow_schema,
    comfy_workflow_name_or_default, image_generate_schema, parse_comfy_mode, parse_workflow_json,
    require_workflow_name, resolve_workflow_json_source, tool_external,
};
use thinclaw_tools::ports::{
    ComfyUiToolHostPort, ToolComfyActionRequest, ToolComfyActionResult, ToolComfyStatus,
    ToolComfyWorkflowRequest, ToolComfyWorkflowResult, ToolHostError, ToolOperationScope,
    job_context_from_tool_scope,
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
        let mode = parse_comfy_mode(&self.config.mode)?;
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

pub struct RootComfyUiToolHost {
    config: ComfyUiConfig,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

pub fn root_comfyui_tool_host(
    config: ComfyUiConfig,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> Arc<dyn ComfyUiToolHostPort> {
    Arc::new(RootComfyUiToolHost { config, secrets })
}

fn comfy_tool_host_error_from_tool(error: ToolError) -> ToolHostError {
    ToolHostError::OperationFailed {
        reason: error.to_string(),
    }
}

async fn execute_root_comfy_tool<T>(
    tool: T,
    request: ToolComfyActionRequest,
    title: &str,
) -> Result<ToolComfyActionResult, ToolHostError>
where
    T: Tool,
{
    let ctx = job_context_from_tool_scope(request.scope, title);
    let output = tool
        .execute(request.params, &ctx)
        .await
        .map_err(comfy_tool_host_error_from_tool)?;
    Ok(ToolComfyActionResult {
        output: output.result,
        artifacts: output.artifacts,
    })
}

#[async_trait]
impl ComfyUiToolHostPort for RootComfyUiToolHost {
    async fn comfy_status(
        &self,
        _scope: ToolOperationScope,
    ) -> Result<ToolComfyStatus, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "comfy_status_structured".to_string(),
        })
    }

    async fn run_comfy_workflow(
        &self,
        _request: ToolComfyWorkflowRequest,
    ) -> Result<ToolComfyWorkflowResult, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "comfy_workflow_structured".to_string(),
        })
    }

    async fn image_generate_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError> {
        execute_root_comfy_tool(
            ImageGenerateTool::new(self.config.clone(), self.secrets.clone()),
            request,
            "image generate",
        )
        .await
    }

    async fn comfy_health_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError> {
        execute_root_comfy_tool(
            ComfyHealthTool::new(self.config.clone(), self.secrets.clone()),
            request,
            "comfy health",
        )
        .await
    }

    async fn comfy_check_deps_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError> {
        execute_root_comfy_tool(
            ComfyCheckDepsTool::new(self.config.clone(), self.secrets.clone()),
            request,
            "comfy check deps",
        )
        .await
    }

    async fn comfy_run_workflow_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError> {
        execute_root_comfy_tool(
            ComfyRunWorkflowTool::new(self.config.clone(), self.secrets.clone()),
            request,
            "comfy run workflow",
        )
        .await
    }

    async fn comfy_manage_action(
        &self,
        request: ToolComfyActionRequest,
    ) -> Result<ToolComfyActionResult, ToolHostError> {
        execute_root_comfy_tool(
            ComfyManageTool::new(self.config.clone(), self.secrets.clone()),
            request,
            "comfy manage",
        )
        .await
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
        IMAGE_GENERATE_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        image_generate_schema(&self.state.config.default_aspect_ratio)
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
        let workflow_name =
            comfy_workflow_name_or_default(&params, &self.state.default_workflow_name());
        let workflow =
            load_workflow(&workflow_name, self.state.config.allow_untrusted_workflows).await?;
        let request = comfy_generate_request(
            &params,
            workflow,
            workflow_name,
            &self.state.config.default_aspect_ratio,
            ComfyGenerationRequestKind::ImageGenerate,
        )?;

        let client = self.state.client().await?;
        let generation = client.generate(request).await.map_err(tool_external)?;

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
        COMFY_HEALTH_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        comfy_health_schema()
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
        COMFY_CHECK_DEPS_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        comfy_check_deps_schema()
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let workflow_name = require_workflow_name(&params)?;
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
        COMFY_RUN_WORKFLOW_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        comfy_run_workflow_schema(&self.state.config.default_aspect_ratio)
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
        let workflow_name = require_workflow_name(&params)?;
        let workflow =
            load_workflow(workflow_name, self.state.config.allow_untrusted_workflows).await?;
        let request = comfy_generate_request(
            &params,
            workflow,
            workflow_name,
            &self.state.config.default_aspect_ratio,
            ComfyGenerationRequestKind::WorkflowRun,
        )?;
        let client = self.state.client().await?;
        let generation = client.generate(request).await.map_err(tool_external)?;

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
        COMFY_MANAGE_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        comfy_manage_schema()
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        if !self.state.config.allow_lifecycle_management {
            return Err(ToolError::ExecutionFailed(
                "ComfyUI lifecycle management is disabled. Set comfyui.allow_lifecycle_management=true.".to_string(),
            ));
        }
        let start = Instant::now();
        let result = match comfy_manage_operation(&params, self.state.config.port)? {
            ComfyManageOperation::HardwareCheck => comfy_hardware_check(),
            ComfyManageOperation::Command(command) => {
                let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
                let cwd = command
                    .use_workspace_dir
                    .then_some(self.state.config.workspace_dir.as_path());
                run_command(&command.program, &args, cwd).await?
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
    const MAX_INLINE_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_INLINE_IMAGE_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
    const MAX_INLINE_IMAGES: usize = 32;
    let mut image_bytes = Vec::new();
    let mut total_bytes = 0_u64;
    for output in &generation.outputs {
        if output.media_type == "image" {
            if image_bytes.len() >= MAX_INLINE_IMAGES
                || output.size_bytes > MAX_INLINE_IMAGE_BYTES
                || total_bytes
                    .checked_add(output.size_bytes)
                    .is_none_or(|total| total > MAX_INLINE_IMAGE_TOTAL_BYTES)
            {
                return Err(ToolError::ExecutionFailed(
                    "generated image artifacts exceed the inline output limits".to_string(),
                ));
            }
            let bytes = thinclaw_platform::read_regular_file_bounded_async(
                output.file_path.clone(),
                MAX_INLINE_IMAGE_BYTES,
            )
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to read generated image {}: {e}",
                    output.file_path.display()
                ))
            })?;
            if u64::try_from(bytes.len()).ok() != Some(output.size_bytes) {
                return Err(ToolError::ExecutionFailed(format!(
                    "generated image {} changed after publication",
                    output.file_path.display()
                )));
            }
            total_bytes += output.size_bytes;
            image_bytes.push(ComfyGenerationImageBytes {
                file_path: output.file_path.clone(),
                bytes,
            });
        }
    }

    comfy_generation_output(generation, duration, image_bytes)
}

async fn load_workflow(name_or_path: &str, allow_untrusted: bool) -> Result<Value, ToolError> {
    const MAX_WORKFLOW_BYTES: u64 = 8 * 1024 * 1024;
    match resolve_workflow_json_source(name_or_path, allow_untrusted)? {
        ComfyWorkflowJsonSource::Bundled(workflow) => return Ok(workflow),
        ComfyWorkflowJsonSource::ApprovedPath => {}
    }
    let path = Path::new(name_or_path);
    let bytes =
        thinclaw_platform::read_regular_file_bounded_async(path.to_path_buf(), MAX_WORKFLOW_BYTES)
            .await
            .map_err(|e| {
                ToolError::InvalidParameters(format!(
                    "failed to read workflow {}: {e}",
                    path.display()
                ))
            })?;
    let content = String::from_utf8(bytes).map_err(|_| {
        ToolError::InvalidParameters(format!("workflow {} is not valid UTF-8", path.display()))
    })?;
    parse_workflow_json(&path.display().to_string(), &content)
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
    let output = thinclaw_platform::bounded_command_output(
        &mut command,
        Duration::from_secs(10 * 60),
        1024 * 1024,
        1024 * 1024,
    )
    .await
    .map_err(|e| {
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

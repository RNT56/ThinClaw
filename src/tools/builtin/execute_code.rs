//! Compatibility adapter for the execute-code built-in tool.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use thinclaw_tools::builtin::execute_code as runtime_execute_code;

use crate::context::JobContext;
use crate::tools::ToolRegistry;
use crate::tools::execution_backend::{
    CommandExecutionRequest, ExecutionBackend, ExecutionBackendKind, ExecutionResult,
    LocalHostExecutionBackend, ProcessStartRequest, ScriptExecutionRequest, StartedProcess,
};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, ToolRateLimitConfig,
};

pub use runtime_execute_code::ToolRpcHost;

struct RootExecutionBackendAdapter {
    inner: Arc<dyn ExecutionBackend>,
}

impl RootExecutionBackendAdapter {
    fn new(inner: Arc<dyn ExecutionBackend>) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

#[async_trait]
impl thinclaw_tools::execution::LocalExecutionBackend for RootExecutionBackendAdapter {
    fn kind(&self) -> ExecutionBackendKind {
        self.inner.kind()
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_shell(request).await
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        self.inner.start_process(request).await
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_script(request).await
    }
}

struct RootToolRpcHost {
    tools: Weak<ToolRegistry>,
}

impl RootToolRpcHost {
    fn new(tools: Weak<ToolRegistry>) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl ToolRpcHost for RootToolRpcHost {
    async fn execute_tool_rpc(
        &self,
        ctx: &JobContext,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();
        if let Some(reason) = tool_policies.denial_reason_for_metadata(tool_name, &ctx.metadata) {
            return Err(ToolError::NotAuthorized(format!(
                "Tool '{}' is denied by policy: {}",
                tool_name, reason
            )));
        }

        if !ToolRegistry::tool_name_allowed_by_metadata(&ctx.metadata, tool_name) {
            return Err(ToolError::NotAuthorized(format!(
                "Tool '{}' is not permitted in this agent context",
                tool_name
            )));
        }

        let tools = self.tools.upgrade().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "tool_rpc mode is unavailable because the tool registry is not wired".to_string(),
            )
        })?;
        let tool = tools.get(tool_name).await.ok_or_else(|| {
            ToolError::ExecutionFailed(format!("Tool '{}' is not registered", tool_name))
        })?;

        let approval = tool.requires_approval(&params);
        let auto_approved = tool_rpc_auto_approves(tool_name, &params);
        if matches!(approval, ApprovalRequirement::Always)
            || (matches!(approval, ApprovalRequirement::UnlessAutoApproved) && !auto_approved)
        {
            return Err(ToolError::NotAuthorized(format!(
                "Tool '{}' requires approval and cannot run inside tool_rpc",
                tool_name
            )));
        }

        if let Some(config) = tool.rate_limit_config()
            && let crate::tools::rate_limiter::RateLimitResult::Limited { retry_after, .. } = tools
                .rate_limiter()
                .check_and_record(&ctx.user_id, tool_name, &config)
                .await
        {
            return Err(ToolError::RateLimited(Some(retry_after)));
        }

        let timeout = tool.execution_timeout();
        let output = tokio::time::timeout(timeout, tool.execute(params, ctx))
            .await
            .map_err(|_| ToolError::Timeout(timeout))?
            .map_err(|e| ToolError::ExecutionFailed(format!("Inner tool failed: {}", e)))?;
        Ok(output.result)
    }
}

pub struct ExecuteCodeTool {
    inner: runtime_execute_code::ExecuteCodeTool,
}

impl ExecuteCodeTool {
    pub fn new() -> Self {
        Self {
            inner: runtime_execute_code::ExecuteCodeTool::new().with_backend(
                RootExecutionBackendAdapter::new(LocalHostExecutionBackend::shared()),
            ),
        }
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.inner = self.inner.with_working_dir(dir);
        self
    }

    pub fn with_network(mut self, allow: bool) -> Self {
        self.inner = self.inner.with_network(allow);
        self
    }

    pub fn with_backend(mut self, backend: Arc<dyn ExecutionBackend>) -> Self {
        self.inner = self
            .inner
            .with_backend(RootExecutionBackendAdapter::new(backend));
        self
    }

    pub fn with_tool_registry(mut self, tools: Weak<ToolRegistry>) -> Self {
        self.inner = self
            .inner
            .with_tool_rpc_host(Arc::new(RootToolRpcHost::new(tools)));
        self
    }

    pub fn from_runtime(inner: runtime_execute_code::ExecuteCodeTool) -> Self {
        Self { inner }
    }

    pub fn into_runtime(self) -> runtime_execute_code::ExecuteCodeTool {
        self.inner
    }
}

impl Default for ExecuteCodeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExecuteCodeTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        self.inner.execute(params, ctx).await
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        self.inner.requires_approval(params)
    }

    fn requires_sanitization(&self) -> bool {
        self.inner.requires_sanitization()
    }

    fn domain(&self) -> ToolDomain {
        self.inner.domain()
    }

    fn execution_timeout(&self) -> Duration {
        self.inner.execution_timeout()
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        self.inner.rate_limit_config()
    }
}

fn tool_rpc_auto_approves(tool_name: &str, params: &serde_json::Value) -> bool {
    runtime_execute_code::tool_rpc_auto_approves(tool_name, params)
}

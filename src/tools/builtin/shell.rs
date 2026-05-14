//! Compatibility adapter for the shell built-in tool.

#[cfg(feature = "acp")]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thinclaw_tools::builtin::shell as runtime_shell;

use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::sandbox::{SandboxManager, SandboxPolicy};
use crate::tools::execution_backend::{
    CommandExecutionRequest, ExecutionBackend, ExecutionBackendKind, ExecutionResult,
    LocalHostExecutionBackend, ProcessStartRequest, ScriptExecutionRequest, StartedProcess,
};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, ToolRateLimitConfig,
};

#[allow(unused_imports)]
pub use runtime_shell::{
    AcpTerminalExecution, AcpTerminalExecutor, ShellSafetyOptions, ShellSmartApprover,
    detect_command_injection, detect_library_injection, requires_explicit_approval,
};

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

struct RootSmartApprover;

#[async_trait]
impl ShellSmartApprover for RootSmartApprover {
    async fn assess_command(
        &self,
        command: &str,
        description: &str,
        working_dir: &str,
    ) -> thinclaw_tools::smart_approve::ApprovalDecision {
        match super::smart_approve::SmartApprover::from_env().await {
            Ok(approver) => {
                approver
                    .assess_command(command, description, working_dir)
                    .await
            }
            Err(_) => thinclaw_tools::smart_approve::ApprovalDecision::Escalate,
        }
    }
}

#[cfg(feature = "acp")]
struct RootAcpTerminalExecutor;

#[cfg(feature = "acp")]
#[async_trait]
impl AcpTerminalExecutor for RootAcpTerminalExecutor {
    async fn execute_terminal(
        &self,
        session_id: &str,
        command: &str,
        cwd: Option<&str>,
        timeout: Duration,
        extra_env: &HashMap<String, String>,
    ) -> Result<Option<AcpTerminalExecution>, String> {
        crate::channels::acp::client_execute_terminal(session_id, command, cwd, timeout, extra_env)
            .await
            .map(|execution| {
                execution.map(|execution| AcpTerminalExecution {
                    output: execution.output,
                    exit_code: execution.exit_code,
                    signal: execution.signal,
                    truncated: execution.truncated,
                })
            })
    }
}

/// Shell command execution tool.
///
/// This wrapper preserves the root constructor and adapter methods while the
/// root-independent implementation lives in `thinclaw-tools`.
pub struct ShellTool {
    inner: runtime_shell::ShellTool,
    sandbox: Option<Arc<SandboxManager>>,
    sandbox_policy: SandboxPolicy,
}

impl std::fmt::Debug for ShellTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellTool")
            .field("inner", &self.inner)
            .field("sandbox", &self.sandbox.is_some())
            .field("sandbox_policy", &self.sandbox_policy)
            .finish()
    }
}

impl ShellTool {
    pub fn new() -> Self {
        let inner =
            runtime_shell::ShellTool::new().with_smart_approver(Arc::new(RootSmartApprover));
        #[cfg(feature = "acp")]
        let inner = {
            let inner = inner;
            inner.with_acp_terminal(Arc::new(RootAcpTerminalExecutor))
        };
        Self {
            inner,
            sandbox: None,
            sandbox_policy: SandboxPolicy::ReadOnly,
        }
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.inner = self.inner.with_working_dir(dir);
        self
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.inner = self.inner.with_base_dir(dir);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.with_timeout(timeout);
        self
    }

    pub fn with_sandbox(mut self, sandbox: Arc<SandboxManager>) -> Self {
        self.sandbox = Some(sandbox);
        self.refresh_sandbox_backend()
    }

    pub fn with_sandbox_policy(mut self, policy: SandboxPolicy) -> Self {
        self.sandbox_policy = policy;
        self.refresh_sandbox_backend()
    }

    pub fn with_safety_config(mut self, config: &SafetyConfig) -> Self {
        self.inner = self.inner.with_safety_options(&ShellSafetyOptions {
            smart_approval_mode: config.smart_approval_mode.parse().ok(),
            external_scanner_mode: config.external_scanner_mode.parse().ok(),
            external_scanner_path: config.external_scanner_path.clone(),
            external_scanner_require_verified: Some(config.external_scanner_require_verified),
        });
        self
    }

    pub fn with_external_scanner(
        mut self,
        mode: runtime_shell::ExternalScannerMode,
        configured_path: Option<PathBuf>,
    ) -> Self {
        self.inner = self.inner.with_external_scanner(mode, configured_path);
        self
    }

    fn refresh_sandbox_backend(mut self) -> Self {
        if let Some(sandbox) = self.sandbox.as_ref()
            && (sandbox.is_initialized() || sandbox.config().enabled)
        {
            let backend =
                crate::tools::execution_backend::DockerSandboxExecutionBackend::from_sandbox(
                    Arc::clone(sandbox),
                    self.sandbox_policy,
                );
            self.inner = self
                .inner
                .with_sandbox_backend(RootExecutionBackendAdapter::new(backend));
        }
        self
    }

    pub fn from_runtime(inner: runtime_shell::ShellTool) -> Self {
        Self {
            inner,
            sandbox: None,
            sandbox_policy: SandboxPolicy::ReadOnly,
        }
    }

    pub fn into_runtime(self) -> runtime_shell::ShellTool {
        self.inner
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellTool {
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

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        self.inner.rate_limit_config()
    }
}

#[allow(dead_code)]
fn _local_backend_adapter() -> Arc<RootExecutionBackendAdapter> {
    RootExecutionBackendAdapter::new(LocalHostExecutionBackend::shared())
}

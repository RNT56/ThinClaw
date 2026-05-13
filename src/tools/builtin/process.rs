//! Compatibility adapter for the extracted background process tool.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_tools::execution::{
    CommandExecutionRequest, ExecutionBackendKind, ExecutionResult, LocalExecutionBackend,
    ProcessStartRequest, ScriptExecutionRequest, StartedProcess,
};

use crate::tools::execution_backend::ExecutionBackend;
use crate::tools::tool::ToolError;

pub use thinclaw_tools::builtin::process::{ProcessTool, SharedProcessRegistry, start_reaper};

pub struct RootProcessBackendAdapter {
    inner: Arc<dyn ExecutionBackend>,
}

impl RootProcessBackendAdapter {
    pub fn shared(inner: Arc<dyn ExecutionBackend>) -> Arc<dyn LocalExecutionBackend> {
        Arc::new(Self { inner })
    }
}

#[async_trait]
impl LocalExecutionBackend for RootProcessBackendAdapter {
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

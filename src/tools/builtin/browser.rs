//! Compatibility adapter for the CDP browser built-in tool.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
pub use runtime_browser::BrowserDockerRuntime;
use thinclaw_tools::builtin::browser as runtime_browser;

use crate::context::JobContext;
use crate::sandbox::docker_chromium::DockerChromiumConfig;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

#[derive(Clone)]
pub struct RootBrowserDockerRuntime {
    config: DockerChromiumConfig,
}

impl RootBrowserDockerRuntime {
    pub fn new(config: DockerChromiumConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl BrowserDockerRuntime for RootBrowserDockerRuntime {
    fn image_label(&self) -> String {
        self.config.image.clone()
    }

    fn http_endpoint(&self) -> String {
        self.config.http_endpoint()
    }

    fn is_available(&self) -> bool {
        DockerChromiumConfig::is_docker_available()
    }

    async fn start(&self) -> Result<(), String> {
        self.config
            .start_container()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        self.config
            .wait_for_ready(timeout)
            .await
            .map_err(|e| e.to_string())
    }

    async fn stop(&self) -> Result<(), String> {
        self.config.stop_container().map_err(|e| e.to_string())
    }
}

/// Browser automation tool.
///
/// This wrapper preserves the historical root constructor API while delegating
/// the runtime implementation to `thinclaw-tools`.
pub struct BrowserTool {
    inner: runtime_browser::BrowserTool,
}

impl BrowserTool {
    pub fn new(profile_dir: PathBuf) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new(profile_dir),
        }
    }

    pub fn new_with_docker(profile_dir: PathBuf, docker_config: DockerChromiumConfig) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new_with_docker(
                profile_dir,
                Arc::new(RootBrowserDockerRuntime::new(docker_config)),
            ),
        }
    }

    pub fn new_with_cloud(profile_dir: PathBuf, cloud_provider: Option<String>) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new_with_cloud(profile_dir, cloud_provider),
        }
    }

    pub fn from_runtime(inner: runtime_browser::BrowserTool) -> Self {
        Self { inner }
    }

    pub fn into_runtime(self) -> runtime_browser::BrowserTool {
        self.inner
    }
}

#[async_trait]
impl Tool for BrowserTool {
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

    fn execution_timeout(&self) -> Duration {
        self.inner.execution_timeout()
    }
}

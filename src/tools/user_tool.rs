//! Fast-path user tool definitions loaded from `~/.thinclaw/user-tools/`.
//!
//! This surface is intentionally operator-trusted: shell commands run with the
//! same workspace/safety constraints as the local dev tools, WASM tools reuse
//! the existing sandbox runtime, and `mcp_proxy` definitions can expose a
//! narrower alias over an already-registered tool.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
pub use thinclaw_tools::user_tool::{
    RenderMode, UserToolApprovalMode, UserToolDefinition, UserToolKind, UserToolLoadResults,
    UserToolRegistrar, build_placeholder_schema, collect_json_placeholders,
    collect_string_placeholders, load_user_tools_with_registrar, render_placeholder_value,
    render_template_json, render_template_string, resolve_definition_path, shell_quote,
    strictest_requirement,
};

use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::ToolRegistry;
use crate::tools::builtin::ShellTool;
use crate::tools::execution::HostMediatedToolInvoker;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolMetadata, ToolOutput, ToolSchema,
};
#[cfg(not(feature = "wasm-runtime"))]
use crate::tools::wasm::WasmToolRuntime;
#[cfg(feature = "wasm-runtime")]
use crate::tools::wasm::{WasmToolLoader, WasmToolRuntime};

#[derive(Debug)]
pub struct ShellCommandTool {
    name: String,
    description: String,
    command_template: String,
    approval_mode: UserToolApprovalMode,
    schema: serde_json::Value,
    inner: ShellTool,
}

impl ShellCommandTool {
    pub fn new(
        definition: UserToolDefinition,
        base_dir: Option<PathBuf>,
        working_dir: Option<PathBuf>,
        safety: Option<&SafetyConfig>,
    ) -> Self {
        let command_template = definition.command.unwrap_or_default();
        let placeholders = collect_string_placeholders(&command_template)
            .into_iter()
            .collect::<Vec<_>>();
        let mut inner = ShellTool::new();
        if let Some(dir) = working_dir {
            inner = inner.with_working_dir(dir);
        }
        if let Some(dir) = base_dir {
            inner = inner.with_base_dir(dir);
        }
        if let Some(safety) = safety {
            inner = inner.with_safety_config(safety);
        }

        Self {
            schema: build_placeholder_schema(&placeholders),
            name: definition.name,
            description: definition.description,
            command_template,
            approval_mode: definition.approval,
            inner,
        }
    }

    fn render_command(&self, params: &serde_json::Value) -> Result<String, ToolError> {
        render_template_string(&self.command_template, params, RenderMode::ShellEscaped)
    }
}

#[async_trait]
impl Tool for ShellCommandTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn metadata(&self) -> ToolMetadata {
        self.inner.metadata()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let mut shell_params = serde_json::json!({
            "command": self.render_command(&params)?,
        });
        if let Some(workdir) = params.get("workdir").and_then(|value| value.as_str()) {
            shell_params["workdir"] = serde_json::Value::String(workdir.to_string());
        }
        self.inner.execute(shell_params, ctx).await
    }

    fn estimated_cost(&self, params: &serde_json::Value) -> Option<rust_decimal::Decimal> {
        self.inner.estimated_cost(params)
    }

    fn estimated_duration(&self, params: &serde_json::Value) -> Option<Duration> {
        self.inner.estimated_duration(params)
    }

    fn requires_sanitization(&self) -> bool {
        self.inner.requires_sanitization()
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let floor = self.approval_mode.requirement();
        match self.render_command(params) {
            Ok(command) => strictest_requirement(
                floor,
                self.inner
                    .requires_approval(&serde_json::json!({ "command": command })),
            ),
            Err(_) => floor,
        }
    }

    fn execution_timeout(&self) -> Duration {
        self.inner.execution_timeout()
    }

    fn domain(&self) -> ToolDomain {
        self.inner.domain()
    }

    fn rate_limit_config(&self) -> Option<crate::tools::ToolRateLimitConfig> {
        self.inner.rate_limit_config()
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }
}

pub struct McpProxyTool {
    name: String,
    description: String,
    target_tool: String,
    target_impl: Arc<dyn Tool>,
    params_template: Option<serde_json::Value>,
    placeholders: Vec<String>,
    approval_mode: UserToolApprovalMode,
    schema: serde_json::Value,
    target_domain: ToolDomain,
    target_metadata: ToolMetadata,
    target_requires_sanitization: bool,
    target_timeout: Duration,
}

impl std::fmt::Debug for McpProxyTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpProxyTool")
            .field("name", &self.name)
            .field("target_tool", &self.target_tool)
            .field("placeholders", &self.placeholders)
            .finish()
    }
}

impl McpProxyTool {
    pub fn new(
        definition: UserToolDefinition,
        target_impl: Arc<dyn Tool>,
        target_domain: ToolDomain,
        target_metadata: ToolMetadata,
        target_requires_sanitization: bool,
        target_timeout: Duration,
    ) -> Result<Self, String> {
        let params_template = match definition.params {
            Some(value) => Some(serde_json::to_value(value).map_err(|err| err.to_string())?),
            None => None,
        };
        let placeholders = params_template
            .as_ref()
            .map(collect_json_placeholders)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        Ok(Self {
            schema: build_placeholder_schema(&placeholders),
            name: definition.name,
            description: definition.description,
            target_tool: definition.target_tool.unwrap_or_default(),
            target_impl,
            params_template,
            placeholders,
            approval_mode: definition.approval,
            target_domain,
            target_metadata,
            target_requires_sanitization,
            target_timeout,
        })
    }

    fn render_params(&self, params: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
        match self.params_template.as_ref() {
            Some(template) => render_template_json(template, params),
            None => Ok(params.clone()),
        }
    }
}

#[async_trait]
impl Tool for McpProxyTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn metadata(&self) -> ToolMetadata {
        self.target_metadata.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        if self.target_tool == self.name {
            return Err(ToolError::ExecutionFailed(format!(
                "User tool '{}' cannot proxy to itself",
                self.name
            )));
        }

        self.target_impl
            .execute(self.render_params(&params)?, ctx)
            .await
    }

    fn requires_sanitization(&self) -> bool {
        self.target_requires_sanitization
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let floor = self.approval_mode.requirement();
        match self.render_params(params) {
            Ok(rendered) => {
                strictest_requirement(floor, self.target_impl.requires_approval(&rendered))
            }
            Err(_) => floor,
        }
    }

    fn execution_timeout(&self) -> Duration {
        self.target_timeout
    }

    fn domain(&self) -> ToolDomain {
        self.target_domain
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }
}

struct RootUserToolRegistrar {
    registry: Arc<ToolRegistry>,
    base_dir: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    safety: Option<SafetyConfig>,
    #[cfg(feature = "wasm-runtime")]
    wasm_loader: Option<WasmToolLoader>,
}

#[async_trait]
impl UserToolRegistrar for RootUserToolRegistrar {
    async fn register_user_tool(
        &self,
        definition: UserToolDefinition,
        #[cfg_attr(not(feature = "wasm-runtime"), allow(unused_variables))] source_dir: &Path,
    ) -> Result<(), String> {
        match definition.kind {
            UserToolKind::Shell => {
                self.registry
                    .register(Arc::new(ShellCommandTool::new(
                        definition,
                        self.base_dir.clone(),
                        self.working_dir.clone(),
                        self.safety.as_ref(),
                    )))
                    .await;
                Ok(())
            }
            UserToolKind::Wasm => {
                #[cfg(not(feature = "wasm-runtime"))]
                {
                    return Err("WASM runtime is not available in this ThinClaw build".to_string());
                }

                #[cfg(feature = "wasm-runtime")]
                {
                    let Some(loader) = self.wasm_loader.as_ref() else {
                        return Err("WASM runtime is not available for user tools".to_string());
                    };
                    let wasm_path = resolve_definition_path(
                        source_dir,
                        definition.wasm_path.as_deref().unwrap_or(""),
                    );
                    let cap_path = definition
                        .capabilities_path
                        .as_deref()
                        .map(|raw| resolve_definition_path(source_dir, raw));
                    loader
                        .load_from_files(&definition.name, &wasm_path, cap_path.as_deref())
                        .await
                        .map_err(|err| err.to_string())
                }
            }
            UserToolKind::McpProxy => {
                let target_name = definition.target_tool.as_deref().unwrap_or_default();
                let Some(target) = self.registry.get(target_name).await else {
                    return Err(format!(
                        "target tool '{}' is not registered yet",
                        target_name
                    ));
                };

                let proxy = McpProxyTool::new(
                    definition,
                    Arc::clone(&target),
                    target.domain(),
                    target.metadata(),
                    target.requires_sanitization(),
                    target.execution_timeout(),
                )?;
                self.registry.register(Arc::new(proxy)).await;
                Ok(())
            }
        }
    }
}

pub async fn load_user_tools_from_dir(
    registry: Arc<ToolRegistry>,
    dir: &Path,
    base_dir: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    safety: Option<&SafetyConfig>,
    #[cfg_attr(not(feature = "wasm-runtime"), allow(unused_variables))] wasm_runtime: Option<
        Arc<WasmToolRuntime>,
    >,
    #[cfg_attr(not(feature = "wasm-runtime"), allow(unused_variables))] secrets_store: Option<
        Arc<dyn SecretsStore + Send + Sync>,
    >,
    #[cfg_attr(not(feature = "wasm-runtime"), allow(unused_variables))] tool_invoker: Option<
        Arc<HostMediatedToolInvoker>,
    >,
) -> UserToolLoadResults {
    #[cfg_attr(not(feature = "wasm-runtime"), allow(unused_variables))]
    let wasm_loader = {
        #[cfg(feature = "wasm-runtime")]
        {
            wasm_runtime.map(|runtime| {
                let mut loader = WasmToolLoader::new(runtime, Arc::clone(&registry));
                if let Some(secrets) = secrets_store {
                    loader = loader.with_secrets_store(secrets);
                }
                if let Some(invoker) = tool_invoker {
                    loader = loader.with_tool_invoker(invoker);
                }
                loader
            })
        }
        #[cfg(not(feature = "wasm-runtime"))]
        {
            None::<()>
        }
    };

    let registrar = RootUserToolRegistrar {
        registry,
        base_dir,
        working_dir,
        safety: safety.cloned(),
        #[cfg(feature = "wasm-runtime")]
        wasm_loader,
    };
    load_user_tools_with_registrar(&registrar, dir).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool::require_str;

    #[derive(Debug)]
    struct EchoProxyTarget;

    #[async_trait]
    impl Tool for EchoProxyTarget {
        fn name(&self) -> &str {
            "test_proxy_target"
        }

        fn description(&self) -> &str {
            "Test proxy target"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(
                require_str(&params, "message")?,
                Duration::from_millis(1),
            ))
        }

        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    #[test]
    fn parses_shell_definition() {
        let definition = UserToolDefinition::from_toml(
            r#"
name = "demo-shell"
description = "Run a demo command"
kind = "shell"
command = "printf %s {input}"
"#,
        )
        .unwrap();

        assert_eq!(definition.name, "demo-shell");
        assert_eq!(definition.kind, UserToolKind::Shell);
        assert_eq!(definition.approval, UserToolApprovalMode::Always);
    }

    #[test]
    fn renders_shell_placeholders_with_quoting() {
        let rendered = render_template_string(
            "printf %s {input}",
            &serde_json::json!({ "input": "hello 'world'" }),
            RenderMode::ShellEscaped,
        )
        .unwrap();

        assert_eq!(rendered, "printf %s 'hello '\"'\"'world'\"'\"''");
    }

    #[test]
    fn renders_proxy_json_templates() {
        let rendered = render_template_json(
            &serde_json::json!({
                "message": "hello {name}",
                "nested": ["{name}", 7]
            }),
            &serde_json::json!({ "name": "alice" }),
        )
        .unwrap();

        assert_eq!(rendered["message"], "hello alice");
        assert_eq!(rendered["nested"][0], "alice");
        assert_eq!(rendered["nested"][1], 7);
    }

    #[tokio::test]
    async fn proxy_tool_forwards_rendered_params() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_sync(Arc::new(EchoProxyTarget));
        let definition = UserToolDefinition::from_toml(
            r#"
name = "proxy-demo"
description = "Proxy demo"
kind = "mcp_proxy"
target_tool = "test_proxy_target"

[params]
message = "hello {name}"
"#,
        )
        .unwrap();
        let target = registry.get("test_proxy_target").await.unwrap();
        let tool = McpProxyTool::new(
            definition,
            Arc::clone(&target),
            target.domain(),
            target.metadata(),
            target.requires_sanitization(),
            target.execution_timeout(),
        )
        .unwrap();

        let output = tool
            .execute(
                serde_json::json!({
                    "name": "alice"
                }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(output.result, serde_json::json!("hello alice"));
    }

    #[test]
    fn user_tools_default_dir_does_not_collide_with_wasm_tools_dir() {
        let extensions = crate::settings::ExtensionsSettings::default();
        let wasm_dir = crate::platform::state_paths().tools_dir;
        assert_ne!(PathBuf::from(&extensions.user_tools_dir), wasm_dir);
        assert!(
            extensions.user_tools_dir.ends_with("user-tools"),
            "user tool fast path should use the canonical hyphenated directory"
        );
    }
}

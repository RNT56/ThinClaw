//! Fast-path user tool definitions loaded from `~/.thinclaw/user-tools/`.
//!
//! This surface is intentionally operator-trusted: shell commands run with the
//! same workspace/safety constraints as the local dev tools, WASM tools reuse
//! the existing sandbox runtime, and `mcp_proxy` definitions can expose a
//! narrower alias over an already-registered tool.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::ToolRegistry;
use crate::tools::builtin::ShellTool;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolMetadata, ToolOutput, ToolSchema,
};
use crate::tools::wasm::{WasmToolLoader, WasmToolRuntime};

static PLACEHOLDER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("placeholder regex"));

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserToolApprovalMode {
    #[default]
    Always,
    AutoApproved,
    Never,
}

impl UserToolApprovalMode {
    fn requirement(self) -> ApprovalRequirement {
        match self {
            Self::Always => ApprovalRequirement::Always,
            Self::AutoApproved => ApprovalRequirement::UnlessAutoApproved,
            Self::Never => ApprovalRequirement::Never,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserToolKind {
    Shell,
    Wasm,
    McpProxy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserToolDefinition {
    pub name: String,
    pub description: String,
    pub kind: UserToolKind,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub approval: UserToolApprovalMode,
    #[serde(default)]
    pub wasm_path: Option<String>,
    #[serde(default)]
    pub capabilities_path: Option<String>,
    #[serde(default)]
    pub target_tool: Option<String>,
    #[serde(default)]
    pub params: Option<toml::Value>,
}

impl UserToolDefinition {
    pub fn from_toml(raw: &str) -> Result<Self, String> {
        toml::from_str(raw).map_err(|err| err.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("user tool name must not be empty".to_string());
        }

        if self.name.contains('/') || self.name.contains('\\') {
            return Err("user tool name must not contain path separators".to_string());
        }

        if self.name.starts_with('.') {
            return Err("user tool name must not start with '.'".to_string());
        }

        match self.kind {
            UserToolKind::Shell => {
                if self
                    .command
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err("shell user tools require a non-empty 'command'".to_string());
                }
            }
            UserToolKind::Wasm => {
                if self
                    .wasm_path
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err("wasm user tools require 'wasm_path'".to_string());
                }
            }
            UserToolKind::McpProxy => {
                if self
                    .target_tool
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err("mcp_proxy user tools require 'target_tool'".to_string());
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct UserToolLoadResults {
    pub loaded: Vec<String>,
    pub errors: Vec<(PathBuf, String)>,
}

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

#[derive(Debug, Clone, Copy)]
enum RenderMode {
    Raw,
    ShellEscaped,
}

fn strictest_requirement(
    left: ApprovalRequirement,
    right: ApprovalRequirement,
) -> ApprovalRequirement {
    use ApprovalRequirement::{Always, Never, UnlessAutoApproved};

    match (left, right) {
        (Always, _) | (_, Always) => Always,
        (UnlessAutoApproved, _) | (_, UnlessAutoApproved) => UnlessAutoApproved,
        _ => Never,
    }
}

fn collect_string_placeholders(raw: &str) -> BTreeSet<String> {
    PLACEHOLDER_PATTERN
        .captures_iter(raw)
        .filter_map(|captures| captures.get(1).map(|group| group.as_str().to_string()))
        .collect()
}

fn collect_json_placeholders(value: &serde_json::Value) -> BTreeSet<String> {
    let mut placeholders = BTreeSet::new();
    collect_json_placeholders_into(value, &mut placeholders);
    placeholders
}

fn collect_json_placeholders_into(value: &serde_json::Value, placeholders: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(raw) => placeholders.extend(collect_string_placeholders(raw)),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_placeholders_into(item, placeholders);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_json_placeholders_into(value, placeholders);
            }
        }
        _ => {}
    }
}

fn build_placeholder_schema(placeholders: &[String]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for placeholder in placeholders {
        properties.insert(
            placeholder.clone(),
            serde_json::json!({
                "type": ["string", "number", "boolean"],
                "description": format!("Value for placeholder {{{}}}", placeholder),
            }),
        );
        required.push(serde_json::Value::String(placeholder.clone()));
    }
    properties.insert(
        "workdir".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Optional working directory override for shell user tools.",
        }),
    );
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn render_template_string(
    template: &str,
    params: &serde_json::Value,
    mode: RenderMode,
) -> Result<String, ToolError> {
    let mut rendered = String::with_capacity(template.len());
    let mut last = 0usize;

    for captures in PLACEHOLDER_PATTERN.captures_iter(template) {
        let Some(matched) = captures.get(0) else {
            continue;
        };
        let Some(name) = captures.get(1).map(|group| group.as_str()) else {
            continue;
        };
        rendered.push_str(&template[last..matched.start()]);
        rendered.push_str(&render_placeholder_value(name, params, mode)?);
        last = matched.end();
    }

    rendered.push_str(&template[last..]);
    Ok(rendered)
}

fn render_template_json(
    template: &serde_json::Value,
    params: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    match template {
        serde_json::Value::String(raw) => Ok(serde_json::Value::String(render_template_string(
            raw,
            params,
            RenderMode::Raw,
        )?)),
        serde_json::Value::Array(items) => Ok(serde_json::Value::Array(
            items
                .iter()
                .map(|item| render_template_json(item, params))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        serde_json::Value::Object(map) => {
            let mut rendered = serde_json::Map::new();
            for (key, value) in map {
                rendered.insert(key.clone(), render_template_json(value, params)?);
            }
            Ok(serde_json::Value::Object(rendered))
        }
        _ => Ok(template.clone()),
    }
}

fn render_placeholder_value(
    name: &str,
    params: &serde_json::Value,
    mode: RenderMode,
) -> Result<String, ToolError> {
    let value = params
        .get(name)
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))?;

    let raw = match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => serde_json::to_string(value)
            .map_err(|err| ToolError::InvalidParameters(err.to_string()))?,
    };

    Ok(match mode {
        RenderMode::Raw => raw,
        RenderMode::ShellEscaped => shell_quote(&raw),
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'"'"'"#))
}

fn resolve_definition_path(base_dir: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

pub async fn load_user_tools_from_dir(
    registry: Arc<ToolRegistry>,
    dir: &Path,
    base_dir: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    safety: Option<&SafetyConfig>,
    wasm_runtime: Option<Arc<WasmToolRuntime>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> UserToolLoadResults {
    let mut results = UserToolLoadResults::default();

    if !dir.exists() {
        return results;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(err) => {
            results.errors.push((dir.to_path_buf(), err.to_string()));
            return results;
        }
    };

    let mut definition_files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
            definition_files.push(path);
        }
    }
    definition_files.sort();

    let wasm_loader = wasm_runtime.map(|runtime| {
        let mut loader = WasmToolLoader::new(runtime, Arc::clone(&registry));
        if let Some(secrets) = secrets_store {
            loader = loader.with_secrets_store(secrets);
        }
        loader
    });

    for path in definition_files {
        let raw = match tokio::fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(err) => {
                results.errors.push((path.clone(), err.to_string()));
                continue;
            }
        };

        let definition = match UserToolDefinition::from_toml(&raw)
            .and_then(|definition| definition.validate().map(|_| definition))
        {
            Ok(definition) => definition,
            Err(err) => {
                results.errors.push((path.clone(), err));
                continue;
            }
        };

        let source_dir = path.parent().unwrap_or(dir);
        let registration = match definition.kind {
            UserToolKind::Shell => {
                registry
                    .register(Arc::new(ShellCommandTool::new(
                        definition.clone(),
                        base_dir.clone(),
                        working_dir.clone(),
                        safety,
                    )))
                    .await;
                Ok(())
            }
            UserToolKind::Wasm => {
                let Some(loader) = wasm_loader.as_ref() else {
                    results.errors.push((
                        path.clone(),
                        "WASM runtime is not available for user tools".to_string(),
                    ));
                    continue;
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
            UserToolKind::McpProxy => {
                let target_name = definition.target_tool.as_deref().unwrap_or_default();
                let Some(target) = registry.get(target_name).await else {
                    results.errors.push((
                        path.clone(),
                        format!("target tool '{}' is not registered yet", target_name),
                    ));
                    continue;
                };

                let proxy = match McpProxyTool::new(
                    definition.clone(),
                    Arc::clone(&target),
                    target.domain(),
                    target.metadata(),
                    target.requires_sanitization(),
                    target.execution_timeout(),
                ) {
                    Ok(proxy) => proxy,
                    Err(err) => {
                        results.errors.push((path.clone(), err));
                        continue;
                    }
                };
                registry.register(Arc::new(proxy)).await;
                Ok(())
            }
        };

        match registration {
            Ok(()) => results.loaded.push(definition.name),
            Err(err) => results.errors.push((path.clone(), err)),
        }
    }

    results
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

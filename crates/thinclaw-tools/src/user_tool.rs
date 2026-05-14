//! Root-independent user tool definition and template helpers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use thinclaw_tools_core::{ApprovalRequirement, ToolError};

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
    pub fn requirement(self) -> ApprovalRequirement {
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

#[async_trait]
pub trait UserToolRegistrar: Send + Sync {
    async fn register_user_tool(
        &self,
        definition: UserToolDefinition,
        source_dir: &Path,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy)]
pub enum RenderMode {
    Raw,
    ShellEscaped,
}

pub fn strictest_requirement(
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

pub fn collect_string_placeholders(raw: &str) -> BTreeSet<String> {
    PLACEHOLDER_PATTERN
        .captures_iter(raw)
        .filter_map(|captures| captures.get(1).map(|group| group.as_str().to_string()))
        .collect()
}

pub fn collect_json_placeholders(value: &serde_json::Value) -> BTreeSet<String> {
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

pub fn build_placeholder_schema(placeholders: &[String]) -> serde_json::Value {
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

pub fn render_template_string(
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

pub fn render_template_json(
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

pub fn render_placeholder_value(
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

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'"'"'"#))
}

pub fn resolve_definition_path(base_dir: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

pub async fn load_user_tools_with_registrar<R>(registrar: &R, dir: &Path) -> UserToolLoadResults
where
    R: UserToolRegistrar + ?Sized,
{
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
        let name = definition.name.clone();
        match registrar.register_user_tool(definition, source_dir).await {
            Ok(()) => results.loaded.push(name),
            Err(err) => results.errors.push((path.clone(), err)),
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

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

    #[derive(Default)]
    struct RecordingRegistrar {
        loaded: Arc<Mutex<Vec<(String, PathBuf)>>>,
    }

    #[async_trait]
    impl UserToolRegistrar for RecordingRegistrar {
        async fn register_user_tool(
            &self,
            definition: UserToolDefinition,
            source_dir: &Path,
        ) -> Result<(), String> {
            self.loaded
                .lock()
                .await
                .push((definition.name, source_dir.to_path_buf()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn loader_reads_sorted_toml_definitions() {
        let unique = format!("thinclaw-user-tools-{}", uuid::Uuid::new_v4());
        let dir = std::env::temp_dir().join(unique);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("b.toml"),
            r#"
name = "b-tool"
description = "B"
kind = "shell"
command = "echo b"
"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            dir.join("a.toml"),
            r#"
name = "a-tool"
description = "A"
kind = "shell"
command = "echo a"
"#,
        )
        .await
        .unwrap();
        tokio::fs::write(dir.join("ignored.txt"), "ignored")
            .await
            .unwrap();

        let registrar = RecordingRegistrar::default();
        let results = load_user_tools_with_registrar(&registrar, &dir).await;

        assert!(results.errors.is_empty());
        assert_eq!(results.loaded, vec!["a-tool", "b-tool"]);
        let loaded = registrar.loaded.lock().await;
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].1, dir);

        let _ = tokio::fs::remove_dir_all(&loaded[0].1).await;
    }
}

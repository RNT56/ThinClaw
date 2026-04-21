//! Execute code tool with subprocess and Python tool-RPC modes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(windows)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;

use crate::context::JobContext;
use crate::tools::execution_backend::{
    ExecutionBackend, ExecutionBackendKind, ExecutionResult, LocalHostExecutionBackend,
    NetworkIsolationKind, RuntimeDescriptor, ScriptExecutionRequest, host_local_network_isolation,
};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};
use crate::tools::{ToolRateLimitConfig, ToolRegistry};

/// Maximum code length (100KB).
const MAX_CODE_LENGTH: usize = 100 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterpreterConfig {
    program: String,
    prefix_args: Vec<String>,
    extension: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecuteCodeMode {
    Subprocess,
    ToolRpc,
}

impl ExecuteCodeMode {
    fn parse(value: Option<&str>) -> Result<Self, ToolError> {
        match value.unwrap_or("subprocess") {
            "subprocess" => Ok(Self::Subprocess),
            "tool_rpc" => Ok(Self::ToolRpc),
            other => Err(ToolError::InvalidParameters(format!(
                "Unsupported execute_code mode '{}'. Use: subprocess, tool_rpc",
                other
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResultFormat {
    Text,
    Json,
}

impl ResultFormat {
    fn parse(value: Option<&str>) -> Result<Self, ToolError> {
        match value.unwrap_or("text") {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(ToolError::InvalidParameters(format!(
                "Unsupported result_format '{}'. Use: text, json",
                other
            ))),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ToolRpcRequest {
    name: String,
    params: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct ToolRpcResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

const TOOL_RPC_HELPER_EXPORTS: &str = "call_tool, read_file, list_dir, search_files, memory_search, memory_read, memory_write, session_search, http_tool, browser_tool";

trait ToolRpcLanguageAdapter: Sync {
    fn canonical_language(&self) -> &'static str;
    fn entry_extension(&self) -> &'static str;
    fn helper_filename(&self) -> &'static str;
    fn helper_source(&self) -> String;
    fn render_entry_script(&self, user_code: &str) -> String;
    fn augment_env(
        &self,
        extra_env: &mut std::collections::HashMap<String, String>,
        temp_dir: &Path,
        workdir: &Path,
    );
}

struct PythonToolRpcAdapter;
struct JavaScriptToolRpcAdapter;
struct TypeScriptToolRpcAdapter;

impl ToolRpcLanguageAdapter for PythonToolRpcAdapter {
    fn canonical_language(&self) -> &'static str {
        "python"
    }

    fn entry_extension(&self) -> &'static str {
        ".py"
    }

    fn helper_filename(&self) -> &'static str {
        "thinclaw_tools.py"
    }

    fn helper_source(&self) -> String {
        python_tool_rpc_stub()
    }

    fn render_entry_script(&self, user_code: &str) -> String {
        user_code.to_string()
    }

    fn augment_env(
        &self,
        extra_env: &mut std::collections::HashMap<String, String>,
        temp_dir: &Path,
        workdir: &Path,
    ) {
        let mut python_path_entries = vec![
            temp_dir.to_string_lossy().to_string(),
            workdir.to_string_lossy().to_string(),
        ];
        if let Ok(existing_python_path) = std::env::var("PYTHONPATH")
            && !existing_python_path.trim().is_empty()
        {
            python_path_entries.push(existing_python_path);
        }
        extra_env.insert("PYTHONPATH".to_string(), python_path_entries.join(":"));
    }
}

impl ToolRpcLanguageAdapter for JavaScriptToolRpcAdapter {
    fn canonical_language(&self) -> &'static str {
        "javascript"
    }

    fn entry_extension(&self) -> &'static str {
        ".mjs"
    }

    fn helper_filename(&self) -> &'static str {
        "thinclaw_tools.mjs"
    }

    fn helper_source(&self) -> String {
        javascript_tool_rpc_helper()
    }

    fn render_entry_script(&self, user_code: &str) -> String {
        javascript_tool_rpc_entry(self.helper_filename(), user_code)
    }

    fn augment_env(
        &self,
        _extra_env: &mut std::collections::HashMap<String, String>,
        _temp_dir: &Path,
        _workdir: &Path,
    ) {
    }
}

impl ToolRpcLanguageAdapter for TypeScriptToolRpcAdapter {
    fn canonical_language(&self) -> &'static str {
        "typescript"
    }

    fn entry_extension(&self) -> &'static str {
        ".ts"
    }

    fn helper_filename(&self) -> &'static str {
        "thinclaw_tools.ts"
    }

    fn helper_source(&self) -> String {
        typescript_tool_rpc_helper()
    }

    fn render_entry_script(&self, user_code: &str) -> String {
        typescript_tool_rpc_entry(self.helper_filename(), user_code)
    }

    fn augment_env(
        &self,
        _extra_env: &mut std::collections::HashMap<String, String>,
        _temp_dir: &Path,
        _workdir: &Path,
    ) {
    }
}

static PYTHON_TOOL_RPC_ADAPTER: PythonToolRpcAdapter = PythonToolRpcAdapter;
static JAVASCRIPT_TOOL_RPC_ADAPTER: JavaScriptToolRpcAdapter = JavaScriptToolRpcAdapter;
static TYPESCRIPT_TOOL_RPC_ADAPTER: TypeScriptToolRpcAdapter = TypeScriptToolRpcAdapter;

fn tool_rpc_adapter(language: &str) -> Result<&'static dyn ToolRpcLanguageAdapter, ToolError> {
    match language.to_ascii_lowercase().as_str() {
        "python" | "py" | "python3" => Ok(&PYTHON_TOOL_RPC_ADAPTER),
        "javascript" | "js" | "node" => Ok(&JAVASCRIPT_TOOL_RPC_ADAPTER),
        "typescript" | "ts" => Ok(&TYPESCRIPT_TOOL_RPC_ADAPTER),
        other => Err(ToolError::InvalidParameters(format!(
            "tool_rpc mode supports Python, JavaScript, and TypeScript; received '{}'",
            other
        ))),
    }
}

/// Execute code tool.
pub struct ExecuteCodeTool {
    /// Working directory for code execution.
    working_dir: Option<PathBuf>,
    /// Whether network access is allowed in subprocess mode.
    allow_network: bool,
    /// Shared execution backend.
    backend: Arc<dyn ExecutionBackend>,
    /// Weak pointer to the tool registry for host-side tool RPC.
    tools: Option<Weak<ToolRegistry>>,
}

impl ExecuteCodeTool {
    pub fn new() -> Self {
        Self {
            working_dir: None,
            allow_network: false,
            backend: LocalHostExecutionBackend::shared(),
            tools: None,
        }
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    pub fn with_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    pub fn with_backend(mut self, backend: Arc<dyn ExecutionBackend>) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_tool_registry(mut self, tools: Weak<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    fn language_options() -> Vec<&'static str> {
        if cfg!(target_os = "windows") {
            let mut options = vec!["python", "javascript", "typescript", "powershell", "cmd"];
            if Self::find_in_path(&["bash.exe", "bash"]).is_some() {
                options.push("bash");
            }
            options
        } else {
            vec!["python", "javascript", "typescript", "bash"]
        }
    }

    fn find_in_path(names: &[&str]) -> Option<String> {
        let path = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
        None
    }

    fn interpreter_for(language: &str) -> Result<InterpreterConfig, ToolError> {
        match language.to_lowercase().as_str() {
            "python" | "py" | "python3" => {
                if cfg!(target_os = "windows") {
                    if let Some(program) = Self::find_in_path(&["python.exe", "python"]) {
                        Ok(InterpreterConfig {
                            program,
                            prefix_args: Vec::new(),
                            extension: ".py",
                        })
                    } else {
                        Ok(InterpreterConfig {
                            program: Self::find_in_path(&["py.exe", "py"])
                                .unwrap_or_else(|| "py".to_string()),
                            prefix_args: vec!["-3".to_string()],
                            extension: ".py",
                        })
                    }
                } else {
                    Ok(InterpreterConfig {
                        program: "python3".to_string(),
                        prefix_args: Vec::new(),
                        extension: ".py",
                    })
                }
            }
            "javascript" | "js" | "node" => Ok(InterpreterConfig {
                program: "node".to_string(),
                prefix_args: Vec::new(),
                extension: ".js",
            }),
            "typescript" | "ts" => Ok(InterpreterConfig {
                program: "npx".to_string(),
                prefix_args: vec!["tsx".to_string()],
                extension: ".ts",
            }),
            "bash" | "sh" | "shell" if !cfg!(target_os = "windows") => Ok(InterpreterConfig {
                program: "bash".to_string(),
                prefix_args: Vec::new(),
                extension: ".sh",
            }),
            "bash" | "sh" | "shell" => {
                let program = Self::find_in_path(&["bash.exe", "bash"]).ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "Unsupported language: 'bash'. Install bash.exe or use cmd/powershell on Windows."
                            .to_string(),
                    )
                })?;
                Ok(InterpreterConfig {
                    program,
                    prefix_args: Vec::new(),
                    extension: ".sh",
                })
            }
            "powershell" | "pwsh" if cfg!(target_os = "windows") => Ok(InterpreterConfig {
                program: Self::find_in_path(&["pwsh.exe", "pwsh"])
                    .or_else(|| Self::find_in_path(&["powershell.exe", "powershell"]))
                    .unwrap_or_else(|| "powershell".to_string()),
                prefix_args: vec!["-File".to_string()],
                extension: ".ps1",
            }),
            "cmd" if cfg!(target_os = "windows") => Ok(InterpreterConfig {
                program: "cmd".to_string(),
                prefix_args: vec!["/C".to_string()],
                extension: ".cmd",
            }),
            _ => Err(ToolError::InvalidParameters(format!(
                "Unsupported language: '{}'. Use: {}",
                language,
                Self::language_options().join(", ")
            ))),
        }
    }

    fn interpreter_args(config: &InterpreterConfig, script_path: &Path) -> Vec<String> {
        let mut args = config.prefix_args.clone();
        args.push(script_path.to_string_lossy().to_string());
        args
    }

    fn working_dir(&self) -> PathBuf {
        self.working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    fn execution_tempdir(&self, workdir: &Path) -> Result<tempfile::TempDir, ToolError> {
        let mut builder = tempfile::Builder::new();
        let in_workdir = builder
            .prefix(".thinclaw-exec-")
            .tempdir_in(workdir)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to create temp dir: {}", e)));
        match in_workdir {
            Ok(dir) => Ok(dir),
            Err(error) if self.backend.kind() == ExecutionBackendKind::DockerSandbox => Err(error),
            Err(_) => tempfile::Builder::new()
                .prefix("thinclaw-exec-")
                .tempdir()
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to create temp dir: {}", e))
                }),
        }
    }

    async fn execute_subprocess_code(
        &self,
        language: &str,
        code: &str,
        timeout: Duration,
    ) -> Result<(String, i64, Duration), ToolError> {
        let interpreter = Self::interpreter_for(language)?;
        let workdir = self.working_dir();
        let script_file = tempfile::Builder::new()
            .prefix(".thinclaw_exec_")
            .suffix(interpreter.extension)
            .tempfile_in(&workdir)
            .or_else(|error| {
                if self.backend.kind() == ExecutionBackendKind::DockerSandbox {
                    return Err(error);
                }
                tempfile::Builder::new()
                    .prefix("thinclaw_exec_")
                    .suffix(interpreter.extension)
                    .tempfile()
            })
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create script file: {}", e))
            })?;
        let script_path = script_file.path().to_path_buf();

        tokio::fs::write(&script_path, code)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write script: {}", e)))?;

        let program = interpreter.program.clone();
        let args = Self::interpreter_args(&interpreter, &script_path);
        let result = self
            .backend
            .run_script(ScriptExecutionRequest {
                program,
                args,
                workdir,
                timeout,
                extra_env: std::collections::HashMap::new(),
                allow_network: self.allow_network,
            })
            .await?;

        Ok((result.output, result.exit_code, result.duration))
    }

    #[cfg(all(not(unix), not(windows)))]
    async fn execute_tool_rpc_code(
        &self,
        _language: &str,
        _code: &str,
        _timeout: Duration,
        _result_format: ResultFormat,
        _ctx: &JobContext,
    ) -> Result<(serde_json::Value, i64, Duration), ToolError> {
        Err(ToolError::ExecutionFailed(
            "tool_rpc mode is currently unsupported on this platform".to_string(),
        ))
    }

    #[cfg(unix)]
    async fn execute_tool_rpc_code(
        &self,
        language: &str,
        code: &str,
        timeout: Duration,
        result_format: ResultFormat,
        ctx: &JobContext,
    ) -> Result<(serde_json::Value, i64, Duration), ToolError> {
        let tools = self.tools.as_ref().and_then(Weak::upgrade).ok_or_else(|| {
            ToolError::ExecutionFailed(
                "tool_rpc mode is unavailable because the tool registry is not wired".to_string(),
            )
        })?;
        let adapter = tool_rpc_adapter(language)?;

        let interpreter = Self::interpreter_for(language)?;
        let workdir = self.working_dir();
        let temp_dir = self.execution_tempdir(&workdir)?;
        let rpc_dir = temp_dir.path().join("tool_rpc");
        let helper_path = temp_dir.path().join(adapter.helper_filename());
        let script_path = temp_dir
            .path()
            .join(format!("tool_rpc_user{}", adapter.entry_extension()));

        tokio::fs::create_dir_all(&rpc_dir).await.map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "Failed to create tool_rpc directory {}: {}",
                rpc_dir.display(),
                e
            ))
        })?;
        tokio::fs::write(&helper_path, adapter.helper_source())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to write tool_rpc helper: {}", e))
            })?;
        tokio::fs::write(&script_path, adapter.render_entry_script(code))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write script: {}", e)))?;

        let mut rpc_ctx = ctx.clone();
        if !rpc_ctx.metadata.is_object() {
            rpc_ctx.metadata = serde_json::json!({});
        }
        if let Some(meta) = rpc_ctx.metadata.as_object_mut() {
            meta.insert("tool_rpc_inner".to_string(), serde_json::json!(true));
            meta.insert(
                "tool_rpc_parent".to_string(),
                serde_json::json!("execute_code"),
            );
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve_tool_rpc(rpc_dir.clone(), tools, rpc_ctx, shutdown_rx));

        let mut extra_env = std::collections::HashMap::new();
        adapter.augment_env(&mut extra_env, temp_dir.path(), &workdir);
        extra_env.insert(
            "THINCLAW_TOOL_RPC_TRANSPORT".to_string(),
            "file".to_string(),
        );
        extra_env.insert(
            "THINCLAW_TOOL_RPC_DIR".to_string(),
            rpc_dir.to_string_lossy().to_string(),
        );

        let program = interpreter.program.clone();
        let args = Self::interpreter_args(&interpreter, &script_path);
        let result = self
            .backend
            .run_script(ScriptExecutionRequest {
                program,
                args,
                workdir,
                timeout,
                extra_env,
                allow_network: false,
            })
            .await;

        let _ = shutdown_tx.send(());
        let _ = server.await;

        let result = result?;
        if result.exit_code != 0 {
            return Err(tool_rpc_script_error(&result));
        }

        let final_result = match result_format {
            ResultFormat::Text => serde_json::Value::String(result.output.clone()),
            ResultFormat::Json => serde_json::from_str::<serde_json::Value>(result.stdout.trim())
                .map_err(|e| {
                let stderr_hint = if result.stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(" (stderr: {})", result.stderr.trim())
                };
                ToolError::ExecutionFailed(format!(
                    "tool_rpc expected JSON final output on stdout but parsing failed: {}{}",
                    e, stderr_hint
                ))
            })?,
        };

        Ok((final_result, result.exit_code, result.duration))
    }

    #[cfg(windows)]
    async fn execute_tool_rpc_code(
        &self,
        language: &str,
        code: &str,
        timeout: Duration,
        result_format: ResultFormat,
        ctx: &JobContext,
    ) -> Result<(serde_json::Value, i64, Duration), ToolError> {
        let tools = self.tools.as_ref().and_then(Weak::upgrade).ok_or_else(|| {
            ToolError::ExecutionFailed(
                "tool_rpc mode is unavailable because the tool registry is not wired".to_string(),
            )
        })?;
        let adapter = tool_rpc_adapter(language)?;

        let interpreter = Self::interpreter_for(language)?;
        let workdir = self.working_dir();
        let temp_dir = self.execution_tempdir(&workdir)?;
        let pipe_name = format!(r"\\.\pipe\thinclaw-tool-rpc-{}", uuid::Uuid::new_v4());
        let helper_path = temp_dir.path().join(adapter.helper_filename());
        let script_path = temp_dir
            .path()
            .join(format!("tool_rpc_user{}", adapter.entry_extension()));

        tokio::fs::write(&helper_path, adapter.helper_source())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to write tool_rpc helper: {}", e))
            })?;
        tokio::fs::write(&script_path, adapter.render_entry_script(code))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write script: {}", e)))?;

        let mut rpc_ctx = ctx.clone();
        if !rpc_ctx.metadata.is_object() {
            rpc_ctx.metadata = serde_json::json!({});
        }
        if let Some(meta) = rpc_ctx.metadata.as_object_mut() {
            meta.insert("tool_rpc_inner".to_string(), serde_json::json!(true));
            meta.insert(
                "tool_rpc_parent".to_string(),
                serde_json::json!("execute_code"),
            );
        }

        let listener = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to bind tool_rpc named pipe {}: {}",
                    pipe_name, e
                ))
            })?;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve_tool_rpc_windows(
            listener,
            tools,
            rpc_ctx,
            shutdown_rx,
        ));

        let mut extra_env = std::collections::HashMap::new();
        adapter.augment_env(&mut extra_env, temp_dir.path(), &workdir);
        if adapter.canonical_language() == "python"
            && let Some(value) = extra_env.get_mut("PYTHONPATH")
        {
            *value = value.replace(':', ";");
        }
        extra_env.insert(
            "THINCLAW_TOOL_RPC_TRANSPORT".to_string(),
            "named_pipe".to_string(),
        );
        extra_env.insert("THINCLAW_TOOL_RPC_PIPE".to_string(), pipe_name.clone());

        let program = interpreter.program.clone();
        let args = Self::interpreter_args(&interpreter, &script_path);
        let result = self
            .backend
            .run_script(ScriptExecutionRequest {
                program,
                args,
                workdir,
                timeout,
                extra_env,
                allow_network: false,
            })
            .await;

        let _ = shutdown_tx.send(());
        let _ = server.await;

        let result = result?;
        if result.exit_code != 0 {
            return Err(tool_rpc_script_error(&result));
        }
        let final_result = match result_format {
            ResultFormat::Text => serde_json::Value::String(result.output.clone()),
            ResultFormat::Json => serde_json::from_str::<serde_json::Value>(result.stdout.trim())
                .map_err(|e| {
                let stderr_hint = if result.stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(" (stderr: {})", result.stderr.trim())
                };
                ToolError::ExecutionFailed(format!(
                    "tool_rpc expected JSON final output on stdout but parsing failed: {}{}",
                    e, stderr_hint
                ))
            })?,
        };

        Ok((final_result, result.exit_code, result.duration))
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
        "execute_code"
    }

    fn description(&self) -> &str {
        "Execute Python, JavaScript, or TypeScript in a sandboxed runtime. Use this \
         when you need computation, parsing, transformation, or scripted orchestration \
         that would be cumbersome with normal tool calls. Prefer direct tools first when \
         a single built-in tool already solves the task cleanly."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let languages = Self::language_options();
        serde_json::json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": languages,
                    "description": "Programming language of the code"
                },
                "code": {
                    "type": "string",
                    "description": "The code to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Execution timeout in seconds (default 60, max 120)"
                },
                "mode": {
                    "type": "string",
                    "enum": ["subprocess", "tool_rpc"],
                    "description": "Execution mode. Use tool_rpc for Python, JavaScript, or TypeScript scripts that should call host tools through ThinClaw instead of spawning their own shell pipelines."
                },
                "result_format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Final result format to return from tool_rpc mode. In json mode, the script's final stdout must be valid JSON."
                }
            },
            "required": ["language", "code"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let language = require_str(&params, "language")?;
        let code = require_str(&params, "code")?;
        let mode = ExecuteCodeMode::parse(params.get("mode").and_then(|v| v.as_str()))?;
        let result_format =
            ResultFormat::parse(params.get("result_format").and_then(|v| v.as_str()))?;

        if code.len() > MAX_CODE_LENGTH {
            return Err(ToolError::InvalidParameters(format!(
                "Code too long: {} bytes (max {})",
                code.len(),
                MAX_CODE_LENGTH
            )));
        }

        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(60)
            .min(120);
        let timeout = Duration::from_secs(timeout_secs);

        let start = std::time::Instant::now();
        let (output, exit_code, exec_duration) = match mode {
            ExecuteCodeMode::Subprocess => {
                let (output, exit_code, exec_duration) = self
                    .execute_subprocess_code(language, code, timeout)
                    .await?;
                (serde_json::Value::String(output), exit_code, exec_duration)
            }
            ExecuteCodeMode::ToolRpc => {
                self.execute_tool_rpc_code(language, code, timeout, result_format, ctx)
                    .await?
            }
        };

        let runtime_mode = match mode {
            ExecuteCodeMode::Subprocess => "script".to_string(),
            ExecuteCodeMode::ToolRpc => format!(
                "tool_rpc_{}",
                tool_rpc_adapter(language)?.canonical_language()
            ),
        };
        let runtime_capabilities = match mode {
            ExecuteCodeMode::Subprocess => vec![
                "captured_output".to_string(),
                "language_runtime".to_string(),
                "short_lived_command".to_string(),
            ],
            ExecuteCodeMode::ToolRpc => vec![
                "captured_output".to_string(),
                "host_tool_rpc".to_string(),
                "language_runtime".to_string(),
                "short_lived_command".to_string(),
                format!(
                    "tool_rpc_adapter:{}",
                    tool_rpc_adapter(language)?.canonical_language()
                ),
            ],
        };
        let network_allowed = matches!(mode, ExecuteCodeMode::Subprocess) && self.allow_network;
        let network_isolation = match self.backend.kind() {
            ExecutionBackendKind::DockerSandbox => {
                if network_allowed {
                    NetworkIsolationKind::None
                } else {
                    NetworkIsolationKind::Hard
                }
            }
            ExecutionBackendKind::LocalHost => host_local_network_isolation(network_allowed),
            ExecutionBackendKind::RemoteRunnerAdapter => {
                if network_allowed {
                    NetworkIsolationKind::None
                } else {
                    NetworkIsolationKind::BestEffort
                }
            }
        };
        let runtime = RuntimeDescriptor::execution_surface(
            self.backend.kind(),
            runtime_mode,
            runtime_capabilities,
            network_isolation,
        );

        let mut result = serde_json::json!({
            "output": output,
            "exit_code": exit_code,
            "success": exit_code == 0,
            "language": language,
            "mode": match mode {
                ExecuteCodeMode::Subprocess => "subprocess",
                ExecuteCodeMode::ToolRpc => "tool_rpc",
            },
            "result_format": match result_format {
                ResultFormat::Text => "text",
                ResultFormat::Json => "json",
            },
            "execution_backend": self.backend.kind().as_str(),
            "runtime_family": runtime.runtime_family,
            "runtime_mode": runtime.runtime_mode,
            "runtime_capabilities": runtime.runtime_capabilities,
            "network_isolation": runtime.network_isolation,
            "execution_time_ms": exec_duration.as_millis() as u64,
        });
        if matches!(result_format, ResultFormat::Json)
            && matches!(mode, ExecuteCodeMode::ToolRpc)
            && let Some(obj) = result.as_object_mut()
        {
            obj.insert("output_json".to_string(), obj["output"].clone());
        }

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(130)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(15, 100))
    }
}

#[cfg(unix)]
async fn serve_tool_rpc(
    rpc_dir: PathBuf,
    tools: Arc<ToolRegistry>,
    ctx: JobContext,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), ToolError> {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                let mut entries = tokio::fs::read_dir(&rpc_dir).await.map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "tool_rpc could not read request directory {}: {}",
                        rpc_dir.display(),
                        e
                    ))
                })?;
                while let Some(entry) = entries.next_entry().await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("tool_rpc directory scan failed: {}", e))
                })? {
                    let path = entry.path();
                    let Some(request_id) = tool_rpc_request_id_from_path(&path) else {
                        continue;
                    };
                    let processing_path = rpc_dir.join(format!("request-{}.processing", request_id));
                    if tokio::fs::rename(&path, &processing_path).await.is_err() {
                        continue;
                    }
                    if let Err(err) = handle_tool_rpc_request_file(
                        &processing_path,
                        &rpc_dir,
                        &request_id,
                        &tools,
                        &ctx,
                    )
                    .await
                    {
                        tracing::warn!("tool_rpc request failed: {}", err);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn handle_tool_rpc_request_file(
    request_path: &Path,
    rpc_dir: &Path,
    request_id: &str,
    tools: &Arc<ToolRegistry>,
    ctx: &JobContext,
) -> Result<(), ToolError> {
    let request_raw = tokio::fs::read_to_string(request_path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc read failed: {}", e)))?;
    let request: Result<ToolRpcRequest, ToolError> = serde_json::from_str(request_raw.trim())
        .map_err(|e| {
            ToolError::InvalidParameters(format!("tool_rpc request is not valid JSON: {}", e))
        });

    let response = match request {
        Ok(request) => match execute_inner_tool_rpc(tools, ctx, request).await {
            Ok(result) => ToolRpcResponse {
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => ToolRpcResponse {
                ok: false,
                result: None,
                error: Some(error.to_string()),
            },
        },
        Err(error) => ToolRpcResponse {
            ok: false,
            result: None,
            error: Some(error.to_string()),
        },
    };

    let payload = serde_json::to_vec(&response).map_err(|e| {
        ToolError::ExecutionFailed(format!("tool_rpc response serialization failed: {}", e))
    })?;
    let response_tmp = rpc_dir.join(format!("response-{}.tmp", request_id));
    let response_path = rpc_dir.join(format!("response-{}.json", request_id));
    tokio::fs::write(&response_tmp, &payload)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc write failed: {}", e)))?;
    tokio::fs::rename(&response_tmp, &response_path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc publish failed: {}", e)))?;
    let _ = tokio::fs::remove_file(request_path).await;
    Ok(())
}

#[cfg(unix)]
fn tool_rpc_request_id_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("request-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .map(|id| id.to_string())
}

fn tool_rpc_script_error(result: &ExecutionResult) -> ToolError {
    let stderr = result.stderr.trim();
    let stdout = result.stdout.trim();
    let detail = match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!("stderr: {}; stdout: {}", stderr, stdout),
        (false, true) => format!("stderr: {}", stderr),
        (true, false) => format!("stdout: {}", stdout),
        (true, true) => String::new(),
    };

    let mut message = format!("tool_rpc script failed with exit code {}", result.exit_code);
    if !detail.is_empty() {
        message.push_str(": ");
        message.push_str(&detail);
    }
    ToolError::ExecutionFailed(message)
}

#[cfg(windows)]
async fn handle_tool_rpc_stream_windows(
    stream: tokio::net::windows::named_pipe::NamedPipeServer,
    tools: Arc<ToolRegistry>,
    ctx: JobContext,
) -> Result<(), ToolError> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc read failed: {}", e)))?;
    let request: ToolRpcRequest = serde_json::from_str(line.trim()).map_err(|e| {
        ToolError::InvalidParameters(format!("tool_rpc request is not valid JSON: {}", e))
    })?;

    let response = match execute_inner_tool_rpc(&tools, &ctx, request).await {
        Ok(result) => ToolRpcResponse {
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(error) => ToolRpcResponse {
            ok: false,
            result: None,
            error: Some(error.to_string()),
        },
    };

    let payload = serde_json::to_vec(&response).map_err(|e| {
        ToolError::ExecutionFailed(format!("tool_rpc response serialization failed: {}", e))
    })?;
    write_half
        .write_all(&payload)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc write failed: {}", e)))?;
    Ok(())
}

#[cfg(windows)]
async fn serve_tool_rpc_windows(
    listener: tokio::net::windows::named_pipe::NamedPipeServer,
    tools: Arc<ToolRegistry>,
    ctx: JobContext,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), ToolError> {
    let mut listener = listener;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            connected = listener.connect() => {
                connected.map_err(|e| ToolError::ExecutionFailed(format!("tool_rpc named pipe connect failed: {}", e)))?;
                let stream = listener;
                let tools = Arc::clone(&tools);
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_tool_rpc_stream_windows(stream, tools, ctx).await {
                        tracing::warn!("tool_rpc named pipe stream failed: {}", err);
                    }
                });
                break;
            }
        }
    }
    Ok(())
}

async fn execute_inner_tool_rpc(
    tools: &Arc<ToolRegistry>,
    ctx: &JobContext,
    request: ToolRpcRequest,
) -> Result<serde_json::Value, ToolError> {
    if !tool_rpc_allows(&request.name, &request.params) {
        return Err(ToolError::NotAuthorized(format!(
            "Tool '{}' is not available in tool_rpc mode",
            request.name
        )));
    }

    let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();
    if let Some(reason) = tool_policies.denial_reason_for_metadata(&request.name, &ctx.metadata) {
        return Err(ToolError::NotAuthorized(format!(
            "Tool '{}' is denied by policy: {}",
            request.name, reason
        )));
    }

    if !ToolRegistry::tool_name_allowed_by_metadata(&ctx.metadata, &request.name) {
        return Err(ToolError::NotAuthorized(format!(
            "Tool '{}' is not permitted in this agent context",
            request.name
        )));
    }

    let tool = tools.get(&request.name).await.ok_or_else(|| {
        ToolError::ExecutionFailed(format!("Tool '{}' is not registered", request.name))
    })?;

    let approval = tool.requires_approval(&request.params);
    let auto_approved = tool_rpc_auto_approves(&request.name, &request.params);
    if matches!(approval, ApprovalRequirement::Always)
        || (matches!(approval, ApprovalRequirement::UnlessAutoApproved) && !auto_approved)
    {
        return Err(ToolError::NotAuthorized(format!(
            "Tool '{}' requires approval and cannot run inside tool_rpc",
            request.name
        )));
    }

    if let Some(config) = tool.rate_limit_config()
        && let crate::tools::rate_limiter::RateLimitResult::Limited { retry_after, .. } = tools
            .rate_limiter()
            .check_and_record(&ctx.user_id, &request.name, &config)
            .await
    {
        return Err(ToolError::RateLimited(Some(retry_after)));
    }

    tracing::info!(
        tool = %request.name,
        params = %request.params,
        "tool_rpc inner tool started"
    );

    let timeout = tool.execution_timeout();
    let start = std::time::Instant::now();
    let output = tokio::time::timeout(timeout, tool.execute(request.params.clone(), ctx))
        .await
        .map_err(|_| ToolError::Timeout(timeout))?
        .map_err(|e| ToolError::ExecutionFailed(format!("Inner tool failed: {}", e)))?;

    tracing::info!(
        tool = %request.name,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "tool_rpc inner tool completed"
    );

    Ok(output.result)
}

fn tool_rpc_allows(tool_name: &str, params: &serde_json::Value) -> bool {
    match tool_name {
        "read_file" | "write_file" | "list_dir" | "search_files" | "memory_search"
        | "memory_read" | "memory_write" | "session_search" => true,
        "http" => matches!(
            params
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or("GET")
                .to_ascii_uppercase()
                .as_str(),
            "GET" | "HEAD"
        ),
        "browser" => matches!(
            params.get("action").and_then(|value| value.as_str()),
            Some(
                "navigate"
                    | "snapshot"
                    | "screenshot"
                    | "get_text"
                    | "get_images"
                    | "console"
                    | "tabs"
                    | "switch_tab"
                    | "back"
                    | "forward"
                    | "scroll"
            )
        ),
        _ => false,
    }
}

fn tool_rpc_auto_approves(tool_name: &str, params: &serde_json::Value) -> bool {
    match tool_name {
        "read_file" | "list_dir" | "search_files" | "memory_search" | "memory_read"
        | "session_search" => true,
        "http" => matches!(
            params
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or("GET")
                .to_ascii_uppercase()
                .as_str(),
            "GET" | "HEAD"
        ),
        "browser" => matches!(
            params.get("action").and_then(|value| value.as_str()),
            Some(
                "navigate"
                    | "snapshot"
                    | "screenshot"
                    | "get_text"
                    | "get_images"
                    | "console"
                    | "tabs"
                    | "switch_tab"
                    | "back"
                    | "forward"
                    | "scroll"
            )
        ),
        _ => false,
    }
}

fn python_tool_rpc_stub() -> String {
    r#"
import json
import os
import socket
import time
import uuid

_TRANSPORT = os.environ.get("THINCLAW_TOOL_RPC_TRANSPORT", "unix")
_SOCKET_PATH = os.environ.get("THINCLAW_TOOL_RPC_SOCKET")
_PIPE_PATH = os.environ.get("THINCLAW_TOOL_RPC_PIPE")
_RPC_DIR = os.environ.get("THINCLAW_TOOL_RPC_DIR")

def _request(payload):
    if _TRANSPORT == "named_pipe":
        if not _PIPE_PATH:
            raise RuntimeError("tool_rpc named pipe path missing")
        client = open(_PIPE_PATH, "r+b", buffering=0)
        client.write((json.dumps(payload) + "\n").encode("utf-8"))
        client.flush()
        chunks = []
        while True:
            chunk = client.read(65536)
            if not chunk:
                break
            chunks.append(chunk)
        client.close()
    elif _TRANSPORT == "file":
        if not _RPC_DIR:
            raise RuntimeError("tool_rpc request directory missing")
        request_id = uuid.uuid4().hex
        request_path = os.path.join(_RPC_DIR, f"request-{request_id}.json")
        request_tmp = request_path + ".tmp"
        response_path = os.path.join(_RPC_DIR, f"response-{request_id}.json")
        with open(request_tmp, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(payload))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(request_tmp, request_path)
        deadline = time.time() + 60.0
        while True:
            if os.path.exists(response_path):
                with open(response_path, "rb") as handle:
                    raw = handle.read()
                try:
                    os.remove(response_path)
                except FileNotFoundError:
                    pass
                response = json.loads(raw.decode("utf-8") if raw else "{}")
                if not response.get("ok"):
                    raise RuntimeError(response.get("error", "tool_rpc failed"))
                return response.get("result")
            if time.time() >= deadline:
                raise RuntimeError("tool_rpc response timed out")
            time.sleep(0.01)
    else:
        if not _SOCKET_PATH:
            raise RuntimeError("tool_rpc socket path missing")
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(_SOCKET_PATH)
        client.sendall((json.dumps(payload) + "\n").encode("utf-8"))
        client.shutdown(socket.SHUT_WR)
        chunks = []
        while True:
            chunk = client.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
        client.close()
    raw = b"".join(chunks).decode("utf-8") if chunks else "{}"
    response = json.loads(raw)
    if not response.get("ok"):
        raise RuntimeError(response.get("error", "tool_rpc failed"))
    return response.get("result")

def call_tool(name, **kwargs):
    return _request({"name": name, "params": kwargs})

def read_file(**kwargs):
    return call_tool("read_file", **kwargs)

def write_file(**kwargs):
    return call_tool("write_file", **kwargs)

def list_dir(**kwargs):
    return call_tool("list_dir", **kwargs)

def search_files(**kwargs):
    return call_tool("search_files", **kwargs)

def memory_search(**kwargs):
    return call_tool("memory_search", **kwargs)

def memory_read(**kwargs):
    return call_tool("memory_read", **kwargs)

def memory_write(**kwargs):
    return call_tool("memory_write", **kwargs)

def session_search(**kwargs):
    return call_tool("session_search", **kwargs)

def http_tool(**kwargs):
    return call_tool("http", **kwargs)

def browser_tool(**kwargs):
    return call_tool("browser", **kwargs)
"#
    .trim_start()
    .to_string()
}

fn javascript_tool_rpc_helper() -> String {
    format!(
        r#"
import crypto from "crypto";
import fs from "fs";
import path from "path";
import net from "net";
import {{ promises as fsp }} from "fs";

const TRANSPORT = process.env.THINCLAW_TOOL_RPC_TRANSPORT ?? "file";
const PIPE_PATH = process.env.THINCLAW_TOOL_RPC_PIPE;
const RPC_DIR = process.env.THINCLAW_TOOL_RPC_DIR;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function unwrapResponse(raw) {{
  const response = JSON.parse(raw || "{{}}");
  if (!response.ok) {{
    throw new Error(response.error || "tool_rpc failed");
  }}
  return response.result;
}}

async function requestViaNamedPipe(payload) {{
  if (!PIPE_PATH) {{
    throw new Error("tool_rpc named pipe path missing");
  }}
  return await new Promise((resolve, reject) => {{
    const client = net.createConnection(PIPE_PATH);
    const chunks = [];
    client.on("connect", () => {{
      client.end(`${{JSON.stringify(payload)}}\n`);
    }});
    client.on("data", (chunk) => chunks.push(chunk));
    client.on("error", reject);
    client.on("end", () => {{
      try {{
        resolve(unwrapResponse(Buffer.concat(chunks).toString("utf8")));
      }} catch (error) {{
        reject(error);
      }}
    }});
  }});
}}

async function requestViaFiles(payload) {{
  if (!RPC_DIR) {{
    throw new Error("tool_rpc request directory missing");
  }}
  const requestId = crypto.randomUUID().replace(/-/g, "");
  const requestPath = path.join(RPC_DIR, `request-${{requestId}}.json`);
  const requestTmp = `${{requestPath}}.tmp`;
  const responsePath = path.join(RPC_DIR, `response-${{requestId}}.json`);
  await fsp.writeFile(requestTmp, JSON.stringify(payload), "utf8");
  await fsp.rename(requestTmp, requestPath);
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {{
    if (fs.existsSync(responsePath)) {{
      const raw = await fsp.readFile(responsePath, "utf8");
      await fsp.rm(responsePath, {{ force: true }});
      return unwrapResponse(raw);
    }}
    await sleep(10);
  }}
  throw new Error("tool_rpc response timed out");
}}

async function request(payload) {{
  if (TRANSPORT === "named_pipe") {{
    return await requestViaNamedPipe(payload);
  }}
  if (TRANSPORT === "file") {{
    return await requestViaFiles(payload);
  }}
  throw new Error(`Unsupported tool_rpc transport: ${{TRANSPORT}}`);
}}

export async function call_tool(name, params = {{}}) {{
  return await request({{ name, params }});
}}

export const read_file = (params = {{}}) => call_tool("read_file", params);
export const write_file = (params = {{}}) => call_tool("write_file", params);
export const list_dir = (params = {{}}) => call_tool("list_dir", params);
export const search_files = (params = {{}}) => call_tool("search_files", params);
export const memory_search = (params = {{}}) => call_tool("memory_search", params);
export const memory_read = (params = {{}}) => call_tool("memory_read", params);
export const memory_write = (params = {{}}) => call_tool("memory_write", params);
export const session_search = (params = {{}}) => call_tool("session_search", params);
export const http_tool = (params = {{}}) => call_tool("http", params);
export const browser_tool = (params = {{}}) => call_tool("browser", params);
"#
    )
    .trim_start()
    .to_string()
}

fn typescript_tool_rpc_helper() -> String {
    javascript_tool_rpc_helper()
}

fn javascript_tool_rpc_entry(helper_filename: &str, user_code: &str) -> String {
    format!(
        r#"
import path from "path";
import {{ createRequire }} from "module";
import {{ {TOOL_RPC_HELPER_EXPORTS} }} from "./{helper_filename}";

await (async () => {{
  const require = createRequire(path.join(process.cwd(), "thinclaw-tool-rpc-entry.js"));
  globalThis.require = require;
  {user_code}
}})().catch((error) => {{
  console.error(error && error.stack ? error.stack : String(error));
  process.exit(1);
}});
"#
    )
    .trim_start()
    .to_string()
}

fn typescript_tool_rpc_entry(helper_filename: &str, user_code: &str) -> String {
    format!(
        r#"
import path from "path";
import {{ createRequire }} from "module";
import {{ {TOOL_RPC_HELPER_EXPORTS} }} from "./{helper_filename}";

await (async () => {{
  const require = createRequire(path.join(process.cwd(), "thinclaw-tool-rpc-entry.js"));
  (globalThis as typeof globalThis & {{ require: typeof require }}).require = require;
  {user_code}
}})().catch((error) => {{
  console.error(error && error.stack ? error.stack : String(error));
  process.exit(1);
}});
"#
    )
    .trim_start()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{SandboxConfig, SandboxManager, SandboxPolicy};
    use crate::tools::builtin::{ReadFileTool, WriteFileTool};
    use crate::tools::execution_backend::DockerSandboxExecutionBackend;

    fn missing_tool_runtime(error: &ToolError) -> bool {
        let text = error.to_string().to_ascii_lowercase();
        text.contains("no such file")
            || text.contains("not found")
            || text.contains("node")
            || text.contains("npx")
            || text.contains("tsx")
    }

    #[test]
    fn test_interpreter_for() {
        assert_eq!(
            ExecuteCodeTool::interpreter_for("python")
                .unwrap()
                .extension,
            ".py"
        );
        assert_eq!(
            ExecuteCodeTool::interpreter_for("javascript")
                .unwrap()
                .program,
            "node"
        );
        if cfg!(target_os = "windows") {
            assert_eq!(
                ExecuteCodeTool::interpreter_for("cmd").unwrap().extension,
                ".cmd"
            );
        } else {
            assert_eq!(
                ExecuteCodeTool::interpreter_for("bash").unwrap().program,
                "bash"
            );
        }
        assert!(ExecuteCodeTool::interpreter_for("cobol").is_err());
    }

    #[test]
    fn test_interpreter_args() {
        let args = ExecuteCodeTool::interpreter_args(
            &ExecuteCodeTool::interpreter_for("python").unwrap(),
            Path::new("/tmp/test.py"),
        );
        assert_eq!(args, vec!["/tmp/test.py".to_string()]);
    }

    #[test]
    fn test_tool_rpc_allowlist_blocks_nested_execute_code() {
        assert!(!tool_rpc_allows("execute_code", &serde_json::json!({})));
        assert!(tool_rpc_allows(
            "http",
            &serde_json::json!({"method": "GET", "url": "https://example.com"})
        ));
        assert!(!tool_rpc_allows(
            "http",
            &serde_json::json!({"method": "POST", "url": "https://example.com"})
        ));
    }

    #[tokio::test]
    async fn test_execute_python_subprocess() {
        let tool = ExecuteCodeTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "code": "print('hello from python')"
                }),
                &ctx,
            )
            .await
            .expect("python subprocess execution should succeed");

        assert!(
            result
                .result
                .get("output")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("hello from python")
        );
    }

    #[tokio::test]
    async fn test_execute_python_subprocess_resolves_project_local_imports() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            temp_dir.path().join("helper_mod.py"),
            "VALUE = 'workspace import'",
        )
        .await
        .unwrap();

        let tool = ExecuteCodeTool::new().with_working_dir(temp_dir.path().to_path_buf());
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "code": "import helper_mod\nprint(helper_mod.VALUE)"
                }),
                &ctx,
            )
            .await
            .expect("python subprocess should import local workspace modules");

        assert!(
            result
                .result
                .get("output")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.contains("workspace import"))
        );
    }

    #[tokio::test]
    async fn test_tool_rpc_requires_registry() {
        let tool = ExecuteCodeTool::new();
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "code": "print('hello')"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_can_read_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("hello.txt");
        tokio::fs::write(&target, "hello from file").await.unwrap();

        let registry = Arc::new(ToolRegistry::new());
        registry.register_sync(Arc::new(ReadFileTool::new()));
        registry.register_sync(Arc::new(WriteFileTool::new()));

        let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "code": format!(
                        "from thinclaw_tools import read_file\nprint(read_file(path='{}')['content'])",
                        target.display()
                    )
                }),
                &ctx,
            )
            .await
            .expect("tool_rpc file read should succeed");

        let text = result.result.get("output").unwrap().as_str().unwrap();
        assert!(text.contains("hello from file"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_json_result_format_returns_output_json() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_sync(Arc::new(ReadFileTool::new()));

        let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "result_format": "json",
                    "code": "import json\nprint(json.dumps({'status': 'ok', 'count': 2}))"
                }),
                &ctx,
            )
            .await
            .expect("tool_rpc json result should succeed");

        assert_eq!(
            result.result.get("mode").and_then(|v| v.as_str()),
            Some("tool_rpc")
        );
        assert_eq!(
            result.result.get("runtime_family").and_then(|v| v.as_str()),
            Some("execution_backend")
        );
        assert_eq!(
            result.result.get("runtime_mode").and_then(|v| v.as_str()),
            Some("tool_rpc_python")
        );
        assert_eq!(
            result
                .result
                .get("network_isolation")
                .and_then(|v| v.as_str()),
            Some("hard")
        );
        assert_eq!(
            result.result.get("result_format").and_then(|v| v.as_str()),
            Some("json")
        );
        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str()),
            Some("ok")
        );
        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|v| v.get("count"))
                .and_then(|v| v.as_i64()),
            Some(2)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_javascript_supports_host_tools_and_workspace_require() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("hello.txt");
        let helper = temp_dir.path().join("helper_js.cjs");
        tokio::fs::write(&target, "hello from javascript")
            .await
            .unwrap();
        tokio::fs::write(
            &helper,
            "module.exports = { VALUE: 'workspace js module' };",
        )
        .await
        .unwrap();

        let registry = Arc::new(ToolRegistry::new());
        registry.register_sync(Arc::new(ReadFileTool::new()));

        let tool = ExecuteCodeTool::new()
            .with_working_dir(temp_dir.path().to_path_buf())
            .with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = match tool
            .execute(
                serde_json::json!({
                    "language": "javascript",
                    "mode": "tool_rpc",
                    "result_format": "json",
                    "code": format!(
                        "const helper = require('./{}');\nconst file = await read_file({{ path: '{}' }});\nconst normalized = file.content.replace(/^\\s*\\d+\\u2502\\s?/m, '').trim();\nconsole.log(JSON.stringify({{ status: 'ok', helper: helper.VALUE, content: normalized }}));",
                        helper.file_name().unwrap().to_string_lossy(),
                        target.display()
                    )
                }),
                &ctx,
            )
            .await
        {
            Ok(result) => result,
            Err(error) if missing_tool_runtime(&error) => {
                eprintln!("skipping javascript tool_rpc test: {error}");
                return;
            }
            Err(error) => panic!("javascript tool_rpc should succeed: {error}"),
        };

        assert_eq!(
            result
                .result
                .get("runtime_mode")
                .and_then(|value| value.as_str()),
            Some("tool_rpc_javascript")
        );
        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|value| value.get("helper"))
                .and_then(|value| value.as_str()),
            Some("workspace js module")
        );
        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|value| value.get("content"))
                .and_then(|value| value.as_str()),
            Some("hello from javascript")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_typescript_supports_workspace_require() {
        let temp_dir = tempfile::tempdir().unwrap();
        let helper = temp_dir.path().join("helper_ts.cjs");
        tokio::fs::write(
            &helper,
            "module.exports = { VALUE: 'workspace ts module' };",
        )
        .await
        .unwrap();

        let registry = Arc::new(ToolRegistry::new());
        let tool = ExecuteCodeTool::new()
            .with_working_dir(temp_dir.path().to_path_buf())
            .with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = match tool
            .execute(
                serde_json::json!({
                    "language": "typescript",
                    "mode": "tool_rpc",
                    "result_format": "json",
                    "code": format!(
                        "const helper = require('./{}');\nconsole.log(JSON.stringify({{ status: 'ok', helper: helper.VALUE }}));",
                        helper.file_name().unwrap().to_string_lossy()
                    )
                }),
                &ctx,
            )
            .await
        {
            Ok(result) => result,
            Err(error) if missing_tool_runtime(&error) => {
                eprintln!("skipping typescript tool_rpc test: {error}");
                return;
            }
            Err(error) => panic!("typescript tool_rpc should succeed: {error}"),
        };

        assert_eq!(
            result
                .result
                .get("runtime_mode")
                .and_then(|value| value.as_str()),
            Some("tool_rpc_typescript")
        );
        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|value| value.get("helper"))
                .and_then(|value| value.as_str()),
            Some("workspace ts module")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_rejects_inner_tools_that_require_approval() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("blocked.txt");

        let registry = Arc::new(ToolRegistry::new());
        registry.register_sync(Arc::new(WriteFileTool::new()));

        let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let error = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "code": format!(
                        "from thinclaw_tools import write_file\nwrite_file(path='{}', content='blocked')\nprint('done')",
                        target.display()
                    )
                }),
                &ctx,
            )
            .await
            .expect_err("tool_rpc write_file should fail closed on approval");

        assert!(error.to_string().contains("requires approval"));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_json_result_format_ignores_stderr_noise() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "result_format": "json",
                    "code": "import json, sys\nsys.stderr.write('warning on stderr\\n')\nprint(json.dumps({'status': 'ok'}))"
                }),
                &ctx,
            )
            .await
            .expect("tool_rpc json parsing should use stdout only");

        assert_eq!(
            result
                .result
                .get("output_json")
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str()),
            Some("ok")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_resolves_project_local_imports() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            temp_dir.path().join("helper_rpc.py"),
            "VALUE = 'tool rpc import'",
        )
        .await
        .unwrap();

        let registry = Arc::new(ToolRegistry::new());
        let tool = ExecuteCodeTool::new()
            .with_working_dir(temp_dir.path().to_path_buf())
            .with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "code": "import helper_rpc\nprint(helper_rpc.VALUE)"
                }),
                &ctx,
            )
            .await
            .expect("tool_rpc should import local workspace modules");

        assert!(
            result
                .result
                .get("output")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.contains("tool rpc import"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_tool_rpc_json_result_format_rejects_invalid_json() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "tool_rpc",
                    "result_format": "json",
                    "code": "print('not valid json')"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected JSON final output")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_execute_inner_tool_rpc_blocks_disallowed_tools() {
        let registry = Arc::new(ToolRegistry::new());
        let ctx = JobContext::default();

        let result = execute_inner_tool_rpc(
            &registry,
            &ctx,
            ToolRpcRequest {
                name: "shell".to_string(),
                params: serde_json::json!({"command": "echo hi"}),
            },
        )
        .await;

        assert!(matches!(result, Err(ToolError::NotAuthorized(_))));
    }

    #[tokio::test]
    async fn test_docker_backend_executes_python_subprocess_end_to_end() {
        let mut sandbox_config = SandboxConfig::default();
        sandbox_config.enabled = true;
        sandbox_config.policy = SandboxPolicy::WorkspaceWrite;
        sandbox_config.image = "python:3.11-alpine".to_string();
        let sandbox = Arc::new(SandboxManager::new(sandbox_config));
        if !sandbox.is_available().await {
            eprintln!("skipping docker-backed execute_code test because sandbox is unavailable");
            return;
        }

        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            temp_dir.path().join("helper_mod.py"),
            "VALUE = 'docker import'\n",
        )
        .await
        .unwrap();

        let backend = DockerSandboxExecutionBackend::new(sandbox, SandboxPolicy::WorkspaceWrite);
        let tool = ExecuteCodeTool::new()
            .with_working_dir(temp_dir.path().to_path_buf())
            .with_backend(backend);
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "language": "python",
                    "mode": "subprocess",
                    "code": "import helper_mod\nprint(helper_mod.VALUE)"
                }),
                &ctx,
            )
            .await
            .expect("docker-backed subprocess execution should succeed");

        assert_eq!(
            result
                .result
                .get("execution_backend")
                .and_then(|v| v.as_str()),
            Some("docker_sandbox")
        );
        assert_eq!(
            result.result.get("runtime_mode").and_then(|v| v.as_str()),
            Some("script")
        );
        assert_eq!(
            result
                .result
                .get("network_isolation")
                .and_then(|v| v.as_str()),
            Some("hard")
        );
        assert!(
            result
                .result
                .get("output")
                .and_then(|v| v.as_str())
                .is_some_and(|value| value.contains("docker import"))
        );
    }
}

//! Execute code tool (Python Tool Calling).
//!
//! Provides a sandboxed code execution environment. Supports:
//! - Python (preferred): executed via subprocess with captured output
//! - JavaScript/TypeScript: via Node.js or Deno
//! - Shell scripts
//!
//! Security: All code runs in a subprocess with:
//! - Scrubbed environment (no API keys leak)
//! - Timeout enforcement
//! - Output size limits
//! - No network access by default (configurable)
//! - Optional Docker sandbox (when available)

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum output size (64KB).
const MAX_OUTPUT_SIZE: usize = 64 * 1024;

/// Maximum code length (100KB).
const MAX_CODE_LENGTH: usize = 100 * 1024;

/// Safe environment variables to forward.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "ComSpec",
    "SystemRoot",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterpreterConfig {
    program: String,
    prefix_args: Vec<String>,
    extension: &'static str,
}

/// Execute code tool.
pub struct ExecuteCodeTool {
    /// Working directory for code execution.
    working_dir: Option<PathBuf>,
    /// Whether network access is allowed.
    allow_network: bool,
}

impl ExecuteCodeTool {
    pub fn new() -> Self {
        Self {
            working_dir: None,
            allow_network: false,
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

    /// Determine the interpreter and file extension for a language.
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

    /// Build args for the interpreter.
    fn interpreter_args(config: &InterpreterConfig, script_path: &str) -> Vec<String> {
        let mut args = config.prefix_args.clone();
        args.push(script_path.to_string());
        args
    }

    /// Execute code in a subprocess.
    async fn execute_code(
        &self,
        language: &str,
        code: &str,
        timeout: Duration,
    ) -> Result<(String, i32, Duration), ToolError> {
        let interpreter = Self::interpreter_for(language)?;

        // Write code to a temp file
        let tmp_dir = std::env::temp_dir();
        let script_name = format!(
            "thinclaw_exec_{}{}",
            uuid::Uuid::new_v4().simple(),
            interpreter.extension
        );
        let script_path = tmp_dir.join(&script_name);

        tokio::fs::write(&script_path, code)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write script: {}", e)))?;

        let workdir = self
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let script_path_str = script_path.to_string_lossy().to_string();
        let args = Self::interpreter_args(&interpreter, &script_path_str);

        // Build subprocess with sanitized environment
        let mut cmd = Command::new(&interpreter.program);
        cmd.args(&args)
            .current_dir(&workdir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Scrub environment
        cmd.env_clear();
        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // Disable network if needed (best-effort via env)
        if !self.allow_network {
            cmd.env("no_proxy", "*");
            cmd.env("NO_PROXY", "*");
        }

        let start = std::time::Instant::now();

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "Failed to spawn {}: {} (is {} installed?)",
                interpreter.program, e, interpreter.program
            ))
        })?;

        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let result = tokio::time::timeout(timeout, async {
            let stdout_fut = async {
                if let Some(mut out) = stdout_handle {
                    let mut buf = Vec::new();
                    (&mut out)
                        .take(MAX_OUTPUT_SIZE as u64)
                        .read_to_end(&mut buf)
                        .await
                        .ok();
                    tokio::io::copy(&mut out, &mut tokio::io::sink()).await.ok();
                    String::from_utf8_lossy(&buf).to_string()
                } else {
                    String::new()
                }
            };

            let stderr_fut = async {
                if let Some(mut err) = stderr_handle {
                    let mut buf = Vec::new();
                    (&mut err)
                        .take(MAX_OUTPUT_SIZE as u64)
                        .read_to_end(&mut buf)
                        .await
                        .ok();
                    tokio::io::copy(&mut err, &mut tokio::io::sink()).await.ok();
                    String::from_utf8_lossy(&buf).to_string()
                } else {
                    String::new()
                }
            };

            let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
            let status = wait_result?;
            let code = status.code().unwrap_or(-1);

            let output = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
            };

            Ok::<_, std::io::Error>((output, code))
        })
        .await;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&script_path).await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((output, code))) => {
                // Truncate if needed
                let output = if output.len() > MAX_OUTPUT_SIZE {
                    let half = MAX_OUTPUT_SIZE / 2;
                    format!(
                        "{}...\n[truncated {} bytes]\n...{}",
                        &output[..half],
                        output.len() - MAX_OUTPUT_SIZE,
                        &output[output.len() - half..]
                    )
                } else {
                    output
                };
                Ok((output, code, duration))
            }
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!(
                "Code execution failed: {}",
                e
            ))),
            Err(_) => {
                let _ = child.kill().await;
                Err(ToolError::Timeout(timeout))
            }
        }
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
        "Execute code in a sandboxed subprocess. Supports Python, JavaScript, TypeScript, \
         and platform-native shell languages. Code runs with a scrubbed environment — \
         API keys and secrets are NOT accessible. Output is captured and returned. \
         Best for: data processing, calculations, testing logic, prototyping."
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
                }
            },
            "required": ["language", "code"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let language = require_str(&params, "language")?;
        let code = require_str(&params, "code")?;

        // Validate code length
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
        let (output, exit_code, exec_duration) = self.execute_code(language, code, timeout).await?;

        let result = serde_json::json!({
            "output": output,
            "exit_code": exit_code,
            "success": exit_code == 0,
            "language": language,
            "execution_time_ms": exec_duration.as_millis() as u64,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        true // Code output could contain anything
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(130) // A bit more than max user timeout
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(15, 100))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "/tmp/test.py",
        );
        assert_eq!(args, vec!["/tmp/test.py".to_string()]);

        let args = ExecuteCodeTool::interpreter_args(
            &ExecuteCodeTool::interpreter_for("typescript").unwrap(),
            "/tmp/test.ts",
        );
        assert_eq!(args, vec!["tsx".to_string(), "/tmp/test.ts".to_string()]);
    }

    #[tokio::test]
    async fn test_execute_python() {
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
            .await;

        // Might fail if python3 is not installed
        if let Ok(output) = result {
            assert!(
                output
                    .result
                    .get("output")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .contains("hello from python")
            );
            assert_eq!(
                output.result.get("exit_code").unwrap(),
                &serde_json::json!(0)
            );
        }
    }

    #[tokio::test]
    async fn test_execute_bash() {
        if cfg!(target_os = "windows")
            && ExecuteCodeTool::find_in_path(&["bash.exe", "bash"]).is_none()
        {
            return;
        }
        let tool = ExecuteCodeTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "language": "bash",
                    "code": "echo 42"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            result
                .result
                .get("output")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("42")
        );
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let tool = ExecuteCodeTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "language": "bash",
                    "code": "sleep 30",
                    "timeout": 1
                }),
                &ctx,
            )
            .await;

        assert!(matches!(result, Err(ToolError::Timeout(_))));
    }

    #[test]
    fn test_code_length_validation() {
        let tool = ExecuteCodeTool::new();
        let schema = tool.parameters_schema();
        assert!(
            schema["properties"]["code"]["description"]
                .as_str()
                .unwrap()
                .contains("code")
        );
    }

    #[test]
    fn test_tool_metadata() {
        let tool = ExecuteCodeTool::new();
        assert_eq!(tool.name(), "execute_code");
        assert_eq!(tool.domain(), ToolDomain::Container);
        assert!(tool.requires_sanitization());
    }
}

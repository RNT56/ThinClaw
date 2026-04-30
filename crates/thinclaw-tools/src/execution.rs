//! Root-independent execution backend DTOs and local process execution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thinclaw_tools_core::ToolError;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

pub const MAX_CAPTURED_OUTPUT_SIZE: usize = 64 * 1024;

const SAFE_EXEC_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "EDITOR",
    "VISUAL",
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionBackendKind {
    LocalHost,
    DockerSandbox,
    RemoteRunnerAdapter,
}

impl ExecutionBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalHost => "local_host",
            Self::DockerSandbox => "docker_sandbox",
            Self::RemoteRunnerAdapter => "remote_runner_adapter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkIsolationKind {
    None,
    Hard,
    BestEffort,
}

impl NetworkIsolationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Hard => "hard",
            Self::BestEffort => "best_effort",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    pub execution_backend: String,
    pub runtime_family: String,
    pub runtime_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
}

impl RuntimeDescriptor {
    pub fn logical_surface(
        execution_backend: impl Into<String>,
        runtime_family: impl Into<String>,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: Option<impl Into<String>>,
    ) -> Self {
        Self {
            execution_backend: execution_backend.into(),
            runtime_family: runtime_family.into(),
            runtime_mode: runtime_mode.into(),
            runtime_capabilities,
            network_isolation: network_isolation.map(Into::into),
        }
    }

    pub fn execution_surface(
        backend: ExecutionBackendKind,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: NetworkIsolationKind,
    ) -> Self {
        Self::logical_surface(
            backend.as_str(),
            "execution_backend",
            runtime_mode,
            runtime_capabilities,
            Some(network_isolation.as_str()),
        )
    }
}

pub fn interactive_chat_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "interactive_chat",
        "agent_surface",
        "interactive_chat",
        vec![
            "conversation_state".to_string(),
            "llm_turn".to_string(),
            "thread_history".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn routine_engine_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "routine_engine",
        "agent_surface",
        "routine_engine",
        vec![
            "routine_orchestration".to_string(),
            "scheduled_execution".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn subagent_executor_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "subagent_executor",
        "agent_surface",
        "subagent_executor",
        vec![
            "delegated_execution".to_string(),
            "llm_turn".to_string(),
            "task_isolation".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn experiment_runner_runtime_descriptor(backend_slug: &str) -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        backend_slug.to_string(),
        "experiment_runner",
        format!("experiment_runner:{backend_slug}"),
        vec![
            "artifact_capture".to_string(),
            "benchmark_execution".to_string(),
            "remote_trial".to_string(),
        ],
        Some(NetworkIsolationKind::BestEffort.as_str()),
    )
}

pub fn local_job_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::execution_surface(
        ExecutionBackendKind::LocalHost,
        "in_memory",
        vec![
            "job_orchestration".to_string(),
            "queue_tracking".to_string(),
        ],
        NetworkIsolationKind::None,
    )
}

#[derive(Debug, Clone)]
pub struct CommandExecutionRequest {
    pub command: String,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ScriptExecutionRequest {
    pub program: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessStartRequest {
    pub command: String,
    pub workdir: Option<PathBuf>,
    pub extra_env: HashMap<String, String>,
    pub kill_on_drop: bool,
}

#[derive(Debug)]
pub struct StartedProcess {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub exit_code: i64,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
    pub duration: Duration,
}

#[async_trait]
pub trait LocalExecutionBackend: Send + Sync {
    fn kind(&self) -> ExecutionBackendKind;

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError>;

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError>;

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError>;
}

#[derive(Default)]
pub struct LocalHostExecutionBackend;

impl LocalHostExecutionBackend {
    pub fn shared() -> Arc<dyn LocalExecutionBackend> {
        Arc::new(Self)
    }
}

#[async_trait]
impl LocalExecutionBackend for LocalHostExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::LocalHost
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command = build_shell_command(&request.command, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn command: {}", e)))?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "shell",
                vec![
                    "captured_output".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        let mut command = shell_command(&request.command);
        if let Some(workdir) = request.workdir.as_ref() {
            command.current_dir(workdir);
        }
        configure_stdio_command(
            &mut command,
            request.workdir.as_deref(),
            &request.extra_env,
            true,
            request.kill_on_drop,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn process: {}", e)))?;

        Ok(StartedProcess {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "process",
                vec![
                    "captured_output".to_string(),
                    "incremental_output".to_string(),
                    "interactive_stdin".to_string(),
                    "long_running_process".to_string(),
                ],
                NetworkIsolationKind::None,
            ),
        })
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command =
            build_script_command(&request.program, &request.args, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to spawn {}: {}", request.program, e))
        })?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "script",
                vec![
                    "captured_output".to_string(),
                    "language_runtime".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }
}

#[derive(Debug)]
struct CollectedOutput {
    stdout: String,
    stderr: String,
    output: String,
    exit_code: i64,
}

fn build_shell_command(command: &str, allow_network: bool) -> Command {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        add_shell_args(&mut sandboxed, command);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--");
        add_shell_args(&mut sandboxed, command);
        return sandboxed;
    }

    shell_command(command)
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = Command::new(shell);
        cmd.arg("-lc").arg(command);
        cmd
    }
}

fn add_shell_args(command: &mut Command, shell_command: &str) {
    #[cfg(windows)]
    {
        command.arg("cmd.exe").arg("/C").arg(shell_command);
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        command.arg(shell).arg("-lc").arg(shell_command);
    }
}

fn build_script_command(program: &str, args: &[String], allow_network: bool) -> Command {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        sandboxed.arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--")
            .arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    let mut command = Command::new(program);
    command.args(args);
    command
}

#[cfg(target_os = "macos")]
fn macos_network_deny_profile() -> &'static str {
    "(version 1) (allow default) (deny network*)"
}

fn configure_spawn(
    command: &mut Command,
    workdir: &Path,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.current_dir(workdir);
    configure_env(command, Some(workdir), extra_env, allow_network);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
}

fn configure_stdio_command(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
    kill_on_drop: bool,
) {
    configure_env(command, workdir, extra_env, allow_network);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(kill_on_drop);
}

fn configure_env(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.env_clear();
    for var in SAFE_EXEC_ENV_VARS {
        if let Ok(value) = std::env::var(var) {
            command.env(var, value);
        }
    }
    command.envs(extra_env);
    if let Some(dir) = workdir {
        command.env("PWD", dir);
    } else if let Ok(dir) = std::env::current_dir() {
        command.env("PWD", dir);
    }
    if !allow_network {
        command.env("no_proxy", "*");
        command.env("NO_PROXY", "*");
    }
}

pub fn host_local_network_deny_support() -> NetworkIsolationKind {
    #[cfg(target_os = "macos")]
    if Path::new("/usr/bin/sandbox-exec").is_file() {
        return NetworkIsolationKind::Hard;
    }

    #[cfg(target_os = "linux")]
    if linux_bubblewrap_program().is_some() {
        return NetworkIsolationKind::Hard;
    }

    NetworkIsolationKind::BestEffort
}

pub fn host_local_network_isolation(allow_network: bool) -> NetworkIsolationKind {
    if allow_network {
        NetworkIsolationKind::None
    } else {
        host_local_network_deny_support()
    }
}

#[cfg(target_os = "linux")]
fn linux_bubblewrap_program() -> Option<&'static str> {
    if Path::new("/usr/bin/bwrap").is_file() {
        Some("/usr/bin/bwrap")
    } else if executable_in_path("bwrap") {
        Some("bwrap")
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn executable_in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
}

async fn collect_child_output(
    child: &mut Child,
    timeout: Duration,
) -> Result<CollectedOutput, ToolError> {
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(mut out) = stdout_handle {
                let mut buf = Vec::new();
                (&mut out)
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
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
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
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
        let code = status.code().unwrap_or(-1) as i64;
        let output = if stderr.is_empty() {
            stdout.clone()
        } else if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };

        Ok::<_, std::io::Error>(CollectedOutput {
            stdout: truncate_output(&stdout),
            stderr: truncate_output(&stderr),
            output: truncate_output(&output),
            exit_code: code,
        })
    })
    .await;

    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(ToolError::ExecutionFailed(format!(
            "Command execution failed: {}",
            err
        ))),
        Err(_) => {
            let _ = child.kill().await;
            Err(ToolError::Timeout(timeout))
        }
    }
}

pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_CAPTURED_OUTPUT_SIZE {
        output.to_string()
    } else {
        let half = MAX_CAPTURED_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(output, half);
        let tail_start = floor_char_boundary(output, output.len().saturating_sub(half));
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &output[..head_end],
            output.len() - MAX_CAPTURED_OUTPUT_SIZE,
            &output[tail_start..]
        )
    }
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    let mut index = index;
    while !value.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

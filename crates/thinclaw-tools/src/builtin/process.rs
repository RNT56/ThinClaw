//! Background process management tool.
//!
//! Allows the agent to start, monitor, and control long-running background
//! processes. Each process gets a short human-friendly ID and its output is
//! captured in a ring buffer for incremental reading.
//!
//! Actions: start, list, poll, wait, kill, write (stdin).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::shell_security::{
    check_safe_bins, classify_hard_block, detect_command_injection, detect_library_injection,
    requires_explicit_approval,
};
use crate::execution::{LocalExecutionBackend, LocalHostExecutionBackend, ProcessStartRequest};
use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};
use thinclaw_types::JobContext;

/// Maximum output buffer size per process (64KB).
const MAX_BUFFER_SIZE: usize = 64 * 1024;

/// Maximum concurrent background processes.
const MAX_PROCESSES: usize = 20;

/// Auto-reaper check interval.
const REAPER_INTERVAL: Duration = Duration::from_secs(5);

/// Status of a tracked process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Completed(i32),
    Failed(String),
    Killed,
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessStatus::Running => write!(f, "running"),
            ProcessStatus::Completed(code) => write!(f, "completed (exit {})", code),
            ProcessStatus::Failed(msg) => write!(f, "failed: {}", msg),
            ProcessStatus::Killed => write!(f, "killed"),
        }
    }
}

/// Thread-safe ring buffer for process output.
#[derive(Debug)]
pub struct OutputBuffer {
    /// Circular buffer storage.
    data: Vec<u8>,
    /// Total bytes written (monotonically increasing, even past capacity).
    total_written: usize,
    /// Maximum capacity.
    capacity: usize,
}

impl OutputBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity.min(4096)),
            total_written: 0,
            capacity,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.data.len() < self.capacity {
                self.data.push(b);
            } else {
                let idx = self.total_written % self.capacity;
                self.data[idx] = b;
            }
            self.total_written += 1;
        }
    }

    /// Read all available content as a string.
    fn read_all(&self) -> String {
        String::from_utf8_lossy(&self.read_all_bytes()).to_string()
    }

    fn read_all_bytes(&self) -> Vec<u8> {
        if self.total_written <= self.capacity {
            self.data.clone()
        } else {
            // Ring buffer wrapped — reconstruct in order
            let start = self.total_written % self.capacity;
            let mut result = Vec::with_capacity(self.capacity);
            result.extend_from_slice(&self.data[start..]);
            result.extend_from_slice(&self.data[..start]);
            result
        }
    }

    /// Read content from byte offset, returning (new_content, new_offset).
    fn read_from(&self, offset: usize) -> (String, usize) {
        if offset >= self.total_written {
            return (String::new(), self.total_written);
        }

        let available_start = self.total_written.saturating_sub(self.capacity);

        let effective_offset = offset.max(available_start);
        let content = self.read_all_bytes();
        let skip = effective_offset.saturating_sub(available_start);
        let content = if skip < content.len() {
            String::from_utf8_lossy(&content[skip..]).to_string()
        } else {
            String::new()
        };

        (content, self.total_written)
    }

    fn total_bytes(&self) -> usize {
        self.total_written
    }
}

/// A tracked background process.
pub struct ProcessEntry {
    /// Short human-friendly ID.
    pub id: String,
    /// The command that was run.
    pub command: String,
    /// Current status.
    pub status: ProcessStatus,
    /// When the process was started.
    pub started_at: Instant,
    /// Execution backend label.
    pub backend: String,
    /// Runtime family label.
    pub runtime_family: String,
    /// Runtime mode label.
    pub runtime_mode: String,
    /// Runtime capability hints.
    pub runtime_capabilities: Vec<String>,
    /// Effective network isolation mode.
    pub network_isolation: Option<String>,
    /// Output buffer (stdout + stderr interleaved).
    pub output: OutputBuffer,
    /// Handle to the child process (for kill/wait).
    pub child: Option<tokio::process::Child>,
    /// Stdin writer (for write action).
    pub stdin: Option<tokio::process::ChildStdin>,
    /// Reader completion flags so wait() can return only after streams drain.
    pub stdout_done: Arc<AtomicBool>,
    pub stderr_done: Arc<AtomicBool>,
}

/// Shared process registry.
pub type SharedProcessRegistry = Arc<RwLock<ProcessRegistry>>;

/// Registry that tracks all background processes.
#[derive(Default)]
pub struct ProcessRegistry {
    processes: HashMap<String, ProcessEntry>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a short process ID.
    fn next_id(&self) -> String {
        let uuid = Uuid::new_v4();
        let hex = format!("{:x}", uuid);
        format!("proc_{}", &hex[..6])
    }

    /// Get a list of all process summaries.
    fn list_summaries(&self) -> Vec<serde_json::Value> {
        self.processes
            .values()
            .map(|p| {
                let runtime = p.started_at.elapsed();
                serde_json::json!({
                    "id": p.id,
                    "command": truncate_str(&p.command, 80),
                    "backend": p.backend,
                    "status": p.status.to_string(),
                    "runtime_secs": runtime.as_secs(),
                    "output_bytes": p.output.total_bytes(),
                })
            })
            .collect()
    }
}

/// Start the auto-reaper background task that polls for completed processes.
pub fn start_reaper(registry: SharedProcessRegistry) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REAPER_INTERVAL).await;
            let mut reg = registry.write().await;
            for entry in reg.processes.values_mut() {
                if entry.status != ProcessStatus::Running {
                    continue;
                }
                if let Some(ref mut child) = entry.child {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            entry.status = ProcessStatus::Completed(status.code().unwrap_or(-1));
                        }
                        Ok(None) => {} // still running
                        Err(e) => {
                            entry.status = ProcessStatus::Failed(e.to_string());
                        }
                    }
                }
            }
        }
    });
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = floor_char_boundary(s, max.saturating_sub(1));
        format!("{}…", &s[..boundary])
    }
}

fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    pos = pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Background process management tool.
pub struct ProcessTool {
    registry: SharedProcessRegistry,
    backend: Arc<dyn LocalExecutionBackend>,
}

impl ProcessTool {
    pub fn new(registry: SharedProcessRegistry) -> Self {
        Self {
            registry,
            backend: LocalHostExecutionBackend::shared(),
        }
    }

    pub fn with_backend(mut self, backend: Arc<dyn LocalExecutionBackend>) -> Self {
        self.backend = backend;
        self
    }
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage long-running background processes. Use this when a command needs to \
         keep running across turns or when you need to poll output, wait, send stdin, \
         or terminate an already-started process."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start", "list", "poll", "wait", "kill", "write"],
                    "description": "The action to perform"
                },
                "command": {
                    "type": "string",
                    "description": "Command to run (for 'start' action)"
                },
                "process_id": {
                    "type": "string",
                    "description": "Process ID (for poll/wait/kill/write actions)"
                },
                "input": {
                    "type": "string",
                    "description": "Text to send to stdin (for 'write' action)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (for 'wait' action, default 30)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset for incremental reads (for 'poll' action, default 0)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let action = require_str(&params, "action")?;

        match action {
            "start" => {
                let command = require_str(&params, "command")?;
                if let Some(reason) = classify_hard_block(command) {
                    return Err(ToolError::NotAuthorized(format!("{}: {}", reason, command)));
                }
                if let Some(reason) = detect_command_injection(command) {
                    return Err(ToolError::NotAuthorized(format!(
                        "Command injection detected ({}): {}",
                        reason, command
                    )));
                }
                if let Some(reason) = detect_library_injection(command) {
                    return Err(ToolError::NotAuthorized(format!(
                        "Security violation ({}): {}",
                        reason, command
                    )));
                }
                if let Some(reason) = check_safe_bins(command) {
                    return Err(ToolError::NotAuthorized(format!(
                        "Blocked by safe bins policy ({}): {}",
                        reason, command
                    )));
                }

                // Check process limit
                {
                    let reg = self.registry.read().await;
                    let running = reg
                        .processes
                        .values()
                        .filter(|p| p.status == ProcessStatus::Running)
                        .count();
                    if running >= MAX_PROCESSES {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Too many running processes ({}/{}). Kill some first.",
                            running, MAX_PROCESSES
                        )));
                    }
                }

                // Spawn the process
                let mut started = self
                    .backend
                    .start_process(ProcessStartRequest {
                        command: command.to_string(),
                        workdir: None,
                        extra_env: (*ctx.extra_env).clone(),
                        kill_on_drop: true,
                    })
                    .await?;

                let stdin = started.stdin.take();
                let stdout = started.stdout.take();
                let stderr = started.stderr.take();
                let runtime_family = started.runtime.runtime_family.clone();
                let runtime_mode = started.runtime.runtime_mode.clone();
                let runtime_capabilities = started.runtime.runtime_capabilities.clone();
                let network_isolation = started.runtime.network_isolation.clone();

                let id = {
                    let reg = self.registry.read().await;
                    reg.next_id()
                };
                let stdout_done = Arc::new(AtomicBool::new(stdout.is_none()));
                let stderr_done = Arc::new(AtomicBool::new(stderr.is_none()));

                let entry = ProcessEntry {
                    id: id.clone(),
                    command: command.to_string(),
                    status: ProcessStatus::Running,
                    started_at: Instant::now(),
                    backend: started.backend.as_str().to_string(),
                    runtime_family: runtime_family.clone(),
                    runtime_mode: runtime_mode.clone(),
                    runtime_capabilities: runtime_capabilities.clone(),
                    network_isolation: network_isolation.clone(),
                    output: OutputBuffer::new(MAX_BUFFER_SIZE),
                    child: Some(started.child),
                    stdin,
                    stdout_done: Arc::clone(&stdout_done),
                    stderr_done: Arc::clone(&stderr_done),
                };

                {
                    let mut reg = self.registry.write().await;
                    reg.processes.insert(id.clone(), entry);
                }

                // Set up output capture tasks
                let registry_clone = Arc::clone(&self.registry);
                let id_clone = id.clone();

                // Spawn stdout reader
                if let Some(stdout) = stdout {
                    let reg = Arc::clone(&registry_clone);
                    let pid = id_clone.clone();
                    let done = Arc::clone(&stdout_done);
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt;
                        let mut reader = stdout;
                        let mut buf = [0u8; 4096];
                        loop {
                            match reader.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    let mut r = reg.write().await;
                                    if let Some(entry) = r.processes.get_mut(&pid) {
                                        entry.output.append(&buf[..n]);
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        done.store(true, Ordering::SeqCst);
                    });
                }

                // Spawn stderr reader
                if let Some(stderr) = stderr {
                    let reg = Arc::clone(&registry_clone);
                    let pid = id_clone.clone();
                    let done = Arc::clone(&stderr_done);
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt;
                        let mut reader = stderr;
                        let mut buf = [0u8; 4096];
                        loop {
                            match reader.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    let mut r = reg.write().await;
                                    if let Some(entry) = r.processes.get_mut(&pid) {
                                        entry.output.append(&buf[..n]);
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        done.store(true, Ordering::SeqCst);
                    });
                }

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "process_id": id,
                        "command": truncate_str(command, 80),
                        "backend": self.backend.kind().as_str(),
                        "runtime_family": runtime_family,
                        "runtime_mode": runtime_mode,
                        "runtime_capabilities": runtime_capabilities,
                        "network_isolation": network_isolation,
                        "status": "running"
                    }),
                    start.elapsed(),
                ))
            }

            "list" => {
                let reg = self.registry.read().await;
                let summaries = reg.list_summaries();
                Ok(ToolOutput::success(
                    serde_json::json!({
                        "processes": summaries,
                        "total": summaries.len(),
                    }),
                    start.elapsed(),
                ))
            }

            "poll" => {
                let process_id = require_str(&params, "process_id")?;
                let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                let reg = self.registry.read().await;
                let entry = reg.processes.get(process_id).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("Unknown process: {}", process_id))
                })?;

                let (content, new_offset) = entry.output.read_from(offset);

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "process_id": process_id,
                        "backend": entry.backend.clone(),
                        "runtime_family": entry.runtime_family.clone(),
                        "runtime_mode": entry.runtime_mode.clone(),
                        "runtime_capabilities": entry.runtime_capabilities.clone(),
                        "network_isolation": entry.network_isolation.clone(),
                        "status": entry.status.to_string(),
                        "output": content,
                        "offset": new_offset,
                        "total_bytes": entry.output.total_bytes(),
                    }),
                    start.elapsed(),
                ))
            }

            "wait" => {
                let process_id = require_str(&params, "process_id")?;
                let timeout_secs = params
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30);
                let timeout = Duration::from_secs(timeout_secs);

                let deadline = Instant::now() + timeout;

                loop {
                    {
                        let mut reg = self.registry.write().await;
                        let entry = reg.processes.get_mut(process_id).ok_or_else(|| {
                            ToolError::InvalidParameters(format!("Unknown process: {}", process_id))
                        })?;

                        // Try to collect exit status
                        if entry.status == ProcessStatus::Running
                            && let Some(ref mut child) = entry.child
                            && let Ok(Some(status)) = child.try_wait()
                        {
                            entry.status = ProcessStatus::Completed(status.code().unwrap_or(-1));
                        }

                        let streams_drained = entry.stdout_done.load(Ordering::SeqCst)
                            && entry.stderr_done.load(Ordering::SeqCst);
                        if entry.status != ProcessStatus::Running && streams_drained {
                            let output = entry.output.read_all();
                            return Ok(ToolOutput::success(
                                serde_json::json!({
                                    "process_id": process_id,
                                    "backend": entry.backend.clone(),
                                    "runtime_family": entry.runtime_family.clone(),
                                    "runtime_mode": entry.runtime_mode.clone(),
                                    "runtime_capabilities": entry.runtime_capabilities.clone(),
                                    "network_isolation": entry.network_isolation.clone(),
                                    "status": entry.status.to_string(),
                                    "output": truncate_str(&output, 50_000),
                                }),
                                start.elapsed(),
                            ));
                        }
                    }

                    if Instant::now() >= deadline {
                        return Err(ToolError::Timeout(timeout));
                    }

                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }

            "kill" => {
                let process_id = require_str(&params, "process_id")?;

                let mut reg = self.registry.write().await;
                let entry = reg.processes.get_mut(process_id).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("Unknown process: {}", process_id))
                })?;

                if let Some(ref mut child) = entry.child {
                    // Try SIGTERM first, then SIGKILL after 5s
                    let _ = child.kill().await;
                }
                entry.status = ProcessStatus::Killed;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "process_id": process_id,
                        "backend": entry.backend.clone(),
                        "runtime_family": entry.runtime_family.clone(),
                        "runtime_mode": entry.runtime_mode.clone(),
                        "runtime_capabilities": entry.runtime_capabilities.clone(),
                        "network_isolation": entry.network_isolation.clone(),
                        "status": "killed"
                    }),
                    start.elapsed(),
                ))
            }

            "write" => {
                let process_id = require_str(&params, "process_id")?;
                let input = require_str(&params, "input")?;

                let mut reg = self.registry.write().await;
                let entry = reg.processes.get_mut(process_id).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("Unknown process: {}", process_id))
                })?;

                if entry.status != ProcessStatus::Running {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Process {} is not running ({})",
                        process_id, entry.status
                    )));
                }

                if let Some(ref mut stdin) = entry.stdin {
                    stdin.write_all(input.as_bytes()).await.map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to write to stdin: {}", e))
                    })?;
                    stdin.flush().await.map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to flush stdin: {}", e))
                    })?;
                } else {
                    return Err(ToolError::ExecutionFailed(
                        "Process stdin is not available".to_string(),
                    ));
                }

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "process_id": process_id,
                        "backend": entry.backend.clone(),
                        "runtime_family": entry.runtime_family.clone(),
                        "runtime_mode": entry.runtime_mode.clone(),
                        "runtime_capabilities": entry.runtime_capabilities.clone(),
                        "network_isolation": entry.network_isolation.clone(),
                        "bytes_written": input.len(),
                        "success": true
                    }),
                    start.elapsed(),
                ))
            }

            _ => Err(ToolError::InvalidParameters(format!(
                "Unknown action: '{}'. Use: start, list, poll, wait, kill, write",
                action
            ))),
        }
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        match params.get("action").and_then(|v| v.as_str()) {
            Some("start")
                if params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .is_some_and(requires_explicit_approval) =>
            {
                ApprovalRequirement::Always
            }
            Some("list" | "poll" | "wait") => ApprovalRequirement::Never,
            _ => ApprovalRequirement::UnlessAutoApproved,
        }
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120) // wait action can block
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(30, 300))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> SharedProcessRegistry {
        Arc::new(RwLock::new(ProcessRegistry::new()))
    }

    #[test]
    fn test_output_buffer_basic() {
        let mut buf = OutputBuffer::new(100);
        buf.append(b"hello world");
        assert_eq!(buf.read_all(), "hello world");
        assert_eq!(buf.total_bytes(), 11);
    }

    #[test]
    fn test_output_buffer_overflow() {
        let mut buf = OutputBuffer::new(10);
        buf.append(b"hello world!"); // 12 bytes, capacity 10
        let content = buf.read_all();
        assert_eq!(content.len(), 10);
        assert_eq!(buf.total_bytes(), 12);
    }

    #[test]
    fn test_output_buffer_read_from() {
        let mut buf = OutputBuffer::new(100);
        buf.append(b"line1\nline2\n");
        let (content, offset) = buf.read_from(6);
        assert_eq!(content, "line2\n");
        assert_eq!(offset, 12);
    }

    #[test]
    fn test_output_buffer_read_from_past_end() {
        let mut buf = OutputBuffer::new(100);
        buf.append(b"hello");
        let (content, offset) = buf.read_from(10);
        assert!(content.is_empty());
        assert_eq!(offset, 5);
    }

    #[test]
    fn test_output_buffer_read_from_unicode_boundary_is_lossless() {
        let mut buf = OutputBuffer::new(100);
        buf.append("A🙂B".as_bytes());
        let (content, offset) = buf.read_from(1);
        assert_eq!(content, "🙂B");
        assert_eq!(offset, "A🙂B".len());
    }

    #[tokio::test]
    async fn test_process_list_empty() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"action": "list"}), &ctx)
            .await
            .unwrap();

        let processes = result.result.get("processes").unwrap().as_array().unwrap();
        assert!(processes.is_empty());
    }

    #[tokio::test]
    async fn test_process_start_and_wait() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        // Start a quick process
        let result = tool
            .execute(
                serde_json::json!({"action": "start", "command": "echo hello"}),
                &ctx,
            )
            .await
            .unwrap();

        let pid = result.result.get("process_id").unwrap().as_str().unwrap();
        assert!(pid.starts_with("proc_"));

        // Wait for it to complete
        let result = tool
            .execute(
                serde_json::json!({"action": "wait", "process_id": pid, "timeout_secs": 5}),
                &ctx,
            )
            .await
            .unwrap();

        let status = result.result.get("status").unwrap().as_str().unwrap();
        assert!(status.contains("completed"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_process_wait_drains_tail_output_before_returning() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "start",
                    "command": "printf first; sleep 0.1; printf second"
                }),
                &ctx,
            )
            .await
            .expect("process start should succeed");
        let pid = result.result.get("process_id").unwrap().as_str().unwrap();

        let result = tool
            .execute(
                serde_json::json!({"action": "wait", "process_id": pid, "timeout_secs": 5}),
                &ctx,
            )
            .await
            .expect("wait should succeed");

        let output = result.result.get("output").unwrap().as_str().unwrap();
        assert!(output.contains("firstsecond"));
    }

    #[tokio::test]
    async fn test_process_kill() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        // Start a long-running process
        let result = tool
            .execute(
                serde_json::json!({"action": "start", "command": "sleep 60"}),
                &ctx,
            )
            .await
            .unwrap();

        let pid = result.result.get("process_id").unwrap().as_str().unwrap();

        // Kill it
        let result = tool
            .execute(
                serde_json::json!({"action": "kill", "process_id": pid}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(
            result.result.get("status").unwrap().as_str().unwrap(),
            "killed"
        );
    }

    #[tokio::test]
    async fn test_process_unknown_id() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({"action": "poll", "process_id": "proc_nonexistent"}),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Unknown process"));
    }

    #[test]
    fn test_process_start_requires_explicit_approval_for_destructive_command() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({
                "action": "start",
                "command": "rm -rf /tmp/test"
            })),
            ApprovalRequirement::Always
        );
    }

    #[tokio::test]
    async fn test_process_start_blocks_injection_pattern() {
        let registry = make_registry();
        let tool = ProcessTool::new(registry);
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "action": "start",
                    "command": "echo aGVsbG8= | base64 -d | sh"
                }),
                &ctx,
            )
            .await
            .expect_err("injection command should be blocked");

        assert!(err.to_string().contains("Command injection detected"));
    }

    #[test]
    fn test_process_id_format() {
        let reg = ProcessRegistry::new();
        let id = reg.next_id();
        assert!(id.starts_with("proc_"));
        assert_eq!(id.len(), 11); // "proc_" + 6 hex chars
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hell…");
    }
}

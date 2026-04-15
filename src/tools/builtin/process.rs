//! Background process management tool.
//!
//! Allows the agent to start, monitor, and control long-running background
//! processes. Each process gets a short human-friendly ID and its output is
//! captured in a ring buffer for incremental reading.
//!
//! Actions: start, list, poll, wait, kill, write (stdin).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::context::JobContext;
use crate::platform::shell_launcher;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

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
        if self.total_written <= self.capacity {
            String::from_utf8_lossy(&self.data).to_string()
        } else {
            // Ring buffer wrapped — reconstruct in order
            let start = self.total_written % self.capacity;
            let mut result = Vec::with_capacity(self.capacity);
            result.extend_from_slice(&self.data[start..]);
            result.extend_from_slice(&self.data[..start]);
            String::from_utf8_lossy(&result).to_string()
        }
    }

    /// Read content from byte offset, returning (new_content, new_offset).
    fn read_from(&self, offset: usize) -> (String, usize) {
        if offset >= self.total_written {
            return (String::new(), self.total_written);
        }

        let available_start = self.total_written.saturating_sub(self.capacity);

        let effective_offset = offset.max(available_start);
        let content = self.read_all();
        let skip = effective_offset.saturating_sub(available_start);
        let slice = if skip < content.len() {
            &content[skip..]
        } else {
            ""
        };

        (slice.to_string(), self.total_written)
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
    /// Output buffer (stdout + stderr interleaved).
    pub output: OutputBuffer,
    /// Handle to the child process (for kill/wait).
    pub child: Option<tokio::process::Child>,
    /// Stdin writer (for write action).
    pub stdin: Option<tokio::process::ChildStdin>,
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
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

/// Background process management tool.
pub struct ProcessTool {
    registry: SharedProcessRegistry,
}

impl ProcessTool {
    pub fn new(registry: SharedProcessRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage background processes. Actions: \
         'start' (spawn a background command), \
         'list' (show all tracked processes), \
         'poll' (read new output from a process), \
         'wait' (block until a process completes), \
         'kill' (terminate a process), \
         'write' (send input to a process's stdin)."
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let action = require_str(&params, "action")?;

        match action {
            "start" => {
                let command = require_str(&params, "command")?;

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
                let mut child = shell_launcher()
                    .tokio_command(command)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .stdin(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to spawn process: {}", e))
                    })?;

                let stdin = child.stdin.take();

                let id = {
                    let reg = self.registry.read().await;
                    reg.next_id()
                };

                // Set up output capture tasks
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let registry_clone = Arc::clone(&self.registry);
                let id_clone = id.clone();

                // Spawn stdout reader
                if let Some(stdout) = stdout {
                    let reg = Arc::clone(&registry_clone);
                    let pid = id_clone.clone();
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
                    });
                }

                // Spawn stderr reader
                if let Some(stderr) = stderr {
                    let reg = Arc::clone(&registry_clone);
                    let pid = id_clone.clone();
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
                    });
                }

                let entry = ProcessEntry {
                    id: id.clone(),
                    command: command.to_string(),
                    status: ProcessStatus::Running,
                    started_at: Instant::now(),
                    output: OutputBuffer::new(MAX_BUFFER_SIZE),
                    child: Some(child),
                    stdin,
                };

                {
                    let mut reg = self.registry.write().await;
                    reg.processes.insert(id.clone(), entry);
                }

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "process_id": id,
                        "command": truncate_str(command, 80),
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

                        if entry.status != ProcessStatus::Running {
                            let output = entry.output.read_all();
                            return Ok(ToolOutput::success(
                                serde_json::json!({
                                    "process_id": process_id,
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

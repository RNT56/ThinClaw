//! `/subagents spawn` — spawn subagents from chat.
//!
//! Allows users to spawn new subagents from within a chat session,
//! assigning them tasks and routing rules.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Subagent spawn request from chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRequest {
    /// Name for the subagent.
    pub name: String,
    /// The task to assign.
    pub task: String,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional system prompt override.
    pub system_prompt: Option<String>,
    /// Which tools the subagent can use.
    pub tools: Option<Vec<String>>,
    /// Channel to report results to.
    pub report_channel: Option<String>,
    /// Max duration in seconds.
    pub timeout_secs: Option<u64>,
}

/// Subagent spawn result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResult {
    /// Assigned subagent ID.
    pub agent_id: String,
    /// Status message.
    pub message: String,
    /// Whether the spawn was successful
    pub success: bool,
}

/// Parse a spawn command from chat input.
///
/// Supported formats:
/// - `/subagents spawn "name" task description`
/// - `/subagents spawn name --model gpt-4o --task "do something"`
pub fn parse_spawn_command(input: &str) -> Result<SpawnRequest, SpawnError> {
    let trimmed = input.trim();

    // Strip the command prefix
    let body = trimmed
        .strip_prefix("/subagents spawn")
        .or_else(|| trimmed.strip_prefix("spawn"))
        .ok_or_else(|| SpawnError::InvalidCommand("Missing 'spawn' prefix".to_string()))?
        .trim();

    if body.is_empty() {
        return Err(SpawnError::InvalidCommand(
            "Usage: /subagents spawn <name> <task>".to_string(),
        ));
    }

    let mut name = String::new();
    let mut task = String::new();
    let mut model = None;
    let mut system_prompt = None;
    let mut tools = None;
    let mut timeout_secs = None;

    let parts = shell_split(body);
    let mut i = 0;

    // First non-flag arg is the name
    while i < parts.len() {
        if parts[i].starts_with("--") {
            break;
        }
        if name.is_empty() {
            name = parts[i].clone();
        } else {
            if !task.is_empty() {
                task.push(' ');
            }
            task.push_str(&parts[i]);
        }
        i += 1;
    }

    // Parse flags
    while i < parts.len() {
        match parts[i].as_str() {
            "--model" if i + 1 < parts.len() => {
                model = Some(parts[i + 1].clone());
                i += 2;
            }
            "--task" if i + 1 < parts.len() => {
                task = parts[i + 1].clone();
                i += 2;
            }
            "--prompt" | "--system" if i + 1 < parts.len() => {
                system_prompt = Some(parts[i + 1].clone());
                i += 2;
            }
            "--tools" if i + 1 < parts.len() => {
                tools = Some(
                    parts[i + 1]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                );
                i += 2;
            }
            "--timeout" if i + 1 < parts.len() => {
                timeout_secs = parts[i + 1].parse().ok();
                i += 2;
            }
            _ => {
                i += 1; // Skip unknown flags
            }
        }
    }

    if name.is_empty() {
        return Err(SpawnError::InvalidCommand(
            "Missing subagent name".to_string(),
        ));
    }

    if task.is_empty() {
        return Err(SpawnError::InvalidCommand(
            "Missing task description".to_string(),
        ));
    }

    Ok(SpawnRequest {
        name,
        task,
        model,
        system_prompt,
        tools,
        report_channel: None,
        timeout_secs,
    })
}

/// Simple shell-like argument splitting (handles quoted strings).
fn shell_split(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';

    for ch in input.chars() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Active subagent tracking.
pub struct SubagentTracker {
    active: HashMap<String, SubagentInfo>,
    id_counter: u64,
}

/// Info about a running subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: String,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: String,
}

/// Subagent status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed(String),
    TimedOut,
}

impl SubagentTracker {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            id_counter: 0,
        }
    }

    /// Register a new subagent.
    pub fn spawn(&mut self, request: &SpawnRequest, timestamp: &str) -> SubagentInfo {
        self.id_counter += 1;
        let id = format!("sub-{}", self.id_counter);

        let info = SubagentInfo {
            id: id.clone(),
            name: request.name.clone(),
            task: request.task.clone(),
            status: SubagentStatus::Running,
            spawned_at: timestamp.to_string(),
        };

        self.active.insert(id, info.clone());
        info
    }

    /// Update subagent status.
    pub fn update_status(&mut self, id: &str, status: SubagentStatus) -> bool {
        if let Some(info) = self.active.get_mut(id) {
            info.status = status;
            true
        } else {
            false
        }
    }

    /// Get subagent info.
    pub fn get(&self, id: &str) -> Option<&SubagentInfo> {
        self.active.get(id)
    }

    /// List all active subagents.
    pub fn list_active(&self) -> Vec<&SubagentInfo> {
        self.active
            .values()
            .filter(|s| s.status == SubagentStatus::Running)
            .collect()
    }

    /// List all subagents.
    pub fn list_all(&self) -> Vec<&SubagentInfo> {
        self.active.values().collect()
    }

    /// Count active.
    pub fn active_count(&self) -> usize {
        self.list_active().len()
    }
}

impl Default for SubagentTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn errors.
#[derive(Debug, Clone)]
pub enum SpawnError {
    InvalidCommand(String),
    LimitReached(usize),
    Other(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCommand(msg) => write!(f, "Invalid spawn command: {}", msg),
            Self::LimitReached(max) => write!(f, "Subagent limit reached (max: {})", max),
            Self::Other(msg) => write!(f, "Spawn error: {}", msg),
        }
    }
}

impl std::error::Error for SpawnError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_spawn() {
        let req =
            parse_spawn_command("/subagents spawn researcher Search for Rust crates").unwrap();
        assert_eq!(req.name, "researcher");
        assert_eq!(req.task, "Search for Rust crates");
    }

    #[test]
    fn test_parse_with_flags() {
        let req = parse_spawn_command(
            "/subagents spawn coder --model gpt-4o --task \"Write a test\" --timeout 300",
        )
        .unwrap();
        assert_eq!(req.name, "coder");
        assert_eq!(req.model, Some("gpt-4o".to_string()));
        assert_eq!(req.task, "Write a test");
        assert_eq!(req.timeout_secs, Some(300));
    }

    #[test]
    fn test_parse_with_tools() {
        let req = parse_spawn_command(
            "/subagents spawn helper --task \"help\" --tools \"shell,web_search\"",
        )
        .unwrap();
        let tools = req.tools.unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"shell".to_string()));
    }

    #[test]
    fn test_parse_missing_name() {
        let result = parse_spawn_command("/subagents spawn");
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_split_quoted() {
        let parts = shell_split(r#"name "multi word task" --flag value"#);
        assert_eq!(parts, vec!["name", "multi word task", "--flag", "value"]);
    }

    #[test]
    fn test_tracker_spawn() {
        let mut tracker = SubagentTracker::new();
        let req = SpawnRequest {
            name: "test".to_string(),
            task: "do stuff".to_string(),
            model: None,
            system_prompt: None,
            tools: None,
            report_channel: None,
            timeout_secs: None,
        };
        let info = tracker.spawn(&req, "2026-03-04T08:00:00Z");
        assert_eq!(info.name, "test");
        assert_eq!(info.status, SubagentStatus::Running);
        assert_eq!(tracker.active_count(), 1);
    }

    #[test]
    fn test_tracker_update_status() {
        let mut tracker = SubagentTracker::new();
        let req = SpawnRequest {
            name: "test".to_string(),
            task: "work".to_string(),
            model: None,
            system_prompt: None,
            tools: None,
            report_channel: None,
            timeout_secs: None,
        };
        let info = tracker.spawn(&req, "now");
        tracker.update_status(&info.id, SubagentStatus::Completed);
        assert_eq!(tracker.active_count(), 0);
        assert_eq!(tracker.list_all().len(), 1);
    }

    #[test]
    fn test_error_display() {
        let err = SpawnError::InvalidCommand("bad".to_string());
        assert!(format!("{}", err).contains("bad"));
    }
}

//! Z.AI tool_stream protocol support.
//!
//! Full implementation of Z.AI streaming protocol for tool call deltas,
//! promoting the partial implementation to complete support.

use serde::{Deserialize, Serialize};

/// Z.AI tool stream event types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStreamEventType {
    /// Tool call started.
    ToolCallStart,
    /// Partial argument delta.
    ToolCallDelta,
    /// Tool call complete (arguments fully received).
    ToolCallComplete,
    /// Tool execution started.
    ToolExecStart,
    /// Tool execution progress.
    ToolExecProgress,
    /// Tool execution complete with result.
    ToolExecComplete,
    /// Tool execution error.
    ToolExecError,
}

/// A Z.AI tool stream event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamEvent {
    /// Event type.
    pub event_type: ToolStreamEventType,
    /// Tool call ID (correlates events for the same call).
    pub call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// For deltas: partial argument JSON.
    pub arguments_delta: Option<String>,
    /// For complete: full arguments JSON.
    pub arguments: Option<String>,
    /// For exec_complete: tool output.
    pub result: Option<String>,
    /// For exec_error: error message.
    pub error: Option<String>,
    /// For exec_progress: progress percentage (0-100).
    pub progress: Option<u8>,
    /// Sequence number for ordering.
    pub seq: u64,
}

/// Accumulates tool call argument deltas into a complete call.
pub struct ToolStreamAccumulator {
    call_id: String,
    tool_name: String,
    argument_buffer: String,
    complete: bool,
    seq: u64,
}

impl ToolStreamAccumulator {
    /// Create a new accumulator for a tool call.
    pub fn new(call_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            argument_buffer: String::new(),
            complete: false,
            seq: 0,
        }
    }

    /// Process a stream event.
    pub fn process(&mut self, event: &ToolStreamEvent) -> AccumulatorState {
        if event.call_id != self.call_id {
            return AccumulatorState::Ignored;
        }

        self.seq = event.seq;

        match event.event_type {
            ToolStreamEventType::ToolCallStart => AccumulatorState::Started,
            ToolStreamEventType::ToolCallDelta => {
                if let Some(delta) = &event.arguments_delta {
                    self.argument_buffer.push_str(delta);
                }
                AccumulatorState::Accumulating {
                    buffer_size: self.argument_buffer.len(),
                }
            }
            ToolStreamEventType::ToolCallComplete => {
                self.complete = true;
                if let Some(args) = &event.arguments {
                    self.argument_buffer = args.clone();
                }
                AccumulatorState::Complete {
                    arguments: self.argument_buffer.clone(),
                }
            }
            ToolStreamEventType::ToolExecComplete => AccumulatorState::ExecComplete {
                result: event.result.clone().unwrap_or_default(),
            },
            ToolStreamEventType::ToolExecError => AccumulatorState::ExecError {
                error: event.error.clone().unwrap_or_default(),
            },
            _ => AccumulatorState::InProgress,
        }
    }

    /// Get the current argument buffer.
    pub fn arguments(&self) -> &str {
        &self.argument_buffer
    }

    /// Whether arguments are complete.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Tool name.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Call ID.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }
}

/// State of the accumulator after processing an event.
#[derive(Debug, Clone, PartialEq)]
pub enum AccumulatorState {
    /// Event was for a different call ID.
    Ignored,
    /// Tool call started.
    Started,
    /// Accumulating argument deltas.
    Accumulating { buffer_size: usize },
    /// General progress.
    InProgress,
    /// Arguments fully received.
    Complete { arguments: String },
    /// Execution completed with result.
    ExecComplete { result: String },
    /// Execution failed.
    ExecError { error: String },
}

/// Build a tool_call_start event.
pub fn tool_call_start(call_id: &str, tool_name: &str, seq: u64) -> ToolStreamEvent {
    ToolStreamEvent {
        event_type: ToolStreamEventType::ToolCallStart,
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments_delta: None,
        arguments: None,
        result: None,
        error: None,
        progress: None,
        seq,
    }
}

/// Build a tool_call_delta event.
pub fn tool_call_delta(call_id: &str, tool_name: &str, delta: &str, seq: u64) -> ToolStreamEvent {
    ToolStreamEvent {
        event_type: ToolStreamEventType::ToolCallDelta,
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments_delta: Some(delta.to_string()),
        arguments: None,
        result: None,
        error: None,
        progress: None,
        seq,
    }
}

/// Build a tool_call_complete event.
pub fn tool_call_complete(
    call_id: &str,
    tool_name: &str,
    arguments: &str,
    seq: u64,
) -> ToolStreamEvent {
    ToolStreamEvent {
        event_type: ToolStreamEventType::ToolCallComplete,
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments_delta: None,
        arguments: Some(arguments.to_string()),
        result: None,
        error: None,
        progress: None,
        seq,
    }
}

/// Build a tool_exec_complete event.
pub fn tool_exec_complete(
    call_id: &str,
    tool_name: &str,
    result: &str,
    seq: u64,
) -> ToolStreamEvent {
    ToolStreamEvent {
        event_type: ToolStreamEventType::ToolExecComplete,
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments_delta: None,
        arguments: None,
        result: Some(result.to_string()),
        error: None,
        progress: None,
        seq,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_event() {
        let event = tool_call_start("c1", "web_search", 1);
        assert_eq!(event.event_type, ToolStreamEventType::ToolCallStart);
        assert_eq!(event.call_id, "c1");
    }

    #[test]
    fn test_accumulate_deltas() {
        let mut acc = ToolStreamAccumulator::new("c1", "web_search");

        let start = tool_call_start("c1", "web_search", 1);
        assert_eq!(acc.process(&start), AccumulatorState::Started);

        let d1 = tool_call_delta("c1", "web_search", r#"{"qu"#, 2);
        let state = acc.process(&d1);
        assert!(matches!(state, AccumulatorState::Accumulating { .. }));

        let d2 = tool_call_delta("c1", "web_search", r#"ery":"test"}"#, 3);
        acc.process(&d2);

        assert_eq!(acc.arguments(), r#"{"query":"test"}"#);
        assert!(!acc.is_complete());
    }

    #[test]
    fn test_complete_event() {
        let mut acc = ToolStreamAccumulator::new("c1", "web_search");
        let complete = tool_call_complete("c1", "web_search", r#"{"query":"test"}"#, 5);
        let state = acc.process(&complete);

        assert!(matches!(state, AccumulatorState::Complete { .. }));
        assert!(acc.is_complete());
    }

    #[test]
    fn test_ignore_other_call() {
        let mut acc = ToolStreamAccumulator::new("c1", "web_search");
        let event = tool_call_start("c2", "other_tool", 1);
        assert_eq!(acc.process(&event), AccumulatorState::Ignored);
    }

    #[test]
    fn test_exec_complete() {
        let mut acc = ToolStreamAccumulator::new("c1", "web_search");
        let event = tool_exec_complete("c1", "web_search", "result data", 10);
        let state = acc.process(&event);
        assert!(matches!(state, AccumulatorState::ExecComplete { .. }));
    }

    #[test]
    fn test_exec_error() {
        let mut acc = ToolStreamAccumulator::new("c1", "shell");
        let event = ToolStreamEvent {
            event_type: ToolStreamEventType::ToolExecError,
            call_id: "c1".to_string(),
            tool_name: "shell".to_string(),
            arguments_delta: None,
            arguments: None,
            result: None,
            error: Some("command failed".to_string()),
            progress: None,
            seq: 5,
        };
        let state = acc.process(&event);
        assert!(matches!(state, AccumulatorState::ExecError { .. }));
    }

    #[test]
    fn test_full_lifecycle() {
        let mut acc = ToolStreamAccumulator::new("c1", "web_search");

        acc.process(&tool_call_start("c1", "web_search", 1));
        acc.process(&tool_call_delta("c1", "web_search", r#"{"q"#, 2));
        acc.process(&tool_call_delta("c1", "web_search", r#"":"hi"}"#, 3));
        acc.process(&tool_call_complete("c1", "web_search", r#"{"q":"hi"}"#, 4));

        assert!(acc.is_complete());
        assert_eq!(acc.arguments(), r#"{"q":"hi"}"#);
        assert_eq!(acc.tool_name(), "web_search");
    }

    #[test]
    fn test_delta_builder() {
        let event = tool_call_delta("c1", "tool", "chunk", 3);
        assert_eq!(event.arguments_delta, Some("chunk".to_string()));
        assert_eq!(event.seq, 3);
    }
}

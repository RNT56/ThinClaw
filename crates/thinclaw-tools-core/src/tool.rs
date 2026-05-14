//! Tool trait and types.

use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use thinclaw_types::JobContext;
pub use thinclaw_types::ToolProfile;

/// How much approval a specific tool invocation requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    /// No approval needed.
    Never,
    /// Needs approval, but session auto-approve can bypass.
    UnlessAutoApproved,
    /// Always needs explicit approval (even if auto-approved).
    Always,
}

/// Which runtime lane is invoking a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionLane {
    Chat,
    DeferredChat,
    Scheduler,
    Worker,
    WorkerRuntime,
    Subagent,
}

impl ToolExecutionLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::DeferredChat => "deferred_chat",
            Self::Scheduler => "scheduler",
            Self::Worker => "worker",
            Self::WorkerRuntime => "worker_runtime",
            Self::Subagent => "subagent",
        }
    }
}

/// Coarse side-effect classification for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffectLevel {
    Read,
    #[default]
    Write,
}

/// Coarse approval classification for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalClass {
    Never,
    #[default]
    Conditional,
    Always,
}

/// Narrow routing intents where a tool is considered authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRouteIntent {
    CurrentTime,
    TranscriptHistory,
    MemoryRecall,
    LocalState,
}

/// Internal metadata used for tool routing and policy decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub authoritative_source: bool,
    pub live_data: bool,
    pub side_effect_level: ToolSideEffectLevel,
    pub approval_class: ToolApprovalClass,
    pub parallel_safe: bool,
    #[serde(default)]
    pub route_intents: Vec<ToolRouteIntent>,
}

impl ToolMetadata {
    pub fn coordination() -> Self {
        Self {
            authoritative_source: true,
            live_data: false,
            side_effect_level: ToolSideEffectLevel::Read,
            approval_class: ToolApprovalClass::Never,
            parallel_safe: true,
            route_intents: Vec::new(),
        }
    }

    pub fn read_only() -> Self {
        Self {
            side_effect_level: ToolSideEffectLevel::Read,
            approval_class: ToolApprovalClass::Never,
            parallel_safe: true,
            ..Self::default()
        }
    }

    pub fn authoritative(intent: ToolRouteIntent) -> Self {
        Self {
            authoritative_source: true,
            route_intents: vec![intent],
            ..Self::read_only()
        }
    }

    pub fn live_authoritative(intent: ToolRouteIntent) -> Self {
        Self {
            authoritative_source: true,
            live_data: true,
            route_intents: vec![intent],
            ..Self::read_only()
        }
    }
}

impl Default for ToolMetadata {
    fn default() -> Self {
        Self {
            authoritative_source: false,
            live_data: false,
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: ToolApprovalClass::Conditional,
            parallel_safe: false,
            route_intents: Vec::new(),
        }
    }
}

/// Rich internal descriptor for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub domain: ToolDomain,
    pub metadata: ToolMetadata,
}

impl ToolDescriptor {
    pub fn is_coordination_tool(&self) -> bool {
        matches!(
            self.name.as_str(),
            "agent_think" | "emit_user_message" | "consult_advisor"
        )
    }

    pub fn is_safe_read_only_orchestrator(&self) -> bool {
        self.domain == ToolDomain::Orchestrator
            && self.metadata.side_effect_level == ToolSideEffectLevel::Read
            && self.metadata.approval_class == ToolApprovalClass::Never
    }

    pub fn supports_route_intent(&self, intent: ToolRouteIntent) -> bool {
        self.metadata.route_intents.contains(&intent)
    }
}

impl ApprovalRequirement {
    /// Whether this invocation requires approval in contexts where
    /// auto-approve is irrelevant (e.g. autonomous worker/scheduler).
    pub fn is_required(&self) -> bool {
        !matches!(self, Self::Never)
    }
}

/// Per-tool rate limit configuration for built-in tool invocations.
///
/// Controls how many times a tool can be invoked per user, per time window.
/// Read-only tools (echo, time, json, file_read, etc.) should NOT be rate limited.
/// Write/external tools (shell, http, file_write, memory_write, create_job) should be.
#[derive(Debug, Clone)]
pub struct ToolRateLimitConfig {
    /// Maximum invocations per minute.
    pub requests_per_minute: u32,
    /// Maximum invocations per hour.
    pub requests_per_hour: u32,
}

impl ToolRateLimitConfig {
    /// Create a config with explicit limits.
    pub fn new(requests_per_minute: u32, requests_per_hour: u32) -> Self {
        Self {
            requests_per_minute,
            requests_per_hour,
        }
    }
}

impl Default for ToolRateLimitConfig {
    /// Default: 60 requests/minute, 1000 requests/hour (generous for WASM HTTP).
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

/// Where a tool should execute: orchestrator process or inside a container.
///
/// Orchestrator tools run in the main agent process (memory access, job mgmt, etc).
/// Container tools run inside Docker containers (shell, file ops, code mods).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDomain {
    /// Safe to run in the orchestrator (pure functions, memory, job management).
    Orchestrator,
    /// Must run inside a sandboxed container (filesystem, shell, code).
    Container,
}

/// Error type for tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Not authorized: {0}")]
    NotAuthorized(String),

    #[error("Rate limited, retry after {0:?}")]
    RateLimited(Option<Duration>),

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The result data.
    pub result: serde_json::Value,
    /// Cost incurred (if any).
    pub cost: Option<Decimal>,
    /// Time taken.
    pub duration: Duration,
    /// Raw output before sanitization (for debugging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// Optional rich artifacts that callers can render without flattening to text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ToolArtifact>,
}

impl ToolOutput {
    /// Create a successful output with a JSON result.
    pub fn success(result: serde_json::Value, duration: Duration) -> Self {
        Self {
            result,
            cost: None,
            duration,
            raw: None,
            artifacts: Vec::new(),
        }
    }

    /// Create a text output.
    pub fn text(text: impl Into<String>, duration: Duration) -> Self {
        Self {
            result: serde_json::Value::String(text.into()),
            cost: None,
            duration,
            raw: None,
            artifacts: Vec::new(),
        }
    }

    /// Set the cost.
    pub fn with_cost(mut self, cost: Decimal) -> Self {
        self.cost = Some(cost);
        self
    }

    /// Set the raw output.
    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }

    /// Attach rich artifacts to this result.
    pub fn with_artifacts(mut self, artifacts: Vec<ToolArtifact>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Append one rich artifact to this result.
    pub fn push_artifact(mut self, artifact: ToolArtifact) -> Self {
        self.artifacts.push(artifact);
        self
    }
}

/// Optional rich tool artifacts that can be rendered alongside the JSON result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolArtifact {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    ResourceLink {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blob: Option<String>,
    },
    Structured {
        content: serde_json::Value,
    },
}

/// Definition of a tool's parameters using JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    /// Create a new tool schema.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    /// Set the parameters schema.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }
}

/// Trait for tools that the agent can use.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name.
    fn name(&self) -> &str;

    /// Get a description of what the tool does.
    fn description(&self) -> &str;

    /// Get the JSON Schema for the tool's parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Internal metadata used for routing and policy decisions.
    ///
    /// The default is intentionally conservative so unannotated or dynamic
    /// tools in restricted lanes require explicit grants.
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::default()
    }

    /// Execute the tool with the given parameters.
    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError>;

    /// Estimate the cost of running this tool with the given parameters.
    fn estimated_cost(&self, _params: &serde_json::Value) -> Option<Decimal> {
        None
    }

    /// Estimate how long this tool will take with the given parameters.
    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        None
    }

    /// Whether this tool's output needs sanitization.
    ///
    /// Returns true for tools that interact with external services,
    /// where the output might contain malicious content.
    fn requires_sanitization(&self) -> bool {
        true
    }

    /// Whether this tool invocation requires user approval.
    ///
    /// Returns `Never` by default (most tools run in a sandboxed environment).
    /// Override to return `UnlessAutoApproved` for tools that need approval
    /// but can be session-auto-approved, or `Always` for invocations that
    /// must always prompt (e.g. destructive shell commands, HTTP with auth).
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    /// Maximum time this tool is allowed to run before the caller kills it.
    /// Override for long-running tools like sandbox execution.
    /// Default: 60 seconds.
    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    /// Where this tool should execute.
    ///
    /// `Orchestrator` tools run in the main agent process (safe, no FS access).
    /// `Container` tools run inside Docker containers (shell, file ops).
    ///
    /// Default: `Orchestrator` (safe for the main process).
    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    /// Per-invocation rate limit for this tool.
    ///
    /// Return `Some(config)` to throttle how often this tool can be called per user.
    /// Read-only tools (echo, time, json, file_read, memory_search, etc.) should
    /// return `None`. Write/external tools (shell, http, file_write, memory_write,
    /// create_job) should return sensible limits to prevent runaway agents.
    ///
    /// Rate limits are per-user, per-tool, and in-memory (reset on restart).
    /// This is orthogonal to `requires_approval()` — a tool can be both
    /// approval-gated and rate limited. Rate limit is checked first (cheaper).
    ///
    /// Default: `None` (no rate limiting).
    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        None
    }

    /// Get the tool schema for LLM function calling.
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }

    /// Build a rich descriptor for runtime policy decisions.
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            domain: self.domain(),
            metadata: self.metadata(),
        }
    }
}

/// Extract a required string parameter from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing or not a string.
pub fn require_str<'a>(params: &'a serde_json::Value, name: &str) -> Result<&'a str, ToolError> {
    params
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

/// Extract a required parameter of any type from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing.
pub fn require_param<'a>(
    params: &'a serde_json::Value,
    name: &str,
) -> Result<&'a serde_json::Value, ToolError> {
    params
        .get(name)
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple no-op tool for testing.
    #[derive(Debug)]
    pub struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes back the input message. Useful for testing."
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to echo back"
                    }
                },
                "required": ["message"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let message = require_str(&params, "message")?;

            Ok(ToolOutput::text(message, Duration::from_millis(1)))
        }

        fn requires_sanitization(&self) -> bool {
            false // Echo is a trusted internal tool
        }
    }

    #[tokio::test]
    async fn test_echo_tool() {
        let tool = EchoTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"message": "hello"}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result, serde_json::json!("hello"));
    }

    #[test]
    fn test_tool_schema() {
        let tool = EchoTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "echo");
        assert!(!schema.description.is_empty());
    }

    #[test]
    fn test_execution_timeout_default() {
        let tool = EchoTool;
        assert_eq!(tool.execution_timeout(), Duration::from_secs(60));
    }

    #[test]
    fn test_require_str_present() {
        let params = serde_json::json!({"name": "alice"});
        assert_eq!(require_str(&params, "name").unwrap(), "alice");
    }

    #[test]
    fn test_require_str_missing() {
        let params = serde_json::json!({});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_str_wrong_type() {
        let params = serde_json::json!({"name": 42});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_param_present() {
        let params = serde_json::json!({"data": [1, 2, 3]});
        assert_eq!(
            require_param(&params, "data").unwrap(),
            &serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn test_require_param_missing() {
        let params = serde_json::json!({});
        let err = require_param(&params, "data").unwrap_err();
        assert!(err.to_string().contains("missing 'data'"));
    }

    #[test]
    fn test_requires_approval_default() {
        let tool = EchoTool;
        // Default requires_approval() returns Never.
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"message": "hi"})),
            ApprovalRequirement::Never
        );
        assert!(!ApprovalRequirement::Never.is_required());
        assert!(ApprovalRequirement::UnlessAutoApproved.is_required());
        assert!(ApprovalRequirement::Always.is_required());
    }

    #[test]
    fn read_only_metadata_is_safe_for_restricted_profiles() {
        let metadata = ToolMetadata::read_only();
        assert_eq!(metadata.approval_class, ToolApprovalClass::Never);
        assert_eq!(metadata.side_effect_level, ToolSideEffectLevel::Read);
        assert!(metadata.parallel_safe);

        let live_authoritative = ToolMetadata::live_authoritative(ToolRouteIntent::CurrentTime);
        assert_eq!(live_authoritative.approval_class, ToolApprovalClass::Never);
        assert_eq!(
            live_authoritative.side_effect_level,
            ToolSideEffectLevel::Read
        );
        assert!(live_authoritative.parallel_safe);
    }

    #[test]
    fn consult_advisor_counts_as_coordination_tool() {
        let descriptor = ToolDescriptor {
            name: "consult_advisor".to_string(),
            description: "Consult the advisor lane.".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            domain: ToolDomain::Orchestrator,
            metadata: ToolMetadata::default(),
        };

        assert!(descriptor.is_coordination_tool());
    }
}

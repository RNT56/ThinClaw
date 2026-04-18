//! MCP protocol types.

use serde::{Deserialize, Serialize};

/// MCP protocol version.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// Generic JSON-RPC request sent to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Request ID.
    pub id: u64,
    /// Method name.
    pub method: String,
    /// Request parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl McpRequest {
    /// Create a new MCP request.
    pub fn new(id: u64, method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }

    /// Create an initialize request with default client capabilities.
    pub fn initialize(id: u64) -> Self {
        Self::initialize_with_capabilities(id, ClientCapabilities::default())
    }

    /// Create an initialize request with explicit client capabilities.
    pub fn initialize_with_capabilities(id: u64, capabilities: ClientCapabilities) -> Self {
        Self::new(
            id,
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": capabilities,
                "clientInfo": {
                    "name": "thinclaw",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        )
    }

    /// Create a tools/list request.
    pub fn list_tools(id: u64, cursor: Option<&str>) -> Self {
        Self::new(
            id,
            "tools/list",
            cursor.map(|cursor| serde_json::json!({ "cursor": cursor })),
        )
    }

    /// Create a tools/call request.
    pub fn call_tool(id: u64, name: &str, arguments: serde_json::Value) -> Self {
        Self::new(
            id,
            "tools/call",
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        )
    }

    /// Create a resources/list request.
    pub fn list_resources(id: u64, cursor: Option<&str>) -> Self {
        Self::new(
            id,
            "resources/list",
            cursor.map(|cursor| serde_json::json!({ "cursor": cursor })),
        )
    }

    /// Create a resources/read request.
    pub fn read_resource(id: u64, uri: &str) -> Self {
        Self::new(
            id,
            "resources/read",
            Some(serde_json::json!({
                "uri": uri
            })),
        )
    }

    /// Create a resources/templates/list request.
    pub fn list_resource_templates(id: u64, cursor: Option<&str>) -> Self {
        Self::new(
            id,
            "resources/templates/list",
            cursor.map(|cursor| serde_json::json!({ "cursor": cursor })),
        )
    }

    /// Create a resources/subscribe request.
    pub fn subscribe_resource(id: u64, uri: &str) -> Self {
        Self::new(
            id,
            "resources/subscribe",
            Some(serde_json::json!({ "uri": uri })),
        )
    }

    /// Create a resources/unsubscribe request.
    pub fn unsubscribe_resource(id: u64, uri: &str) -> Self {
        Self::new(
            id,
            "resources/unsubscribe",
            Some(serde_json::json!({ "uri": uri })),
        )
    }

    /// Create a prompts/list request.
    pub fn list_prompts(id: u64, cursor: Option<&str>) -> Self {
        Self::new(
            id,
            "prompts/list",
            cursor.map(|cursor| serde_json::json!({ "cursor": cursor })),
        )
    }

    /// Create a prompts/get request.
    pub fn get_prompt(id: u64, name: &str, arguments: Option<serde_json::Value>) -> Self {
        let mut params = serde_json::json!({ "name": name });
        if let Some(arguments) = arguments {
            params["arguments"] = arguments;
        }
        Self::new(id, "prompts/get", Some(params))
    }

    /// Create a completion/complete request.
    pub fn complete(id: u64, reference: serde_json::Value, argument: CompleteArgument) -> Self {
        Self::new(
            id,
            "completion/complete",
            Some(serde_json::json!({
                "ref": reference,
                "argument": argument
            })),
        )
    }

    /// Create a logging/setLevel request.
    pub fn set_logging_level(id: u64, level: McpLoggingLevel) -> Self {
        Self::new(
            id,
            "logging/setLevel",
            Some(serde_json::json!({
                "level": level
            })),
        )
    }
}

/// Generic JSON-RPC notification sent to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpNotification {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Notification method.
    pub method: String,
    /// Notification parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl McpNotification {
    /// Create a new notification.
    pub fn new(method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }

    /// Create an initialized notification.
    pub fn initialized() -> Self {
        Self::new("notifications/initialized", None)
    }

    /// Create a roots/list_changed notification.
    pub fn roots_list_changed() -> Self {
        Self::new("notifications/roots/list_changed", None)
    }

    /// Create a cancelled notification.
    pub fn cancelled(request_id: u64, reason: Option<&str>) -> Self {
        let mut params = serde_json::json!({ "requestId": request_id });
        if let Some(reason) = reason {
            params["reason"] = serde_json::Value::String(reason.to_string());
        }
        Self::new("notifications/cancelled", Some(params))
    }
}

/// Response from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Request ID.
    pub id: u64,
    /// Result (on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error (on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

impl McpResponse {
    /// Create a success response.
    pub fn success(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn error(id: u64, error: McpError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// MCP error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    /// Error code.
    pub code: i32,
    /// Error message.
    pub message: String,
    /// Additional data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl McpError {
    /// Create a method-not-found error.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Unsupported MCP client request: {method}"),
            data: None,
        }
    }

    /// Create an invalid-request error.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    /// Create a request-cancelled error.
    pub fn request_cancelled(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
            data: None,
        }
    }
}

/// A decoded transport-level MCP message.
#[derive(Debug, Clone)]
pub enum McpTransportMessage {
    Request(McpRequest),
    Response(McpResponse),
    Notification(McpNotification),
}

impl McpTransportMessage {
    /// Parse a single JSON-RPC message from a string.
    pub fn parse_str(message: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(message)?;
        Self::from_value(value)
    }

    /// Parse a single JSON-RPC message from a value.
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let has_method = value.get("method").is_some();
        let has_id = value.get("id").is_some();

        match (has_method, has_id) {
            (true, true) => serde_json::from_value(value).map(Self::Request),
            (true, false) => serde_json::from_value(value).map(Self::Notification),
            (false, true) => serde_json::from_value(value).map(Self::Response),
            (false, false) => Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "message is missing both method and id",
            ))),
        }
    }
}

/// Client capabilities advertised during initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<ClientRootsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<ClientSamplingCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<ClientElicitationCapability>,
}

/// Roots client capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientRootsCapability {
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Sampling client capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientSamplingCapability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ClientSamplingToolsCapability>,
}

/// Sampling tool-use client capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientSamplingToolsCapability {}

/// Form elicitation client capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientElicitationCapability {
    #[serde(default)]
    pub forms: bool,
}

/// Result of the initialize handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitializeResult {
    /// Protocol version supported by the server.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: Option<String>,

    /// Server capabilities.
    #[serde(default)]
    pub capabilities: ServerCapabilities,

    /// Server information.
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,

    /// Instructions for using this server.
    pub instructions: Option<String>,
}

/// Server capabilities advertised during initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Tool capabilities.
    #[serde(default)]
    pub tools: Option<ToolsCapability>,

    /// Resource capabilities.
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,

    /// Resource template capabilities.
    #[serde(rename = "resourceTemplates", default)]
    pub resource_templates: Option<ResourceTemplatesCapability>,

    /// Prompt capabilities.
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,

    /// Completion capabilities.
    #[serde(default)]
    pub completion: Option<CompletionCapability>,

    /// Logging capabilities.
    #[serde(default)]
    pub logging: Option<LoggingCapability>,
}

/// Tool-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Whether the tool list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Resource-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Whether subscriptions are supported.
    #[serde(default)]
    pub subscribe: bool,

    /// Whether the resource list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Resource template-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceTemplatesCapability {
    /// Whether resource templates can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Prompt-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptsCapability {
    /// Whether the prompt list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Completion capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionCapability {}

/// Logging capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingCapability {}

/// Server information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,

    /// Server version.
    pub version: Option<String>,
}

/// Pagination cursor parameter/result helper.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CursorPage {
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
}

/// Result of listing tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListToolsResult {
    #[serde(default)]
    pub tools: Vec<McpTool>,
    #[serde(flatten)]
    pub cursor: CursorPage,
}

/// Result of listing resources.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListResourcesResult {
    #[serde(default)]
    pub resources: Vec<McpResource>,
    #[serde(flatten)]
    pub cursor: CursorPage,
}

/// Result of listing resource templates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListResourceTemplatesResult {
    #[serde(default, rename = "resourceTemplates")]
    pub resource_templates: Vec<McpResourceTemplate>,
    #[serde(flatten)]
    pub cursor: CursorPage,
}

/// Result of reading a resource.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadResourceResult {
    #[serde(default)]
    pub contents: Vec<McpResourceContents>,
}

/// Result of listing prompts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListPromptsResult {
    #[serde(default)]
    pub prompts: Vec<McpPrompt>,
    #[serde(flatten)]
    pub cursor: CursorPage,
}

/// Result of fetching a prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetPromptResult {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub messages: Vec<McpPromptMessage>,
}

/// Result of completion/complete.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteResult {
    #[serde(default)]
    pub completion: CompleteResultPayload,
}

/// Inner completion payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteResultPayload {
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(rename = "hasMore", default)]
    pub has_more: bool,
}

/// Result of calling a tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(rename = "structuredContent", default)]
    pub structured_content: Option<serde_json::Value>,
    #[serde(default)]
    pub is_error: bool,
}

/// A tool definition returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name.
    pub name: String,
    /// Optional human-readable title.
    #[serde(default)]
    pub title: Option<String>,
    /// Tool description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema for input parameters.
    #[serde(
        default = "default_input_schema",
        rename = "inputSchema",
        alias = "input_schema"
    )]
    pub input_schema: serde_json::Value,
    /// Optional JSON Schema for structured output.
    #[serde(default, rename = "outputSchema", alias = "output_schema")]
    pub output_schema: Option<serde_json::Value>,
    /// Optional annotations from the MCP server.
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
    /// Optional icon metadata.
    #[serde(default)]
    pub icons: Vec<McpIcon>,
    /// Optional execution metadata.
    #[serde(default)]
    pub execution: Option<McpToolExecution>,
}

/// Default input schema (empty object).
fn default_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

/// Icon metadata for tools/resources/prompts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpIcon {
    pub src: String,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
}

/// Execution metadata for a tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpToolExecution {
    #[serde(rename = "taskSupport", default)]
    pub task_support: bool,
}

/// Annotations for an MCP tool that provide hints about its behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpToolAnnotations {
    #[serde(default, rename = "destructiveHint", alias = "destructive_hint")]
    pub destructive_hint: bool,

    #[serde(default, rename = "sideEffectsHint", alias = "side_effects_hint")]
    pub side_effects_hint: bool,

    #[serde(default, rename = "readOnlyHint", alias = "read_only_hint")]
    pub read_only_hint: bool,

    #[serde(default, rename = "idempotentHint", alias = "idempotent_hint")]
    pub idempotent_hint: bool,

    #[serde(default, rename = "openWorldHint", alias = "open_world_hint")]
    pub open_world_hint: bool,

    #[serde(default, rename = "executionTimeHint", alias = "execution_time_hint")]
    pub execution_time_hint: Option<ExecutionTimeHint>,
}

/// Hint about how long a tool typically takes to execute.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTimeHint {
    Fast,
    Medium,
    Slow,
}

impl McpTool {
    /// Check if this tool requires user approval based on its annotations.
    pub fn requires_approval(&self) -> bool {
        self.annotations.as_ref().is_some_and(|annotations| {
            annotations.destructive_hint
                || (annotations.side_effects_hint && !annotations.read_only_hint)
        })
    }
}

/// Resource descriptor returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
    #[serde(default)]
    pub icons: Vec<McpIcon>,
}

/// Resource template descriptor returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceTemplate {
    #[serde(rename = "uriTemplate")]
    pub uri_template: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
}

/// Resource contents returned by resources/read or embedded tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpResourceContents {
    #[serde(rename = "text")]
    Text {
        uri: String,
        #[serde(rename = "mimeType", default)]
        mime_type: Option<String>,
        text: String,
    },
    #[serde(rename = "blob")]
    Blob {
        uri: String,
        #[serde(rename = "mimeType", default)]
        mime_type: Option<String>,
        blob: String,
    },
}

impl McpResourceContents {
    /// The canonical resource URI.
    pub fn uri(&self) -> &str {
        match self {
            Self::Text { uri, .. } | Self::Blob { uri, .. } => uri,
        }
    }
}

/// Prompt descriptor returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<McpPromptArgument>,
}

/// Prompt argument metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// Prompt message returned by prompts/get.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: PromptContent,
}

/// Prompt message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptContent {
    Blocks(Vec<ContentBlock>),
    Block(ContentBlock),
    Text(String),
}

/// Completion argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteArgument {
    pub name: String,
    pub value: String,
}

/// Sampling request delivered from a server to the client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingCreateMessageRequest {
    #[serde(default)]
    pub messages: Vec<McpPromptMessage>,
    #[serde(rename = "systemPrompt", default)]
    pub system_prompt: Option<String>,
    #[serde(rename = "includeContext", default)]
    pub include_context: Option<String>,
    #[serde(rename = "maxTokens", default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(rename = "stopSequences", default)]
    pub stop_sequences: Vec<String>,
    #[serde(rename = "modelPreferences", default)]
    pub model_preferences: Option<serde_json::Value>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Elicitation request delivered from a server to the client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ElicitationCreateRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(rename = "requestedSchema", alias = "schema", default)]
    pub requested_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Sampling result returned back to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingCreateMessageResult {
    pub role: String,
    pub content: SamplingResponseContent,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(rename = "stopReason", default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Content returned for a sampling response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SamplingResponseContent {
    Blocks(Vec<ContentBlock>),
    Block(ContentBlock),
    Text(String),
}

/// Logging levels for `logging/setLevel`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpLoggingLevel {
    Debug,
    Info,
    #[default]
    Warning,
    Error,
}

/// A content block in a tool result or prompt message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "audio")]
    Audio {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "resource_link")]
    ResourceLink {
        uri: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(rename = "mimeType", default)]
        mime_type: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    #[serde(rename = "resource")]
    EmbeddedResource { resource: McpResourceContents },
}

impl ContentBlock {
    /// Get text content if this is a text block.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::EmbeddedResource {
                resource: McpResourceContents::Text { text, .. },
            } => Some(text),
            _ => None,
        }
    }
}

/// A parsed progress notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressNotification {
    #[serde(rename = "progressToken", default)]
    pub progress_token: Option<serde_json::Value>,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub total: Option<f64>,
    #[serde(default)]
    pub message: Option<String>,
}

/// A parsed log notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingMessageNotification {
    pub level: Option<McpLoggingLevel>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub logger: Option<String>,
}

/// A parsed cancellation notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CancelledNotification {
    #[serde(rename = "requestId", default)]
    pub request_id: Option<u64>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// A parsed resource-updated notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUpdatedNotification {
    pub uri: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_deserialize_camel_case_input_schema() {
        let json = serde_json::json!({
            "name": "list_issues",
            "description": "List GitHub issues",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "owner": { "type": "string" },
                    "repo": { "type": "string" }
                },
                "required": ["owner", "repo"]
            }
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        assert_eq!(tool.name, "list_issues");
        assert_eq!(tool.description, "List GitHub issues");

        let props = tool.input_schema.get("properties").expect("has properties");
        assert!(props.get("owner").is_some());
        assert!(props.get("repo").is_some());
    }

    #[test]
    fn test_mcp_tool_deserialize_snake_case_alias() {
        let json = serde_json::json!({
            "name": "search",
            "description": "Search",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        let props = tool.input_schema.get("properties").expect("has properties");
        assert!(props.get("query").is_some());
    }

    #[test]
    fn test_mcp_tool_missing_schema_gets_default() {
        let json = serde_json::json!({
            "name": "ping",
            "description": "Ping"
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema["properties"].is_object());
    }

    #[test]
    fn test_list_tools_result_supports_pagination() {
        let server_response = serde_json::json!({
            "tools": [{
                "name": "github-copilot_list_issues",
                "description": "List issues for a repository",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner" },
                        "repo": { "type": "string", "description": "Repository name" },
                        "state": { "type": "string", "enum": ["open", "closed", "all"] }
                    },
                    "required": ["owner", "repo"]
                }
            }],
            "nextCursor": "cursor-2"
        });

        let result: ListToolsResult =
            serde_json::from_value(server_response).expect("deserialize ListToolsResult");
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.cursor.next_cursor.as_deref(), Some("cursor-2"));
    }

    #[test]
    fn test_transport_message_parsing() {
        let notification = McpTransportMessage::parse_str(
            r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#,
        )
        .expect("notification");
        assert!(matches!(
            notification,
            McpTransportMessage::Notification(McpNotification { .. })
        ));

        let request =
            McpTransportMessage::parse_str(r#"{"jsonrpc":"2.0","id":1,"method":"roots/list"}"#)
                .expect("request");
        assert!(matches!(
            request,
            McpTransportMessage::Request(McpRequest { .. })
        ));
    }

    #[test]
    fn test_content_block_variants() {
        let value = serde_json::json!({
            "type": "audio",
            "data": "Zm9v",
            "mimeType": "audio/wav"
        });
        let block: ContentBlock = serde_json::from_value(value).expect("audio block");
        match block {
            ContentBlock::Audio { mime_type, .. } => assert_eq!(mime_type, "audio/wav"),
            other => panic!("unexpected block: {other:?}"),
        }
    }
}

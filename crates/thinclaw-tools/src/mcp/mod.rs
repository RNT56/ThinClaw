//! Root-independent Model Context Protocol helpers.

pub mod config;
pub mod protocol;
pub mod session;
pub mod stdio;

pub use config::{
    ConfigError, McpCapabilityPolicy, McpLoggingLevel, McpRuntimeHealth, McpServerConfig,
    McpServersFile, McpTransport, OAuthConfig, load_mcp_servers_from, save_mcp_servers_to,
};
pub use protocol::{
    CallToolResult, CancelledNotification, ClientCapabilities, ClientElicitationCapability,
    ClientRootsCapability, ClientSamplingCapability, ClientSamplingToolsCapability,
    CompleteArgument, CompleteResult, ContentBlock, ElicitationCreateRequest, ExecutionTimeHint,
    GetPromptResult, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, LoggingMessageNotification, McpError, McpIcon,
    McpNotification, McpPrompt, McpPromptArgument, McpPromptMessage, McpRequest, McpResource,
    McpResourceContents, McpResourceTemplate, McpResponse, McpTool, McpToolAnnotations,
    McpToolExecution, McpTransportMessage, PROTOCOL_VERSION, ProgressNotification, PromptContent,
    ReadResourceResult, ResourceUpdatedNotification, SamplingCreateMessageRequest,
    SamplingCreateMessageResult, SamplingResponseContent,
};
pub use session::{McpSession, McpSessionManager};
pub use stdio::{McpInboundHandler, StdioTransport};

//! Root-independent Model Context Protocol helpers.

pub mod auth;
pub mod config;
pub mod protocol;
pub mod session;
pub mod stdio;

pub use auth::{
    AccessToken, AuthError, AuthorizationServerMetadata, ClientRegistrationRequest,
    ClientRegistrationResponse, DEFAULT_OAUTH_CALLBACK_PORT, OAuthDiscoveryBundle,
    OAuthFlowOptions, PkceChallenge, ProtectedResourceMetadata, authorize_mcp_server,
    authorize_mcp_server_with_options, bind_callback_listener, build_authorization_url,
    discover_authorization_server, discover_full_oauth_metadata, discover_oauth_bundle,
    discover_oauth_endpoints, discover_protected_resource, exchange_code_for_token,
    find_available_port, get_access_token, is_authenticated, refresh_access_token, register_client,
    store_client_id, store_tokens,
};
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

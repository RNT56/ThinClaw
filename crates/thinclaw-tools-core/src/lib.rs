//! Core tool traits, schemas, and execution metadata.

pub mod builder;
pub mod canvas;
pub mod execution_descriptor;
pub mod mcp;
pub mod mcp_interaction;
pub mod mcp_logging;
pub mod rate_limiter;
pub mod tool;
pub mod url_guard;

pub use builder::{
    BuildLog, BuildPhase, BuildRequirement, BuildResult, BuilderConfig, Language, SoftwareType,
};
pub use canvas::{
    ButtonStyle, CanvasAction, FormField, KvItem, NotifyLevel, PanelPosition, UiComponent,
};
pub use rate_limiter::{LimitType, RateLimitResult, RateLimiter};
pub use tool::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolArtifact, ToolDescriptor, ToolDomain,
    ToolError, ToolExecutionLane, ToolMetadata, ToolOutput, ToolProfile, ToolRateLimitConfig,
    ToolRouteIntent, ToolSchema, ToolSideEffectLevel, require_param, require_str,
};
pub use url_guard::{
    GuardedUrl, OutboundUrlGuardOptions, validate_outbound_url, validate_outbound_url_pinned,
};

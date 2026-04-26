//! Core tool traits, schemas, and execution metadata.

pub mod rate_limiter;
pub mod tool;
pub mod url_guard;

pub use rate_limiter::{LimitType, RateLimitResult, RateLimiter};
pub use tool::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolArtifact, ToolDescriptor, ToolDomain,
    ToolError, ToolExecutionLane, ToolMetadata, ToolOutput, ToolProfile, ToolRateLimitConfig,
    ToolRouteIntent, ToolSchema, ToolSideEffectLevel, require_param, require_str,
};
pub use url_guard::{OutboundUrlGuardOptions, validate_outbound_url};

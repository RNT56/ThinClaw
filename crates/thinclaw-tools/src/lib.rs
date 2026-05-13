//! Tool runtime crate.
//!
//! Core tool traits are already extracted in `thinclaw-tools-core`; registry,
//! execution, MCP, WASM, and built-ins will move here next.

pub mod browser_args;
pub mod builder;
pub mod builtin;
pub mod execution;
pub mod intent_display;
pub mod mcp;
pub mod registry;
pub mod smart_approve;
pub mod user_tool;
pub mod wasm;

pub use registry::{
    ToolRegistry, deny_reason_for_lane, deny_reason_for_profile, descriptor_allowed_for_profile,
    tool_allowed_for_lane,
};
pub use thinclaw_tools_core::*;

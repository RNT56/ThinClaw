//! Tool runtime crate.
//!
//! Core tool traits are already extracted in `thinclaw-tools-core`; registry,
//! execution, MCP, WASM, and built-ins will move here next.

pub mod browser_args;
pub mod intent_display;
pub mod mcp;
pub mod registry;
pub mod smart_approve;
pub mod wasm;

pub use registry::{ToolRegistry, descriptor_allowed_for_profile, tool_allowed_for_lane};
pub use thinclaw_tools_core::*;

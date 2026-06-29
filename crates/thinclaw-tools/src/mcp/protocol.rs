//! Façade: the MCP protocol DTOs now live in `thinclaw-tools-core` so light
//! consumers can use them without the heavyweight tool runtime. Re-exported here
//! for path stability (`thinclaw_tools::mcp::protocol::*`).

pub use thinclaw_tools_core::mcp::*;

//! Compatibility adapter for the extracted background process tool.

pub use crate::tools::execution_backend::RootExecutionBackendAdapter as RootProcessBackendAdapter;

pub use thinclaw_tools::builtin::process::{ProcessTool, SharedProcessRegistry, start_reaper};

//! RPC commands — Cost tracking, channel status, ClawHub, routing, Gmail,
//! canvas panels, heartbeat, workspace management.
//!
//! Extracted from `rpc.rs` for better modularity, then split into focused
//! submodules. This `mod.rs` is a façade: it declares the submodules and
//! re-exports the stable public Tauri command API so the existing
//! `crate::thinclaw::commands::*` paths keep resolving unchanged.
//!
//! - `helpers`: shared JSON-shape / value-normalization helpers
//! - `cost`: cost summary/export/reset + remote cost-summary mapper
//! - `routing`: smart-routing config, rules CRUD, status, simulation
//! - `channels`: channel-status list + Gmail status/OAuth
//! - `plugins`: ClawHub, cache stats, lifecycle/manifest, default agent
//! - `canvas`: canvas-panel management
//! - `heartbeat`: heartbeat-interval updates
//! - `workspace_files`: local agent-workspace path / reveal / list / write

mod canvas;
mod channels;
mod cost;
mod heartbeat;
mod helpers;
mod plugins;
mod routing;
mod workspace_files;

pub use canvas::*;
pub use channels::*;
pub use cost::*;
pub use heartbeat::*;
pub use plugins::*;
pub use routing::*;
pub use workspace_files::*;

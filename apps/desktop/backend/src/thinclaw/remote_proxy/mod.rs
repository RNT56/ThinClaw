//! RemoteGatewayProxy — HTTP/SSE client for remote ThinClaw gateway.
//!
//! When ThinClaw Desktop is in "remote" mode, all agent interactions are forwarded
//! to a remote ThinClaw HTTP server instead of the embedded in-process engine.
//!
//! Architecture:
//!   Frontend → Tauri IPC → Command handler → RemoteGatewayProxy → HTTP API
//!   Remote SSE stream → RemoteGatewayProxy → Tauri emit("thinclaw-event")
//!
//! The proxy is intentionally thin: it does not transform data but passes
//! raw JSON responses back to the command handlers who already know the
//! expected shape (same as local mode, since the remote ThinClaw server
//! and the local embedded engine share the same API definitions).
//!
//! This `mod.rs` is a façade: the proxy type lives in `core` and its many
//! endpoint methods are split across domain submodules that each provide an
//! `impl RemoteGatewayProxy` block. The public type/state are re-exported here
//! so `crate::thinclaw::remote_proxy::{RemoteGatewayProxy, ConnectionState}`
//! keep resolving unchanged.
//!
//! - `core`: type, connection state, construction, request primitives, health
//! - `chat`: chat/thread lifecycle, approval, transcript export, compaction
//! - `memory`: workspace file read/write/delete/list + memory search
//! - `routines`: routine CRUD, trigger, history
//! - `providers`: pairing, skills, provider/routing config, costs, cache, logs
//! - `jobs`: jobs, autonomy, learning, experiments, MCP
//! - `extensions`: extensions, hooks, tools, settings/config, diagnostics
//! - `sse`: background SSE subscription + connection state

mod chat;
mod core;
mod extensions;
mod jobs;
mod memory;
mod providers;
mod routines;
mod sse;

#[cfg(test)]
mod tests;

pub use core::{ConnectionState, RemoteGatewayProxy};

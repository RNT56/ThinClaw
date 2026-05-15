//! ThinClaw module — agent engine integration
//!
//! Core modules:
//! - runtime_bridge, tauri_channel, secrets_adapter, event_mapping (engine)
//! - commands (Tauri command handlers)
//! - config (identity.json, keychain, workspace paths)
//! - ui_types (UiEvent enum for frontend events)

pub mod commands;
pub mod config;
pub mod deploy;
pub mod event_mapping; // StatusUpdate → UiEvent conversion
pub mod fleet;
pub mod remote_proxy; // HTTP/SSE proxy client for remote ThinClaw gateway
pub mod runtime_bridge; // ThinClaw lifecycle manager (ThinClawRuntimeState)
mod runtime_builder; // Engine construction logic (extracted from runtime_bridge)
pub mod sanitizer; // LLM token stripping
pub mod secrets_adapter; // SecretsStore adapter (Keychain → ThinClaw trait)
pub mod tauri_channel; // impl Channel for TauriChannel
pub mod tool_bridge; // ToolBridge trait + TauriToolBridge (3-tier approval)
pub mod ui_types; // UiEvent enum + supporting types

// Legacy WS modules removed in Phase 4:
// - frames.rs (WsFrame protocol)
// - normalizer.rs (WS event → UiEvent)
// - ws_client.rs (WebSocket RPC client)
// - ipc.rs (MCP request handler for WS)

pub use commands::ThinClawManager;
pub use config::ThinClawConfig;

// Re-export UI types from canonical location
pub use ui_types::{UiEvent, UiMessage, UiSession, UiUsage};

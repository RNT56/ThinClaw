//! OpenClaw module — agent engine integration
//!
//! Core modules:
//! - ironclaw_bridge, ironclaw_channel, ironclaw_secrets, ironclaw_types (engine)
//! - commands (Tauri command handlers)
//! - config (identity.json, keychain, workspace paths)
//! - ui_types (UiEvent enum for frontend events)

pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
pub mod ironclaw_bridge; // IronClaw lifecycle manager (IronClawState)
pub mod ironclaw_channel; // impl Channel for TauriChannel
pub mod ironclaw_secrets; // SecretsStore adapter (Keychain → IronClaw trait)
pub mod ironclaw_types; // StatusUpdate → UiEvent conversion
pub mod sanitizer; // LLM token stripping
pub mod ui_types; // UiEvent enum + supporting types

// Legacy WS modules removed in Phase 4:
// - frames.rs (WsFrame protocol)
// - normalizer.rs (WS event → UiEvent)
// - ws_client.rs (WebSocket RPC client)
// - ipc.rs (MCP request handler for WS)

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;

// Re-export UI types from canonical location
pub use ui_types::{UiEvent, UiMessage, UiSession, UiUsage};

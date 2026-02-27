//! OpenClaw module — agent engine integration
//!
//! After IronClaw integration (Phase 4), the following will be deleted:
//! - frames, normalizer, ws_client (WS bridge — replaced by TauriChannel)
//! - OpenClawManager (process lifecycle — replaced by IronClawState)

pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
pub mod sanitizer; // NEW: LLM token stripping (extracted from normalizer)
pub mod ui_types; // NEW: UiEvent enum + supporting types (extracted from normalizer)

// --- Legacy WS bridge (removed in Phase 4) ---
mod frames;
pub mod ipc;
mod normalizer;
mod ws_client;

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;
pub use frames::{WsError, WsFrame};

// Re-export UI types from new canonical location
pub use ui_types::{UiEvent, UiMessage, UiSession, UiUsage};

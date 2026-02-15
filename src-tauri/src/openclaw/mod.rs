//! OpenClaw module - OpenClawEngine gateway integration for Slack/Telegram bots
//!
//! This module provides:
//! - WebSocket client for OpenClawEngine Gateway protocol
//! - Event normalization to stable UI events
//! - Configuration generation
//! - Tauri commands for frontend integration

pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
mod frames;
pub mod ipc;
mod normalizer;
mod ws_client;

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;
pub use frames::{WsError, WsFrame};
pub use normalizer::{UiEvent, UiMessage, UiSession, UiUsage};

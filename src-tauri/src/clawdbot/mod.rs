//! Clawdbot module - Moltbot gateway integration for Slack/Telegram bots
//!
//! This module provides:
//! - WebSocket client for Moltbot Gateway protocol
//! - Event normalization to stable UI events
//! - Configuration generation
//! - Tauri commands for frontend integration

pub mod commands;
mod config;
mod frames;
mod normalizer;
mod ws_client;

pub use commands::ClawdbotManager;
pub use config::ClawdbotConfig;
pub use frames::{WsError, WsFrame};
pub use normalizer::{UiEvent, UiMessage, UiSession, UiUsage};

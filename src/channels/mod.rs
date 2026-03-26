//! Multi-channel input system.
//!
//! Channels receive messages from external sources (CLI, HTTP, etc.)
//! and convert them to a unified message format for the agent to process.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         ChannelManager                              │
//! │                                                                     │
//! │   ┌──────────────┐   ┌─────────────┐   ┌─────────────┐             │
//! │   │ ReplChannel  │   │ HttpChannel │   │ WasmChannel │   ...       │
//! │   └──────┬───────┘   └──────┬──────┘   └──────┬──────┘             │
//! │          │                 │                 │                      │
//! │          └─────────────────┴─────────────────┘                      │
//! │                            │                                        │
//! │                   select_all (futures)                              │
//! │                            │                                        │
//! │                            ▼                                        │
//! │                     MessageStream                                   │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # WASM Channels
//!
//! WASM channels allow dynamic loading of channel implementations at runtime.
//! See the [`wasm`] module for details.

pub mod ack_reaction;
pub mod canvas_gateway;
mod channel;
mod discord;
pub mod forward_download;
mod gmail;
pub mod gmail_wiring;
pub mod group_priming;
pub mod health_monitor;
mod http;
#[cfg(target_os = "macos")]
mod apple_mail;
#[cfg(target_os = "macos")]
mod imessage;
pub mod imessage_wiring;
mod manager;
mod nostr;
pub mod reaction_machine;
mod repl;
pub mod self_message;
mod signal;
mod slack;
pub mod status_view;
mod telegram;
pub mod tool_stream;
pub mod wasm;
pub mod web;
mod webhook_server;

pub use channel::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
pub use discord::DiscordChannel;
pub use gmail::GmailChannel;
pub use health_monitor::{ChannelHealthMonitor, ChannelHealthStatus, HealthMonitorConfig};
pub use http::HttpChannel;
#[cfg(target_os = "macos")]
pub use apple_mail::{
    AppleMailChannel, AppleMailConfig, AppleMailDiagnostic, ensure_app_running,
};
#[cfg(target_os = "macos")]
pub use imessage::{IMessageChannel, IMessageConfig, IMessageDiagnostic};
pub use manager::ChannelManager;
pub use nostr::NostrChannel;
pub use repl::ReplChannel;
pub use self_message::{SelfMessageConfig, TrustedMetadata};
pub use signal::SignalChannel;
pub use slack::SlackChannel;
pub use telegram::TelegramChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};

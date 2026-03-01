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

mod channel;
mod discord;
mod http;
#[cfg(target_os = "macos")]
mod imessage;
mod manager;
mod nostr;
mod repl;
mod signal;
mod slack;
mod telegram;
pub mod wasm;
pub mod web;
mod webhook_server;

pub use channel::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
pub use discord::DiscordChannel;
pub use http::HttpChannel;
#[cfg(target_os = "macos")]
pub use imessage::IMessageChannel;
pub use manager::ChannelManager;
pub use nostr::NostrChannel;
pub use repl::ReplChannel;
pub use signal::SignalChannel;
pub use slack::SlackChannel;
pub use telegram::TelegramChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};

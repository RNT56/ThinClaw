//! Multi-channel input system.
//!
//! Channels receive messages from external sources and convert them to a
//! unified message format for the agent to process. ThinClaw uses a
//! **hybrid architecture**:
//!
//! - **Native Rust**: Discord (Gateway WS), Signal, Gmail, Nostr,
//!   iMessage, Apple Mail, REPL, HTTP, Gateway — for persistent
//!   connections, local filesystem access, and full `Channel` trait.
//! - **WASM sandboxed**: Telegram, WhatsApp, Slack, Discord
//!   (interactions) — for stateless webhook/polling channels with
//!   credential isolation.
//!
//! See `docs/CHANNEL_ARCHITECTURE.md` for decision rationale.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         ChannelManager                              │
//! │                                                                     │
//! │  Native Channels              WASM Channels (sandboxed)             │
//! │  ┌──────────────┐             ┌──────────────────────┐              │
//! │  │ DiscordCh    │             │ WasmChannel:telegram │              │
//! │  │ SignalCh     │             │ WasmChannel:whatsapp │              │
//! │  │ GmailCh      │             │ WasmChannel:slack    │              │
//! │  │ NostrCh      │             │ WasmChannel:discord  │              │
//! │  │ ReplCh  ...  │             └──────────┬───────────┘              │
//! │  └──────┬───────┘                        │                          │
//! │         └────────────────────────────────┘                          │
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
#[cfg(target_os = "macos")]
mod apple_mail;
mod bluebubbles;
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
mod imessage;
mod manager;
mod nostr;
pub mod reaction_machine;
mod repl;
pub mod self_message;
mod signal;
pub mod status_view;
pub mod tool_stream;
pub mod wasm;
pub mod web;
mod webhook_server;

#[cfg(target_os = "macos")]
pub use apple_mail::{AppleMailChannel, AppleMailConfig, AppleMailDiagnostic, ensure_app_running};
pub use bluebubbles::{BlueBubblesChannel, BlueBubblesConfig, BlueBubblesDiagnostic};
pub use channel::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
pub use discord::DiscordChannel;
pub use gmail::GmailChannel;
pub use health_monitor::{ChannelHealthMonitor, ChannelHealthStatus, HealthMonitorConfig};
pub use http::HttpChannel;
#[cfg(target_os = "macos")]
pub use imessage::{IMessageChannel, IMessageConfig, IMessageDiagnostic};
pub use manager::ChannelManager;
pub use nostr::NostrChannel;
pub use repl::ReplChannel;
pub use self_message::{SelfMessageConfig, TrustedMetadata};
pub use signal::SignalChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};

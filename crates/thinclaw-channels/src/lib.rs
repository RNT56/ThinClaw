//! Channel runtime crate.

pub mod ack_reaction;
pub mod canvas_gateway;
pub mod forward_download;
pub mod gmail;
pub mod gmail_wiring;
pub mod group_priming;
pub mod health_monitor;
pub mod http;
pub mod manager;
pub mod pairing;
pub mod reaction_machine;
pub mod self_message;
pub mod status_view;
pub mod wasm;
pub mod webhook_server;

pub use gmail::GmailChannel;
pub use health_monitor::{ChannelHealthMonitor, ChannelHealthStatus, HealthMonitorConfig};
pub use http::{HttpChannel, HttpConfig};
pub use manager::{
    ChannelDescriptor, ChannelManager, ChannelStatusChangeEvent, IncomingEvent, SlashCommand,
    legacy_session_key_aliases, mint_session_key, normalize_incoming_event, parse_slash_command,
};
pub use thinclaw_channels_core::*;

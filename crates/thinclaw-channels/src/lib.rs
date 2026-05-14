//! Channel runtime crate.

pub mod ack_reaction;
pub mod acp;
#[cfg(target_os = "macos")]
pub mod apple_mail;
pub mod bluebubbles;
pub mod canvas_gateway;
pub mod discord;
pub mod forward_download;
pub mod gmail;
pub mod gmail_wiring;
pub mod group_priming;
pub mod health_monitor;
pub mod http;
#[cfg(target_os = "macos")]
pub mod imessage;
pub mod manager;
pub mod native_lifecycle;
pub mod native_lifecycle_clients;
#[cfg(feature = "nostr")]
pub mod nostr;
#[cfg(feature = "nostr")]
pub mod nostr_runtime;
pub mod pairing;
pub mod reaction_machine;
pub mod repl;
pub mod self_message;
pub mod signal;
pub mod status_view;
pub mod tui;
mod util;
pub mod wasm;
pub mod webhook_server;

#[cfg(target_os = "macos")]
pub use apple_mail::{AppleMailChannel, AppleMailConfig, AppleMailDiagnostic, ensure_app_running};
pub use bluebubbles::{BlueBubblesChannel, BlueBubblesConfig, BlueBubblesDiagnostic};
pub use discord::{DiscordChannel, DiscordConfig};
pub use gmail::GmailChannel;
pub use health_monitor::{ChannelHealthMonitor, ChannelHealthStatus, HealthMonitorConfig};
pub use http::{HttpChannel, HttpConfig};
#[cfg(target_os = "macos")]
pub use imessage::{IMessageChannel, IMessageConfig, IMessageDiagnostic};
pub use manager::{
    ChannelDescriptor, ChannelManager, ChannelStatusChangeEvent, IncomingEvent,
    NativeLifecycleChannelConfig, SlashCommand, legacy_session_key_aliases, mint_session_key,
    native_lifecycle_channel_descriptors, normalize_incoming_event, parse_slash_command,
};
pub use native_lifecycle::{
    NativeLifecycleChannel, NativeLifecycleClient, NativeLifecycleEvent, NativeLifecycleIngress,
    NativeLifecycleKind, NativeLifecycleWebhookConfig, NativeOutboundMessage,
    browser_push_event_from_payload, matrix_events_from_payload, native_lifecycle_webhook_routes,
    voice_call_event_from_payload,
};
pub use native_lifecycle_clients::{
    ApnsNativeClient, ApnsNativeConfig, BrowserPushNativeClient, BrowserPushNativeConfig,
    MatrixNativeClient, MatrixNativeConfig, NativeEndpointRegistry, NativeHttpClient,
    NativeHttpRequest, NativeHttpResponse, ReqwestNativeHttpClient, VoiceCallNativeClient,
    VoiceCallNativeConfig,
};
#[cfg(feature = "nostr")]
pub use nostr::NostrChannel;
#[cfg(feature = "nostr")]
pub use nostr_runtime::{NostrConfig, NostrDmProtocol, NostrInboundDm, NostrRuntime};
pub use signal::{SignalChannel, SignalConfig};
pub use thinclaw_channels_core::*;
pub use tui::{TuiChannel, TuiEvent, TuiRuntime, TuiUpdate};

//! # thinclaw-client
//!
//! A typed async Rust client for driving a [ThinClaw](https://thinclaw.dev)
//! gateway — the analog of an Agent SDK. It wraps the gateway's HTTP + SSE
//! surface: send chat messages, stream responses and tool-call events, browse
//! thread history, and resolve tool approvals.
//!
//! ## Quick start
//!
//! ```no_run
//! use std::time::Duration;
//! use thinclaw_client::Client;
//!
//! # async fn run() -> thinclaw_client::Result<()> {
//! let client = Client::from_env()?; // THINCLAW_GATEWAY_URL + THINCLAW_GATEWAY_TOKEN
//! let reply = client
//!     .send_and_wait("What's the weather like?", None, Duration::from_secs(60))
//!     .await?;
//! println!("{reply}");
//! # Ok(())
//! # }
//! ```
//!
//! ## Streaming with tool-call visibility
//!
//! ```no_run
//! use futures::StreamExt;
//! use thinclaw_client::{Client, SseEvent};
//!
//! # async fn run() -> thinclaw_client::Result<()> {
//! let client = Client::from_env()?;
//! let mut events = Box::pin(client.events().await?);
//! client.send_message("Run the tests", None).await?;
//! while let Some(event) = events.next().await {
//!     match event? {
//!         SseEvent::ToolStarted { name, .. } => println!("→ {name}"),
//!         SseEvent::Response { content, .. } => { println!("{content}"); break; }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Stability
//!
//! The gateway routes this client binds are operator-facing infrastructure, not
//! yet a versioned public API — treat this crate as pre-1.0 and expect breaking
//! changes. [`SseEvent`] models a curated subset of the server's events; any
//! unmodeled event surfaces as [`SseEvent::Unknown`] with its raw payload, so
//! new server events never break deserialization.

mod client;
mod error;
mod sse;
mod wire;

pub use client::Client;
pub use error::{ClientError, Result};
pub use wire::{
    ApprovalAction, HistoryResponse, SendMessageResponse, SseEvent, ThreadInfo, ThreadListResponse,
    ToolCallInfo, TurnInfo,
};

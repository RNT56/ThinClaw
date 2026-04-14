//! ThinClaw — Secure Personal AI Agent Framework
//!
//! An LLM-powered autonomous agent with multi-channel interaction,
//! WASM-sandboxed tools, and persistent memory.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────────────┐
//! │                              User Interaction Layer                              │
//! │                                                                                  │
//! │  Native (Rust)                        WASM-sandboxed                             │
//! │  ┌──────────┐ ┌──────────┐            ┌────────────────┐                         │
//! │  │   REPL   │ │ Discord  │            │ WasmCh:telegram│                         │
//! │  │  Signal  │ │  Gmail   │            │ WasmCh:slack   │                         │
//! │  │  Nostr   │ │ iMessage │            │ WasmCh:whatsapp│                         │
//! │  │   HTTP   │ │ Gateway  │            │ WasmCh:discord │                         │
//! │  └────┬─────┘ └────┬─────┘            └───────┬────────┘                         │
//! │       └─────────────┴────────────────────────┬┘                                  │
//! └──────────────────────────────────────────────┼──────────────────────────────────┘
//!                                    ▼
//! ┌──────────────────────────────────────────────────────────────────────────────────┐
//! │                              Main Agent Loop                                      │
//! │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                      │
//! │  │ Message Router │──│  LLM Reasoning │──│ Action Executor│                      │
//! │  └────────────────┘  └───────┬────────┘  └───────┬────────┘                      │
//! │         ▲                    │                   │                               │
//! │         │         ┌──────────┴───────────────────┴──────────┐                    │
//! │         │         ▼                                         ▼                    │
//! │  ┌──────┴─────────────┐                         ┌───────────────────────┐        │
//! │  │   Safety Layer     │                         │    Self-Repair        │        │
//! │  │ - Input sanitizer  │                         │ - Stuck job detection │        │
//! │  │ - Injection defense│                         │ - Tool fixer          │        │
//! │  └────────────────────┘                         └───────────────────────┘        │
//! └──────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **Multi-channel interaction** - CLI, HTTP, Discord, and WASM-sandboxed channels (Telegram, WhatsApp, and Slack)
//! - **Parallel job execution** - Run multiple jobs with isolated contexts
//! - **Pluggable tools** - MCP, 3rd party services, dynamic tools
//! - **Self-repair** - Detect and fix stuck jobs and broken tools
//! - **Prompt injection defense** - Sanitize all external data
//! - **Continuous learning** - Improve estimates from historical data

pub mod agent;
pub mod api;
pub mod app;
#[cfg(feature = "repl")]
pub mod boot_screen;
pub mod bootstrap;
pub mod branding;
pub mod channels;
pub mod cli;
pub mod config;
pub mod context;
pub mod db;
#[cfg(feature = "document-extraction")]
pub mod document_extraction;
pub mod error;
pub mod estimation;
pub mod evaluation;
pub mod experiments;
pub mod extensions;
pub mod hardware_bridge;
pub mod history;
pub mod hooks;
pub mod i18n;
pub mod identity;
pub mod llm;
pub mod media;
pub mod observability;
#[cfg(feature = "docker-sandbox")]
pub mod orchestrator;
pub mod pairing;
pub mod profile;
pub mod profile_evolution;
pub mod qr_pairing;
pub mod registry;
pub mod safety;
pub mod sandbox;
pub mod sandbox_types;
pub mod secrets;
#[cfg(feature = "repl")]
pub mod service;
pub mod settings;
pub mod setup;
pub mod skills;
pub mod tailscale;
pub mod talk_mode;
pub mod tauri_commands;
pub mod terminal_branding;
pub mod timezone;
pub mod tools;
pub mod tracing_fmt;
pub mod tui;
#[cfg(feature = "tunnel")]
pub mod tunnel;
pub mod update_checker;
pub mod util;
#[cfg(feature = "voice")]
pub mod voice_wake;
#[cfg(feature = "docker-sandbox")]
pub mod worker;
pub mod workspace;

#[cfg(test)]
pub mod testing;

pub use config::Config;
pub use error::{Error, Result};

/// Re-export commonly used types.
pub mod prelude {
    pub use crate::agent::{Agent, BackgroundTasksHandle};
    pub use crate::app::{AppBuilder, AppComponents};
    pub use crate::channels::{
        Channel, ChannelManager, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    };
    pub use crate::config::Config;
    pub use crate::context::{JobContext, JobState};
    pub use crate::error::{Error, Result};
    pub use crate::identity::{
        ConversationKind, ConversationScope, LinkedConversationRecall, ResolvedIdentity,
    };
    pub use crate::llm::LlmProvider;
    pub use crate::safety::{SanitizedOutput, Sanitizer};
    pub use crate::tools::{Tool, ToolOutput, ToolRegistry};
    pub use crate::workspace::{MemoryDocument, Workspace};
}

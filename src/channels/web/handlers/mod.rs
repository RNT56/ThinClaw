//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.

pub mod autonomy;
pub mod chat;
pub mod experiments;
pub mod extensions;
pub mod gateway;
pub mod jobs;
pub mod learning;
pub mod logs;
pub mod mcp;
pub mod memory;
#[cfg(feature = "nostr")]
pub mod nostr;
pub mod pairing;
pub mod projects;
pub mod providers;
pub mod routines;
pub mod settings;
pub mod skills;

pub(crate) use autonomy::*;
pub(crate) use chat::*;
pub(crate) use experiments::*;
pub(crate) use extensions::*;
pub(crate) use gateway::*;
pub(crate) use jobs::*;
pub(crate) use learning::*;
pub(crate) use logs::*;
pub(crate) use mcp::*;
pub(crate) use memory::*;
#[cfg(feature = "nostr")]
pub(crate) use nostr::*;
pub(crate) use pairing::*;
pub(crate) use projects::*;
pub(crate) use providers::*;
pub(crate) use routines::*;
pub(crate) use settings::*;
pub(crate) use skills::*;

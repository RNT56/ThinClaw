//! Request and response DTOs for the web gateway API.
//!
//! Submodules group the DTOs by API domain. This façade re-exports every type
//! so existing `crate::web::types::X` / `thinclaw_gateway::web::types::X`
//! import paths keep working unchanged.

mod autonomy;
mod chat;
mod common;
mod experiments;
mod extensions;
mod hooks;
mod jobs;
mod logs;
mod memory;
mod pairing;
mod routines;
mod settings;
mod skills;
mod sse;
mod status;
mod ws;

pub use autonomy::*;
pub use chat::*;
pub use common::*;
pub use experiments::*;
pub use extensions::*;
pub use hooks::*;
pub use jobs::*;
pub use logs::*;
pub use memory::*;
pub use pairing::*;
pub use routines::*;
pub use settings::*;
pub use skills::*;
pub use sse::*;
pub use status::*;
pub use ws::*;

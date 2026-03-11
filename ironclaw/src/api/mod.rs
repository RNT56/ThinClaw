//! Framework-agnostic API surface for IronClaw.
//!
//! These modules contain the business logic that both the web gateway
//! and the Tauri integration call into. Each function takes explicit
//! dependencies (`&Agent`, `&Arc<Workspace>`, etc.) and returns typed
//! `ApiResult<T>` values — no Axum, no Tauri, no framework specifics.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |---|---|
//! | [`chat`] | Send messages, resolve approvals, abort turns |
//! | [`sessions`] | Thread CRUD, history pagination |
//! | [`memory`] | Workspace file CRUD, tree, vector search |
//! | [`config`] | User settings CRUD, import/export |
//! | [`extensions`] | Extension lifecycle (install/activate/remove) |
//! | [`skills`] | Skill registry + catalog operations |
//! | [`routines`] | Routine CRUD + manual trigger |
//! | [`system`] | Engine status, model info |

pub mod chat;
pub mod config;
pub mod extensions;
pub mod memory;
pub mod routines;
pub mod sessions;
pub mod skills;
pub mod system;

mod error;
pub use error::{ApiError, ApiResult};

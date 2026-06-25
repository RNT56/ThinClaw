//! Central extension manager that dispatches operations by ExtensionKind.
//!
//! Holds references to channel runtime, WASM tool runtime, MCP infrastructure,
//! secrets store, and tool registry. All extension operations (search, install,
//! auth, activate, list, remove) flow through here.
//!
//! Lifecycle events (install, activate, deactivate, remove) are forwarded to an
//! optional [`crate::extensions::lifecycle_hooks::AuditLogHook`] for the plugin
//! lifecycle audit trail.
//!
//! This module is a façade over focused submodules:
//! - [`core`] — the [`ExtensionManager`] struct, construction, post-construction
//!   setters, shared helpers, and the public setup DTOs.
//! - [`lifecycle`] — the top-level `search`/`install`/`auth`/`activate`/`list`/
//!   `remove` dispatchers.
//! - [`install`] — the bundled/standard/fallback install chain and WASM download.
//! - [`mcp`] — MCP config/client lifecycle and MCP auth/activation.
//! - [`wasm`] — WASM tool/channel auth and (hot-)activation.
//! - [`native`] — native dynamic-library plugin glue (operator-only, default-off).
//! - [`setup`] — OAuth callback completion and setup schema/status/validation.

mod core;
mod install;
mod lifecycle;
mod mcp;
mod native;
mod setup;
mod wasm;

// Public API surface preserved at `crate::extensions::manager::…`.
pub use core::{AuthRequestContext, ExtensionManager, ExtensionSetupSchema, SetupResult};

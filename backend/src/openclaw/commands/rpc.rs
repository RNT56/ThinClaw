//! RPC commands — re-export hub.
//!
//! This module was originally a monolithic 3400+ line file. It has been split
//! into domain-specific submodules for maintainability:
//!
//! - `rpc_skills`        — Skills management (install, list, toggle)
//! - `rpc_routines`      — Cron/routine CRUD, lint, audit, delete, toggle
//! - `rpc_config`        — Config CRUD, settings toggles, autonomy, bootstrap, cloud model, system presence
//! - `rpc_orchestration` — Sub-agent registry, session spawning, canvas, agent profiles
//! - `rpc_extensions`    — Hooks, extensions, diagnostics, tools, pairing, compaction
//! - `rpc_dashboard`     — Cost tracking, channel status, ClawHub, routing, Gmail, workspace

// All command functions are re-exported from the parent module via mod.rs

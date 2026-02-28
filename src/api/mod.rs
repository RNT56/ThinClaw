//! Framework-agnostic API surface for IronClaw.
//!
//! This module provides callable Rust functions that expose all agent
//! capabilities without framework dependencies (no Axum, no Tauri).
//! Each submodule maps to a domain:
//!
//! - `chat` — send messages, resolve approvals, abort turns
//! - `sessions` — list, create, delete, rename sessions
//! - `memory` — read/write workspace files (SOUL.md, MEMORY.md)
//! - `config` — get/set agent configuration
//! - `skills` — list, toggle, install skills
//! - `extensions` — list, toggle extensions
//! - `routines` — list, trigger scheduled routines
//! - `system` — health, status, diagnostics, model list
//!
//! These functions are the target for both:
//! - Tauri `#[tauri::command]` wrappers (Scrappy desktop app)
//! - Any future REST/gRPC gateway

// Submodules will be added in Phase 2 (Steps 2.1–2.10)

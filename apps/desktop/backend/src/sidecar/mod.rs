//! Sidecar process management — local inference/voice servers spawned as
//! Tauri sidecars (llama.cpp chat/embedding/summarizer, whisper STT, optional
//! MLX Python backends) plus the CLI-tool path trackers (image/tts).
//!
//! This `mod.rs` is a façade: it declares the submodules and re-exports the
//! stable public API so existing `crate::sidecar::*` paths (including the
//! registered `#[tauri::command]` functions) keep resolving unchanged.
//!
//! - `types`: process handle, launch options, serialized events/DTOs
//! - `core`: `SidecarManager` definition, construction, accessors, status
//! - `lifecycle`: process spawn/teardown for each managed server
//! - `commands`: Tauri command surface + reusable embedding-start core

mod commands;
mod core;
mod lifecycle;
mod types;

pub use commands::*;
pub use core::SidecarManager;
pub use types::{ChatServerConfig, ChatServerOptions, SidecarEvent, SidecarProcess, SidecarStatus};

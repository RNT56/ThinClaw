//! Configuration module for ThinClawEngine
//!
//! Split into focused submodules:
//! - `types`: All configuration structs and data types
//! - `identity`: Identity management, persistence, and API key updates
//! - `engine`: Engine config generation, file I/O, and data migration
//! - `keychain`: macOS Keychain integration for encrypted API key storage

mod engine;
mod identity;
pub mod keychain;
mod types;

pub use types::*;

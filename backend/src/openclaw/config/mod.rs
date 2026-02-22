//! Configuration module for OpenClawEngine
//!
//! Split into focused submodules:
//! - `types`: All configuration structs and data types
//! - `identity`: Identity management, persistence, and API key updates
//! - `engine`: Engine config generation, file I/O, and data migration

mod engine;
mod identity;
mod types;

pub use types::*;

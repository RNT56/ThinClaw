//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.
//!
//! `skills` is the canonical implementation used by `server.rs`.
//! All other endpoint handlers live inline in `server.rs`.

pub mod skills;

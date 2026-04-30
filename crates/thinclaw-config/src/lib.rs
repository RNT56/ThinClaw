//! Runtime configuration crate placeholder.
//!
//! The root crate still owns resolved configuration while dependency cycles are
//! removed from settings, secrets, tools, and channels.

pub mod formats;
pub mod mdns_discovery;
pub mod model_compat;
pub mod network_modes;
pub mod provider_catalog;
pub mod watcher;

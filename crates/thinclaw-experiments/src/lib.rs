//! Experiments subsystem: records/DTOs, lifecycle policy, opportunity derivation,
//! cost attribution, and operator messages.
//!
//! This is a façade: each concern lives in its own submodule and is re-exported
//! here so existing `thinclaw_experiments::*` import paths are unchanged.

mod cost;
mod messages;
mod opportunities;
mod policy;
mod support;
mod types;

pub use cost::*;
pub use messages::*;
pub use opportunities::*;
pub use policy::*;
pub use support::*;
pub use types::*;

#[cfg(test)]
mod tests;

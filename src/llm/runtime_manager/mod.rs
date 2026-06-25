//! LLM runtime manager: the live provider-resolution and routing hot path.
//!
//! This module is a façade over focused submodules. It declares them and
//! re-exports the stable public API so external callers keep importing
//! `crate::llm::runtime_manager::{...}` (and the `crate::llm::{...}` aliases in
//! `llm/mod.rs`) unchanged.
//!
//! Submodule responsibilities:
//! - [`types`]: public DTOs (`RuntimeStatus`, `RouteSimulation*`) and the
//!   internal runtime value types (snapshot, roles, resolved route).
//! - [`manager`]: `LlmRuntimeManager` struct + construction, status, hot reload,
//!   and provider handles.
//! - [`provider`]: the `RuntimeLlmProvider` `LlmProvider` adapter — request
//!   resolution, metadata shaping, and cascade escalation.
//! - [`routing`]: route-target resolution, provider-chain building/caching,
//!   route-health tracking, and pricing.
//! - [`simulation`]: read-only route simulation and advisor-readiness probes.
//! - [`credentials`]: hydrating provider API keys from the encrypted secrets
//!   store.
//! - [`provider_build`]: provider construction and routing-policy assembly.
//! - [`provider_slots`]: provider-slot algebra (selector parsing, role/cost
//!   resolution, pool ordering).
//! - [`settings_defaults`]: deriving and validating runtime `ProvidersSettings`.

mod credentials;
mod manager;
mod provider;
mod provider_build;
mod provider_slots;
mod routing;
mod settings_defaults;
mod simulation;
mod types;

#[cfg(test)]
mod tests;

pub use credentials::hydrate_runtime_credentials_from_secrets;
pub use manager::LlmRuntimeManager;
pub use settings_defaults::{
    derive_runtime_defaults, normalize_providers_settings, validate_providers_settings,
};
pub use types::{RouteSimulationResult, RouteSimulationScore, RuntimeLlmProvider, RuntimeStatus};

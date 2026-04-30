//! LLM runtime crate.
//!
//! Provider-neutral traits and DTOs are already extracted in
//! `thinclaw-llm-core`; provider implementations will move here next.

pub mod circuit_breaker;
pub mod cost_tracker;
pub mod costs;
pub mod discovery;
pub mod extended_context;
pub mod failover;
pub mod gemini;
pub mod llms_txt;
pub mod model_guidance;
pub mod model_metadata_sync;
pub mod provider_presets;
pub mod reasoning_tags;
pub mod response_cache;
pub mod response_cache_ext;
pub mod retry;
pub mod rig_adapter;
pub mod route_planner;
pub mod smart_routing;

pub use rig_adapter::RigAdapter;
pub use smart_routing::SmartRoutingProvider;
pub use thinclaw_llm_core::*;

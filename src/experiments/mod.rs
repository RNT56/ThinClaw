//! Compatibility facade for experiment domain types and runtime adapters.

pub mod adapters;
pub mod artifact_store;
pub mod runner;

pub use artifact_store::{ArtifactStore, LocalArtifactStore, default_artifact_root};
pub use thinclaw_experiments::*;

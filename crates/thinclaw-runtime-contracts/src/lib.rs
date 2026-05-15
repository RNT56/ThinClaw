//! Shared runtime contracts for ThinClaw clients and Desktop host services.
//!
//! This crate contains implementation-free DTOs only. Runtime-specific crates
//! own loading, persistence, process management, and permission checks.

pub mod asset;
pub mod direct;
pub mod model;
pub mod provider;
pub mod runtime;
pub mod secret;

pub use asset::{
    AssetKind, AssetNamespace, AssetOrigin, AssetRecord, AssetRef, AssetStatus, AssetVisibility,
};
pub use direct::{
    DirectAttachedDocument, DirectChatMessage, DirectChatPayload, DirectConversation,
    DirectStreamChunk, DirectTokenUsage,
};
pub use model::{
    ModelCapabilitySet, ModelCategory, ModelDescriptor, ModelDiscoveryResult, ModelPricing,
    ProviderDiscoveryResult,
};
pub use provider::{ApiStyle, ProviderEndpoint};
pub use runtime::{
    LocalRuntimeEndpoint, LocalRuntimeKind, LocalRuntimeSnapshot, RuntimeCapability,
    RuntimeExposurePolicy, RuntimeReadiness,
};
pub use secret::{
    ProviderCredentialDescriptor, SecretAccessMode, SecretConsumer, SecretDescriptor,
    canonical_secret_name, legacy_secret_aliases,
};

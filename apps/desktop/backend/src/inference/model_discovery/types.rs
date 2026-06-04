//! Shared model discovery contracts.
//!
//! Desktop Direct Workbench uses the implementation-free contracts crate for
//! provider/model metadata so WebUI, Desktop, and future mobile clients do not
//! drift into separate wire shapes.

pub use thinclaw_runtime_contracts::{
    ModelCategory, ModelDescriptor as CloudModelEntry, ModelDiscoveryResult as DiscoveryResult,
    ModelPricing, ProviderDiscoveryResult,
};

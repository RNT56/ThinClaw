//! Agent run artifact compatibility facade and root runtime adapters.

pub use thinclaw_agent::run_artifact::*;

pub fn run_runtime_descriptor(
    runtime: &crate::tools::execution_backend::RuntimeDescriptor,
) -> RunRuntimeDescriptor {
    RunRuntimeDescriptor::new(
        runtime.execution_backend.clone(),
        runtime.runtime_family.clone(),
        runtime.runtime_mode.clone(),
        runtime.runtime_capabilities.clone(),
        runtime.network_isolation.clone(),
    )
}

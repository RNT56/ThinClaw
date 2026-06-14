//! Durable GitHub repo project supervision primitives.
//!
//! This module contains runtime pieces used by the project supervisor. The
//! shared data model lives in `thinclaw-repo-projects` once that crate is
//! wired into the workspace.

pub mod ci;
pub mod executor;
pub mod github;
pub mod github_provider;
pub mod merge_gate;
pub mod pipeline;
pub mod prompts;
pub mod supervisor;
pub mod workspace;

#[cfg(all(test, feature = "libsql"))]
mod pipeline_tests;

/// Shallow-merge the keys of `patch` (an object) into `current` (treated as an
/// object), returning a new JSON object value. Non-object inputs are treated as
/// empty objects. Shared by the executor and the GitHub pipeline so task
/// metadata accumulates consistently across subsystems.
pub(crate) fn merge_metadata(
    current: &serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    let mut root = current.as_object().cloned().unwrap_or_default();
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            root.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(root)
}

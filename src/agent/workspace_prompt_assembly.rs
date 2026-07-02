//! Removed: `RootWorkspacePromptAssemblyPort` and `WorkspacePromptAssemblyPort`.
//!
//! This adapter used to implement `thinclaw_agent::ports::WorkspacePromptAssemblyPort`,
//! but `load_prompt_materials` always returned empty materials and a grep
//! audit confirmed there was no production caller of the port's methods
//! anywhere in the codebase (only the now-removed `RootAgentRuntimePorts`
//! wiring in `src/agent/root_ports.rs` referenced it, and nothing ever read
//! that field). Keeping it around risked a future caller silently receiving
//! an empty prompt (no workspace identity, no recall, no skills) instead of
//! a hard failure.
//!
//! The port trait, `WorkspacePromptRequest`, and `WorkspacePromptAssembly`
//! DTOs were removed from `crates/thinclaw-agent/src/ports.rs`. The
//! `WorkspacePromptMaterials` DTO was kept because it is still consumed by
//! the live `assemble_workspace_prompt_materials` helper in
//! `crates/thinclaw-agent/src/prompt_assembly.rs`.
//!
//! The live prompt-assembly path for the dispatcher is unrelated to this
//! removed port: see `assemble_dispatcher_prompt_materials` /
//! `DispatcherPromptMaterials` in `crates/thinclaw-agent/src/prompt_assembly.rs`,
//! wired from `src/agent/dispatcher/prompt_context.rs`.
//!
//! This module is intentionally left as an empty placeholder (rather than
//! deleted outright) because file deletion is out of scope for the change
//! that removed its contents; it is no longer declared in `src/agent/mod.rs`
//! and is not compiled.

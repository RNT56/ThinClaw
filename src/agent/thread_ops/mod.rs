//! Thread and session operations for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate thread management (user input
//! processing, undo/redo, approval, auth, persistence) from the core loop.
//!
//! This module is a façade for a set of focused submodules, each of which adds
//! a cohesive group of inherent methods to [`crate::agent::Agent`]:
//!
//! - [`hydration`]: thread visibility, hydration from persisted history, and
//!   persisted-subagent resume.
//! - [`compaction_context`]: post-compaction context-fragment assembly and
//!   transient runtime cleanup.
//! - [`persistence`]: message/runtime-snapshot persistence, learning-event
//!   recording, and context-pressure tracking.
//! - [`input`]: the interactive user-input turn driver.
//! - [`approval`]: the tool-approval flow, deferred-tool replay, and auth
//!   interception/token handling.
//! - [`lifecycle`]: undo/redo/resume, interrupt, compact, clear, and
//!   new/switch thread operations.
//!
//! All methods stay scoped to `crate::agent`; nothing here widens the public
//! API surface.

mod approval;
mod compaction_context;
mod hydration;
mod input;
mod lifecycle;
mod persistence;

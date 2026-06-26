//! LLM onboarding: inference provider, model selection, smart routing,
//! fallback, and embeddings.
//!
//! This is a façade module. The behaviour is implemented as `impl SetupWizard`
//! blocks split across cohesive submodules:
//!
//! - [`steps`] — top-level step entry points called by the flow dispatcher
//! - [`providers`] — provider credential/slot bookkeeping and per-provider setup
//! - [`models`] — model discovery/selection and the AI-stack summary
//!
//! Methods shared between these submodules (and the few reached from sibling
//! wizard steps) are scoped to `pub(in crate::setup::wizard)` rather than
//! widened to `pub`.

mod models;
mod providers;
mod steps;

//! Compatibility alias for the canonical PostgreSQL workspace repository.
//!
//! Keeping a second copy of the workspace SQL allowed the two database
//! contracts to drift. The implementation now lives in `thinclaw-workspace`,
//! and both the direct workspace surface and `thinclaw-db::PgBackend` use it.

pub(crate) use thinclaw_workspace::repository::Repository;

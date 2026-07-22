//! Portable, encrypted whole-agent backup bundles.
//!
//! A *bundle* is a single encrypted file that carries an agent's exportable
//! state — configuration, workspace files, and a database payload — so it can
//! be moved between machines or kept as an offline backup. The format is:
//!
//! ```text
//! passphrase ─scrypt─► key ─XChaCha20Poly1305─► seal( gzip( tar( manifest + sections ) ) )
//! ```
//!
//! This crate owns the root-independent, fully-testable core: the AEAD
//! [`envelope`], the [`manifest`] schema, and [`bundle`] assembly/extraction
//! with path-traversal-safe restore. Gathering the actual state (querying the
//! database, reading the workspace) is the caller's job — the root CLI wires
//! those in and hands byte payloads to [`bundle::BundleWriter`].

pub mod bundle;
pub mod envelope;
pub mod error;
pub mod manifest;

pub use bundle::{BundleWriter, MAX_SEALED_BUNDLE_BYTES, OpenBundle};
pub use error::{PortabilityError, Result};
pub use manifest::{BundleManifest, BundleSection, MANIFEST_VERSION, SectionKind};

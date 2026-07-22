//! The bundle manifest: a versioned, self-describing index of everything a
//! bundle carries, with per-section checksums for integrity verification.

use serde::{Deserialize, Serialize};

/// Current bundle manifest schema version. Bumped on incompatible changes.
pub const MANIFEST_VERSION: u16 = 1;

/// The archive path where the manifest itself is stored inside the tar payload.
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// What kind of state a section holds. Restore uses this to route each section
/// to the right handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// Logical, portable configuration (settings key/value JSON).
    Config,
    /// A file tree rooted at the section's archive prefix (e.g. the workspace).
    Files,
    /// A backend-appropriate database payload (LibSQL snapshot or `pg_dump`).
    Database,
    /// Free-form metadata not covered by the above.
    Metadata,
}

/// One logical unit of exported state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSection {
    /// Stable section name (unique within a bundle), e.g. `settings`,
    /// `workspace`, `database`.
    pub name: String,
    /// What kind of state this is.
    pub kind: SectionKind,
    /// Archive path (for single-blob sections) or path prefix (for `Files`
    /// trees) inside the tar payload.
    pub archive_path: String,
    /// Uncompressed byte length of the section's payload (0 for `Files` trees,
    /// which span many entries).
    #[serde(default)]
    pub byte_len: u64,
    /// Lowercase hex SHA-256 of the section's payload. Empty for `Files` trees
    /// (each file is verified individually is out of scope; the whole archive is
    /// still AEAD-authenticated as one unit).
    #[serde(default)]
    pub sha256: String,
    /// Optional human-readable note (e.g. `"libsql snapshot"`, `"pg_dump -Fc"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The bundle manifest, serialized to `manifest.json` at the archive root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// Manifest schema version.
    pub manifest_version: u16,
    /// Version string of the ThinClaw build that produced the bundle.
    pub producer_version: String,
    /// RFC 3339 creation timestamp, stamped by the caller (kept out of the crate
    /// so the crate stays free of a clock dependency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// The sections carried by this bundle, in export order.
    pub sections: Vec<BundleSection>,
}

impl BundleManifest {
    pub fn new(producer_version: impl Into<String>) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            producer_version: producer_version.into(),
            created_at: None,
            sections: Vec::new(),
        }
    }

    /// Look up a section by name.
    pub fn section(&self, name: &str) -> Option<&BundleSection> {
        self.sections.iter().find(|s| s.name == name)
    }
}

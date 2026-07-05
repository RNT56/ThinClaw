//! Errors for portable bundle assembly, sealing, and restoration.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PortabilityError {
    #[error("bundle I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("bundle serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The file is not a ThinClaw bundle, or its header is corrupt.
    #[error("not a ThinClaw bundle: {0}")]
    BadFormat(String),

    /// The bundle format version is newer than this build understands.
    #[error("unsupported bundle version {found} (this build supports up to {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },

    /// Decryption failed: wrong passphrase, or the bundle was tampered with.
    /// These are deliberately indistinguishable (AEAD authentication failure).
    #[error("decryption failed: wrong passphrase or corrupted/tampered bundle")]
    Decryption,

    /// Key derivation failed (e.g. invalid scrypt parameters).
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    /// A bundle entry declared a path that would escape the extraction root
    /// (absolute path or `..` traversal). Never extracted.
    #[error("unsafe archive path rejected: {0}")]
    UnsafePath(String),

    /// A section's contents did not match the checksum recorded in the manifest.
    #[error("integrity check failed for section '{0}': checksum mismatch")]
    ChecksumMismatch(String),

    /// The manifest referenced a section that is absent from the archive.
    #[error("bundle is missing declared section '{0}'")]
    MissingSection(String),
}

pub type Result<T> = std::result::Result<T, PortabilityError>;

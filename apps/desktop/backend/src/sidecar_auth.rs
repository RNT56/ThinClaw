//! Backend-only ephemeral authentication for managed loopback sidecars.

use std::io::Write as _;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(any(feature = "mlx", feature = "vllm", test))]
use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

/// Opaque per-launch credential. It is intentionally non-cloneable,
/// non-serializable, zeroizing, and redacted in diagnostics.
pub struct EphemeralSidecarAuth(Zeroizing<String>);

impl std::fmt::Debug for EphemeralSidecarAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EphemeralSidecarAuth([REDACTED])")
    }
}

impl EphemeralSidecarAuth {
    #[cfg(any(feature = "mlx", feature = "vllm", test))]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(Zeroizing::new(hex::encode(bytes)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn from_value(value: String) -> Result<Self, String> {
        if !(32..=4096).contains(&value.len()) || !value.is_ascii() {
            return Err("Sidecar credential must contain 32..=4096 ASCII bytes".to_string());
        }
        Ok(Self(Zeroizing::new(value)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarAuthSink {
    #[allow(dead_code)]
    InheritedPipe,
    PrivateFile,
}

/// Owner-private, single-link auth artifact. `TempPath` provides exactly-once
/// cleanup on every Rust-side failure path; child adapters unlink it as soon as
/// they have read the credential.
pub struct PrivateSidecarAuthFile {
    path: tempfile::TempPath,
}

impl std::fmt::Debug for PrivateSidecarAuthFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrivateSidecarAuthFile")
            .field("sink", &SidecarAuthSink::PrivateFile)
            .finish_non_exhaustive()
    }
}

impl PrivateSidecarAuthFile {
    pub fn create(parent: &Path, auth: &EphemeralSidecarAuth) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| format!("Could not inspect sidecar auth directory: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Sidecar auth directory must be a real directory".to_string());
        }
        let mut file = tempfile::Builder::new()
            .prefix(".thinclaw-sidecar-auth-")
            .tempfile_in(parent)
            .map_err(|error| format!("Could not create private sidecar auth file: {error}"))?;
        file.write_all(auth.expose().as_bytes())
            .and_then(|()| file.flush())
            .and_then(|()| file.as_file().sync_all())
            .map_err(|error| format!("Could not publish private sidecar auth file: {error}"))?;
        Ok(Self {
            path: file.into_temp_path(),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    #[cfg(any(feature = "mlx", feature = "vllm"))]
    pub fn consumed(&self) -> bool {
        !self.path.exists()
    }

    pub fn remove(self) -> Result<(), String> {
        self.path
            .close()
            .map_err(|error| format!("Could not remove private sidecar auth file: {error}"))
    }

    #[cfg(test)]
    pub fn path_buf(&self) -> PathBuf {
        self.path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_is_redacted_private_and_removed() {
        let directory = tempfile::tempdir().unwrap();
        let auth = EphemeralSidecarAuth::generate();
        assert!(!format!("{auth:?}").contains(auth.expose()));
        let file = PrivateSidecarAuthFile::create(directory.path(), &auth).unwrap();
        let path = file.path_buf();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), auth.expose());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
        file.remove().unwrap();
        assert!(!path.exists());
    }
}

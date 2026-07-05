//! Bundle assembly and extraction: a gzip-compressed tar of a manifest plus
//! named sections, sealed as one AEAD unit by [`crate::envelope`].
//!
//! `BundleWriter` accumulates sections (single blobs or file trees), then
//! `finish` embeds the manifest, compresses, and seals. `OpenBundle` reverses
//! it: decrypt, decompress, index every tar entry, and expose checksum-verified
//! section reads plus path-traversal-safe file-tree extraction.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

use crate::envelope;
use crate::error::{PortabilityError, Result};
use crate::manifest::{BundleManifest, BundleSection, MANIFEST_ENTRY, SectionKind};

/// Builds a bundle from sections, then seals it under a passphrase.
pub struct BundleWriter {
    manifest: BundleManifest,
    builder: tar::Builder<GzEncoder<Vec<u8>>>,
}

impl BundleWriter {
    pub fn new(producer_version: impl Into<String>) -> Self {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        Self {
            manifest: BundleManifest::new(producer_version),
            builder: tar::Builder::new(encoder),
        }
    }

    /// Set the manifest's `created_at` (RFC 3339). Kept caller-supplied so the
    /// crate needs no clock dependency.
    pub fn created_at(mut self, timestamp: impl Into<String>) -> Self {
        self.manifest.created_at = Some(timestamp.into());
        self
    }

    /// Add a single-blob section (config JSON, a database dump, metadata). The
    /// blob is checksummed so restore can verify integrity.
    pub fn add_blob(
        &mut self,
        name: impl Into<String>,
        kind: SectionKind,
        archive_path: impl Into<String>,
        bytes: &[u8],
        note: Option<String>,
    ) -> Result<()> {
        let archive_path = archive_path.into();
        append_bytes(&mut self.builder, &archive_path, bytes)?;
        self.manifest.sections.push(BundleSection {
            name: name.into(),
            kind,
            archive_path,
            byte_len: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            note,
        });
        Ok(())
    }

    /// Add a file-tree section: every regular file under `dir` is stored under
    /// `archive_prefix/<relative path>`. Symlinks are skipped (they could escape
    /// the tree on restore). `skip` is called with each entry's path *relative to
    /// `dir`*; returning `true` excludes that file — or, for a directory, its
    /// whole subtree (e.g. to omit `logs/`, `.env`, or the live database file).
    /// Returns the number of files added.
    pub fn add_dir(
        &mut self,
        name: impl Into<String>,
        archive_prefix: impl Into<String>,
        dir: &Path,
        skip: &dyn Fn(&Path) -> bool,
    ) -> Result<usize> {
        let archive_prefix = archive_prefix.into();
        let mut files = Vec::new();
        collect_files(dir, dir, skip, &mut files)?;
        files.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic order
        for (rel, bytes) in &files {
            let archive_path = format!("{archive_prefix}/{rel}");
            append_bytes(&mut self.builder, &archive_path, bytes)?;
        }
        let count = files.len();
        self.manifest.sections.push(BundleSection {
            name: name.into(),
            kind: SectionKind::Files,
            archive_path: archive_prefix,
            byte_len: 0,
            sha256: String::new(),
            note: Some(format!("{count} files")),
        });
        Ok(count)
    }

    /// Embed the manifest, finalize compression, and seal under `passphrase`.
    pub fn finish(mut self, passphrase: &str) -> Result<Vec<u8>> {
        let manifest_bytes = serde_json::to_vec_pretty(&self.manifest)?;
        append_bytes(&mut self.builder, MANIFEST_ENTRY, &manifest_bytes)?;
        let encoder = self.builder.into_inner()?;
        let compressed = encoder.finish()?;
        envelope::seal(passphrase, &compressed)
    }
}

/// A decrypted, indexed bundle ready to read sections from and restore.
#[derive(Debug)]
pub struct OpenBundle {
    manifest: BundleManifest,
    entries: BTreeMap<String, Vec<u8>>,
}

impl OpenBundle {
    /// Decrypt and index a sealed bundle. Fails with
    /// [`PortabilityError::Decryption`] on a wrong passphrase or tampering.
    pub fn open(sealed: &[u8], passphrase: &str) -> Result<Self> {
        let compressed = envelope::open(passphrase, sealed)?;
        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut tar_bytes = Vec::new();
        decoder.read_to_end(&mut tar_bytes)?;

        let mut entries = BTreeMap::new();
        let mut archive = tar::Archive::new(&tar_bytes[..]);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry
                .path()?
                .to_str()
                .ok_or_else(|| PortabilityError::BadFormat("non-UTF-8 archive path".to_string()))?
                .to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            entries.insert(path, bytes);
        }

        let manifest_bytes = entries
            .get(MANIFEST_ENTRY)
            .ok_or_else(|| PortabilityError::BadFormat("bundle has no manifest".to_string()))?;
        let manifest: BundleManifest = serde_json::from_slice(manifest_bytes)?;
        if manifest.manifest_version > crate::manifest::MANIFEST_VERSION {
            return Err(PortabilityError::UnsupportedVersion {
                found: manifest.manifest_version,
                supported: crate::manifest::MANIFEST_VERSION,
            });
        }
        Ok(Self { manifest, entries })
    }

    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    /// Read a single-blob section's bytes, verifying its checksum against the
    /// manifest.
    pub fn section_bytes(&self, name: &str) -> Result<&[u8]> {
        let section = self
            .manifest
            .section(name)
            .ok_or_else(|| PortabilityError::MissingSection(name.to_string()))?;
        let bytes = self
            .entries
            .get(&section.archive_path)
            .ok_or_else(|| PortabilityError::MissingSection(name.to_string()))?;
        if !section.sha256.is_empty() && sha256_hex(bytes) != section.sha256 {
            return Err(PortabilityError::ChecksumMismatch(name.to_string()));
        }
        Ok(bytes)
    }

    /// Restore a `Files` section into `dest`, recreating the tree under the
    /// section's archive prefix. Rejects any entry whose path would escape
    /// `dest` (absolute or `..`). Returns the number of files written.
    pub fn extract_files(&self, name: &str, dest: &Path) -> Result<usize> {
        let section = self
            .manifest
            .section(name)
            .ok_or_else(|| PortabilityError::MissingSection(name.to_string()))?;
        let prefix = format!("{}/", section.archive_path);
        let mut written = 0;
        for (path, bytes) in &self.entries {
            let Some(rel) = path.strip_prefix(&prefix) else {
                continue;
            };
            if rel.is_empty() {
                continue;
            }
            let safe = safe_relative(rel)?;
            let target = dest.join(&safe);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, bytes)?;
            written += 1;
        }
        Ok(written)
    }
}

/// Append raw bytes as a tar entry at `archive_path` (long paths and checksum
/// handled by `append_data`).
fn append_bytes(
    builder: &mut tar::Builder<GzEncoder<Vec<u8>>>,
    archive_path: &str,
    bytes: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    builder.append_data(&mut header, archive_path, bytes)?;
    Ok(())
}

/// Recursively collect regular files under `root`, keyed by their `/`-joined
/// path relative to `root`. Skips symlinks and anything `skip` excludes.
fn collect_files(
    root: &Path,
    dir: &Path,
    skip: &dyn Fn(&Path) -> bool,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| PortabilityError::BadFormat("path outside root".to_string()))?;
        if skip(rel) {
            continue;
        }
        if file_type.is_dir() {
            collect_files(root, &path, skip, out)?;
        } else if file_type.is_file() {
            let rel_str = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => s.to_str(),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            if rel_str.is_empty() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            out.push((rel_str, bytes));
        }
    }
    Ok(())
}

/// Validate that a relative archive path stays inside the extraction root:
/// only `Normal` components are allowed (no root, prefix, or `..`).
fn safe_relative(rel: &str) -> Result<PathBuf> {
    let candidate = Path::new(rel);
    let mut safe = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PortabilityError::UnsafePath(rel.to_string()));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(PortabilityError::UnsafePath(rel.to_string()));
    }
    Ok(safe)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trip_with_checksum() {
        let mut writer = BundleWriter::new("test-1.0");
        writer
            .add_blob(
                "settings",
                SectionKind::Config,
                "settings.json",
                b"{\"k\":\"v\"}",
                None,
            )
            .unwrap();
        writer
            .add_blob(
                "database",
                SectionKind::Database,
                "db/dump.bin",
                b"\x00\x01\x02DUMP",
                Some("libsql snapshot".to_string()),
            )
            .unwrap();
        let sealed = writer.finish("pw").unwrap();

        let opened = OpenBundle::open(&sealed, "pw").unwrap();
        assert_eq!(opened.manifest().producer_version, "test-1.0");
        assert_eq!(opened.section_bytes("settings").unwrap(), b"{\"k\":\"v\"}");
        assert_eq!(
            opened.section_bytes("database").unwrap(),
            b"\x00\x01\x02DUMP"
        );
        assert_eq!(
            opened
                .manifest()
                .section("database")
                .unwrap()
                .note
                .as_deref(),
            Some("libsql snapshot")
        );
    }

    #[test]
    fn wrong_passphrase_cannot_open() {
        let mut writer = BundleWriter::new("v");
        writer
            .add_blob("s", SectionKind::Config, "s.json", b"data", None)
            .unwrap();
        let sealed = writer.finish("right").unwrap();
        assert!(matches!(
            OpenBundle::open(&sealed, "wrong").unwrap_err(),
            PortabilityError::Decryption
        ));
    }

    #[test]
    fn file_tree_round_trips() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("skills/rust")).unwrap();
        std::fs::write(src.path().join("MEMORY.md"), b"# memory").unwrap();
        std::fs::write(src.path().join("skills/rust/SKILL.md"), b"skill").unwrap();

        let mut writer = BundleWriter::new("v");
        let count = writer
            .add_dir("workspace", "workspace", src.path(), &|_| false)
            .unwrap();
        assert_eq!(count, 2);
        let sealed = writer.finish("pw").unwrap();

        let opened = OpenBundle::open(&sealed, "pw").unwrap();
        let dest = tempfile::tempdir().unwrap();
        let written = opened.extract_files("workspace", dest.path()).unwrap();
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read(dest.path().join("MEMORY.md")).unwrap(),
            b"# memory"
        );
        assert_eq!(
            std::fs::read(dest.path().join("skills/rust/SKILL.md")).unwrap(),
            b"skill"
        );
    }

    #[test]
    fn checksum_mismatch_is_detected() {
        // Tamper with a section's recorded checksum and confirm read rejects it.
        let mut writer = BundleWriter::new("v");
        writer
            .add_blob("s", SectionKind::Config, "s.json", b"payload", None)
            .unwrap();
        let sealed = writer.finish("pw").unwrap();
        let mut opened = OpenBundle::open(&sealed, "pw").unwrap();
        // Corrupt the manifest's checksum in memory.
        opened.manifest.sections[0].sha256 = "deadbeef".to_string();
        assert!(matches!(
            opened.section_bytes("s").unwrap_err(),
            PortabilityError::ChecksumMismatch(_)
        ));
    }

    #[test]
    fn add_dir_skip_excludes_subtrees_and_files() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("logs")).unwrap();
        std::fs::create_dir_all(src.path().join("skills")).unwrap();
        std::fs::write(src.path().join("logs/app.log"), b"noise").unwrap();
        std::fs::write(src.path().join(".env"), b"SECRET=1").unwrap();
        std::fs::write(src.path().join("thinclaw.db"), b"livedb").unwrap();
        std::fs::write(src.path().join("settings.json"), b"{}").unwrap();
        std::fs::write(src.path().join("skills/SKILL.md"), b"keep").unwrap();

        let skip = |rel: &Path| {
            let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
            let name = rel.file_name().and_then(|n| n.to_str());
            first == Some("logs") || name == Some(".env") || name == Some("thinclaw.db")
        };
        let mut writer = BundleWriter::new("v");
        let count = writer
            .add_dir("workspace", "workspace", src.path(), &skip)
            .unwrap();
        assert_eq!(count, 2, "only settings.json and skills/SKILL.md kept");

        let sealed = writer.finish("pw").unwrap();
        let opened = OpenBundle::open(&sealed, "pw").unwrap();
        let dest = tempfile::tempdir().unwrap();
        opened.extract_files("workspace", dest.path()).unwrap();
        assert!(dest.path().join("settings.json").exists());
        assert!(dest.path().join("skills/SKILL.md").exists());
        assert!(!dest.path().join("logs/app.log").exists());
        assert!(!dest.path().join(".env").exists());
        assert!(!dest.path().join("thinclaw.db").exists());
    }

    #[test]
    fn safe_relative_rejects_traversal() {
        assert!(safe_relative("../etc/passwd").is_err());
        assert!(safe_relative("a/../../b").is_err());
        assert!(safe_relative("/abs/path").is_err());
        assert!(safe_relative("ok/nested/file.txt").is_ok());
    }

    #[test]
    fn missing_section_errors() {
        let mut writer = BundleWriter::new("v");
        writer
            .add_blob("s", SectionKind::Config, "s.json", b"x", None)
            .unwrap();
        let sealed = writer.finish("pw").unwrap();
        let opened = OpenBundle::open(&sealed, "pw").unwrap();
        assert!(matches!(
            opened.section_bytes("nope").unwrap_err(),
            PortabilityError::MissingSection(_)
        ));
    }
}

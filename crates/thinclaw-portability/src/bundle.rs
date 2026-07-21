//! Bundle assembly and extraction: a gzip-compressed tar of a manifest plus
//! named sections, sealed as one AEAD unit by [`crate::envelope`].
//!
//! `BundleWriter` accumulates sections (single blobs or file trees), then
//! `finish` embeds the manifest, compresses, and seals. `OpenBundle` reverses
//! it: decrypt, decompress, index every tar entry, and expose checksum-verified
//! section reads plus path-traversal-safe file-tree extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

/// Hard ceilings keep the current in-memory bundle format from becoming an
/// allocation or decompression bomb. A future streaming format can raise these
/// without weakening existing importers.
pub const MAX_SEALED_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_TREE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_SOURCE_NODES: usize = 200_000;
const MAX_ARCHIVE_PATH_BYTES: usize = 4 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_SECTIONS: usize = 128;
const MAX_DIRECTORY_DEPTH: usize = 128;

use crate::envelope;
use crate::error::{PortabilityError, Result};
use crate::manifest::{BundleManifest, BundleSection, MANIFEST_ENTRY, SectionKind};

/// Builds a bundle from sections, then seals it under a passphrase.
pub struct BundleWriter {
    manifest: BundleManifest,
    builder: tar::Builder<GzEncoder<Vec<u8>>>,
    entry_count: usize,
    unpacked_bytes: u64,
}

impl BundleWriter {
    pub fn new(producer_version: impl Into<String>) -> Self {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        Self {
            manifest: BundleManifest::new(producer_version),
            builder: tar::Builder::new(encoder),
            entry_count: 0,
            unpacked_bytes: 0,
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
        let name = name.into();
        let archive_path = archive_path.into();
        validate_new_section(&self.manifest, &name, kind, &archive_path, note.as_deref())?;
        self.append_tracked(&archive_path, bytes)?;
        self.manifest.sections.push(BundleSection {
            name,
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
        let name = name.into();
        let archive_prefix = archive_prefix.into();
        validate_new_section(
            &self.manifest,
            &name,
            SectionKind::Files,
            &archive_prefix,
            None,
        )?;
        let root_metadata = std::fs::symlink_metadata(dir)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(PortabilityError::BadFormat(
                "backup source is not a real directory".to_string(),
            ));
        }
        let root = Dir::open_ambient_dir(dir, ambient_authority())?;
        let mut files = Vec::new();
        let remaining_entries = MAX_ARCHIVE_ENTRIES.saturating_sub(self.entry_count + 1);
        let remaining_bytes = MAX_FILE_TREE_BYTES.saturating_sub(self.unpacked_bytes);
        let mut budget = CollectionBudget {
            remaining_entries,
            remaining_bytes,
            remaining_nodes: MAX_SOURCE_NODES,
        };
        collect_files(&root, Path::new(""), skip, &mut files, &mut budget, 0)?;
        files.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic order
        for (rel, bytes) in &files {
            let archive_path = format!("{archive_prefix}/{rel}");
            self.append_tracked(&archive_path, bytes)?;
        }
        let count = files.len();
        self.manifest.sections.push(BundleSection {
            name,
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
        validate_manifest(&self.manifest)?;
        let manifest_bytes = serde_json::to_vec_pretty(&self.manifest)?;
        if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(PortabilityError::BadFormat(
                "bundle manifest exceeds the supported size".to_string(),
            ));
        }
        self.append_tracked(MANIFEST_ENTRY, &manifest_bytes)?;
        let encoder = self.builder.into_inner()?;
        let compressed = encoder.finish()?;
        if compressed.len() as u64 > MAX_SEALED_BUNDLE_BYTES {
            return Err(PortabilityError::BadFormat(
                "compressed bundle exceeds the supported size".to_string(),
            ));
        }
        let sealed = envelope::seal(passphrase, &compressed)?;
        if sealed.len() as u64 > MAX_SEALED_BUNDLE_BYTES {
            return Err(PortabilityError::BadFormat(
                "sealed bundle exceeds the supported size".to_string(),
            ));
        }
        Ok(sealed)
    }

    fn append_tracked(&mut self, archive_path: &str, bytes: &[u8]) -> Result<()> {
        validate_archive_path(archive_path)?;
        if archive_path != MANIFEST_ENTRY
            && self.manifest.sections.iter().any(|section| {
                section.kind != SectionKind::Files && section.archive_path == archive_path
            })
        {
            return Err(PortabilityError::BadFormat(format!(
                "duplicate archive path '{archive_path}'"
            )));
        }
        let byte_len = u64::try_from(bytes.len()).map_err(|_| {
            PortabilityError::BadFormat("bundle entry size does not fit this platform".to_string())
        })?;
        if byte_len > MAX_ENTRY_BYTES
            || self.entry_count >= MAX_ARCHIVE_ENTRIES
            || self.unpacked_bytes.saturating_add(byte_len) > MAX_UNPACKED_BYTES
        {
            return Err(PortabilityError::BadFormat(
                "bundle entry, count, or aggregate size exceeds supported limits".to_string(),
            ));
        }
        append_bytes(&mut self.builder, archive_path, bytes)?;
        self.entry_count += 1;
        self.unpacked_bytes += byte_len;
        Ok(())
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
        if sealed.len() as u64 > MAX_SEALED_BUNDLE_BYTES {
            return Err(PortabilityError::BadFormat(
                "sealed bundle exceeds the supported size".to_string(),
            ));
        }
        let compressed = envelope::open(passphrase, sealed)?;
        let mut entries = BTreeMap::new();
        let decoder = GzDecoder::new(&compressed[..]).take(MAX_UNPACKED_BYTES.saturating_add(1));
        let mut archive = tar::Archive::new(decoder);
        let mut unpacked_bytes = 0_u64;
        let mut entry_count = 0_usize;
        for entry in archive.entries()? {
            let mut entry = entry?;
            entry_count = entry_count.saturating_add(1);
            if entry_count > MAX_ARCHIVE_ENTRIES {
                return Err(PortabilityError::BadFormat(
                    "bundle contains too many archive entries".to_string(),
                ));
            }
            if !entry.header().entry_type().is_file() {
                return Err(PortabilityError::BadFormat(
                    "bundle contains a non-regular archive entry".to_string(),
                ));
            }
            let path = entry.path()?.into_owned();
            let path = path
                .to_str()
                .ok_or_else(|| PortabilityError::BadFormat("non-UTF-8 archive path".to_string()))?
                .to_string();
            validate_archive_path(&path)?;
            let declared_size = entry.size();
            let entry_limit = if path == MANIFEST_ENTRY {
                MAX_MANIFEST_BYTES
            } else {
                MAX_ENTRY_BYTES
            };
            if declared_size > entry_limit
                || unpacked_bytes.saturating_add(declared_size) > MAX_UNPACKED_BYTES
            {
                return Err(PortabilityError::BadFormat(
                    "bundle entry or aggregate size exceeds supported limits".to_string(),
                ));
            }
            let capacity = usize::try_from(declared_size).map_err(|_| {
                PortabilityError::BadFormat(
                    "bundle entry size does not fit this platform".to_string(),
                )
            })?;
            let mut bytes = Vec::with_capacity(capacity);
            entry
                .by_ref()
                .take(entry_limit.saturating_add(1))
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 != declared_size {
                return Err(PortabilityError::BadFormat(
                    "bundle entry length does not match its header".to_string(),
                ));
            }
            unpacked_bytes += declared_size;
            if entries.insert(path.clone(), bytes).is_some() {
                return Err(PortabilityError::BadFormat(format!(
                    "bundle contains duplicate archive path '{path}'"
                )));
            }
        }

        let mut decoder = archive.into_inner();
        let mut trailing = [0_u8; 8 * 1024];
        loop {
            let read = decoder.read(&mut trailing)?;
            if read == 0 {
                break;
            }
            if trailing[..read].iter().any(|byte| *byte != 0) {
                return Err(PortabilityError::BadFormat(
                    "bundle contains non-zero data after the tar end marker".to_string(),
                ));
            }
        }
        if decoder.limit() == 0 {
            return Err(PortabilityError::BadFormat(
                "decompressed bundle exceeds the supported size".to_string(),
            ));
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
        validate_manifest(&manifest)?;
        validate_manifest_entries(&manifest, &entries)?;
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
        if section.byte_len != bytes.len() as u64 {
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
        if section.kind != SectionKind::Files {
            return Err(PortabilityError::BadFormat(format!(
                "section '{name}' is not a file tree"
            )));
        }
        match std::fs::symlink_metadata(dest) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(PortabilityError::UnsafePath(dest.display().to_string()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(dest)?;
                let metadata = std::fs::symlink_metadata(dest)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(PortabilityError::UnsafePath(dest.display().to_string()));
                }
            }
            Err(error) => return Err(error.into()),
        }
        let root = Dir::open_ambient_dir(dest, ambient_authority())?;
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
            write_capability_rooted_file(&root, &safe, bytes)?;
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

struct CollectionBudget {
    remaining_entries: usize,
    remaining_bytes: u64,
    remaining_nodes: usize,
}

/// Recursively collect regular files through directory capabilities. Every
/// directory and file open rejects its final symlink component, and resolution
/// cannot escape the already-open source root.
fn collect_files(
    dir: &Dir,
    relative: &Path,
    skip: &dyn Fn(&Path) -> bool,
    out: &mut Vec<(String, Vec<u8>)>,
    budget: &mut CollectionBudget,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(PortabilityError::BadFormat(
            "backup source exceeds the supported directory depth".to_string(),
        ));
    }
    for entry in dir.entries()? {
        if budget.remaining_nodes == 0 {
            return Err(PortabilityError::BadFormat(
                "backup source contains too many filesystem entries".to_string(),
            ));
        }
        budget.remaining_nodes -= 1;
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_str().ok_or_else(|| {
            PortabilityError::BadFormat("backup source contains a non-UTF-8 path".to_string())
        })?;
        if name_text.is_empty() || name_text.chars().any(char::is_control) {
            return Err(PortabilityError::BadFormat(
                "backup source contains a malformed path".to_string(),
            ));
        }
        let rel = relative.join(&name);
        if skip(&rel) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let child = dir.open_dir_nofollow(&name)?;
            collect_files(&child, &rel, skip, out, budget, depth + 1)?;
        } else if file_type.is_file() {
            if budget.remaining_entries == 0 {
                return Err(PortabilityError::BadFormat(
                    "backup source contains too many files".to_string(),
                ));
            }
            let rel_str = portable_relative_path(&rel)?;
            validate_archive_path(&rel_str)?;
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let mut file = dir.open_with(&name, &options)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.len() > MAX_ENTRY_BYTES
                || metadata.len() > budget.remaining_bytes
            {
                return Err(PortabilityError::BadFormat(
                    "backup source file exceeds the supported size".to_string(),
                ));
            }
            let capacity = usize::try_from(metadata.len()).map_err(|_| {
                PortabilityError::BadFormat(
                    "backup source file size does not fit this platform".to_string(),
                )
            })?;
            let mut bytes = Vec::with_capacity(capacity);
            std::io::Read::by_ref(&mut file)
                .take(MAX_ENTRY_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 != metadata.len() {
                return Err(PortabilityError::BadFormat(
                    "backup source file changed while it was read".to_string(),
                ));
            }
            budget.remaining_entries -= 1;
            budget.remaining_bytes -= metadata.len();
            out.push((rel_str, bytes));
        }
    }
    Ok(())
}

fn portable_relative_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                PortabilityError::BadFormat("backup path is not UTF-8".to_string())
            })?),
            _ => {
                return Err(PortabilityError::BadFormat(
                    "backup path is not strictly relative".to_string(),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(PortabilityError::BadFormat(
            "backup path is empty".to_string(),
        ));
    }
    Ok(parts.join("/"))
}

/// Validate that a relative archive path stays inside the extraction root:
/// only `Normal` components are allowed (no root, prefix, or `..`).
fn safe_relative(rel: &str) -> Result<PathBuf> {
    let candidate = Path::new(rel);
    let mut safe = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(PortabilityError::UnsafePath(rel.to_string()));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(PortabilityError::UnsafePath(rel.to_string()));
    }
    Ok(safe)
}

fn validate_archive_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > MAX_ARCHIVE_PATH_BYTES
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path.split('/').any(str::is_empty)
    {
        return Err(PortabilityError::UnsafePath(path.to_string()));
    }
    let safe = safe_relative(path)?;
    if portable_relative_path(&safe)? != path {
        return Err(PortabilityError::UnsafePath(path.to_string()));
    }
    Ok(())
}

fn write_capability_rooted_file(root: &Dir, relative: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = relative
        .file_name()
        .ok_or_else(|| PortabilityError::UnsafePath(relative.display().to_string()))?;
    let mut parent = root.try_clone()?;
    if let Some(parent_path) = relative.parent() {
        for component in parent_path.components() {
            let Component::Normal(part) = component else {
                return Err(PortabilityError::UnsafePath(relative.display().to_string()));
            };
            match parent.create_dir(part) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            parent = parent
                .open_dir_nofollow(part)
                .map_err(|_| PortabilityError::UnsafePath(relative.display().to_string()))?;
        }
    }

    match parent.symlink_metadata(file_name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(PortabilityError::UnsafePath(relative.display().to_string()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let stage_name = format!(".thinclaw-restore-{}.tmp", uuid::Uuid::new_v4().simple());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let result = (|| -> Result<()> {
        let mut stage = parent.open_with(&stage_name, &options)?;
        if !stage.metadata()?.is_file() {
            return Err(PortabilityError::UnsafePath(stage_name.clone()));
        }
        stage.write_all(bytes)?;
        stage.sync_all()?;
        if stage.metadata()?.len() != bytes.len() as u64 {
            return Err(PortabilityError::BadFormat(
                "restored file changed while it was written".to_string(),
            ));
        }
        drop(stage);
        parent.rename(&stage_name, &parent, file_name)?;
        let installed = parent.symlink_metadata(file_name)?;
        if installed.file_type().is_symlink() || !installed.is_file() {
            return Err(PortabilityError::UnsafePath(relative.display().to_string()));
        }
        parent.try_clone()?.into_std_file().sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = parent.remove_file(&stage_name);
    }
    result
}

fn validate_new_section(
    manifest: &BundleManifest,
    name: &str,
    kind: SectionKind,
    archive_path: &str,
    note: Option<&str>,
) -> Result<()> {
    validate_section_basics(name, archive_path, note)?;
    if manifest.sections.len() >= MAX_MANIFEST_SECTIONS {
        return Err(PortabilityError::BadFormat(
            "bundle contains too many sections".to_string(),
        ));
    }
    if manifest.sections.iter().any(|section| section.name == name) {
        return Err(PortabilityError::BadFormat(format!(
            "duplicate bundle section name '{name}'"
        )));
    }
    for section in &manifest.sections {
        if section_namespaces_overlap(section.kind, &section.archive_path, kind, archive_path) {
            return Err(PortabilityError::BadFormat(format!(
                "bundle section archive namespace '{archive_path}' overlaps '{}'",
                section.archive_path
            )));
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &BundleManifest) -> Result<()> {
    if manifest.manifest_version == 0
        || manifest.manifest_version > crate::manifest::MANIFEST_VERSION
        || manifest.producer_version.is_empty()
        || manifest.producer_version.len() > 256
        || manifest.producer_version.chars().any(char::is_control)
        || manifest.sections.is_empty()
        || manifest.sections.len() > MAX_MANIFEST_SECTIONS
        || manifest.created_at.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
        })
    {
        return Err(PortabilityError::BadFormat(
            "bundle manifest metadata is malformed or unsupported".to_string(),
        ));
    }

    let mut names = BTreeSet::new();
    for (index, section) in manifest.sections.iter().enumerate() {
        validate_section_basics(
            &section.name,
            &section.archive_path,
            section.note.as_deref(),
        )?;
        if !names.insert(section.name.clone()) {
            return Err(PortabilityError::BadFormat(format!(
                "duplicate bundle section name '{}'",
                section.name
            )));
        }
        match section.kind {
            SectionKind::Files if section.byte_len != 0 || !section.sha256.is_empty() => {
                return Err(PortabilityError::BadFormat(format!(
                    "file-tree section '{}' has invalid blob metadata",
                    section.name
                )));
            }
            SectionKind::Files => {}
            _ if section.byte_len > MAX_ENTRY_BYTES
                || section.sha256.len() != 64
                || !section
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
            {
                return Err(PortabilityError::BadFormat(format!(
                    "blob section '{}' has invalid size or checksum metadata",
                    section.name
                )));
            }
            _ => {}
        }
        for previous in &manifest.sections[..index] {
            if section_namespaces_overlap(
                previous.kind,
                &previous.archive_path,
                section.kind,
                &section.archive_path,
            ) {
                return Err(PortabilityError::BadFormat(format!(
                    "bundle section archive namespace '{}' overlaps '{}'",
                    section.archive_path, previous.archive_path
                )));
            }
        }
    }
    Ok(())
}

fn validate_section_basics(name: &str, archive_path: &str, note: Option<&str>) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || archive_path == MANIFEST_ENTRY
        || note.is_some_and(|value| value.len() > 1_024 || value.chars().any(char::is_control))
    {
        return Err(PortabilityError::BadFormat(
            "bundle section metadata is malformed".to_string(),
        ));
    }
    validate_archive_path(archive_path)
}

fn section_namespaces_overlap(
    left_kind: SectionKind,
    left: &str,
    right_kind: SectionKind,
    right: &str,
) -> bool {
    left == right
        || left_kind == SectionKind::Files && right.starts_with(&format!("{left}/"))
        || right_kind == SectionKind::Files && left.starts_with(&format!("{right}/"))
}

fn validate_manifest_entries(
    manifest: &BundleManifest,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for section in &manifest.sections {
        match section.kind {
            SectionKind::Files => {
                let prefix = format!("{}/", section.archive_path);
                let total = entries
                    .iter()
                    .filter(|(path, _)| path.starts_with(&prefix))
                    .try_fold(0_u64, |total, (_, bytes)| {
                        total.checked_add(bytes.len() as u64).ok_or_else(|| {
                            PortabilityError::BadFormat(
                                "file-tree section size overflowed".to_string(),
                            )
                        })
                    })?;
                if total > MAX_FILE_TREE_BYTES {
                    return Err(PortabilityError::BadFormat(format!(
                        "file-tree section '{}' exceeds the supported size",
                        section.name
                    )));
                }
            }
            _ => {
                let bytes = entries
                    .get(&section.archive_path)
                    .ok_or_else(|| PortabilityError::MissingSection(section.name.clone()))?;
                if bytes.len() as u64 != section.byte_len || sha256_hex(bytes) != section.sha256 {
                    return Err(PortabilityError::ChecksumMismatch(section.name.clone()));
                }
            }
        }
    }

    for path in entries
        .keys()
        .filter(|path| path.as_str() != MANIFEST_ENTRY)
    {
        let owners = manifest
            .sections
            .iter()
            .filter(|section| match section.kind {
                SectionKind::Files => path.starts_with(&format!("{}/", section.archive_path)),
                _ => path.as_str() == section.archive_path,
            })
            .count();
        if owners != 1 {
            return Err(PortabilityError::BadFormat(format!(
                "archive entry '{path}' is not owned by exactly one manifest section"
            )));
        }
    }
    Ok(())
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
        assert!(validate_archive_path("a/./b").is_err());
        assert!(validate_archive_path("a\\..\\b").is_err());
        assert!(safe_relative("ok/nested/file.txt").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn extraction_rejects_symlinked_destination_component() {
        let mut entries = BTreeMap::new();
        entries.insert("workspace/link/escape.txt".to_string(), b"owned".to_vec());
        entries.insert(MANIFEST_ENTRY.to_string(), b"{}".to_vec());
        let bundle = OpenBundle {
            manifest: BundleManifest {
                manifest_version: crate::manifest::MANIFEST_VERSION,
                producer_version: "test".to_string(),
                created_at: None,
                sections: vec![BundleSection {
                    name: "workspace".to_string(),
                    kind: SectionKind::Files,
                    archive_path: "workspace".to_string(),
                    byte_len: 0,
                    sha256: String::new(),
                    note: None,
                }],
            },
            entries,
        };
        let destination = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), destination.path().join("link")).unwrap();

        assert!(
            bundle
                .extract_files("workspace", destination.path())
                .is_err()
        );
        assert!(!outside.path().join("escape.txt").exists());
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

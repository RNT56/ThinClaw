//! Archive manifest — JSON index of all files in a cloud archive.
//!
//! The manifest is the "table of contents" for a cloud backup. It records
//! every file, its checksum, size, and classification. The manifest itself
//! is encrypted before upload.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use super::encryption::encrypted_size_limit;
use super::provider::validate_object_key;

/// Maximum decrypted size supported by the current in-memory migration
/// format. Larger archives require a future streaming format.
pub const MAX_ARCHIVE_FILE_BYTES: usize = 512 * 1024 * 1024;
/// Prevent a small manifest from driving unbounded per-entry work.
pub const MAX_MANIFEST_FILES: usize = 50_000;
/// Maximum decrypted JSON size accepted for an archive manifest.
pub const MAX_MANIFEST_JSON_BYTES: usize = 8 * 1024 * 1024;

/// Top-level archive manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveManifest {
    /// Manifest format version (currently 1)
    pub version: u32,
    /// App version that created this archive
    pub app_version: String,
    /// Database schema migration count
    pub schema_version: u32,
    /// When this archive was created (Unix ms)
    pub created_at_ms: i64,
    /// Encryption metadata
    pub encryption: EncryptionMeta,
    /// All files in the archive
    pub files: Vec<ManifestFile>,
    /// Summary statistics
    pub statistics: ArchiveStatistics,
}

/// Encryption metadata stored in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionMeta {
    /// Encryption algorithm (always "AES-256-GCM")
    pub algorithm: String,
    /// Key derivation method (always "HKDF-SHA256")
    pub key_derivation: String,
    /// Key identifier (for key rotation tracking)
    pub key_id: String,
}

/// A single file in the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    /// Cloud storage key (e.g. "db/thinclaw.db.enc")
    pub key: String,
    /// Original local path relative to app_data_dir (e.g. "thinclaw.db")
    pub original_path: String,
    /// Original uncompressed/unencrypted size in bytes
    pub size_bytes: u64,
    /// Encrypted blob size in bytes
    pub encrypted_size_bytes: u64,
    /// SHA-256 hash of the original file (hex)
    pub sha256: String,
    /// File classification
    #[serde(rename = "type")]
    pub file_type: FileType,
}

/// Classification of a file for UI display and progress grouping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Database,
    Document,
    ChatImage,
    GeneratedImage,
    VectorIndex,
    Preview,
    AgentState,
    Other,
}

impl FileType {
    /// Human-readable label for UI display
    pub fn label(&self) -> &str {
        match self {
            FileType::Database => "Database",
            FileType::Document => "Documents",
            FileType::ChatImage => "Chat Images",
            FileType::GeneratedImage => "Generated Images",
            FileType::VectorIndex => "Vector Indices",
            FileType::Preview => "Previews",
            FileType::AgentState => "Agent State",
            FileType::Other => "Other",
        }
    }

    /// Determine file type from its relative path.
    pub fn from_path(path: &str) -> Self {
        if matches!(path, "thinclaw.db" | "thinclaw-runtime.db" | "ironclaw.db") {
            FileType::Database
        } else if path.starts_with("documents/") {
            FileType::Document
        } else if path.starts_with("images/") {
            FileType::ChatImage
        } else if path.starts_with("generated/") {
            FileType::GeneratedImage
        } else if path.starts_with("vectors/") {
            FileType::VectorIndex
        } else if path.starts_with("previews/") {
            FileType::Preview
        } else if path.starts_with("thinclaw/") {
            FileType::AgentState
        } else {
            FileType::Other
        }
    }
}

/// Summary statistics for the archive.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArchiveStatistics {
    pub total_files: u32,
    pub total_size_bytes: u64,
    pub encrypted_size_bytes: u64,
    pub conversations: Option<u32>,
    pub messages: Option<u32>,
    pub documents: Option<u32>,
    pub generated_images: Option<u32>,
}

impl ArchiveManifest {
    /// Create a new empty manifest.
    pub fn new(app_version: String, schema_version: u32, key_id: String) -> Self {
        Self {
            version: 1,
            app_version,
            schema_version,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            encryption: EncryptionMeta {
                algorithm: "AES-256-GCM".to_string(),
                key_derivation: "HKDF-SHA256".to_string(),
                key_id,
            },
            files: Vec::new(),
            statistics: ArchiveStatistics::default(),
        }
    }

    /// Add a file to the manifest with its SHA-256 hash.
    pub fn add_file(
        &mut self,
        key: String,
        original_path: String,
        original_data: &[u8],
        encrypted_size: u64,
    ) {
        let sha256 = compute_sha256(original_data);
        let file_type = FileType::from_path(&original_path);

        self.files.push(ManifestFile {
            key,
            original_path,
            size_bytes: original_data.len() as u64,
            encrypted_size_bytes: encrypted_size,
            sha256,
            file_type,
        });

        self.recalculate_statistics();
    }

    /// Replace the manifest entry for a local path, or append it if new.
    pub fn upsert_file(
        &mut self,
        key: String,
        original_path: String,
        original_data: &[u8],
        encrypted_size: u64,
    ) {
        self.files
            .retain(|file| file.original_path != original_path);
        self.add_file(key, original_path, original_data, encrypted_size);
    }

    /// Remove a local path and return its previous cloud object key.
    pub fn remove_file(&mut self, original_path: &str) -> Option<String> {
        let index = self
            .files
            .iter()
            .position(|file| file.original_path == original_path)?;
        let removed = self.files.remove(index);
        self.recalculate_statistics();
        Some(removed.key)
    }

    fn recalculate_statistics(&mut self) {
        self.statistics.total_files = u32::try_from(self.files.len()).unwrap_or(u32::MAX);
        self.statistics.total_size_bytes = self
            .files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes));
        self.statistics.encrypted_size_bytes = self.files.iter().fold(0_u64, |total, file| {
            total.saturating_add(file.encrypted_size_bytes)
        });
    }

    /// Serialize the manifest to JSON.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    /// Deserialize a manifest from JSON.
    pub fn from_json(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Validate all attacker-controlled manifest metadata before any object is
    /// fetched or destination path is created.
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported manifest version {}", self.version));
        }
        if self.encryption.algorithm != "AES-256-GCM"
            || self.encryption.key_derivation != "HKDF-SHA256"
        {
            return Err("manifest declares unsupported encryption metadata".to_string());
        }
        if self.app_version.is_empty()
            || self.app_version.len() > 256
            || self.encryption.key_id.is_empty()
            || self.encryption.key_id.len() > 512
        {
            return Err("manifest metadata is empty or exceeds its size limit".to_string());
        }
        if self.files.len() > MAX_MANIFEST_FILES {
            return Err(format!(
                "manifest contains more than {MAX_MANIFEST_FILES} files"
            ));
        }

        let mut keys = HashSet::with_capacity(self.files.len());
        let mut paths = HashSet::with_capacity(self.files.len());
        let mut total_plaintext = 0_u64;
        let mut total_encrypted = 0_u64;
        let mut has_primary_database = false;
        for file in &self.files {
            validate_object_key(&file.key).map_err(|error| error.to_string())?;
            validate_manifest_path(&file.original_path)?;
            let expected_key = match file.original_path.as_str() {
                "thinclaw.db" => "db/thinclaw.db.enc".to_string(),
                "thinclaw-runtime.db" | "ironclaw.db" => "db/thinclaw-runtime.db.enc".to_string(),
                path if supported_data_path(path) => format!("{path}.enc"),
                _ => {
                    return Err(format!(
                        "manifest destination '{}' is outside supported data roots",
                        file.original_path
                    ));
                }
            };
            if file.key != expected_key && !is_versioned_object_key(file) {
                return Err(format!(
                    "manifest object key '{}' does not match destination '{}'",
                    file.key, file.original_path
                ));
            }
            if !file.key.ends_with(".enc") {
                return Err(format!(
                    "manifest object key '{}' is not an encrypted object",
                    file.key
                ));
            }
            if !keys.insert(file.key.clone()) {
                return Err(format!("duplicate manifest object key '{}'", file.key));
            }
            if !paths.insert(file.original_path.clone()) {
                return Err(format!(
                    "duplicate manifest destination '{}'",
                    file.original_path
                ));
            }
            if file.original_path == "thinclaw.db" {
                has_primary_database = true;
            }
            if file.size_bytes > MAX_ARCHIVE_FILE_BYTES as u64 {
                return Err(format!(
                    "manifest file '{}' exceeds the {}-byte restore limit",
                    file.original_path, MAX_ARCHIVE_FILE_BYTES
                ));
            }
            let encrypted_limit = encrypted_size_limit(file.size_bytes as usize) as u64;
            if file.encrypted_size_bytes == 0 || file.encrypted_size_bytes > encrypted_limit {
                return Err(format!(
                    "encrypted size for '{}' is empty or exceeds its declared plaintext bound",
                    file.original_path
                ));
            }
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(format!(
                    "manifest file '{}' has an invalid SHA-256 digest",
                    file.original_path
                ));
            }
            if file.file_type != FileType::from_path(&file.original_path) {
                return Err(format!(
                    "manifest file '{}' has an inconsistent type",
                    file.original_path
                ));
            }
            total_plaintext = total_plaintext
                .checked_add(file.size_bytes)
                .ok_or_else(|| "manifest plaintext size total overflows".to_string())?;
            total_encrypted = total_encrypted
                .checked_add(file.encrypted_size_bytes)
                .ok_or_else(|| "manifest encrypted size total overflows".to_string())?;
        }

        if self.statistics.total_files as usize != self.files.len()
            || self.statistics.total_size_bytes != total_plaintext
            || self.statistics.encrypted_size_bytes != total_encrypted
        {
            return Err("manifest statistics do not match its file entries".to_string());
        }
        if !has_primary_database {
            return Err("manifest does not contain the primary database".to_string());
        }
        Ok(())
    }

    /// Check if the schema version is compatible with the current app.
    ///
    /// Returns `Ok(true)` if forward-compatible (app is same or newer),
    /// `Ok(false)` if the archive is from a newer app (needs update).
    pub fn is_schema_compatible(&self, current_schema_version: u32) -> bool {
        self.schema_version <= current_schema_version
    }

    /// Get files grouped by type (for progress UI).
    pub fn files_by_type(&self) -> std::collections::HashMap<FileType, Vec<&ManifestFile>> {
        let mut groups: std::collections::HashMap<FileType, Vec<&ManifestFile>> =
            std::collections::HashMap::new();
        for file in &self.files {
            groups.entry(file.file_type.clone()).or_default().push(file);
        }
        groups
    }
}

/// Generate a fresh immutable object key bound to both its logical path and
/// plaintext digest. The random suffix prevents an upload from overwriting an
/// object referenced by the currently committed manifest.
pub fn new_versioned_object_key(original_path: &str, sha256: &str) -> String {
    format!(
        "objects/v1/{}/{}/{}.enc",
        compute_sha256(original_path.as_bytes()),
        sha256,
        uuid::Uuid::new_v4()
    )
}

fn is_versioned_object_key(file: &ManifestFile) -> bool {
    let segments = file.key.split('/').collect::<Vec<_>>();
    if segments.len() != 5
        || segments[0] != "objects"
        || segments[1] != "v1"
        || segments[2] != compute_sha256(file.original_path.as_bytes())
        || segments[3] != file.sha256
    {
        return false;
    }
    let Some(uuid) = segments[4].strip_suffix(".enc") else {
        return false;
    };
    uuid::Uuid::parse_str(uuid).is_ok()
}

fn validate_manifest_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 4_096 || path.contains('\0') {
        return Err("manifest destination is empty, too long, or contains NUL".to_string());
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(format!("manifest destination '{path}' must be relative"));
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(format!(
            "manifest destination '{path}' cannot contain a drive prefix"
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(format!(
            "manifest destination '{path}' contains control characters"
        ));
    }
    for segment in path.split(['/', '\\']) {
        if segment.is_empty() || segment == "." || segment == ".." || segment.len() > 255 {
            return Err(format!(
                "manifest destination '{path}' is not a normalized relative path"
            ));
        }
    }
    Ok(())
}

fn supported_data_path(path: &str) -> bool {
    [
        "documents/",
        "images/",
        "generated/",
        "vectors/",
        "previews/",
        "thinclaw/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

/// Compute SHA-256 hash of data, returning hex string.
pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Inline hex encoding (avoid adding `hex` crate dependency).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_roundtrip() {
        let mut manifest =
            ArchiveManifest::new("0.1.0".to_string(), 12, "test-key-2026".to_string());

        manifest.add_file(
            "db/thinclaw.db.enc".to_string(),
            "thinclaw.db".to_string(),
            b"fake db data",
            128,
        );

        manifest.add_file(
            "documents/test.pdf.enc".to_string(),
            "documents/test.pdf".to_string(),
            b"fake pdf data",
            256,
        );

        let json = manifest.to_json().unwrap();
        let restored = ArchiveManifest::from_json(&json).unwrap();

        assert_eq!(restored.version, 1);
        assert_eq!(restored.files.len(), 2);
        assert_eq!(restored.statistics.total_files, 2);
        assert_eq!(restored.files[0].file_type, FileType::Database);
        assert_eq!(restored.files[1].file_type, FileType::Document);
    }

    #[test]
    fn test_file_type_classification() {
        assert_eq!(FileType::from_path("thinclaw.db"), FileType::Database);
        assert_eq!(FileType::from_path("documents/a.pdf"), FileType::Document);
        assert_eq!(FileType::from_path("images/b.png"), FileType::ChatImage);
        assert_eq!(
            FileType::from_path("generated/c.png"),
            FileType::GeneratedImage
        );
        assert_eq!(
            FileType::from_path("vectors/global.usearch"),
            FileType::VectorIndex
        );
        assert_eq!(FileType::from_path("previews/d.jpg"), FileType::Preview);
        assert_eq!(
            FileType::from_path("thinclaw/soul.md"),
            FileType::AgentState
        );
        assert_eq!(FileType::from_path("random.txt"), FileType::Other);
    }

    #[test]
    fn test_schema_compatibility() {
        let manifest = ArchiveManifest::new("0.1.0".to_string(), 10, "k".to_string());
        assert!(manifest.is_schema_compatible(10)); // same version
        assert!(manifest.is_schema_compatible(12)); // newer app
        assert!(!manifest.is_schema_compatible(8)); // older app
    }

    #[test]
    fn test_sha256() {
        let hash = compute_sha256(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    // ── A6-2: Extended manifest tests ────────────────────────────────────

    #[test]
    fn test_sha256_known_values() {
        // Empty input
        assert_eq!(
            compute_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // "abc" — standard SHA-256 test vector
        assert_eq!(
            compute_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_manifest_file_integrity() {
        let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 5, "k".to_string());

        let data = b"important document content";
        let expected_sha = compute_sha256(data);

        manifest.add_file(
            "documents/test.pdf.enc".to_string(),
            "documents/test.pdf".to_string(),
            data,
            100,
        );

        assert_eq!(manifest.files[0].sha256, expected_sha);
        assert_eq!(manifest.files[0].size_bytes, data.len() as u64);
        assert_eq!(manifest.files[0].encrypted_size_bytes, 100);
    }

    #[test]
    fn test_json_stability() {
        let mut manifest = ArchiveManifest::new("2.5.0".to_string(), 42, "key-xyz".to_string());
        manifest.add_file(
            "images/cat.png.enc".to_string(),
            "images/cat.png".to_string(),
            b"meow",
            999,
        );

        let json = manifest.to_json().unwrap();
        let json_str = String::from_utf8_lossy(&json);

        assert!(json_str.contains("\"version\": 1"));
        assert!(json_str.contains("\"app_version\": \"2.5.0\""));
        assert!(json_str.contains("\"schema_version\": 42"));
        assert!(json_str.contains("\"algorithm\": \"AES-256-GCM\""));
        assert!(json_str.contains("\"key_derivation\": \"HKDF-SHA256\""));
        assert!(json_str.contains("\"key_id\": \"key-xyz\""));
        assert!(json_str.contains("\"type\": \"chat_image\""));

        let restored = ArchiveManifest::from_json(&json).unwrap();
        assert_eq!(restored.app_version, "2.5.0");
        assert_eq!(restored.schema_version, 42);
        assert_eq!(restored.encryption.key_id, "key-xyz");
        assert_eq!(restored.files[0].file_type, FileType::ChatImage);
    }

    #[test]
    fn test_schema_version_edge_cases() {
        let m = ArchiveManifest::new("0.1.0".to_string(), 0, "k".to_string());
        assert!(m.is_schema_compatible(0));
        assert!(m.is_schema_compatible(1));

        let m = ArchiveManifest::new("0.1.0".to_string(), u32::MAX, "k".to_string());
        assert!(m.is_schema_compatible(u32::MAX));
        assert!(!m.is_schema_compatible(u32::MAX - 1));
    }

    #[test]
    fn test_files_by_type_grouping() {
        let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 1, "k".to_string());

        manifest.add_file("k1".into(), "documents/a.pdf".into(), b"a", 10);
        manifest.add_file("k2".into(), "documents/b.pdf".into(), b"b", 20);
        manifest.add_file("k3".into(), "images/c.png".into(), b"c", 30);
        manifest.add_file("k4".into(), "thinclaw.db".into(), b"d", 40);

        let groups = manifest.files_by_type();

        assert_eq!(groups.get(&FileType::Document).unwrap().len(), 2);
        assert_eq!(groups.get(&FileType::ChatImage).unwrap().len(), 1);
        assert_eq!(groups.get(&FileType::Database).unwrap().len(), 1);
        assert!(!groups.contains_key(&FileType::VectorIndex));
    }

    #[test]
    fn test_statistics_accumulation() {
        let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 1, "k".to_string());

        manifest.add_file("k1".into(), "documents/a.pdf".into(), b"12345", 10);
        assert_eq!(manifest.statistics.total_files, 1);
        assert_eq!(manifest.statistics.total_size_bytes, 5);
        assert_eq!(manifest.statistics.encrypted_size_bytes, 10);

        manifest.add_file("k2".into(), "images/b.png".into(), b"abc", 7);
        assert_eq!(manifest.statistics.total_files, 2);
        assert_eq!(manifest.statistics.total_size_bytes, 5 + 3);
        assert_eq!(manifest.statistics.encrypted_size_bytes, 10 + 7);
    }

    #[test]
    fn test_empty_manifest_roundtrip() {
        let manifest = ArchiveManifest::new("0.1.0".to_string(), 0, "k".to_string());
        assert!(manifest.files.is_empty());

        let json = manifest.to_json().unwrap();
        let restored = ArchiveManifest::from_json(&json).unwrap();

        assert!(restored.files.is_empty());
        assert_eq!(restored.statistics.total_files, 0);
        assert_eq!(restored.version, 1);
    }

    #[test]
    fn test_large_manifest() {
        let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 99, "k".to_string());

        for i in 0..500 {
            let data = format!("file content {}", i);
            manifest.add_file(
                format!("documents/file_{}.txt.enc", i),
                format!("documents/file_{}.txt", i),
                data.as_bytes(),
                data.len() as u64 + 28,
            );
        }

        assert_eq!(manifest.statistics.total_files, 500);

        let json = manifest.to_json().unwrap();
        let restored = ArchiveManifest::from_json(&json).unwrap();

        assert_eq!(restored.files.len(), 500);
        for file in &restored.files {
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn test_file_type_label() {
        assert_eq!(FileType::Database.label(), "Database");
        assert_eq!(FileType::Document.label(), "Documents");
        assert_eq!(FileType::ChatImage.label(), "Chat Images");
        assert_eq!(FileType::GeneratedImage.label(), "Generated Images");
        assert_eq!(FileType::VectorIndex.label(), "Vector Indices");
        assert_eq!(FileType::Preview.label(), "Previews");
        assert_eq!(FileType::AgentState.label(), "Agent State");
        assert_eq!(FileType::Other.label(), "Other");
    }

    #[test]
    fn test_malformed_json_rejected() {
        let result = ArchiveManifest::from_json(b"not json at all");
        assert!(result.is_err());

        let result = ArchiveManifest::from_json(b"{}");
        assert!(result.is_err());

        let result = ArchiveManifest::from_json(b"{\"version\":1}");
        assert!(result.is_err());
    }
}

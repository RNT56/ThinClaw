//! Archive manifest — JSON index of all files in a cloud archive.
//!
//! The manifest is the "table of contents" for a cloud backup. It records
//! every file, its checksum, size, and classification. The manifest itself
//! is encrypted before upload.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// Cloud storage key (e.g. "db/openclaw.db.enc")
    pub key: String,
    /// Original local path relative to app_data_dir (e.g. "openclaw.db")
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
        if path.starts_with("openclaw.db") || path.ends_with(".db") {
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
        } else if path.starts_with("openclaw/") {
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

        // Update statistics
        self.statistics.total_files = self.files.len() as u32;
        self.statistics.total_size_bytes = self.files.iter().map(|f| f.size_bytes).sum();
        self.statistics.encrypted_size_bytes =
            self.files.iter().map(|f| f.encrypted_size_bytes).sum();
    }

    /// Serialize the manifest to JSON.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    /// Deserialize a manifest from JSON.
    pub fn from_json(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
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
            "db/openclaw.db.enc".to_string(),
            "openclaw.db".to_string(),
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
        assert_eq!(FileType::from_path("openclaw.db"), FileType::Database);
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
            FileType::from_path("openclaw/soul.md"),
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
        manifest.add_file("k4".into(), "openclaw.db".into(), b"d", 40);

        let groups = manifest.files_by_type();

        assert_eq!(groups.get(&FileType::Document).unwrap().len(), 2);
        assert_eq!(groups.get(&FileType::ChatImage).unwrap().len(), 1);
        assert_eq!(groups.get(&FileType::Database).unwrap().len(), 1);
        assert!(groups.get(&FileType::VectorIndex).is_none());
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

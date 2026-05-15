//! A6-3: Integration test — full local → cloud → local roundtrip.
//!
//! Uses an in-memory `MockProvider` (HashMap-based) so no Docker/MinIO needed.
//! Tests the complete pipeline:
//! 1. Create test files on disk
//! 2. Encrypt + upload them via the encryption + manifest modules
//! 3. Download + decrypt them back
//! 4. Verify all data matches (SHA-256 + byte-level)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cloud::encryption::{self, MasterKey};
use crate::cloud::manifest::{compute_sha256, ArchiveManifest, FileType};
use crate::cloud::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

use async_trait::async_trait;

// ── In-memory mock provider ──────────────────────────────────────────────

/// A HashMap-based cloud provider for testing.
struct MockProvider {
    name: String,
    storage: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MockProvider {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn len(&self) -> usize {
        self.storage.lock().unwrap().len()
    }

    fn total_bytes(&self) -> u64 {
        self.storage
            .lock()
            .unwrap()
            .values()
            .map(|v| v.len() as u64)
            .sum()
    }
}

#[async_trait]
impl CloudProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        Ok(CloudStatus {
            connected: true,
            storage_used: self.total_bytes(),
            storage_available: Some(1_000_000_000),
            provider_name: self.name.clone(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        self.storage
            .lock()
            .unwrap()
            .insert(key.to_string(), data.to_vec());
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        self.storage
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| CloudError::NotFound(key.to_string()))
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        self.storage.lock().unwrap().remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let store = self.storage.lock().unwrap();
        Ok(store
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| CloudEntry {
                key: k.clone(),
                size: v.len() as u64,
                last_modified: 0,
                checksum: None,
            })
            .collect())
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        Ok(self.total_bytes())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Simulate a full local → mock-cloud → local roundtrip without the migration engine
/// (which requires AppHandle/SqlitePool). Tests the core encrypt+manifest+provider pipeline.
#[tokio::test]
async fn test_full_encrypt_upload_download_decrypt_roundtrip() {
    let provider = MockProvider::new("test-mock");
    let master_key = MasterKey::generate();

    // ── Simulate "local" files ──────────────────────────────────────────
    let test_files: Vec<(&str, Vec<u8>)> = vec![
        (
            "documents/readme.md",
            b"# Hello World\n\nThis is a test document.".to_vec(),
        ),
        ("documents/report.pdf", {
            let mut v = vec![0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]; // PDF header
            v.extend_from_slice(&[0x0a; 1000]);
            v
        }),
        (
            "images/photo.png",
            (0..5000_u32).flat_map(|i| i.to_le_bytes()).collect(),
        ),
        (
            "generated/art.png",
            (0..3000_u32).flat_map(|i| (i * 7).to_le_bytes()).collect(),
        ),
        ("vectors/global.usearch", vec![0xAB; 8000]),
        ("previews/thumb.jpg", {
            let mut v = Vec::new();
            for _ in 0..500 {
                v.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]);
            }
            v
        }),
        ("thinclaw/MEMORY.md", b"Agent memory state".to_vec()),
    ];

    // ── Phase 1: Encrypt + Upload ───────────────────────────────────────
    let mut manifest = ArchiveManifest::new("0.1.0-test".to_string(), 5, "test-key".to_string());

    for (path, data) in &test_files {
        let cloud_key = format!("{}.enc", path);

        // Encrypt
        let encrypted = encryption::encrypt(&master_key, path, data).unwrap();
        assert_ne!(&encrypted, data, "Encrypted should differ from plaintext");

        // Upload
        provider.put(&cloud_key, &encrypted).await.unwrap();

        // Record in manifest
        manifest.add_file(cloud_key, path.to_string(), data, encrypted.len() as u64);
    }

    assert_eq!(manifest.files.len(), test_files.len());
    assert_eq!(manifest.statistics.total_files, test_files.len() as u32);

    // Upload encrypted manifest
    let manifest_json = manifest.to_json().unwrap();
    let encrypted_manifest =
        encryption::encrypt(&master_key, "manifest.json", &manifest_json).unwrap();
    provider
        .put("manifest.json.enc", &encrypted_manifest)
        .await
        .unwrap();

    // Verify cloud has all files + manifest
    assert_eq!(provider.len(), test_files.len() + 1); // files + manifest

    // ── Phase 2: Download manifest ──────────────────────────────────────
    let dl_manifest_enc = provider.get("manifest.json.enc").await.unwrap();
    let dl_manifest_json =
        encryption::decrypt(&master_key, "manifest.json", &dl_manifest_enc).unwrap();
    let dl_manifest = ArchiveManifest::from_json(&dl_manifest_json).unwrap();

    assert_eq!(dl_manifest.version, 1);
    assert_eq!(dl_manifest.app_version, "0.1.0-test");
    assert_eq!(dl_manifest.schema_version, 5);
    assert_eq!(dl_manifest.files.len(), test_files.len());
    assert!(dl_manifest.is_schema_compatible(5));
    assert!(dl_manifest.is_schema_compatible(10));

    // ── Phase 3: Download + decrypt all files ───────────────────────────
    for manifest_file in &dl_manifest.files {
        // Download
        let encrypted = provider.get(&manifest_file.key).await.unwrap();

        // Decrypt
        let decrypted =
            encryption::decrypt(&master_key, &manifest_file.original_path, &encrypted).unwrap();

        // Verify SHA-256
        let hash = compute_sha256(&decrypted);
        assert_eq!(
            hash, manifest_file.sha256,
            "SHA-256 mismatch for '{}'",
            manifest_file.original_path
        );

        // Verify size
        assert_eq!(
            decrypted.len() as u64,
            manifest_file.size_bytes,
            "Size mismatch for '{}'",
            manifest_file.original_path
        );

        // Verify byte-level match with original
        let original = test_files
            .iter()
            .find(|(p, _)| *p == manifest_file.original_path)
            .map(|(_, d)| d)
            .unwrap();

        assert_eq!(
            &decrypted, original,
            "Data mismatch for '{}'",
            manifest_file.original_path
        );
    }
}

/// Test that wrong key cannot decrypt cloud data (security invariant).
#[tokio::test]
async fn test_wrong_key_cannot_decrypt_cloud_data() {
    let provider = MockProvider::new("test-mock");
    let key1 = MasterKey::generate();
    let key2 = MasterKey::generate(); // attacker's key

    let data = b"sensitive data";
    let encrypted = encryption::encrypt(&key1, "secret.txt", data).unwrap();
    provider.put("secret.txt.enc", &encrypted).await.unwrap();

    let downloaded = provider.get("secret.txt.enc").await.unwrap();
    let result = encryption::decrypt(&key2, "secret.txt", &downloaded);
    assert!(result.is_err(), "Wrong key must not decrypt");
}

/// Test that provider operations work correctly.
#[tokio::test]
async fn test_mock_provider_operations() {
    let provider = MockProvider::new("test-mock");

    // Put + Get
    provider.put("a/b.txt", b"hello").await.unwrap();
    assert_eq!(provider.get("a/b.txt").await.unwrap(), b"hello");

    // Exists
    assert!(provider.exists("a/b.txt").await.unwrap());
    assert!(!provider.exists("nonexistent").await.unwrap());

    // List
    provider.put("a/c.txt", b"world").await.unwrap();
    let list = provider.list("a/").await.unwrap();
    assert_eq!(list.len(), 2);

    // Delete
    provider.delete("a/b.txt").await.unwrap();
    assert!(!provider.exists("a/b.txt").await.unwrap());

    // Get after delete → NotFound
    let result = provider.get("a/b.txt").await;
    assert!(matches!(result, Err(CloudError::NotFound(_))));

    // Usage
    assert_eq!(provider.usage().await.unwrap(), 5); // "world" = 5 bytes
}

/// Test the manifest file type classification for all uploaded files.
#[tokio::test]
async fn test_file_type_classification_in_roundtrip() {
    let master_key = MasterKey::generate();
    let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 1, "k".to_string());

    let paths_and_types = vec![
        ("thinclaw.db", FileType::Database),
        ("documents/a.pdf", FileType::Document),
        ("images/b.png", FileType::ChatImage),
        ("generated/c.png", FileType::GeneratedImage),
        ("vectors/d.usearch", FileType::VectorIndex),
        ("previews/e.jpg", FileType::Preview),
        ("thinclaw/SOUL.md", FileType::AgentState),
        ("misc.bin", FileType::Other),
    ];

    for (path, _expected_type) in &paths_and_types {
        let data = format!("content of {}", path);
        let _encrypted = encryption::encrypt(&master_key, path, data.as_bytes()).unwrap();
        manifest.add_file(
            format!("{}.enc", path),
            path.to_string(),
            data.as_bytes(),
            100,
        );
    }

    // Verify types
    for (i, (_, expected_type)) in paths_and_types.iter().enumerate() {
        assert_eq!(
            &manifest.files[i].file_type, expected_type,
            "Type mismatch for file index {}",
            i
        );
    }

    // Roundtrip through JSON
    let json = manifest.to_json().unwrap();
    let restored = ArchiveManifest::from_json(&json).unwrap();

    for (i, (_, expected_type)) in paths_and_types.iter().enumerate() {
        assert_eq!(
            &restored.files[i].file_type, expected_type,
            "Type mismatch after JSON roundtrip for index {}",
            i
        );
    }
}

/// Encrypt + upload many files, verify all can be individually decrypted.
#[tokio::test]
async fn test_many_files_roundtrip() {
    let provider = MockProvider::new("bulk-test");
    let master_key = MasterKey::generate();

    let file_count = 100;
    let mut files_data: Vec<(String, Vec<u8>)> = Vec::new();

    for i in 0..file_count {
        let path = format!("documents/doc_{:04}.txt", i);
        let data: Vec<u8> =
            format!("Document {} content with unique data: {}", i, i * 31337).into_bytes();
        let encrypted = encryption::encrypt(&master_key, &path, &data).unwrap();
        provider
            .put(&format!("{}.enc", path), &encrypted)
            .await
            .unwrap();
        files_data.push((path, data));
    }

    assert_eq!(provider.len(), file_count);

    // Decrypt all
    for (path, original) in &files_data {
        let cloud_key = format!("{}.enc", path);
        let encrypted = provider.get(&cloud_key).await.unwrap();
        let decrypted = encryption::decrypt(&master_key, path, &encrypted).unwrap();
        assert_eq!(&decrypted, original, "Mismatch for {}", path);
    }
}

// ── A6-4: Schema Migration Restore Tests ────────────────────────────────

/// Verify that a manifest created by an older version can be restored by a newer app.
#[tokio::test]
async fn test_schema_migration_on_restore() {
    let provider = MockProvider::new("schema-test");
    let master_key = MasterKey::generate();

    // ── Phase 1: Create archive at schema version 5 ─────────────────────
    let old_schema_version = 5u32;
    let mut manifest = ArchiveManifest::new(
        "0.8.0".to_string(),
        old_schema_version,
        "old-key".to_string(),
    );

    // Simulate old-version files
    let test_files = vec![
        (
            "thinclaw.db",
            b"SQLite format 3\0 - old schema v5 data".to_vec(),
        ),
        ("documents/notes.md", b"# Old notes\nFrom v0.8.0".to_vec()),
        ("images/old_photo.png", vec![0x89; 400]),
    ];

    for (path, data) in &test_files {
        let cloud_key = format!("{}.enc", path);
        let encrypted = encryption::encrypt(&master_key, path, data).unwrap();
        provider.put(&cloud_key, &encrypted).await.unwrap();
        manifest.add_file(cloud_key, path.to_string(), data, encrypted.len() as u64);
    }

    // Upload manifest
    let manifest_json = manifest.to_json().unwrap();
    let enc_manifest = encryption::encrypt(&master_key, "manifest.json", &manifest_json).unwrap();
    provider
        .put("manifest.json.enc", &enc_manifest)
        .await
        .unwrap();

    // ── Phase 2: "New app" at schema version 12 downloads the archive ───
    let current_schema_version = 12u32;

    let dl_enc = provider.get("manifest.json.enc").await.unwrap();
    let dl_json = encryption::decrypt(&master_key, "manifest.json", &dl_enc).unwrap();
    let dl_manifest = ArchiveManifest::from_json(&dl_json).unwrap();

    // Forward-compatible: old archive → new app should be OK
    assert!(
        dl_manifest.is_schema_compatible(current_schema_version),
        "Archive from schema v{} should be compatible with app at v{}",
        dl_manifest.schema_version,
        current_schema_version
    );

    assert_eq!(dl_manifest.schema_version, old_schema_version);
    assert_eq!(dl_manifest.app_version, "0.8.0");

    // All files should still decrypt correctly
    for file in &dl_manifest.files {
        let encrypted = provider.get(&file.key).await.unwrap();
        let decrypted = encryption::decrypt(&master_key, &file.original_path, &encrypted).unwrap();
        let hash = compute_sha256(&decrypted);
        assert_eq!(
            hash, file.sha256,
            "SHA-256 mismatch for {}",
            file.original_path
        );
    }

    // ── Phase 3: Test backward-incompatible scenario ────────────────────
    // Archive from newer app (v12) → older app (v5) should be rejected
    let future_manifest = ArchiveManifest::new(
        "2.0.0".to_string(),
        current_schema_version,
        "new-key".to_string(),
    );

    assert!(
        !future_manifest.is_schema_compatible(old_schema_version),
        "Archive from schema v{} should NOT be compatible with app at v{}",
        current_schema_version,
        old_schema_version
    );
}

/// Verify the exact version boundary: same schema version is compatible.
#[tokio::test]
async fn test_schema_exact_version_boundary() {
    let manifest = ArchiveManifest::new("1.0.0".to_string(), 10, "k".to_string());

    // Same version → compatible
    assert!(manifest.is_schema_compatible(10));
    // One higher → compatible
    assert!(manifest.is_schema_compatible(11));
    // One lower → NOT compatible
    assert!(!manifest.is_schema_compatible(9));
}

// ── A6-5: Crash-Resume Tests ────────────────────────────────────────────

/// Simulate a migration crash after uploading 5 of 10 files, then resume.
#[tokio::test]
async fn test_migration_resume_after_crash() {
    let provider = MockProvider::new("crash-resume-test");
    let master_key = MasterKey::generate();

    // ── Prepare 10 test files ───────────────────────────────────────────
    let total_files = 10;
    let test_files: Vec<(String, Vec<u8>)> = (0..total_files)
        .map(|i| {
            let path = format!("documents/file_{:02}.txt", i);
            let data = format!("Content of file {} with unique seed {}", i, i * 42).into_bytes();
            (path, data)
        })
        .collect();

    // ── Phase 1: Upload first 5 files (simulating work before crash) ────
    let crash_point = 5;
    let mut partial_manifest =
        ArchiveManifest::new("0.1.0".to_string(), 7, "crash-key".to_string());

    for (path, data) in &test_files[..crash_point] {
        let cloud_key = format!("{}.enc", path);
        let encrypted = encryption::encrypt(&master_key, path, data).unwrap();
        provider.put(&cloud_key, &encrypted).await.unwrap();
        partial_manifest.add_file(cloud_key, path.clone(), data, encrypted.len() as u64);
    }

    // At crash point: 5 files uploaded, partial manifest in memory
    assert_eq!(partial_manifest.files.len(), crash_point);
    assert_eq!(provider.len(), crash_point);

    // ── CRASH! (manifest is lost — only cloud data survives) ────────────

    // ── Phase 2: Resume — detect what's already uploaded ────────────────
    // In a real implementation, we'd scan the cloud for existing .enc files.
    // Here we simulate resume by checking which keys exist.

    let mut resume_manifest = ArchiveManifest::new("0.1.0".to_string(), 7, "crash-key".to_string());

    for (path, data) in &test_files {
        let cloud_key = format!("{}.enc", path);

        if provider.exists(&cloud_key).await.unwrap() {
            // File already uploaded — verify it and add to manifest
            let existing = provider.get(&cloud_key).await.unwrap();
            let decrypted = encryption::decrypt(&master_key, path, &existing).unwrap();
            assert_eq!(&decrypted, data, "Existing file corrupt: {}", path);
            resume_manifest.add_file(cloud_key, path.clone(), data, existing.len() as u64);
        } else {
            // File missing — upload it
            let encrypted = encryption::encrypt(&master_key, path, data).unwrap();
            provider.put(&cloud_key, &encrypted).await.unwrap();
            resume_manifest.add_file(cloud_key, path.clone(), data, encrypted.len() as u64);
        }
    }

    // ── Phase 3: Verify complete archive ────────────────────────────────
    assert_eq!(resume_manifest.files.len(), total_files);
    assert_eq!(provider.len(), total_files);

    // Upload the completed manifest
    let manifest_json = resume_manifest.to_json().unwrap();
    let enc_manifest = encryption::encrypt(&master_key, "manifest.json", &manifest_json).unwrap();
    provider
        .put("manifest.json.enc", &enc_manifest)
        .await
        .unwrap();

    // ── Phase 4: Full restore from cloud ────────────────────────────────
    let dl_enc = provider.get("manifest.json.enc").await.unwrap();
    let dl_json = encryption::decrypt(&master_key, "manifest.json", &dl_enc).unwrap();
    let final_manifest = ArchiveManifest::from_json(&dl_json).unwrap();

    assert_eq!(final_manifest.files.len(), total_files);

    for file in &final_manifest.files {
        let encrypted = provider.get(&file.key).await.unwrap();
        let decrypted = encryption::decrypt(&master_key, &file.original_path, &encrypted).unwrap();

        // Verify SHA-256
        let hash = compute_sha256(&decrypted);
        assert_eq!(
            hash, file.sha256,
            "SHA-256 mismatch after resume: {}",
            file.original_path
        );

        // Verify matches original
        let original = test_files
            .iter()
            .find(|(p, _)| p == &file.original_path)
            .unwrap();
        assert_eq!(
            &decrypted, &original.1,
            "Data mismatch after resume: {}",
            file.original_path
        );
    }
}

/// Test that re-uploading (overwriting) an already-uploaded file is idempotent.
#[tokio::test]
async fn test_idempotent_reupload() {
    let provider = MockProvider::new("idempotent-test");
    let master_key = MasterKey::generate();

    let path = "documents/important.txt";
    let data = b"This file might be uploaded multiple times";

    // Upload twice (simulating retry after uncertain completion)
    for _ in 0..3 {
        let encrypted = encryption::encrypt(&master_key, path, data).unwrap();
        provider
            .put(&format!("{}.enc", path), &encrypted)
            .await
            .unwrap();
    }

    // Only one key in storage (overwritten each time)
    assert_eq!(provider.len(), 1);

    // File still decrypts correctly
    let downloaded = provider.get(&format!("{}.enc", path)).await.unwrap();
    let decrypted = encryption::decrypt(&master_key, path, &downloaded).unwrap();
    assert_eq!(&decrypted, data);
}

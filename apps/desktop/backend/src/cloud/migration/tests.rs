use super::*;
use crate::cloud::provider::{CloudEntry, CloudStatus};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn pending_file(original_path: &str, data: &[u8]) -> PendingRestoreFile {
    PendingRestoreFile {
        original_path: original_path.to_string(),
        size_bytes: data.len() as u64,
        sha256: compute_sha256(data),
    }
}

async fn write_test_pending_restore(
    app_data_dir: &Path,
    migration_id: &str,
    marker: &PendingRestore,
    staged_files: &[(&str, &[u8])],
) -> (PathBuf, Vec<u8>) {
    let staging_dir = restore_staging_dir(app_data_dir, migration_id);
    tokio::fs::create_dir_all(&staging_dir).await.unwrap();
    for (original_path, data) in staged_files {
        let relative = validated_manifest_relative_path(original_path).unwrap();
        let path = staging_dir.join(relative);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, data).await.unwrap();
    }
    let marker_json = serde_json::to_vec(marker).unwrap();
    tokio::fs::write(staging_dir.join(PENDING_RESTORE_MARKER), &marker_json)
        .await
        .unwrap();
    (staging_dir, marker_json)
}

#[test]
fn test_validated_manifest_relative_path_rejects_traversal() {
    for path in [
        "../thinclaw.db",
        "documents/../../thinclaw.db",
        "/tmp/thinclaw.db",
        "\\tmp\\thinclaw.db",
        "C:\\tmp\\thinclaw.db",
        "documents/./report.txt",
        "documents//report.txt",
    ] {
        assert!(
            validated_manifest_relative_path(path).is_err(),
            "path should be rejected: {}",
            path
        );
    }

    assert_eq!(
        validated_manifest_relative_path("documents/report.txt").unwrap(),
        PathBuf::from("documents").join("report.txt")
    );
    assert_eq!(
        validated_manifest_relative_path("documents\\report.txt").unwrap(),
        PathBuf::from("documents").join("report.txt")
    );
}

#[tokio::test]
async fn test_restore_staging_keeps_live_files_unchanged_when_databases_present() {
    let tmp = tempfile::tempdir().unwrap();
    let staging_dir = restore_staging_dir(tmp.path(), "migration-test");
    let open_live = tmp.path().join("thinclaw.db");
    let runtime_live = tmp.path().join("thinclaw-runtime.db");
    let doc_live = tmp.path().join("documents").join("report.txt");
    let open_staged = staging_dir.join("thinclaw.db");
    let runtime_staged = staging_dir.join("thinclaw-runtime.db");
    let doc_staged = staging_dir.join("documents").join("report.txt");

    tokio::fs::write(&open_live, b"old-open").await.unwrap();
    tokio::fs::write(&runtime_live, b"old-runtime")
        .await
        .unwrap();
    tokio::fs::create_dir_all(doc_live.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&doc_live, b"old-doc").await.unwrap();

    let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 1, "test-key".to_string());
    manifest.add_file(
        "db/thinclaw.db.enc".to_string(),
        "thinclaw.db".to_string(),
        b"new-open",
        64,
    );
    manifest.add_file(
        "db/thinclaw-runtime.db.enc".to_string(),
        "thinclaw-runtime.db".to_string(),
        b"new-runtime",
        64,
    );
    manifest.add_file(
        "documents/report.txt.enc".to_string(),
        "documents/report.txt".to_string(),
        b"new-doc",
        64,
    );
    manifest.files[0].file_type = FileType::Other;
    manifest.files[1].file_type = FileType::Other;

    prepare_restore_staging_dir(tmp.path(), &staging_dir)
        .await
        .unwrap();
    let targets = build_restore_targets(tmp.path(), &staging_dir, &manifest).unwrap();
    for target in &targets {
        let data: &[u8] = match target.manifest_file.original_path.as_str() {
            "thinclaw.db" => b"new-open",
            "thinclaw-runtime.db" => b"new-runtime",
            "documents/report.txt" => b"new-doc",
            other => panic!("unexpected manifest path: {}", other),
        };
        stage_restore_file(&staging_dir, &target.staged_path, data)
            .await
            .unwrap();
    }

    let staged_databases: Vec<&RestoreTarget<'_>> = targets
        .iter()
        .filter(|target| target.file_type == FileType::Database)
        .collect();

    assert_eq!(staged_databases.len(), 2);
    assert_eq!(tokio::fs::read(&open_live).await.unwrap(), b"old-open");
    assert_eq!(
        tokio::fs::read(&runtime_live).await.unwrap(),
        b"old-runtime"
    );
    assert_eq!(tokio::fs::read(&doc_live).await.unwrap(), b"old-doc");
    assert_eq!(tokio::fs::read(&open_staged).await.unwrap(), b"new-open");
    assert_eq!(
        tokio::fs::read(&runtime_staged).await.unwrap(),
        b"new-runtime"
    );
    assert_eq!(tokio::fs::read(&doc_staged).await.unwrap(), b"new-doc");
    assert!(!staging_dir.join(".thinclaw.db.restoring").exists());
    assert!(!staging_dir.join(".thinclaw-runtime.db.restoring").exists());
}

#[tokio::test]
async fn pending_restore_publishes_and_cleans_the_complete_set() {
    let tmp = tempfile::tempdir().unwrap();
    let new_database = b"new-database";
    let new_document = b"new-document";
    tokio::fs::write(tmp.path().join("thinclaw.db"), b"old-database")
        .await
        .unwrap();
    tokio::fs::create_dir_all(tmp.path().join("documents"))
        .await
        .unwrap();
    tokio::fs::write(tmp.path().join("documents/report.txt"), b"old-document")
        .await
        .unwrap();

    let marker = PendingRestore {
        version: 1,
        migration_id: "restore-success".to_string(),
        files: vec![
            pending_file("thinclaw.db", new_database),
            pending_file("documents/report.txt", new_document),
        ],
    };
    let (staging_dir, _) = write_test_pending_restore(
        tmp.path(),
        &marker.migration_id,
        &marker,
        &[
            ("thinclaw.db", new_database),
            ("documents/report.txt", new_document),
        ],
    )
    .await;

    assert!(apply_pending_restore(tmp.path()).await.unwrap());
    assert_eq!(
        tokio::fs::read(tmp.path().join("thinclaw.db"))
            .await
            .unwrap(),
        new_database
    );
    assert_eq!(
        tokio::fs::read(tmp.path().join("documents/report.txt"))
            .await
            .unwrap(),
        new_document
    );
    assert!(!staging_dir.exists());
}

#[tokio::test]
async fn pending_restore_validates_every_staged_file_before_mutating_live_data() {
    let tmp = tempfile::tempdir().unwrap();
    let expected_database = b"new-database";
    let expected_document = b"new-document";
    tokio::fs::write(tmp.path().join("thinclaw.db"), b"old-database")
        .await
        .unwrap();
    tokio::fs::create_dir_all(tmp.path().join("documents"))
        .await
        .unwrap();
    tokio::fs::write(tmp.path().join("documents/report.txt"), b"old-document")
        .await
        .unwrap();

    let marker = PendingRestore {
        version: 1,
        migration_id: "restore-invalid".to_string(),
        files: vec![
            pending_file("thinclaw.db", expected_database),
            pending_file("documents/report.txt", expected_document),
        ],
    };
    write_test_pending_restore(
        tmp.path(),
        &marker.migration_id,
        &marker,
        &[
            ("thinclaw.db", b"bad-database"),
            ("documents/report.txt", expected_document),
        ],
    )
    .await;

    assert!(apply_pending_restore(tmp.path()).await.is_err());
    assert_eq!(
        tokio::fs::read(tmp.path().join("thinclaw.db"))
            .await
            .unwrap(),
        b"old-database"
    );
    assert_eq!(
        tokio::fs::read(tmp.path().join("documents/report.txt"))
            .await
            .unwrap(),
        b"old-document"
    );
}

#[tokio::test]
async fn pending_restore_rolls_back_an_interrupted_uncommitted_activation() {
    let tmp = tempfile::tempdir().unwrap();
    let expected_database = b"new-database";
    let expected_document = b"new-document";
    let database_path = tmp.path().join("thinclaw.db");
    let document_path = tmp.path().join("documents/report.txt");
    tokio::fs::write(&database_path, b"old-database")
        .await
        .unwrap();
    tokio::fs::create_dir_all(document_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&document_path, b"old-document")
        .await
        .unwrap();

    let marker = PendingRestore {
        version: 1,
        migration_id: "restore-interrupted".to_string(),
        files: vec![
            pending_file("thinclaw.db", expected_database),
            pending_file("documents/report.txt", expected_document),
        ],
    };
    let (staging_dir, _) = write_test_pending_restore(
        tmp.path(),
        &marker.migration_id,
        &marker,
        &[
            ("thinclaw.db", b"bad-database"),
            ("documents/report.txt", expected_document),
        ],
    )
    .await;
    let publications = build_restore_publications(tmp.path(), &staging_dir, &marker).unwrap();
    let document = publications
        .iter()
        .find(|publication| publication.file.original_path == "documents/report.txt")
        .unwrap();
    tokio::fs::rename(&document_path, &document.backup_path)
        .await
        .unwrap();
    tokio::fs::write(&document_path, expected_document)
        .await
        .unwrap();

    assert!(apply_pending_restore(tmp.path()).await.is_err());
    assert_eq!(
        tokio::fs::read(&document_path).await.unwrap(),
        b"old-document"
    );
    assert_eq!(
        tokio::fs::read(&database_path).await.unwrap(),
        b"old-database"
    );
    assert!(!document.backup_path.exists());
}

#[tokio::test]
async fn committed_restore_never_reinstates_backups_during_cleanup_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let new_database = b"new-database";
    let new_document = b"new-document";
    let database_path = tmp.path().join("thinclaw.db");
    let document_path = tmp.path().join("documents/report.txt");
    tokio::fs::write(&database_path, new_database)
        .await
        .unwrap();
    tokio::fs::create_dir_all(document_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&document_path, new_document)
        .await
        .unwrap();

    let marker = PendingRestore {
        version: 1,
        migration_id: "restore-committed".to_string(),
        files: vec![
            pending_file("thinclaw.db", new_database),
            pending_file("documents/report.txt", new_document),
        ],
    };
    let (staging_dir, marker_json) =
        write_test_pending_restore(tmp.path(), &marker.migration_id, &marker, &[]).await;
    let publications = build_restore_publications(tmp.path(), &staging_dir, &marker).unwrap();
    for publication in &publications {
        tokio::fs::write(&publication.backup_path, b"old-value")
            .await
            .unwrap();
    }
    tokio::fs::write(
        staging_dir.join(RESTORE_COMMIT_MARKER),
        restore_commit_payload(&marker_json, &marker.migration_id),
    )
    .await
    .unwrap();

    assert!(apply_pending_restore(tmp.path()).await.unwrap());
    assert_eq!(tokio::fs::read(&database_path).await.unwrap(), new_database);
    assert_eq!(tokio::fs::read(&document_path).await.unwrap(), new_document);
    assert!(!staging_dir.exists());
}

#[derive(Clone)]
struct CasTestProvider {
    capability: CloudSyncCapability,
    state: Arc<Mutex<CasTestState>>,
}

#[derive(Default)]
struct CasTestState {
    objects: HashMap<String, (Vec<u8>, u64)>,
    next_version: u64,
}

impl CasTestProvider {
    fn new(capability: CloudSyncCapability) -> Self {
        Self {
            capability,
            state: Arc::new(Mutex::new(CasTestState::default())),
        }
    }
}

#[async_trait]
impl CloudProvider for CasTestProvider {
    fn name(&self) -> &str {
        "migration-test"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        Ok(CloudStatus {
            connected: true,
            storage_used: 0,
            storage_available: None,
            provider_name: self.name().to_string(),
        })
    }

    fn sync_capability(&self) -> CloudSyncCapability {
        self.capability
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        let mut state = self.state.lock().unwrap();
        state.next_version += 1;
        let version = state.next_version;
        state
            .objects
            .insert(key.to_string(), (data.to_vec(), version));
        Ok(())
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
        let state = self.state.lock().unwrap();
        let data = state
            .objects
            .get(key)
            .map(|(data, _)| data)
            .ok_or_else(|| CloudError::NotFound(key.to_string()))?;
        if data.len() > max_bytes {
            return Err(CloudError::ObjectTooLarge { limit: max_bytes });
        }
        Ok(data.clone())
    }

    async fn get_versioned_bounded(
        &self,
        key: &str,
        max_bytes: usize,
    ) -> Result<VersionedObject, CloudError> {
        let state = self.state.lock().unwrap();
        let (data, version) = state
            .objects
            .get(key)
            .ok_or_else(|| CloudError::NotFound(key.to_string()))?;
        if data.len() > max_bytes {
            return Err(CloudError::ObjectTooLarge { limit: max_bytes });
        }
        Ok(VersionedObject {
            data: data.clone(),
            version: ObjectVersion::new(version.to_string())?,
        })
    }

    async fn put_if_version(
        &self,
        key: &str,
        data: &[u8],
        expected: Option<&ObjectVersion>,
    ) -> Result<ObjectVersion, CloudError> {
        let mut state = self.state.lock().unwrap();
        let current = state.objects.get(key).map(|(_, version)| *version);
        let matches = match (current, expected) {
            (None, None) => true,
            (Some(current), Some(expected)) => current.to_string() == expected.as_str(),
            _ => false,
        };
        if !matches {
            return Err(CloudError::ArchiveConflict);
        }
        state.next_version += 1;
        let version = state.next_version;
        state
            .objects
            .insert(key.to_string(), (data.to_vec(), version));
        ObjectVersion::new(version.to_string())
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        self.state.lock().unwrap().objects.remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .objects
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, (data, _))| CloudEntry {
                key: key.clone(),
                size: data.len() as u64,
                last_modified: 0,
                checksum: None,
            })
            .collect())
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        Ok(0)
    }
}

fn test_manifest() -> ArchiveManifest {
    let mut manifest = ArchiveManifest::new("test".to_string(), 1, "test-key".to_string());
    manifest.add_file(
        "db/thinclaw.db.enc".to_string(),
        "thinclaw.db".to_string(),
        b"database",
        64,
    );
    manifest
}

#[test]
fn concurrent_v1_migrators_derive_one_archive_identity() {
    let mut legacy = test_manifest();
    legacy.version = 1;
    legacy.archive_id.clear();
    legacy.writer_id.clear();
    legacy.generation = 0;
    let first_writer = uuid::Uuid::new_v4().to_string();
    let second_writer = uuid::Uuid::new_v4().to_string();
    let mut first = test_manifest();
    let mut second = test_manifest();

    carry_forward_archive_identity(&mut first, Some(&legacy), &first_writer).unwrap();
    carry_forward_archive_identity(&mut second, Some(&legacy), &second_writer).unwrap();

    assert_eq!(first.archive_id, second.archive_id);
    assert_eq!(first.generation, 1);
    assert_eq!(second.generation, 1);
    assert_ne!(first.writer_id, second.writer_id);
}

#[tokio::test]
async fn rollback_requires_the_migration_owned_version() {
    let provider = CasTestProvider::new(CloudSyncCapability::StrongCas);
    let previous_bytes = b"previous".to_vec();
    let previous_version = provider
        .put_if_version(MANIFEST_KEY, &previous_bytes, None)
        .await
        .unwrap();
    let previous = ExistingManifest {
        ciphertext: previous_bytes.clone(),
        version: previous_version.clone(),
        manifest: test_manifest(),
    };
    let migration_version = provider
        .put_if_version(MANIFEST_KEY, b"migration", Some(&previous_version))
        .await
        .unwrap();
    let competing_version = provider
        .put_if_version(MANIFEST_KEY, b"competing", Some(&migration_version))
        .await
        .unwrap();

    assert!(
        restore_previous_manifest_if_owned(&provider, &migration_version, Some(&previous))
            .await
            .is_err()
    );
    let observed = provider
        .get_versioned_bounded(MANIFEST_KEY, 1024)
        .await
        .unwrap();
    assert_eq!(observed.data, b"competing");
    assert_eq!(observed.version, competing_version);
}

#[tokio::test]
async fn concurrent_migration_cannot_blindly_overwrite_the_winner() {
    let provider = CasTestProvider::new(CloudSyncCapability::StrongCas);
    let initial = provider
        .put_if_version(MANIFEST_KEY, b"initial", None)
        .await
        .unwrap();
    let winner = provider
        .put_if_version(MANIFEST_KEY, b"winner", Some(&initial))
        .await
        .unwrap();

    assert!(matches!(
        provider
            .put_if_version(MANIFEST_KEY, b"stale-migrator", Some(&initial))
            .await,
        Err(CloudError::ArchiveConflict)
    ));
    let observed = provider
        .get_versioned_bounded(MANIFEST_KEY, 1024)
        .await
        .unwrap();
    assert_eq!(observed.data, b"winner");
    assert_eq!(observed.version, winner);
}

#[tokio::test]
async fn owned_migration_revision_can_be_rolled_back_by_cas() {
    let provider = CasTestProvider::new(CloudSyncCapability::StrongCas);
    let previous_bytes = b"previous".to_vec();
    let previous_version = provider
        .put_if_version(MANIFEST_KEY, &previous_bytes, None)
        .await
        .unwrap();
    let previous = ExistingManifest {
        ciphertext: previous_bytes.clone(),
        version: previous_version.clone(),
        manifest: test_manifest(),
    };
    let migration_version = provider
        .put_if_version(MANIFEST_KEY, b"migration", Some(&previous_version))
        .await
        .unwrap();

    restore_previous_manifest_if_owned(&provider, &migration_version, Some(&previous))
        .await
        .unwrap();
    assert_eq!(provider.get(MANIFEST_KEY).await.unwrap(), previous_bytes);
}

#[tokio::test]
async fn backup_restore_selects_latest_immutable_manifest_without_shared_pointer() {
    let provider = CasTestProvider::new(CloudSyncCapability::BackupOnly);
    provider.put(MANIFEST_KEY, b"legacy-shared").await.unwrap();
    provider
        .put(
            "backups/manifests/00000000000000000001-old.json.enc",
            b"old-backup",
        )
        .await
        .unwrap();
    provider
        .put(
            "backups/manifests/00000000000000000002-new.json.enc",
            b"new-backup",
        )
        .await
        .unwrap();

    let (key, data) = find_manifest_for_restore(&provider).await.unwrap().unwrap();
    assert_eq!(key, "backups/manifests/00000000000000000002-new.json.enc");
    assert_eq!(data, b"new-backup");
    assert_eq!(provider.get(MANIFEST_KEY).await.unwrap(), b"legacy-shared");
}

#[tokio::test]
async fn backup_publication_refuses_the_mutable_live_manifest_key() {
    let provider = CasTestProvider::new(CloudSyncCapability::BackupOnly);
    assert!(
        publish_immutable_backup_manifest(&provider, MANIFEST_KEY, b"backup")
            .await
            .is_err()
    );
    assert!(matches!(
        provider.get(MANIFEST_KEY).await,
        Err(CloudError::NotFound(_))
    ));
}

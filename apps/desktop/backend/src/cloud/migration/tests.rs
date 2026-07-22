use super::*;
use std::path::PathBuf;

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

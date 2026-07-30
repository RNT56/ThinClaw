use super::*;

#[test]
fn active_download_guard_rejects_duplicate_artifact_identity() {
    let download_id = "hf-test-active-download";
    let first = ActiveHfDownloadGuard::acquire(download_id).unwrap();
    assert!(ActiveHfDownloadGuard::acquire(download_id).is_err());
    drop(first);
    assert!(ActiveHfDownloadGuard::acquire(download_id).is_ok());
}

#[cfg(unix)]
#[test]
fn post_publish_parent_sync_failure_does_not_turn_install_into_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let category = temp.path().join("LLM");
    let published = category.join("published-model");
    std::fs::create_dir_all(&published).expect("published model");
    std::fs::write(published.join("model.gguf"), minimal_gguf()).expect("published artifact");
    let attempted = std::sync::atomic::AtomicBool::new(false);

    sync_published_hf_category_with(&category, |path| {
        assert_eq!(path, category);
        attempted.store(true, std::sync::atomic::Ordering::SeqCst);
        Err(std::io::Error::other("injected parent sync failure"))
    });

    assert!(attempted.load(std::sync::atomic::Ordering::SeqCst));
    assert!(published.join("model.gguf").is_file());
}

#[tokio::test]
async fn cancellable_hf_operation_interrupts_an_in_flight_wait() {
    let cancel = std::sync::Arc::new(tokio::sync::Notify::new());
    let task_cancel = cancel.clone();
    let operation = tokio::spawn(async move {
        cancellable_hf_operation(&task_cancel, std::future::pending::<Result<(), String>>())
            .await
    });
    tokio::task::yield_now().await;
    cancel.notify_one();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), operation)
        .await
        .expect("cancellation should be prompt")
        .expect("operation task");
    assert_eq!(result, Err(HF_DOWNLOAD_CANCELLED.to_string()));

    let notify = tokio::sync::Notify::new();
    assert_eq!(
        cancellable_hf_operation(&notify, async { Ok::<_, String>(7_u8) }).await,
        Ok(7)
    );
}

#[tokio::test]
async fn http_multifile_download_publishes_atomic_manifest_and_progress() {
    use sha2::Digest as _;

    let first = minimal_gguf();
    let second = minimal_gguf();
    let first_hash = hex::encode(sha2::Sha256::digest(&first));
    let second_hash = hex::encode(sha2::Sha256::digest(&second));
    let server = MockHttpServer::spawn(|_| {
        vec![
            MockHttpResponse::bytes(200, first.clone()),
            MockHttpResponse::bytes(200, second.clone()),
        ]
    })
    .await;
    let temp = tempfile::tempdir().expect("app data");
    let client = build_hf_client_with_token(None).expect("HTTP client");
    let files = vec![
        PlannedDownloadFile {
            path: "model-Q4_K_M-00001-of-00002.gguf".to_string(),
            expected_size: Some(first.len() as u64),
            sha256: Some(first_hash),
        },
        PlannedDownloadFile {
            path: "model-Q4_K_M-00002-of-00002.gguf".to_string(),
            expected_size: Some(second.len() as u64),
            sha256: Some(second_hash),
        },
    ];
    let manifest = managed_gguf_manifest(
        "model-Q4_K_M-00001-of-00002.gguf",
        &[
            "model-Q4_K_M-00001-of-00002.gguf",
            "model-Q4_K_M-00002-of-00002.gguf",
        ],
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded_events = events.clone();
    let cancel = tokio::sync::Notify::new();

    let result = download_planned_hf_files_with_http(
        temp.path(),
        &client,
        &server.base_url,
        Some("hf_test_download_token"),
        move |event| {
            recorded_events.lock().expect("download events").push(event);
        },
        "owner/repo",
        "0123456789abcdef0123456789abcdef01234567",
        &files,
        "http-download",
        "LLM",
        "http-download-id",
        &cancel,
        &manifest,
    )
    .await
    .expect("multi-file HTTP download");

    let destination = temp.path().join("models/LLM/http-download");
    assert_eq!(result.destination_dir, destination);
    assert_eq!(result.total_bytes, (first.len() + second.len()) as u64);
    assert_eq!(result.downloaded_files.len(), 2);
    assert_eq!(
        std::fs::read(destination.join(&files[0].path)).expect("first published shard"),
        first
    );
    assert_eq!(
        std::fs::read(destination.join(&files[1].path)).expect("second published shard"),
        second
    );
    let persisted: crate::model_manager::ManagedModelManifest = serde_json::from_slice(
        &std::fs::read(destination.join(crate::model_manager::MODEL_MANIFEST_FILENAME))
            .expect("published manifest"),
    )
    .expect("valid published manifest");
    assert_eq!(persisted.install_id, manifest.install_id);
    assert_eq!(persisted.files, manifest.files);
    let category_entries: Vec<String> = std::fs::read_dir(temp.path().join("models/LLM"))
        .expect("model category")
        .map(|entry| {
            entry
                .expect("category entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(category_entries, ["http-download"]);

    {
        let events = events.lock().expect("download events");
        assert!(events.iter().any(|event| event["status"] == "downloading"));
        assert_eq!(
            events.last().and_then(|event| event["status"].as_str()),
            Some("completed")
        );
        assert_eq!(
            events.last().and_then(|event| event["percentage"].as_f64()),
            Some(100.0)
        );
    }

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.headers.get("authorization").map(String::as_str)
            == Some("Bearer hf_test_download_token")
    }));
    assert!(requests[0].target.ends_with(&files[0].path));
    assert!(requests[1].target.ends_with(&files[1].path));
    server.finish().await;
}

#[tokio::test]
async fn mock_app_handle_resolves_app_data_managed_state_and_progress_events() {
    use sha2::Digest as _;
    use tauri::Listener as _;

    let first = minimal_gguf();
    let second = minimal_gguf();
    let first_hash = hex::encode(sha2::Sha256::digest(&first));
    let second_hash = hex::encode(sha2::Sha256::digest(&second));
    let server = MockHttpServer::spawn(|_| {
        vec![
            MockHttpResponse::bytes(200, first.clone()),
            MockHttpResponse::bytes(200, second.clone()),
        ]
    })
    .await;
    let temp = tempfile::tempdir().expect("mock app data");
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = temp.path().to_string_lossy().to_string();
    let app = tauri::test::mock_builder()
        .manage(crate::secret_store::SecretStore::new())
        .manage(crate::model_manager::DownloadManager::new())
        .build(context)
        .expect("mock Tauri app");
    let app_handle = app.handle().clone();
    assert_eq!(
        app_handle
            .path()
            .app_data_dir()
            .expect("mock app data path"),
        temp.path()
    );
    assert!(app_handle
        .try_state::<crate::secret_store::SecretStore>()
        .is_some());
    assert!(app_handle
        .try_state::<crate::model_manager::DownloadManager>()
        .is_some());

    let active_engine = crate::engine::direct_runtime_get_active_engine_info();
    let capabilities = direct_runtime_get_hf_capabilities();
    assert_eq!(
        capabilities.len(),
        HF_CAPABILITY_PROFILES
            .iter()
            .filter(|profile| profile.engine_id == active_engine.id)
            .count()
    );
    assert!(capabilities
        .iter()
        .all(|profile| profile.engine_id == active_engine.id));

    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded_events = events.clone();
    app_handle.listen("download_progress", move |event| {
        recorded_events.lock().expect("Tauri progress events").push(
            serde_json::from_str::<serde_json::Value>(event.payload())
                .expect("JSON progress event"),
        );
    });

    let files = vec![
        PlannedDownloadFile {
            path: "model-Q4_K_M-00001-of-00002.gguf".to_string(),
            expected_size: Some(first.len() as u64),
            sha256: Some(first_hash),
        },
        PlannedDownloadFile {
            path: "model-Q4_K_M-00002-of-00002.gguf".to_string(),
            expected_size: Some(second.len() as u64),
            sha256: Some(second_hash),
        },
    ];
    let manifest = managed_gguf_manifest(
        "model-Q4_K_M-00001-of-00002.gguf",
        &[
            "model-Q4_K_M-00001-of-00002.gguf",
            "model-Q4_K_M-00002-of-00002.gguf",
        ],
    );
    let manager = app_handle.state::<crate::model_manager::DownloadManager>();
    let (cancel, _registration) = manager
        .register("mock-app-download-id")
        .expect("managed download registration");
    let client = build_hf_client_with_token(None).expect("HTTP client");

    let result = download_planned_hf_files_from_app(
        &app_handle,
        &client,
        &server.base_url,
        Some("hf_mock_app_token"),
        "owner/repo",
        "0123456789abcdef0123456789abcdef01234567",
        &files,
        "mock-app-download",
        "LLM",
        "mock-app-download-id",
        &cancel,
        &manifest,
    )
    .await
    .expect("mock AppHandle download wrapper");

    assert_eq!(
        result.destination_dir,
        temp.path().join("models/LLM/mock-app-download")
    );
    assert!(result
        .destination_dir
        .join(crate::model_manager::MODEL_MANIFEST_FILENAME)
        .is_file());
    {
        let events = events.lock().expect("Tauri progress events");
        assert!(events.iter().any(|event| event["status"] == "downloading"));
        assert_eq!(
            events.last().and_then(|event| event["status"].as_str()),
            Some("completed")
        );
    }
    assert!(server.requests().iter().all(|request| {
        request.headers.get("authorization").map(String::as_str)
            == Some("Bearer hf_mock_app_token")
    }));
    server.finish().await;
}

#[tokio::test]
async fn http_download_cancellation_removes_private_staging_state() {
    let delayed_body = vec![0_u8; 4_096];
    let server = MockHttpServer::spawn(|_| {
        vec![MockHttpResponse::bytes(200, delayed_body.clone())
            .with_body_delay(std::time::Duration::from_millis(500))]
    })
    .await;
    let temp = tempfile::tempdir().expect("app data");
    let app_data = temp.path().to_path_buf();
    let client = build_hf_client_with_token(None).expect("HTTP client");
    let base_url = server.base_url.clone();
    let files = vec![PlannedDownloadFile {
        path: "model.gguf".to_string(),
        expected_size: Some(delayed_body.len() as u64),
        sha256: None,
    }];
    let manifest = managed_gguf_manifest("model.gguf", &["model.gguf"]);
    let cancel = Arc::new(tokio::sync::Notify::new());
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        download_planned_hf_files_with_http(
            &app_data,
            &client,
            &base_url,
            None,
            |_| {},
            "owner/repo",
            "0123456789abcdef0123456789abcdef01234567",
            &files,
            "cancelled-download",
            "LLM",
            "cancelled-download-id",
            &task_cancel,
            &manifest,
        )
        .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !server.requests().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("download request reached server");
    cancel.notify_one();
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
        .await
        .expect("cancellation completed")
        .expect("download task")
        .expect_err("download was cancelled")
        .to_string();
    assert!(error.contains(HF_DOWNLOAD_CANCELLED));

    let category = temp.path().join("models/LLM");
    assert!(!category.join("cancelled-download").exists());
    if category.exists() {
        assert!(std::fs::read_dir(&category)
            .expect("model category")
            .all(|entry| !entry
                .expect("category entry")
                .file_name()
                .to_string_lossy()
                .starts_with(HF_STAGING_PREFIX)));
    }
    server.finish().await;
}

#[test]
fn stale_staging_cleanup_requires_exact_name_marker_and_age() {
    let temp = tempfile::tempdir().expect("tempdir");
    let category = temp.path().join("LLM");
    std::fs::create_dir(&category).expect("category");

    let mut stale = DownloadStagingGuard::create(&category).expect("stale staging");
    let stale_path = stale.path.clone();
    std::fs::write(stale_path.join("partial.gguf"), b"partial").expect("partial");
    stale
        .marker
        .as_ref()
        .expect("marker")
        .set_modified(
            std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(2 * 60 * 60))
                .expect("old timestamp"),
        )
        .expect("age marker");
    stale.committed = true;
    drop(stale);

    let mut fresh = DownloadStagingGuard::create(&category).expect("fresh staging");
    let fresh_path = fresh.path.clone();
    fresh.committed = true;
    drop(fresh);

    let invalid_marker = category.join(".thinclaw-hf-11111111111111111111111111111111.staging");
    std::fs::create_dir(&invalid_marker).expect("invalid marker directory");
    std::fs::write(
        invalid_marker.join(HF_STAGING_MARKER_FILENAME),
        b"not-owned",
    )
    .expect("invalid marker");

    let lookalike = category.join(".thinclaw-hf-not-a-uuid.staging");
    std::fs::create_dir(&lookalike).expect("lookalike");

    let removed = cleanup_stale_hf_staging_dirs_at(
        &category,
        std::time::SystemTime::now(),
        std::time::Duration::from_secs(60 * 60),
        std::time::Duration::from_secs(30 * 24 * 60 * 60),
    )
    .expect("cleanup");

    assert_eq!(removed, 1);
    assert!(!stale_path.exists());
    assert!(fresh_path.is_dir());
    assert!(invalid_marker.is_dir());
    assert!(lookalike.is_dir());
}

#[test]
fn stale_staging_cleanup_recovers_legacy_unmarked_directory_after_long_grace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let category = temp.path().join("LLM");
    std::fs::create_dir(&category).expect("category");
    let legacy = category.join(".thinclaw-hf-22222222222222222222222222222222.staging");
    std::fs::create_dir(&legacy).expect("legacy staging");
    std::fs::write(legacy.join("partial.gguf"), b"partial").expect("legacy partial");

    let now = std::time::SystemTime::now();
    assert_eq!(
        cleanup_stale_hf_staging_dirs_at(
            &category,
            now,
            std::time::Duration::from_secs(60 * 60),
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
        )
        .expect("fresh legacy cleanup"),
        0
    );
    assert!(legacy.is_dir());

    assert_eq!(
        cleanup_stale_hf_staging_dirs_at(
            &category,
            now + std::time::Duration::from_secs(31 * 24 * 60 * 60),
            std::time::Duration::from_secs(60 * 60),
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
        )
        .expect("stale legacy cleanup"),
        1
    );
    assert!(!legacy.exists());
}

#[test]
fn staged_artifact_validation_checks_required_content_without_parsing_large_tokenizers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let gguf_dir = temp.path().join("gguf");
    std::fs::create_dir(&gguf_dir).expect("gguf directory");
    std::fs::write(gguf_dir.join("model.gguf"), minimal_gguf()).expect("gguf");
    let gguf_manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "gguf".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "llamacpp".to_string(),
        format: "gguf".to_string(),
        artifact_kind: "gguf_single".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: Some("model.gguf".to_string()),
        files: vec!["model.gguf".to_string()],
        quantization: Some("Q4_K_M".to_string()),
    };
    assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_ok());
    std::fs::write(gguf_dir.join("model.gguf"), b"GGUFpayload").expect("mistagged gguf");
    assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_err());
    std::fs::write(gguf_dir.join("model.gguf"), b"nope").expect("bad gguf");
    assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_err());

    let mlx_dir = temp.path().join("mlx");
    std::fs::create_dir(&mlx_dir).expect("mlx directory");
    std::fs::write(mlx_dir.join("config.json"), br#"{"model_type":"test"}"#).expect("config");
    std::fs::write(mlx_dir.join("model.safetensors"), b"weights").expect("weights");
    let tokenizer = std::fs::File::create(mlx_dir.join("tokenizer.json")).expect("tokenizer");
    tokenizer
        .set_len(MAX_HF_API_RESPONSE_BYTES as u64 + 1)
        .expect("large tokenizer");
    let mlx_manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "mlx".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "mlx".to_string(),
        format: "mlx".to_string(),
        artifact_kind: "directory".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: None,
        files: vec![
            "config.json".to_string(),
            "model.safetensors".to_string(),
            "tokenizer.json".to_string(),
        ],
        quantization: None,
    };
    assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_ok());
    std::fs::write(mlx_dir.join("generation_config.json"), b"").expect("empty auxiliary file");
    let mut manifest_with_empty_auxiliary = mlx_manifest.clone();
    manifest_with_empty_auxiliary
        .files
        .push("generation_config.json".to_string());
    assert!(
        validate_staged_hf_artifact(&mlx_dir, &manifest_with_empty_auxiliary).is_err(),
        "staging and managed inventory must reject the same empty declared files"
    );
    std::fs::remove_file(mlx_dir.join("generation_config.json"))
        .expect("remove empty auxiliary file");
    std::fs::write(mlx_dir.join("config.json"), b"{invalid").expect("bad config");
    assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_err());
    std::fs::write(mlx_dir.join("config.json"), b"{}").expect("config");
    std::fs::write(mlx_dir.join("model.safetensors"), b"").expect("empty weights");
    assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_err());
}

#[test]
fn staged_mlx_vision_artifact_requires_config_and_vision_tensor_keys() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("mlx-vision");
    std::fs::create_dir(&directory).expect("MLX vision directory");
    std::fs::write(
        directory.join("config.json"),
        br#"{
            "architectures":["LlavaForConditionalGeneration"],
            "vision_config":{}
        }"#,
    )
    .expect("vision config");
    std::fs::write(directory.join("model.safetensors"), [0_u8; 8]).expect("vision weights");
    std::fs::write(
        directory.join("model.safetensors.index.json"),
        br#"{"weight_map":{"vision_tower.layer.weight":"model.safetensors"}}"#,
    )
    .expect("vision weight index");
    std::fs::write(directory.join("tokenizer.json"), b"tokenizer").expect("tokenizer");
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "mlx-vision".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("vision".to_string()),
        runtime: "mlx".to_string(),
        format: "mlx".to_string(),
        artifact_kind: "directory".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: None,
        files: vec![
            "config.json".to_string(),
            "model.safetensors".to_string(),
            "model.safetensors.index.json".to_string(),
            "tokenizer.json".to_string(),
        ],
        quantization: None,
    };
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

    std::fs::write(
        directory.join("model.safetensors.index.json"),
        br#"{"weight_map":{"language_model.layer.weight":"model.safetensors"}}"#,
    )
    .expect("text-only weight index");
    assert!(
        validate_staged_hf_artifact(&directory, &manifest).is_err(),
        "a config marker without vision tensor keys must not be installed"
    );

    std::fs::write(
        directory.join("model.safetensors.index.json"),
        br#"{"weight_map":{"vision_model.layer.weight":"model.safetensors"}}"#,
    )
    .expect("restore vision weight index");
    std::fs::write(
        directory.join("config.json"),
        br#"{"architectures":["LlamaForCausalLM"]}"#,
    )
    .expect("text-only config");
    assert!(
        validate_staged_hf_artifact(&directory, &manifest).is_err(),
        "a tagged text-only repository must not complete a vision install"
    );
}

#[test]
fn staged_gguf_validation_parses_every_model_and_mmproj_shard() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("vision");
    std::fs::create_dir(&directory).expect("vision directory");
    let files = [
        "model-00001-of-00002.gguf",
        "model-00002-of-00002.gguf",
        "mmproj-00001-of-00002.gguf",
        "mmproj-00002-of-00002.gguf",
    ];
    for relative in files {
        std::fs::write(directory.join(relative), minimal_gguf()).expect("GGUF shard");
    }
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "vision-gguf".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("vision".to_string()),
        runtime: "llamacpp".to_string(),
        format: "gguf".to_string(),
        artifact_kind: "gguf_sharded".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: Some("mmproj".to_string()),
        companion_path: Some("mmproj-00001-of-00002.gguf".to_string()),
        primary_path: Some("model-00001-of-00002.gguf".to_string()),
        files: files.into_iter().map(str::to_string).collect(),
        quantization: Some("Q4_K_M".to_string()),
    };
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

    std::fs::write(directory.join("model-00002-of-00002.gguf"), b"GGUFpayload")
        .expect("corrupt model shard");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
    std::fs::write(directory.join("model-00002-of-00002.gguf"), minimal_gguf())
        .expect("restore model shard");

    std::fs::write(directory.join("mmproj-00002-of-00002.gguf"), b"GGUFpayload")
        .expect("corrupt projector shard");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
}

#[test]
fn staged_mflux_artifact_requires_supported_flux_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("mflux");
    std::fs::create_dir(&directory).expect("mflux directory");
    std::fs::write(
        directory.join("config.json"),
        br#"{
            "_class_name":"FluxPipeline",
            "model_type":"flux-rectified-flow",
            "original_model":"black-forest-labs/FLUX.1-schnell",
            "quantization":{"method":"mflux","bits":4}
        }"#,
    )
    .expect("mflux config");
    std::fs::write(directory.join("weights.safetensors"), b"weights").expect("mflux weights");
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "mflux".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "Diffusion".to_string(),
        task: Some("diffusion".to_string()),
        runtime: "mlx".to_string(),
        format: "mflux".to_string(),
        artifact_kind: "directory".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: None,
        files: vec!["config.json".to_string(), "weights.safetensors".to_string()],
        quantization: None,
    };
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

    std::fs::write(directory.join("config.json"), b"{}").expect("unsupported config");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
}

#[test]
fn staged_mlx_embedding_artifact_requires_pinned_text_vector_architecture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("mlx-embedding");
    std::fs::create_dir(&directory).expect("MLX embedding directory");
    std::fs::write(
        directory.join("config.json"),
        br#"{"model_type":"bert","architectures":["BertModel"]}"#,
    )
    .expect("embedding config");
    std::fs::write(directory.join("model.safetensors"), b"weights").expect("embedding weights");
    std::fs::write(directory.join("tokenizer.json"), b"tokenizer")
        .expect("embedding tokenizer");
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "mlx-embedding".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "Embedding".to_string(),
        task: Some("embedding".to_string()),
        runtime: "mlx".to_string(),
        format: "mlx".to_string(),
        artifact_kind: "directory".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: None,
        files: vec![
            "config.json".to_string(),
            "model.safetensors".to_string(),
            "tokenizer.json".to_string(),
        ],
        quantization: None,
    };
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

    std::fs::write(
        directory.join("config.json"),
        br#"{"model_type":"qwen2","architectures":["Qwen2Model"]}"#,
    )
    .expect("unsupported embedding config");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());

    std::fs::write(
        directory.join("config.json"),
        br#"{
            "model_type":"modernbert",
            "architectures":["ModernBertForMaskedLM"]
        }"#,
    )
    .expect("token-level embedding config");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
}

#[test]
fn staged_vllm_artifact_requires_awq_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("awq");
    std::fs::create_dir(&directory).expect("AWQ directory");
    std::fs::write(
        directory.join("config.json"),
        br#"{"quantization_config":{"quant_method":"AWQ"}}"#,
    )
    .expect("AWQ config");
    std::fs::write(directory.join("model.safetensors"), b"weights").expect("AWQ weights");
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "awq".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "vllm".to_string(),
        format: "awq".to_string(),
        artifact_kind: "directory".to_string(),
        artifact_id: Some("main".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: None,
        files: vec!["config.json".to_string(), "model.safetensors".to_string()],
        quantization: None,
    };
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

    std::fs::write(
        directory.join("config.json"),
        br#"{"quantization_config":{"quant_method":"gptq"}}"#,
    )
    .expect("mistagged config");
    assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
}

#[test]
fn only_llamacpp_vision_requires_an_mmproj_companion() {
    assert!(task_requires_mmproj("llamacpp", HfModelTask::Vision));
    assert!(!task_requires_mmproj("llamacpp", HfModelTask::Chat));
    assert!(!task_requires_mmproj("mlx", HfModelTask::Vision));
    assert!(!task_requires_mmproj("vllm", HfModelTask::Vision));
}

#[cfg(unix)]
#[test]
fn stale_staging_cleanup_never_follows_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let category = temp.path().join("LLM");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&category).expect("category");
    std::fs::create_dir(&outside).expect("outside");
    std::fs::write(outside.join("keep"), b"safe").expect("outside file");
    let link = category.join(".thinclaw-hf-33333333333333333333333333333333.staging");
    symlink(&outside, &link).expect("staging symlink");

    assert_eq!(
        cleanup_stale_hf_staging_dirs_at(
            &category,
            std::time::SystemTime::now() + std::time::Duration::from_secs(365 * 24 * 60 * 60),
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .expect("cleanup"),
        0
    );
    assert!(link.exists());
    assert_eq!(
        std::fs::read(outside.join("keep")).expect("outside file"),
        b"safe"
    );
}

#[test]
fn tree_next_link_keeps_the_exact_recursive_projection() {
    let current = reqwest::Url::parse(
        "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?recursive=true&expand=false&limit=1000",
    )
    .unwrap();
    let expected = parse_hf_tree_route(&current).unwrap().identity;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::LINK,
        reqwest::header::HeaderValue::from_static(
            "<https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?expand=false&recursive=true&limit=1000&cursor=abc>; rel=\"next\"",
        ),
    );
    let next = parse_tree_next_link(&headers, &current, &expected)
        .unwrap()
        .expect("next link");
    assert!(next
        .query_pairs()
        .any(|(key, value)| key == "cursor" && value == "abc"));

    headers.insert(
        reqwest::header::LINK,
        reqwest::header::HeaderValue::from_static(
            "<https://example.com/api/models/acme/model/tree/0123456789012345678901234567890123456789?expand=false&recursive=true&limit=1000&cursor=abc>; rel=\"next\"",
        ),
    );
    assert!(parse_tree_next_link(&headers, &current, &expected).is_err());
}

#[test]
fn tree_next_link_rejects_query_drift_and_invalid_cursors() {
    let current = reqwest::Url::parse(
        "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?recursive=true&expand=false&limit=1000",
    )
    .unwrap();
    let expected = parse_hf_tree_route(&current).unwrap().identity;
    let route = "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789";
    for query in [
        "recursive=false&expand=false&limit=1000&cursor=abc",
        "recursive=true&expand=true&limit=1000&cursor=abc",
        "recursive=true&expand=false&limit=999&cursor=abc",
        "recursive=true&limit=1000&cursor=abc",
        "recursive=true&expand=false&limit=1000&extra=true&cursor=abc",
        "recursive=true&expand=false&limit=1000&cursor=abc&cursor=def",
        "recursive=true&expand=false&limit=1000&cursor=",
    ] {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_str(&format!("<{route}?{query}>; rel=\"next\""))
                .unwrap(),
        );
        assert!(
            parse_tree_next_link(&headers, &current, &expected).is_err(),
            "tree pagination drift should be rejected: {query}"
        );
    }

    let oversized = "x".repeat(8_193);
    let oversized_url = reqwest::Url::parse(&format!(
        "{route}?recursive=true&expand=false&limit=1000&cursor={oversized}"
    ))
    .unwrap();
    assert!(parse_hf_tree_route(&oversized_url).is_err());
    let control_url = reqwest::Url::parse(&format!(
        "{route}?recursive=true&expand=false&limit=1000&cursor=abc%0Adef"
    ))
    .unwrap();
    assert!(parse_hf_tree_route(&control_url).is_err());
}

#[tokio::test]
async fn tree_pagination_cycle_is_rejected_before_repeating_a_request() {
    let revision = "0123456789012345678901234567890123456789";
    let server = MockHttpServer::spawn(|base_url| {
        let page_url = |cursor: &str| {
            let mut url =
                hf_model_api_url_at(base_url, "owner/repo", &["tree", revision]).unwrap();
            url.query_pairs_mut()
                .append_pair("recursive", "true")
                .append_pair("expand", "false")
                .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string())
                .append_pair("cursor", cursor);
            url
        };
        let page_a = page_url("page-a");
        let page_b = page_url("page-b");
        let mut page_a_reordered =
            hf_model_api_url_at(base_url, "owner/repo", &["tree", revision]).unwrap();
        page_a_reordered
            .query_pairs_mut()
            .append_pair("expand", "false")
            .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string())
            .append_pair("recursive", "true")
            .append_pair("cursor", "page-a");
        vec![
            MockHttpResponse::json(200, serde_json::json!([]))
                .with_header("Link", format!("<{page_a}>; rel=\"next\"")),
            MockHttpResponse::json(200, serde_json::json!([]))
                .with_header("Link", format!("<{page_b}>; rel=\"next\"")),
            MockHttpResponse::json(200, serde_json::json!([]))
                .with_header("Link", format!("<{page_a_reordered}>; rel=\"next\"")),
        ]
    })
    .await;
    let client = build_hf_client_with_token(None).unwrap();

    let error = fetch_repo_tree(&client, &server.base_url, "owner/repo", revision)
        .await
        .expect_err("tree pagination cycle");

    assert!(error.contains("cycle"));
    assert_eq!(server.requests().len(), 3);
    server.finish().await;
}

#[test]
fn search_next_link_keeps_the_exact_filter_route() {
    let current = hf_model_search_url(
        "whisper",
        "mlx",
        2,
        Some("automatic-speech-recognition"),
        false,
    )
    .expect("search URL");
    let expected = parse_hf_search_route(&current)
        .expect("search route")
        .identity;

    // The live Hub Link header rewrites `expand[]` keys to `expand`.
    let current_pairs: Vec<(String, String)> = current
        .query_pairs()
        .map(|(key, value)| {
            (
                if key == "expand[]" {
                    "expand".to_string()
                } else {
                    key.into_owned()
                },
                value.into_owned(),
            )
        })
        .collect();
    let mut next = current.clone();
    next.set_query(None);
    {
        let mut query = next.query_pairs_mut();
        for (key, value) in &current_pairs {
            query.append_pair(key, value);
        }
        query.append_pair("cursor", "opaque-page-token");
    }
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::LINK,
        reqwest::header::HeaderValue::from_str(&format!("<{next}>; rel=\"next\""))
            .expect("Link header"),
    );
    let parsed = parse_search_next_link(&headers, &current, &expected)
        .expect("valid Link")
        .expect("next page");
    assert!(parsed
        .query_pairs()
        .any(|(key, value)| key == "cursor" && value == "opaque-page-token"));

    let changed = next.as_str().replace("filter=mlx", "filter=gguf");
    headers.insert(
        reqwest::header::LINK,
        reqwest::header::HeaderValue::from_str(&format!("<{changed}>; rel=\"next\""))
            .expect("changed Link header"),
    );
    assert!(parse_search_next_link(&headers, &current, &expected).is_err());

    let duplicate_cursor = format!("{}&cursor=second", next.as_str());
    assert!(parse_hf_search_route(
        &reqwest::Url::parse(&duplicate_cursor).expect("duplicate cursor URL")
    )
    .is_err());
}

#[test]
fn gguf_artifacts_group_complete_shards_and_keep_quantizations_separate() {
    let revision = "0123456789012345678901234567890123456789";
    let tree = vec![
        tree_file("Q4/model-Q4_K_M-00002-of-00002.gguf", 20),
        tree_file("Q4/model-Q4_K_M-00001-of-00002.gguf", 10),
        tree_file("model-Q8_0.gguf", 40),
        tree_file("vision/mmproj-F16.gguf", 5),
    ];
    let (artifacts, companions, warnings) =
        build_gguf_artifacts("acme/model", revision, &tree).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(artifacts.len(), 2);
    assert_eq!(companions.len(), 1);
    let q4 = artifacts
        .iter()
        .find(|artifact| artifact.quant_type.as_deref() == Some("Q4_K_M"))
        .expect("Q4 artifact");
    assert_eq!(q4.files.len(), 2);
    assert_eq!(q4.total_size, 30);
    assert!(q4.files[0].path.ends_with("00001-of-00002.gguf"));
    assert_eq!(
        q4.download_id,
        artifact_download_id("acme/model", revision, &q4.id)
    );
    let q8 = artifacts
        .iter()
        .find(|artifact| artifact.quant_type.as_deref() == Some("Q8_0"))
        .expect("Q8 artifact");
    assert_eq!(q8.files.len(), 1);
    assert_eq!(companions[0].quant_type.as_deref(), Some("F16"));
}

#[test]
fn gguf_limits_apply_per_alternative_not_across_repo() {
    let revision = "0123456789012345678901234567890123456789";
    let per_file = 90 * 1024 * 1024 * 1024_u64;
    let tree = vec![
        tree_file("model-Q2_K.gguf", per_file),
        tree_file("model-Q3_K_M.gguf", per_file),
        tree_file("model-Q4_K_M.gguf", per_file),
    ];
    let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
    assert_eq!(artifacts.len(), 3);
    assert!(warnings.is_empty());
    assert!(
        artifacts
            .iter()
            .map(|artifact| artifact.total_size)
            .sum::<u64>()
            > MAX_HF_DOWNLOAD_BYTES
    );
}

#[test]
fn incomplete_shard_sets_are_not_downloadable_artifacts() {
    let revision = "0123456789012345678901234567890123456789";
    let tree = vec![
        tree_file("model-Q4_K_M-00001-of-00002.gguf", 10),
        tree_file("model-Q8_0.gguf", 40),
    ];
    let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].quant_type.as_deref(), Some("Q8_0"));
    assert_eq!(warnings.len(), 1);
}

#[test]
fn gguf_artifacts_with_unknown_or_zero_sizes_are_not_downloadable() {
    let revision = "0123456789012345678901234567890123456789";
    let tree = vec![
        tree_file("model-Q4_K_M-00001-of-00002.gguf", 10),
        tree_file("model-Q4_K_M-00002-of-00002.gguf", 0),
        tree_file("model-Q8_0.gguf", 40),
    ];
    let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].quant_type.as_deref(), Some("Q8_0"));
    assert!(warnings.iter().any(|warning| {
        warning.contains("Q4_K_M") && warning.contains("sizes are missing or zero")
    }));
}

#[test]
fn nested_vllm_directory_artifact_preserves_required_paths() {
    let revision = "0123456789012345678901234567890123456789";
    let profile = capability_profile("vllm", HfModelTask::Chat).unwrap();
    let tree = vec![
        tree_file("config.json", 10),
        tree_file("weights/model-00001-of-00002.safetensors", 20),
        tree_file("weights/model-00002-of-00002.safetensors", 30),
        tree_file("tokenizer/tokenizer.json", 5),
        tree_file("README.md", 100),
    ];
    let artifact = build_directory_artifact("acme/model", revision, profile, &tree).unwrap();
    assert_eq!(artifact.files.len(), 4);
    assert_eq!(artifact.total_size, 65);
    assert!(artifact
        .files
        .iter()
        .any(|file| file.path == "weights/model-00002-of-00002.safetensors"));
    assert!(!artifact.files.iter().any(|file| file.path == "README.md"));
    assert_eq!(artifact.primary_file, None);

    let missing_tokenizer = vec![
        tree_file("config.json", 10),
        tree_file("model.safetensors", 20),
    ];
    assert!(validate_directory_layout(profile, &missing_tokenizer).is_err());
}

#[test]
fn directory_artifacts_never_manifest_empty_files() {
    let revision = "0123456789012345678901234567890123456789";
    let profile = capability_profile("vllm", HfModelTask::Chat).unwrap();
    let required_empty = vec![
        tree_file("config.json", 10),
        tree_file("model.safetensors", 0),
        tree_file("tokenizer.json", 5),
    ];
    assert!(
        build_directory_artifact("acme/model", revision, profile, &required_empty).is_err()
    );

    let auxiliary_empty = vec![
        tree_file("config.json", 10),
        tree_file("model.safetensors", 20),
        tree_file("tokenizer.json", 5),
        tree_file("generation_config.json", 0),
    ];
    assert!(
        build_directory_artifact("acme/model", revision, profile, &auxiliary_empty).is_err()
    );

    let ignored_repository_placeholder = vec![
        tree_file("config.json", 10),
        tree_file("model.safetensors", 20),
        tree_file("tokenizer.json", 5),
        tree_file(".gitkeep", 0),
    ];
    let artifact = build_directory_artifact(
        "acme/model",
        revision,
        profile,
        &ignored_repository_placeholder,
    )
    .unwrap();
    assert!(!artifact.files.iter().any(|file| file.path == ".gitkeep"));
}

#[test]
fn mlx_whisper_accepts_npz_weights() {
    let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
    let files = vec![tree_file("config.json", 10), tree_file("weights.npz", 20)];
    assert!(validate_directory_layout(profile, &files).is_ok());
    assert!(validate_directory_layout(
        profile,
        &[
            tree_file("config.json", 10),
            tree_file("weights.safetensors", 20),
        ],
    )
    .is_ok());
    assert!(validate_directory_layout(
        profile,
        &[
            tree_file("config.json", 10),
            tree_file("model.safetensors", 20),
        ],
    )
    .is_err());
    assert!(validate_directory_layout(profile, &[tree_file("weights.npz", 20)]).is_err());
    assert!(validate_directory_layout(profile, &[tree_file("config.json", 10)]).is_err());
}

#[test]
fn mflux_requires_the_complete_component_layout() {
    let profile = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
    let flat = vec![
        tree_file("config.json", 10),
        tree_file("ae.safetensors", 20),
        tree_file("flux-schnell.safetensors", 30),
    ];
    assert!(validate_directory_layout(profile, &flat).is_err());

    let components = vec![
        tree_file("config.json", 10),
        tree_file("transformer/0.safetensors", 10),
        tree_file("vae/0.safetensors", 10),
        tree_file("text_encoder/0.safetensors", 10),
        tree_file("text_encoder_2/0.safetensors", 10),
        tree_file("tokenizer/vocab.json", 10),
        tree_file("tokenizer_2/tokenizer.json", 10),
    ];
    assert!(validate_directory_layout(profile, &components).is_ok());

    for missing_prefix in [
        "transformer/",
        "vae/",
        "text_encoder/",
        "text_encoder_2/",
        "tokenizer/",
        "tokenizer_2/",
    ] {
        let incomplete: Vec<_> = components
            .iter()
            .filter(|file| !file.path.starts_with(missing_prefix))
            .cloned()
            .collect();
        assert!(
            validate_directory_layout(profile, &incomplete).is_err(),
            "missing {missing_prefix} must be rejected"
        );
    }
}

#[test]
fn install_destination_identity_avoids_repo_revision_and_companion_collisions() {
    let artifact = HfDownloadArtifact {
        id: "gguf-artifact".to_string(),
        download_id: "download".to_string(),
        label: "Q4_K_M".to_string(),
        layout: HfArtifactLayout::GgufVariants,
        files: vec![],
        primary_file: None,
        quant_type: Some("Q4_K_M".to_string()),
        is_mmproj: false,
        total_size: 0,
        total_size_display: "0 B".to_string(),
    };
    let first = default_destination_name(
        "a_b/c",
        "0123456789012345678901234567890123456789",
        HfModelTask::Vision,
        &artifact,
        Some("projector-a"),
    );
    let other_repo = default_destination_name(
        "a/b_c",
        "0123456789012345678901234567890123456789",
        HfModelTask::Vision,
        &artifact,
        Some("projector-a"),
    );
    let other_revision = default_destination_name(
        "a_b/c",
        "1123456789012345678901234567890123456789",
        HfModelTask::Vision,
        &artifact,
        Some("projector-a"),
    );
    let other_companion = default_destination_name(
        "a_b/c",
        "0123456789012345678901234567890123456789",
        HfModelTask::Vision,
        &artifact,
        Some("projector-b"),
    );
    assert_ne!(first, other_repo);
    assert_ne!(first, other_revision);
    assert_ne!(first, other_companion);
}

#[test]
fn mlx_family_narrowing_rejects_false_positive_tasks() {
    let metadata = HfRepoMetadata {
        sha: "0123456789012345678901234567890123456789".to_string(),
        tags: vec![
            "mlx".to_string(),
            "automatic-speech-recognition".to_string(),
        ],
        pipeline_tag: Some("automatic-speech-recognition".to_string()),
    };
    let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
    assert!(metadata_matches_profile(
        &metadata,
        "mlx-community/whisper-large-v3",
        profile
    ));
    assert!(!metadata_matches_profile(
        &metadata,
        "mlx-community/parakeet-tdt",
        profile
    ));

    let diffusion = HfRepoMetadata {
        sha: metadata.sha,
        tags: vec!["mflux".to_string(), "text-to-image".to_string()],
        pipeline_tag: Some("text-to-image".to_string()),
    };
    let diffusion_profile = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
    assert!(metadata_matches_profile(
        &diffusion,
        "dhairyashil/FLUX.1-schnell-mflux-4bit",
        diffusion_profile,
    ));
    assert!(metadata_matches_profile(
        &diffusion,
        "flux2-conversions/plain-FLUX.1-dev",
        diffusion_profile,
    ));
    assert!(metadata_matches_profile(
        &diffusion,
        "developer/plain-FLUX.1-schnell",
        diffusion_profile,
    ));
    assert!(!metadata_matches_profile(
        &diffusion,
        "Runpod/FLUX.2-klein-4B-mflux-4bit",
        diffusion_profile,
    ));
    assert!(!metadata_matches_profile(
        &diffusion,
        "filipstrand/FLUX.1-Krea-dev-mflux-4bit",
        diffusion_profile,
    ));
    assert!(!metadata_matches_profile(
        &diffusion,
        "owner/FLUX.1-dev-controlnet-mflux",
        diffusion_profile,
    ));
    assert!(!metadata_matches_profile(
        &diffusion,
        "dev-conversions/plain-FLUX.1",
        diffusion_profile,
    ));
    assert!(!metadata_matches_profile(
        &diffusion,
        "owner/FLUX.1-dev-schnell",
        diffusion_profile,
    ));
}

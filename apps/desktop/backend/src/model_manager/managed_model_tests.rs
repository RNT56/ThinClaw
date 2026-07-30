use super::*;

fn minimal_gguf() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn manifest(primary_path: Option<&str>, files: &[&str]) -> ManagedModelManifest {
    ManagedModelManifest {
        schema_version: 1,
        install_id: "hf:owner/repo:q4_k_m".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef".to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "llamacpp".to_string(),
        format: "gguf".to_string(),
        artifact_kind: if files.len() > 1 {
            "sharded".to_string()
        } else {
            "single_file".to_string()
        },
        artifact_id: Some("q4_k_m".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: primary_path.map(str::to_string),
        files: files.iter().map(|value| (*value).to_string()).collect(),
        quantization: Some("Q4_K_M".to_string()),
    }
}

#[test]
fn manifest_rejects_unsafe_and_unknown_paths() {
    let mut unsafe_manifest = manifest(Some("../model.gguf"), &["../model.gguf"]);
    assert!(validate_manifest_fields(&unsafe_manifest).is_err());

    unsafe_manifest = manifest(Some("/tmp/model.gguf"), &["/tmp/model.gguf"]);
    assert!(validate_manifest_fields(&unsafe_manifest).is_err());

    unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf"]);
    unsafe_manifest.schema_version = 2;
    assert!(validate_manifest_fields(&unsafe_manifest).is_err());

    unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf"]);
    unsafe_manifest.task = Some("embedding".to_string());
    assert!(validate_manifest_fields(&unsafe_manifest).is_err());

    unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf", MODEL_MANIFEST_FILENAME]);
    assert!(validate_managed_model_manifest(&unsafe_manifest).is_err());
}

#[test]
fn manifest_rejects_runtime_format_task_and_layout_mismatches() {
    let mut mlx_with_gguf = manifest(None, &["config.json", "model.safetensors"]);
    mlx_with_gguf.runtime = "mlx".to_string();
    mlx_with_gguf.format = "gguf".to_string();
    assert!(validate_manifest_fields(&mlx_with_gguf).is_err());

    let mut mlx_file = mlx_with_gguf.clone();
    mlx_file.format = "mlx".to_string();
    mlx_file.primary_path = Some("model.safetensors".to_string());
    assert!(validate_manifest_fields(&mlx_file).is_err());

    let mut mlx_directory = mlx_with_gguf;
    mlx_directory.format = "mlx".to_string();
    assert!(validate_manifest_fields(&mlx_directory).is_ok());

    let mut mlx_tts = mlx_directory.clone();
    mlx_tts.category = "TTS".to_string();
    mlx_tts.task = Some("tts".to_string());
    assert!(validate_manifest_fields(&mlx_tts).is_err());

    let mut vllm_with_mlx = mlx_directory.clone();
    vllm_with_mlx.runtime = "vllm".to_string();
    assert!(validate_manifest_fields(&vllm_with_mlx).is_err());

    let mut vllm_awq = vllm_with_mlx;
    vllm_awq.format = "awq".to_string();
    assert!(validate_manifest_fields(&vllm_awq).is_ok());

    let mut llama_awq = manifest(Some("model.awq"), &["model.awq"]);
    llama_awq.format = "awq".to_string();
    assert!(validate_manifest_fields(&llama_awq).is_err());

    let llama_wrong_extension = manifest(Some("model.bin"), &["model.bin"]);
    assert!(validate_manifest_fields(&llama_wrong_extension).is_err());

    let mut llama_stt = manifest(Some("model.bin"), &["model.bin"]);
    llama_stt.category = "STT".to_string();
    llama_stt.task = Some("stt".to_string());
    llama_stt.format = "bin".to_string();
    assert!(validate_manifest_fields(&llama_stt).is_ok());

    let mut mlx_companion = mlx_directory;
    mlx_companion.task = Some("vision".to_string());
    mlx_companion.companion_artifact_id = Some("projector".to_string());
    mlx_companion.companion_path = Some("model.safetensors".to_string());
    assert!(validate_manifest_fields(&mlx_companion).is_err());

    let mut llama_vision_without_projector = manifest(Some("model.gguf"), &["model.gguf"]);
    llama_vision_without_projector.task = Some("vision".to_string());
    assert!(validate_manifest_fields(&llama_vision_without_projector).is_err());

    let mut llama_vision_wrong_projector =
        manifest(Some("model.gguf"), &["model.gguf", "projector.gguf"]);
    llama_vision_wrong_projector.task = Some("vision".to_string());
    llama_vision_wrong_projector.companion_artifact_id = Some("projector".to_string());
    llama_vision_wrong_projector.companion_path = Some("projector.gguf".to_string());
    assert!(validate_manifest_fields(&llama_vision_wrong_projector).is_err());
}

#[test]
fn manifest_groups_shards_into_one_inventory_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("owner_repo--q4");
    fs::create_dir_all(&install).expect("install directory");
    fs::write(install.join("model-00001-of-00002.gguf"), minimal_gguf()).expect("first shard");
    fs::write(install.join("model-00002-of-00002.gguf"), minimal_gguf()).expect("second shard");
    let manifest = manifest(
        Some("model-00001-of-00002.gguf"),
        &["model-00001-of-00002.gguf", "model-00002-of-00002.gguf"],
    );
    write_managed_model_manifest(&install, &manifest).expect("write manifest");

    let mut found = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].repo_id.as_deref(), Some("owner/repo"));
    assert_eq!(found[0].artifact_id.as_deref(), Some("q4_k_m"));
    assert_eq!(found[0].category, "LLM");
    assert!(found[0].path.ends_with("model-00001-of-00002.gguf"));
    assert_eq!(found[0].install_root, "LLM/owner_repo--q4");

    fs::write(install.join("model-00002-of-00002.gguf"), b"GGUFpayload")
        .expect("corrupt second shard");
    found.clear();
    visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, "managed-invalid");
    assert!(!found[0].compatible);
    assert!(found[0]
        .compatibility_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("invalid")));
}

#[test]
fn managed_mlx_directories_become_invalid_when_any_declared_file_is_empty() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");

    for (category, task, install_name) in [
        ("LLM", "chat", "mlx-chat"),
        ("Embedding", "embedding", "mlx-embedding"),
    ] {
        let config = if category == "Embedding" {
            br#"{"model_type":"bert"}"#.as_slice()
        } else {
            br#"{"model_type":"llama"}"#.as_slice()
        };
        let declared_files = [
            ("config.json", config),
            ("model.safetensors", b"weights".as_slice()),
            ("tokenizer.json", b"{}".as_slice()),
            ("generation_config.json", b"{}".as_slice()),
        ];
        let install = models.join(category).join(install_name);
        fs::create_dir_all(&install).expect("install directory");
        for (relative, contents) in declared_files {
            fs::write(install.join(relative), contents).expect("managed model file");
        }

        let mut managed = manifest(
            None,
            &[
                "config.json",
                "model.safetensors",
                "tokenizer.json",
                "generation_config.json",
            ],
        );
        managed.install_id = format!("mlx:{task}");
        managed.category = category.to_string();
        managed.task = Some(task.to_string());
        managed.runtime = "mlx".to_string();
        managed.format = "mlx".to_string();
        managed.artifact_kind = "directory".to_string();
        managed.quantization = None;
        write_managed_model_manifest(&install, &managed).expect("write manifest");

        let install_root = format!("{category}/{install_name}");
        let scan_install = || {
            let mut found = Vec::new();
            let mut visited = 0;
            scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
            found
                .into_iter()
                .find(|model| model.install_root == install_root)
                .expect("managed install inventory entry")
        };

        let healthy = scan_install();
        assert_eq!(healthy.source, "huggingface");
        #[cfg(feature = "mlx")]
        assert!(healthy.compatible);

        for (relative, contents) in declared_files {
            fs::write(install.join(relative), []).expect("truncate managed model file");

            let invalid = scan_install();
            assert_eq!(invalid.source, "managed-invalid");
            assert!(!invalid.compatible);
            assert!(invalid
                .compatibility_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("empty")));

            fs::write(install.join(relative), contents).expect("restore managed model file");
            assert_ne!(scan_install().source, "managed-invalid");
        }

        fs::write(install.join("config.json"), b"{malformed").expect("corrupt managed config");
        let invalid = scan_install();
        assert_eq!(invalid.source, "managed-invalid");
        assert!(!invalid.compatible);
    }
}

#[test]
fn vision_manifest_round_trips_task_and_companion_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("owner_repo--vision");
    let primary_relative = "weights/model-Q4_K_M.gguf";
    let companion_relative = "projector/mmproj-F16.gguf";
    fs::create_dir_all(install.join("weights")).expect("weights directory");
    fs::create_dir_all(install.join("projector")).expect("projector directory");
    fs::write(install.join(primary_relative), minimal_gguf()).expect("primary model");
    fs::write(install.join(companion_relative), minimal_gguf()).expect("companion model");

    let mut manifest = manifest(
        Some(primary_relative),
        &[primary_relative, companion_relative],
    );
    manifest.install_id = "hf:owner/repo:vision:q4_k_m:mmproj_f16".to_string();
    manifest.revision = Some("0123456789abcdef0123456789abcdef01234567".to_string());
    manifest.task = Some("vision".to_string());
    manifest.artifact_kind = "gguf_single".to_string();
    manifest.companion_artifact_id = Some("mmproj_f16".to_string());
    manifest.companion_path = Some(companion_relative.to_string());
    write_managed_model_manifest(&install, &manifest).expect("write manifest");

    let persisted = read_managed_model_manifest(&install).expect("read manifest");
    assert_eq!(persisted.task.as_deref(), Some("vision"));
    assert_eq!(
        persisted.companion_artifact_id.as_deref(),
        Some("mmproj_f16")
    );
    assert_eq!(
        persisted.companion_path.as_deref(),
        Some(companion_relative)
    );

    let mut found = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

    assert_eq!(found.len(), 1);
    let model = &found[0];
    assert_eq!(model.task.as_deref(), Some("vision"));
    assert_eq!(model.repo_id.as_deref(), Some("owner/repo"));
    assert_eq!(
        model.revision.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(model.artifact_id.as_deref(), Some("q4_k_m"));
    assert_eq!(model.companion_artifact_id.as_deref(), Some("mmproj_f16"));
    assert_eq!(
        Path::new(&model.path),
        install
            .join(primary_relative)
            .canonicalize()
            .expect("primary")
    );
    assert_eq!(
        Path::new(model.companion_path.as_deref().expect("companion path")),
        install
            .join(companion_relative)
            .canonicalize()
            .expect("companion")
    );
    assert_eq!(model.install_root, "LLM/owner_repo--vision");

    fs::write(install.join(companion_relative), b"GGUFpayload")
        .expect("corrupt companion model");
    found.clear();
    visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, "managed-invalid");
    assert!(!found[0].compatible);
}

#[test]
fn invalid_manifest_is_one_visible_incompatible_install() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("Embedding").join("misleading-whisper-name");
    fs::create_dir_all(&install).expect("install directory");
    fs::write(install.join("model.gguf"), b"model").expect("model");
    fs::write(install.join(MODEL_MANIFEST_FILENAME), b"{invalid").expect("invalid manifest");

    let mut found = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].category, "Embedding");
    assert!(found[0].repo_id.is_none());
    assert_eq!(found[0].source, "managed-invalid");
    assert!(!found[0].compatible);
    assert_eq!(found[0].install_root, "Embedding/misleading-whisper-name");
}

#[cfg(unix)]
#[test]
fn broken_manifest_symlink_is_one_visible_incompatible_install() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("broken-manifest");
    fs::create_dir_all(&install).expect("install directory");
    fs::write(install.join("model.gguf"), b"model").expect("model");
    symlink(
        "missing-manifest.json",
        install.join(MODEL_MANIFEST_FILENAME),
    )
    .expect("broken manifest symlink");

    let mut found = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, "managed-invalid");
    assert!(!found[0].compatible);
    assert_eq!(found[0].install_root, "LLM/broken-manifest");
}

#[test]
fn deleting_legacy_file_preserves_sibling_models() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let group = models.join("LLM").join("group");
    fs::create_dir_all(&group).expect("model group");
    fs::write(group.join("a.gguf"), b"a").expect("first model");
    fs::write(group.join("b.gguf"), b"b").expect("second model");

    delete_model_install(&models, Path::new("LLM/group/a.gguf"))
        .expect("delete selected model");

    assert!(!group.join("a.gguf").exists());
    assert!(group.join("b.gguf").exists());
    assert!(group.is_dir());
}

#[test]
fn inventory_paths_are_portable_and_listed_unicode_model_is_deletable() {
    let normalized = normalize_inventory_relative_path(r"LLM\Group Name\模型.gguf", '\\');
    assert_eq!(normalized, "LLM/Group Name/模型.gguf");
    assert_eq!(
        validate_model_relative(&normalized, false)
            .expect("portable inventory path")
            .components()
            .count(),
        3
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let group = models.join("LLM").join("Group Name");
    fs::create_dir_all(&group).expect("model group");
    fs::write(group.join("模型.gguf"), b"GGUFmodel").expect("unicode model");

    let mut found = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].install_root, "LLM/Group Name/模型.gguf");

    delete_model_install(&models, Path::new(&found[0].install_root))
        .expect("delete inventory path");
    assert!(!group.join("模型.gguf").exists());
    assert!(group.is_dir());
}

#[test]
fn deleting_directory_install_preserves_sibling_installs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let group = models.join("LLM").join("group");
    let selected = group.join("install-a");
    let sibling = group.join("install-b");
    fs::create_dir_all(&selected).expect("selected install");
    fs::create_dir_all(&sibling).expect("sibling install");
    fs::write(selected.join("config.json"), b"{}").expect("selected config");
    fs::write(selected.join("model.safetensors"), b"a").expect("selected model");
    fs::write(sibling.join("config.json"), b"{}").expect("sibling config");
    fs::write(sibling.join("model.safetensors"), b"b").expect("sibling model");

    delete_model_install(&models, Path::new("LLM/group/install-a"))
        .expect("delete selected install");

    assert!(!selected.exists());
    assert!(sibling.join("model.safetensors").is_file());
    assert!(group.is_dir());
}

#[test]
fn deletion_rejects_parent_that_is_not_an_inventory_install_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let group = models.join("LLM").join("group");
    fs::create_dir_all(&group).expect("model group");
    fs::write(group.join("a.gguf"), b"a").expect("first model");
    fs::write(group.join("b.gguf"), b"b").expect("second model");

    let error = delete_model_install(&models, Path::new("LLM/group"))
        .expect_err("undeclared parent must not be deletable");

    assert!(error.contains("not a declared model installation"));
    assert!(group.join("a.gguf").exists());
    assert!(group.join("b.gguf").exists());
}

#[test]
fn compatibility_matrix_is_category_and_runtime_specific() {
    let supported = [
        ("llamacpp", "LLM", Some("chat"), "gguf", false),
        ("llamacpp", "LLM", Some("vision"), "gguf", false),
        ("llamacpp", "Embedding", Some("embedding"), "gguf", false),
        ("llamacpp", "STT", Some("stt"), "bin", false),
        ("llamacpp", "TTS", Some("tts"), "onnx", false),
        ("mlx", "LLM", Some("chat"), "mlx", true),
        ("mlx", "Embedding", Some("embedding"), "mlx", true),
        ("mlx", "STT", Some("stt"), "mlx", true),
        ("mlx", "Diffusion", Some("diffusion"), "mflux", true),
        ("vllm", "LLM", Some("chat"), "awq", true),
        ("vllm", "LLM", Some("vision"), "awq", true),
    ];
    for (runtime, category, task, format, is_directory) in supported {
        assert!(
            model_compatibility(runtime, category, task, format, is_directory, Some(runtime),)
                .0,
            "expected {runtime}/{task:?}/{category}/{format} to be compatible"
        );
    }

    let rejected = [
        ("llamacpp", "LLM", Some("chat"), "awq", false),
        ("llamacpp", "LLM", Some("chat"), "gguf", true),
        ("llamacpp", "STT", Some("stt"), "gguf", false),
        ("llamacpp", "TTS", Some("tts"), "gguf", false),
        ("mlx", "LLM", Some("chat"), "gguf", true),
        ("mlx", "LLM", Some("chat"), "mlx", false),
        ("mlx", "Diffusion", Some("diffusion"), "mlx", true),
        ("mlx", "TTS", Some("tts"), "mlx", true),
        ("vllm", "LLM", Some("chat"), "mlx", true),
        ("vllm", "Embedding", Some("embedding"), "awq", true),
        ("ollama", "LLM", Some("chat"), "gguf", false),
    ];
    for (runtime, category, task, format, is_directory) in rejected {
        assert!(
            !model_compatibility(runtime, category, task, format, is_directory, None).0,
            "expected {runtime}/{task:?}/{category}/{format} to be rejected"
        );
    }

    assert!(
        !model_compatibility("llamacpp", "LLM", Some("chat"), "gguf", false, Some("mlx"),).0
    );
}

#[test]
fn engine_launch_path_must_be_an_exact_compatible_inventory_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("mlx-chat");
    fs::create_dir_all(&install).expect("install directory");
    fs::write(install.join("config.json"), b"{}").expect("model config");
    fs::write(install.join("model.safetensors"), b"model").expect("model weights");
    fs::write(install.join("tokenizer.json"), b"{}").expect("tokenizer");
    write_managed_model_manifest(
        &install,
        &ManagedModelManifest {
            schema_version: 1,
            install_id: "mlx-chat".to_string(),
            source: "test".to_string(),
            repo_id: Some("owner/model".to_string()),
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
        },
    )
    .expect("manifest");

    assert_eq!(
        resolve_compatible_inventory_model_path_in(&models, &install, "mlx", "LLM")
            .expect("compatible model"),
        install.canonicalize().expect("canonical install")
    );
    assert!(
        resolve_compatible_inventory_model_path_in(&models, &install, "vllm", "LLM").is_err()
    );
    assert!(
        resolve_compatible_inventory_model_path_in(&models, &install, "mlx", "Embedding")
            .is_err()
    );
    assert!(resolve_compatible_inventory_model_path_in(
        &models,
        &install.join("model.safetensors"),
        "mlx",
        "LLM",
    )
    .is_err());
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    assert!(
        resolve_compatible_inventory_model_path_in(&models, &outside, "mlx", "LLM",).is_err()
    );
}

#[test]
fn inventory_resolver_supports_file_entries_and_declared_companions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("vision");
    fs::create_dir_all(&install).expect("install directory");
    let primary = install.join("model.gguf");
    let companion = install.join("mmproj-F16.gguf");
    fs::write(&primary, minimal_gguf()).expect("model");
    fs::write(&companion, minimal_gguf()).expect("projector");
    let mut vision = manifest(Some("model.gguf"), &["model.gguf", "mmproj-F16.gguf"]);
    vision.task = Some("vision".to_string());
    vision.companion_artifact_id = Some("mmproj_f16".to_string());
    vision.companion_path = Some("mmproj-F16.gguf".to_string());
    write_managed_model_manifest(&install, &vision).expect("manifest");

    let resolved = resolve_compatible_inventory_model_in(&models, &primary, "llamacpp", "LLM")
        .expect("resolved file model");
    assert_eq!(resolved.path, primary.canonicalize().expect("primary"));
    assert_eq!(
        resolved.install_root,
        install.canonicalize().expect("install")
    );
    assert_eq!(
        resolved.companion_path,
        Some(companion.canonicalize().expect("companion"))
    );
    assert!(resolve_compatible_inventory_model_in(&models, &primary, "mlx", "LLM").is_err());
    assert!(
        resolve_compatible_inventory_model_in(&models, &primary, "llamacpp", "Embedding",)
            .is_err()
    );
    assert!(
        resolve_compatible_inventory_model_in(&models, &install, "llamacpp", "LLM").is_err()
    );
}

#[test]
fn mlx_embedding_config_allowlist_only_accepts_pinned_two_dimensional_text_models() {
    for model_type in [
        "bert",
        "BERT",
        "xlm_roberta",
        "xlm-roberta",
        "qwen3",
        "gemma3_text",
        "gemma3-text",
    ] {
        assert!(
            is_supported_mlx_embedding_config(&serde_json::json!({"model_type": model_type})),
            "expected {model_type} to be supported"
        );
    }
    assert!(is_supported_mlx_embedding_config(&serde_json::json!({
        "model_type": "modernbert",
        "architectures": ["ModernBertModel"]
    })));

    for config in [
        serde_json::json!({}),
        serde_json::json!({"model_type": null}),
        serde_json::json!({"model_type": 42}),
        serde_json::json!({"model_type": "qwen2"}),
        serde_json::json!({"model_type": "lfm2"}),
        serde_json::json!({"model_type": "colqwen2_5"}),
        serde_json::json!({"model_type": "siglip"}),
        serde_json::json!({"model_type": "modernbert"}),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": []
        }),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": ["ModernBertForMaskedLM"]
        }),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": ["ModernBertModel", "ModernBertForMaskedLM"]
        }),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": [42]
        }),
    ] {
        assert!(!is_supported_mlx_embedding_config(&config));
    }
}

#[test]
fn managed_mlx_vision_inventory_requires_config_and_vision_tensor_keys() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    let install = models.join("LLM").join("mlx-vision");
    fs::create_dir_all(&install).expect("install directory");
    fs::write(
        install.join("config.json"),
        br#"{
            "architectures":["LlavaForConditionalGeneration"],
            "vision_config":{}
        }"#,
    )
    .expect("vision config");
    fs::write(install.join("model.safetensors"), [0_u8; 8]).expect("vision weights");
    fs::write(
        install.join("model.safetensors.index.json"),
        br#"{"weight_map":{"multi_modal_projector.weight":"model.safetensors"}}"#,
    )
    .expect("vision weight index");
    fs::write(install.join("tokenizer.json"), b"tokenizer").expect("tokenizer");
    let manifest = ManagedModelManifest {
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

    assert!(classify_mlx_vision_directory(&install).expect("valid vision directory"));
    assert!(manifest_model(&install, &models, manifest.clone()).is_ok());

    fs::write(
        install.join("model.safetensors.index.json"),
        br#"{"weight_map":{"language_model.layer.weight":"model.safetensors"}}"#,
    )
    .expect("text-only weight index");
    assert!(classify_mlx_vision_directory(&install).is_err());
    assert!(
        manifest_model(&install, &models, manifest.clone()).is_err(),
        "managed inventory must reject a vision marker without vision tensors"
    );

    fs::write(
        install.join("model.safetensors.index.json"),
        br#"{"weight_map":{"vision_model.layer.weight":"model.safetensors"}}"#,
    )
    .expect("restore vision weight index");
    fs::write(
        install.join("config.json"),
        br#"{"architectures":["LlamaForCausalLM"]}"#,
    )
    .expect("text-only config");
    assert!(
        !classify_mlx_vision_directory(&install).expect("text-only classification")
    );
    assert!(
        manifest_model(&install, &models, manifest).is_err(),
        "managed inventory must reject text-only contents declared as vision"
    );
}

#[test]
fn legacy_mlx_embedding_inventory_uses_the_shared_config_allowlist() {
    let temp = tempfile::tempdir().expect("tempdir");
    let embedding = temp.path().join("embedding");
    fs::create_dir(&embedding).expect("embedding directory");
    fs::write(embedding.join("model.safetensors"), b"weights").expect("embedding weights");
    fs::write(embedding.join("tokenizer.json"), b"{}").expect("embedding tokenizer");

    fs::write(
        embedding.join("config.json"),
        br#"{
            "model_type":"bert",
            "quantization":{"bits":4,"group_size":64}
        }"#,
    )
    .expect("supported embedding config");
    assert_eq!(
        validated_legacy_directory_format(&embedding, "Embedding"),
        Some("mlx")
    );

    fs::write(
        embedding.join("config.json"),
        br#"{
            "model_type":"qwen2",
            "quantization":{"bits":4,"group_size":64}
        }"#,
    )
    .expect("unsupported embedding config");
    assert_eq!(
        validated_legacy_directory_format(&embedding, "Embedding"),
        None
    );
}

#[test]
fn legacy_directory_format_requires_runtime_specific_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    assert!(!is_supported_mflux_config(&serde_json::json!({})));
    assert!(!is_supported_mflux_config(&serde_json::json!({
        "_class_name": "FluxPipeline",
        "model_type": "flux-rectified-flow",
        "original_model": "black-forest-labs/FLUX.1-Krea-dev",
        "quantization": {"method": "mflux", "bits": 4}
    })));
    assert!(!is_supported_mflux_config(&serde_json::json!({
        "_class_name": "FluxPipeline",
        "model_type": "flux-rectified-flow",
        "original_model": "owner/FLUX.1-dev-schnell",
        "quantization": {"method": "mflux", "bits": 4}
    })));
    assert!(is_supported_mflux_config(&serde_json::json!({
        "_class_name": "FluxPipeline",
        "model_type": "flux-rectified-flow",
        "original_model": "developer/FLUX.1-schnell",
        "quantization": {"method": "mflux", "bits": 4}
    })));

    let mlx = temp.path().join("mlx");
    fs::create_dir(&mlx).expect("mlx directory");
    fs::write(
        mlx.join("config.json"),
        br#"{"quantization":{"bits":4,"group_size":64}}"#,
    )
    .expect("mlx config");
    fs::write(mlx.join("model.safetensors"), b"weights").expect("mlx weights");
    fs::write(mlx.join("tokenizer.json"), b"{}").expect("mlx tokenizer");
    assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), Some("mlx"));
    fs::write(mlx.join("model.safetensors"), []).expect("empty mlx weights");
    assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), None);
    fs::write(mlx.join("model.safetensors"), b"weights").expect("restore mlx weights");
    fs::write(mlx.join("tokenizer.json"), []).expect("empty mlx tokenizer");
    assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), None);
    fs::write(mlx.join("tokenizer.json"), b"{}").expect("restore mlx tokenizer");

    let mlx_without_tokenizer = temp.path().join("mlx-without-tokenizer");
    fs::create_dir(&mlx_without_tokenizer).expect("mlx directory");
    fs::write(
        mlx_without_tokenizer.join("config.json"),
        br#"{"quantization":{"bits":4,"group_size":64}}"#,
    )
    .expect("mlx config");
    fs::write(mlx_without_tokenizer.join("model.safetensors"), b"weights")
        .expect("mlx weights");
    assert_eq!(
        validated_legacy_directory_format(&mlx_without_tokenizer, "LLM"),
        None
    );

    let mlx_npz = temp.path().join("mlx-npz");
    fs::create_dir(&mlx_npz).expect("mlx npz directory");
    fs::write(mlx_npz.join("config.json"), b"{}").expect("mlx npz config");
    fs::write(mlx_npz.join("weights.npz"), b"weights").expect("mlx npz weights");
    assert_eq!(
        validated_legacy_directory_format(&mlx_npz, "STT"),
        Some("mlx")
    );

    let mlx_whisper_safetensors = temp.path().join("mlx-whisper-safetensors");
    fs::create_dir(&mlx_whisper_safetensors).expect("mlx whisper directory");
    fs::write(mlx_whisper_safetensors.join("config.json"), b"{}").expect("mlx whisper config");
    fs::write(
        mlx_whisper_safetensors.join("weights.safetensors"),
        b"weights",
    )
    .expect("mlx whisper weights");
    assert_eq!(
        validated_legacy_directory_format(&mlx_whisper_safetensors, "STT"),
        Some("mlx")
    );

    let mlx_wrong_whisper_name = temp.path().join("mlx-wrong-whisper-name");
    fs::create_dir(&mlx_wrong_whisper_name).expect("wrong whisper directory");
    fs::write(mlx_wrong_whisper_name.join("config.json"), b"{}").expect("wrong whisper config");
    fs::write(mlx_wrong_whisper_name.join("model.safetensors"), b"weights")
        .expect("wrong whisper weights");
    assert_eq!(
        validated_legacy_directory_format(&mlx_wrong_whisper_name, "STT"),
        None
    );

    let mflux = temp.path().join("mflux");
    fs::create_dir(&mflux).expect("mflux directory");
    fs::write(
        mflux.join("config.json"),
        br#"{
            "_class_name":"FluxPipeline",
            "model_type":"flux-rectified-flow",
            "original_model":"black-forest-labs/FLUX.1-schnell",
            "quantization":{"method":"mflux","bits":4}
        }"#,
    )
    .expect("mflux config");
    for component in [
        "transformer",
        "vae",
        "text_encoder",
        "text_encoder_2",
        "tokenizer",
        "tokenizer_2",
    ] {
        let directory = mflux.join(component);
        fs::create_dir(&directory).expect("mflux component");
        let filename = if component.starts_with("tokenizer") {
            "tokenizer.json"
        } else {
            "model.safetensors"
        };
        fs::write(directory.join(filename), b"component").expect("mflux component file");
    }
    assert_eq!(
        validated_legacy_directory_format(&mflux, "Diffusion"),
        Some("mflux")
    );
    assert!(is_model_bundle_dir(&mflux));
    fs::write(mflux.join("transformer").join("model.safetensors"), [])
        .expect("empty mflux component");
    assert_eq!(validated_legacy_directory_format(&mflux, "Diffusion"), None);
    assert!(!is_model_bundle_dir(&mflux));

    let flat_diffusion = temp.path().join("flat-diffusion");
    fs::create_dir(&flat_diffusion).expect("flat diffusion directory");
    fs::write(
        flat_diffusion.join("config.json"),
        br#"{"quantization":{"bits":4,"group_size":64}}"#,
    )
    .expect("flat diffusion config");
    fs::write(flat_diffusion.join("flux.safetensors"), b"weights")
        .expect("flat diffusion weights");
    assert_eq!(
        validated_legacy_directory_format(&flat_diffusion, "Diffusion"),
        None
    );

    let awq = temp.path().join("awq");
    fs::create_dir(&awq).expect("awq directory");
    fs::write(
        awq.join("config.json"),
        br#"{"quantization_config":{"quant_method":"awq"}}"#,
    )
    .expect("awq config");
    fs::write(awq.join("model.safetensors"), b"weights").expect("awq weights");
    assert_eq!(validated_legacy_directory_format(&awq, "LLM"), None);
    fs::write(awq.join("tokenizer.json"), []).expect("empty awq tokenizer");
    assert_eq!(validated_legacy_directory_format(&awq, "LLM"), None);
    fs::write(awq.join("tokenizer.json"), b"{}").expect("awq tokenizer");
    assert_eq!(validated_legacy_directory_format(&awq, "LLM"), Some("awq"));

    let ambiguous = temp.path().join("ambiguous");
    fs::create_dir(&ambiguous).expect("ambiguous directory");
    fs::write(ambiguous.join("config.json"), b"{}").expect("ambiguous config");
    fs::write(ambiguous.join("model.safetensors"), b"weights").expect("ambiguous weights");
    assert_eq!(validated_legacy_directory_format(&ambiguous, "LLM"), None);
}

#[test]
fn invalid_legacy_single_files_remain_visible_but_incompatible() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models = temp.path().join("models");
    for category in ["LLM", "STT", "TTS", "Diffusion"] {
        fs::create_dir_all(models.join(category)).expect("category");
    }

    let bad_gguf = models.join("LLM").join("broken.gguf");
    fs::write(&bad_gguf, b"not-gguf").expect("bad gguf");
    let gguf = legacy_model(&bad_gguf, &models, 8, false);
    assert!(!gguf.compatible);
    assert!(gguf
        .compatibility_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("GGUF")));

    let empty_stt = models.join("STT").join("empty.bin");
    fs::write(&empty_stt, []).expect("empty stt");
    let stt = legacy_model(&empty_stt, &models, 0, false);
    assert!(!stt.compatible);

    let piper = models.join("TTS").join("voice.onnx");
    fs::write(&piper, b"onnx").expect("piper");
    let tts = legacy_model(&piper, &models, 4, false);
    assert!(!tts.compatible);
    fs::write(
        PathBuf::from(format!("{}.json", piper.to_string_lossy())),
        b"{invalid",
    )
    .expect("invalid piper config");
    assert!(validate_legacy_single_file(&piper, "TTS", "onnx").is_err());

    let empty_diffusion = models.join("Diffusion").join("empty.safetensors");
    fs::write(&empty_diffusion, []).expect("empty diffusion");
    let diffusion = legacy_model(&empty_diffusion, &models, 0, false);
    assert!(!diffusion.compatible);

    assert_eq!(gguf.relative_path, "LLM/broken.gguf");
    assert_eq!(stt.relative_path, "STT/empty.bin");
    assert_eq!(tts.relative_path, "TTS/voice.onnx");
    assert_eq!(diffusion.relative_path, "Diffusion/empty.safetensors");
}

#[cfg(unix)]
#[test]
fn legacy_mflux_layout_rejects_component_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let mflux = temp.path().join("mflux");
    fs::create_dir(&mflux).expect("mflux directory");
    fs::write(
        mflux.join("config.json"),
        br#"{
            "_class_name":"FluxPipeline",
            "model_type":"flux-rectified-flow",
            "original_model":"black-forest-labs/FLUX.1-dev",
            "quantization":{"method":"mflux","bits":4}
        }"#,
    )
    .expect("mflux config");
    for component in [
        "transformer",
        "vae",
        "text_encoder",
        "text_encoder_2",
        "tokenizer",
        "tokenizer_2",
    ] {
        let directory = mflux.join(component);
        fs::create_dir(&directory).expect("component");
        fs::write(
            directory.join(if component.starts_with("tokenizer") {
                "tokenizer.json"
            } else {
                "model.safetensors"
            }),
            b"component",
        )
        .expect("component file");
    }
    let outside = temp.path().join("outside.json");
    fs::write(&outside, b"outside").expect("outside file");
    symlink(&outside, mflux.join("tokenizer").join("linked.json")).expect("component symlink");

    assert_eq!(validated_legacy_directory_format(&mflux, "Diffusion"), None);
    assert!(!is_model_bundle_dir(&mflux));
}

#[test]
fn standard_assets_are_revision_pinned_with_exact_integrity_metadata() {
    let expected = [
        (
            "vae-ft-mse-840000-ema-pruned.safetensors",
            334_641_190,
            "735e4c3a447a3255760d7f86845f09f937809baa529c17370d83e4c3758f3c75",
        ),
        (
            "t5xxl_fp16.safetensors",
            9_787_841_024,
            "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635",
        ),
        (
            "clip_l.safetensors",
            246_144_152,
            "660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd",
        ),
        (
            "clip_g.safetensors",
            1_389_382_176,
            "ec310df2af79c318e24d20511b601a591ca8cd4f1fce1d8dff822a356bcdb1f4",
        ),
        (
            "scheduler_config.json",
            308,
            "699cce92eb7c122e2eb7dfdea78e6187fda76a5ed4a8e42319b85610e620e091",
        ),
    ];
    let assets = get_standard_assets();
    assert_eq!(assets.len(), expected.len());
    for (filename, size, sha256) in expected {
        let asset = assets
            .iter()
            .find(|asset| asset.filename == filename)
            .expect("pinned asset");
        assert_eq!(asset.size, size);
        assert_eq!(asset.sha256, sha256);
        assert!(!asset.url.contains("/resolve/main/"));
        let revision = asset
            .url
            .split("/resolve/")
            .nth(1)
            .and_then(|suffix| suffix.split('/').next())
            .expect("revision");
        assert_eq!(revision.len(), 40);
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

#[test]
fn existing_standard_asset_validation_checks_hash_and_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("scheduler_config.json");
    fs::write(&path, b"{}").expect("asset");
    let asset = StandardAsset {
        name: "test".to_string(),
        category: "other".to_string(),
        filename: "scheduler_config.json".to_string(),
        url: "https://huggingface.co/owner/repo/resolve/0123456789abcdef0123456789abcdef01234567/scheduler_config.json".to_string(),
        size: 2,
        sha256: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .to_string(),
    };
    assert!(validate_existing_standard_asset(&path, &asset).is_ok());

    fs::write(&path, b"[]").expect("corrupt asset");
    assert!(validate_existing_standard_asset(&path, &asset).is_err());
    assert!(validate_expected_download_integrity(2, 3, &asset.sha256, &asset.sha256).is_err());
}

#[test]
fn standard_asset_verification_cache_is_pinned_and_metadata_sensitive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("scheduler_config.json");
    let verification_path = temp.path().join(STANDARD_ASSET_VERIFICATION_FILENAME);
    fs::write(&path, b"{}").expect("asset");
    let asset = StandardAsset {
        name: "test".to_string(),
        category: "other".to_string(),
        filename: "scheduler_config.json".to_string(),
        url: "https://huggingface.co/owner/repo/resolve/0123456789abcdef0123456789abcdef01234567/scheduler_config.json".to_string(),
        size: 2,
        sha256: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .to_string(),
    };

    assert!(validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok());
    let cached = load_standard_asset_verification_manifest(&verification_path);
    let record = cached
        .entries
        .get(&standard_asset_cache_key(&asset))
        .expect("cached record");
    assert_eq!(record.pinned_size, asset.size);
    assert_eq!(record.pinned_sha256, asset.sha256);
    assert_eq!(
        record.stamp,
        inspect_existing_standard_asset(&path, &asset).expect("stamp")
    );
    assert!(validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok());

    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(&path, b"[]").expect("same-size corruption");
    assert_ne!(
        record.stamp,
        inspect_existing_standard_asset(&path, &asset).expect("changed stamp")
    );
    assert!(
        validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_err()
    );
}

#[test]
fn deterministic_model_partial_cleanup_is_name_and_age_scoped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let destination = temp.path().join("model.safetensors");
    let partial = model_partial_path(&destination).expect("partial path");
    assert_eq!(
        partial,
        model_partial_path(&destination).expect("same partial path")
    );
    assert!(is_owned_model_partial_name(
        partial.file_name().expect("partial filename")
    ));
    fs::write(&partial, b"partial").expect("partial");
    assert_eq!(
        prepare_model_partial_path(&destination).expect("immediate retry recovery"),
        partial
    );
    assert!(!partial.exists());
    fs::write(&partial, b"partial").expect("new partial");
    let lookalike = temp.path().join(".thinclaw-download-not-owned.part");
    fs::write(&lookalike, b"keep").expect("lookalike");

    let now = std::time::SystemTime::now();
    assert_eq!(
        cleanup_stale_model_partials_at(
            temp.path(),
            now,
            std::time::Duration::from_secs(7 * 24 * 60 * 60),
        )
        .expect("fresh cleanup"),
        0
    );
    assert_eq!(
        cleanup_stale_model_partials_at(
            temp.path(),
            now + std::time::Duration::from_secs(8 * 24 * 60 * 60),
            std::time::Duration::from_secs(7 * 24 * 60 * 60),
        )
        .expect("stale cleanup"),
        1
    );
    assert!(!partial.exists());
    assert!(lookalike.exists());

    fs::create_dir(&partial).expect("unsafe partial directory");
    assert!(prepare_model_partial_path(&destination).is_err());
    assert!(partial.is_dir());
}

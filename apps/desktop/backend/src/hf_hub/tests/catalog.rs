use super::*;

#[test]
fn format_bytes_display() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(500), "500 B");
    assert_eq!(format_bytes(1024), "1 KB");
    assert_eq!(format_bytes(1_500_000), "1 MB");
    assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    assert_eq!(format_bytes(8_000_000_000), "7.5 GB");
}

#[test]
fn destination_subdir_must_remain_visible_to_inventory() {
    assert!(validate_relative_subdir("owner_model").is_ok());
    assert!(validate_relative_subdir("standard-model").is_ok());
    assert!(validate_relative_subdir(".hidden").is_err());
    assert!(validate_relative_subdir("..hidden").is_err());
    assert!(validate_relative_subdir("standard").is_err());
    assert!(validate_relative_subdir("STANDARD").is_err());
}

#[test]
fn search_limit_is_always_bounded_and_nonzero() {
    assert_eq!(normalized_hf_search_limit(None), 20);
    assert_eq!(normalized_hf_search_limit(Some(0)), 1);
    assert_eq!(normalized_hf_search_limit(Some(25)), 25);
    assert_eq!(normalized_hf_search_limit(Some(u32::MAX)), 100);
}

#[test]
fn search_url_requests_the_complete_card_projection() {
    let url = hf_model_search_url("flux model", "mflux", 25, Some("text-to-image"), true)
        .expect("search URL");
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    assert!(pairs.contains(&("search".to_string(), "flux model".to_string())));
    assert!(pairs.contains(&("filter".to_string(), "mflux".to_string())));
    assert!(pairs.contains(&("limit".to_string(), "25".to_string())));
    assert!(pairs.contains(&("pipeline_tag".to_string(), "text-to-image".to_string())));
    let expanded: Vec<&str> = pairs
        .iter()
        .filter_map(|(key, value)| (key == "expand[]").then_some(value.as_str()))
        .collect();
    assert_eq!(
        expanded,
        [
            "sha",
            "downloads",
            "likes",
            "tags",
            "lastModified",
            "gated",
            "pipeline_tag",
            "config",
        ]
    );

    let ordinary = hf_model_search_url("chat", "mlx", 25, Some("text-generation"), false)
        .expect("ordinary search URL");
    assert!(
        !ordinary
            .query_pairs()
            .any(|(key, value)| key == "expand[]" && value == "config"),
        "private config projection is only needed for compatibility-filtered searches"
    );
}

#[tokio::test]
async fn http_search_sends_auth_projection_filters_and_follows_locked_pagination() {
    let server = MockHttpServer::spawn(|base_url| {
        let first_url = hf_model_search_url_at(
            base_url,
            "embedding model",
            "mlx",
            100,
            Some("feature-extraction"),
            true,
        )
        .expect("first search URL");
        let mut next_url = first_url.clone();
        next_url.query_pairs_mut().append_pair("cursor", "page-two");
        vec![
            MockHttpResponse::json(
                200,
                serde_json::json!([{
                    "id": "owner/unsupported-qwen2",
                    "sha": "1111111111111111111111111111111111111111",
                    "downloads": 900,
                    "likes": 1,
                    "tags": ["mlx", "feature-extraction"],
                    "pipeline_tag": "feature-extraction",
                    "config": {
                        "model_type": "qwen2",
                        "architectures": ["Qwen2Model"]
                    },
                    "lastModified": "2026-01-01T00:00:00Z",
                    "gated": false
                }]),
            )
            .with_header("Link", format!("<{next_url}>; rel=\"next\"")),
            MockHttpResponse::json(
                200,
                serde_json::json!([{
                    "id": "owner/supported-bert",
                    "sha": "2222222222222222222222222222222222222222",
                    "downloads": 800,
                    "likes": 2,
                    "tags": ["mlx", "feature-extraction"],
                    "pipeline_tag": "feature-extraction",
                    "config": {
                        "model_type": "bert",
                        "architectures": ["BertModel"]
                    },
                    "lastModified": "2026-01-02T00:00:00Z",
                    "gated": false
                }]),
            ),
        ]
    })
    .await;
    let profile = HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Embedding,
        category: "Embedding",
        pipeline_tags: &["feature-extraction"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    };
    let client =
        build_hf_client_with_token(Some("hf_test_search_token")).expect("authenticated client");

    let result =
        discover_hf_models_with_http(&client, &server.base_url, "embedding model", profile, 1)
            .await
            .expect("HTTP search");

    assert_eq!(result.models.len(), 1);
    assert_eq!(result.models[0].id, "owner/supported-bert");
    assert_eq!(
        result.models[0].revision.as_deref(),
        Some("2222222222222222222222222222222222222222")
    );
    assert!(!result.has_more);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer hf_test_search_token")
        );
        let url = server
            .base_url
            .join(&request.target)
            .expect("observed search URL");
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(pairs.contains(&("search".to_string(), "embedding model".to_string())));
        assert!(pairs.contains(&("filter".to_string(), "mlx".to_string())));
        assert!(pairs.contains(&("pipeline_tag".to_string(), "feature-extraction".to_string())));
        assert!(pairs.contains(&("sort".to_string(), "downloads".to_string())));
        assert!(pairs.contains(&("direction".to_string(), "-1".to_string())));
        assert!(pairs.contains(&("expand[]".to_string(), "config".to_string())));
        assert!(pairs.contains(&("expand[]".to_string(), "gated".to_string())));
    }
    assert!(!requests[0].target.contains("cursor="));
    assert!(requests[1].target.contains("cursor=page-two"));
    server.finish().await;
}

#[tokio::test]
async fn http_statuses_have_stable_auth_gated_and_rate_limit_remediation() {
    let server = MockHttpServer::spawn(|_| {
        vec![
            MockHttpResponse::json(401, serde_json::json!({"error": "unauthorized"})),
            MockHttpResponse::json(403, serde_json::json!({"error": "gated"})),
            MockHttpResponse::json(429, serde_json::json!({"error": "rate limited"})),
        ]
    })
    .await;
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let url =
        hf_model_search_url_at(&server.base_url, "model", "gguf", 1, None, false).unwrap();
    let identity = parse_hf_search_route(&url).unwrap().identity;
    let client = build_hf_client_with_token(None).expect("HTTP client");

    let unauthorized = fetch_repo_metadata(&client, &server.base_url, "owner/repo", None)
        .await
        .expect_err("metadata authentication failure");
    assert_eq!(unauthorized, HF_HTTP_UNAUTHORIZED_MESSAGE);

    let forbidden = fetch_repo_tree(&client, &server.base_url, "owner/repo", revision)
        .await
        .expect_err("gated tree failure");
    assert_eq!(forbidden, HF_HTTP_FORBIDDEN_MESSAGE);

    let rate_limited = fetch_hf_model_page(&client, &url, "gguf", &identity)
        .await
        .expect_err("search rate limit");
    assert_eq!(rate_limited, HF_HTTP_RATE_LIMIT_MESSAGE);
    server.finish().await;
}

#[tokio::test]
async fn http_plan_pins_metadata_tree_and_complete_gguf_shards() {
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let server = MockHttpServer::spawn(|_| {
        vec![
            MockHttpResponse::json(
                200,
                serde_json::json!({
                    "sha": "0123456789abcdef0123456789abcdef01234567",
                    "tags": ["gguf", "text-generation"],
                    "pipeline_tag": "text-generation"
                }),
            ),
            MockHttpResponse::json(
                200,
                serde_json::json!([
                    {
                        "type": "file",
                        "path": "weights/model-Q4_K_M-00002-of-00002.gguf",
                        "size": 24,
                        "lfs": {
                            "size": 24,
                            "oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        }
                    },
                    {
                        "type": "file",
                        "path": "weights/model-Q4_K_M-00001-of-00002.gguf",
                        "size": 24,
                        "lfs": {
                            "size": 24,
                            "oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        }
                    }
                ]),
            ),
        ]
    })
    .await;
    let client =
        build_hf_client_with_token(Some("hf_test_plan_token")).expect("authenticated client");

    let plan = build_model_file_plan_with_http(
        &client,
        &server.base_url,
        "owner/repo",
        "llamacpp",
        HfModelTask::Chat,
        Some(revision),
    )
    .await
    .expect("revision-pinned plan");

    assert_eq!(plan.revision, revision);
    assert_eq!(plan.artifacts.len(), 1);
    assert_eq!(plan.artifacts[0].files.len(), 2);
    assert_eq!(
        plan.artifacts[0].primary_file.as_deref(),
        Some("weights/model-Q4_K_M-00001-of-00002.gguf")
    );
    assert_eq!(
        plan.artifacts[0].files[0].sha256.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(plan.artifacts[0].total_size, 48);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0]
        .target
        .starts_with(&format!("/api/models/owner/repo/revision/{revision}?")));
    assert!(requests[1]
        .target
        .starts_with(&format!("/api/models/owner/repo/tree/{revision}?")));
    assert!(requests.iter().all(|request| {
        request.headers.get("authorization").map(String::as_str)
            == Some("Bearer hf_test_plan_token")
    }));
    server.finish().await;
}

#[tokio::test]
#[ignore = "requires live access to the public Hugging Face Hub"]
async fn live_hf_search_metadata_and_tree_smoke_share_one_immutable_revision() {
    let base_url = production_hf_base_url();
    let client = build_hf_client_with_token(None).expect("live Hub client");
    let search_url =
        hf_model_search_url_at(&base_url, "tinyllamas-stories-gguf", "gguf", 5, None, false)
            .expect("live search URL");
    let identity = parse_hf_search_route(&search_url)
        .expect("live search route")
        .identity;
    let page = fetch_hf_model_page(&client, &search_url, "gguf", &identity)
        .await
        .expect("live public search");
    assert!(page
        .cards
        .iter()
        .any(|candidate| candidate.card.id == "klosax/tinyllamas-stories-gguf"));

    let metadata =
        fetch_repo_metadata(&client, &base_url, "klosax/tinyllamas-stories-gguf", None)
            .await
            .expect("live repository metadata");
    let tree = fetch_repo_tree(
        &client,
        &base_url,
        "klosax/tinyllamas-stories-gguf",
        &metadata.sha,
    )
    .await
    .expect("live immutable tree");
    assert!(tree
        .iter()
        .any(|file| file.path.to_ascii_lowercase().ends_with(".gguf")));
}

#[tokio::test]
#[ignore = "downloads a 2.79 MB GGUF from the public Hugging Face Hub"]
async fn live_pinned_gguf_download_verifies_redirect_hash_validation_and_atomic_manifest() {
    use sha2::Digest as _;

    const REPO_ID: &str = "aladar/tiny-random-LlamaForCausalLM-GGUF";
    const REVISION: &str = "6a57244a5aa2fcff3bd09899c8266a6a2df714a9";
    const FILE_PATH: &str = "tiny-random-LlamaForCausalLM.gguf";
    const FILE_SIZE: u64 = 2_789_632;
    const FILE_SHA256: &str =
        "f411bb6b67997e9d9e769c2d8438acd9759a4a3b2dc258500eeeb3c715d23e96";

    let base_url = production_hf_base_url();
    let planning_client = build_hf_client_with_token(None).expect("live Hub planning client");
    let plan = build_model_file_plan_with_http(
        &planning_client,
        &base_url,
        REPO_ID,
        "llamacpp",
        HfModelTask::Chat,
        Some(REVISION),
    )
    .await
    .expect("live revision-pinned GGUF plan");
    assert_eq!(plan.revision, REVISION);
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.files.len() == 1 && artifact.files[0].path == FILE_PATH)
        .expect("single-file GGUF artifact");
    assert_eq!(artifact.primary_file.as_deref(), Some(FILE_PATH));
    assert_eq!(artifact.files[0].size, FILE_SIZE);
    assert_eq!(artifact.files[0].sha256.as_deref(), Some(FILE_SHA256));

    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "hf-live-pinned-download-smoke".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some(REPO_ID.to_string()),
        revision: Some(REVISION.to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "llamacpp".to_string(),
        format: "gguf".to_string(),
        artifact_kind: "gguf_single".to_string(),
        artifact_id: Some(artifact.id.clone()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: Some(FILE_PATH.to_string()),
        files: vec![FILE_PATH.to_string()],
        quantization: artifact.quant_type.clone(),
    };
    crate::model_manager::validate_managed_model_manifest(&manifest)
        .expect("live smoke manifest");
    let files = vec![PlannedDownloadFile {
        path: FILE_PATH.to_string(),
        expected_size: Some(FILE_SIZE),
        sha256: Some(FILE_SHA256.to_string()),
    }];
    let temp = tempfile::tempdir().expect("live smoke app data");
    let download_client = build_hf_download_client().expect("live Hub download client");
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded_events = events.clone();
    let cancel = tokio::sync::Notify::new();

    let result = download_planned_hf_files_with_http(
        temp.path(),
        &download_client,
        &base_url,
        None,
        move |event| {
            recorded_events
                .lock()
                .expect("live download events")
                .push(event);
        },
        REPO_ID,
        REVISION,
        &files,
        "live-pinned-gguf-smoke",
        "LLM",
        "live-pinned-gguf-smoke-download",
        &cancel,
        &manifest,
    )
    .await
    .expect("live pinned GGUF download");

    let destination = temp.path().join("models/LLM/live-pinned-gguf-smoke");
    assert_eq!(result.destination_dir, destination);
    assert_eq!(result.downloaded_files, vec![FILE_PATH.to_string()]);
    assert_eq!(result.total_bytes, FILE_SIZE);
    let downloaded_path = destination.join(FILE_PATH);
    let downloaded = std::fs::read(&downloaded_path).expect("published live GGUF");
    assert_eq!(downloaded.len() as u64, FILE_SIZE);
    assert_eq!(hex::encode(sha2::Sha256::digest(&downloaded)), FILE_SHA256);
    crate::gguf::read_gguf_metadata(
        downloaded_path
            .to_str()
            .expect("live GGUF path is valid UTF-8"),
    )
    .expect("published live GGUF metadata");

    let persisted: crate::model_manager::ManagedModelManifest = serde_json::from_slice(
        &std::fs::read(destination.join(crate::model_manager::MODEL_MANIFEST_FILENAME))
            .expect("published live manifest"),
    )
    .expect("valid published live manifest");
    assert_eq!(persisted.repo_id.as_deref(), Some(REPO_ID));
    assert_eq!(persisted.revision.as_deref(), Some(REVISION));
    assert_eq!(persisted.files, vec![FILE_PATH.to_string()]);
    let category_entries: Vec<String> = std::fs::read_dir(temp.path().join("models/LLM"))
        .expect("live model category")
        .map(|entry| {
            entry
                .expect("live category entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(category_entries, ["live-pinned-gguf-smoke"]);
    assert_eq!(
        events
            .lock()
            .expect("live download events")
            .last()
            .and_then(|event| event["status"].as_str()),
        Some("completed")
    );
}

#[test]
fn parse_model_card_basic() {
    let json = serde_json::json!({
        "id": "unsloth/Llama-3-8B-GGUF",
        "downloads": 50000,
        "likes": 120,
        "tags": ["gguf", "text-generation"],
        "lastModified": "2024-06-01T00:00:00.000Z",
        "gated": false
    });
    let card = parse_model_card(&json).expect("should parse");
    assert_eq!(card.id, "unsloth/Llama-3-8B-GGUF");
    assert_eq!(card.author, "unsloth");
    assert_eq!(card.name, "Llama-3-8B-GGUF");
    assert_eq!(card.downloads, 50000.0);
    assert_eq!(card.likes, 120);
    assert!(!card.gated);
    assert_eq!(card.revision, None);
}

#[test]
fn parse_model_card_preserves_standalone_pipeline_tag() {
    let json = serde_json::json!({
        "id": "owner/model",
        "downloads": 1,
        "likes": 0,
        "tags": ["mlx", "MLX"],
        "pipeline_tag": "text-generation",
        "lastModified": "",
        "gated": false
    });
    let card = parse_model_card(&json).expect("model card");
    assert_eq!(card.tags, ["text-generation", "mlx"]);
    let candidate = parse_model_search_candidate(&json).expect("search candidate");
    assert!(model_matches_profile(
        &candidate,
        capability_profile("mlx", HfModelTask::Chat).unwrap()
    ));
}

#[test]
fn search_filters_mlx_embedding_configs_without_exposing_them_in_cards() {
    let embedding = capability_profile("mlx", HfModelTask::Embedding).unwrap();
    let candidate = |model_type: &str, architectures: &[&str]| {
        parse_model_search_candidate(&serde_json::json!({
            "id": format!("owner/{model_type}"),
            "downloads": 1,
            "likes": 0,
            "tags": ["mlx", "feature-extraction"],
            "pipeline_tag": "feature-extraction",
            "config": {
                "model_type": model_type,
                "architectures": architectures,
            },
            "lastModified": "",
            "gated": false
        }))
        .expect("search candidate")
    };

    assert!(model_matches_profile(
        &candidate("bert", &["BertModel"]),
        embedding
    ));
    assert!(model_matches_profile(
        &candidate("xlm-roberta", &["XLMRobertaModel"]),
        embedding
    ));
    assert!(model_matches_profile(
        &candidate("qwen3", &["Qwen3Model"]),
        embedding
    ));
    assert!(model_matches_profile(
        &candidate("gemma3_text", &["Gemma3TextModel"]),
        embedding
    ));
    assert!(model_matches_profile(
        &candidate("modernbert", &["ModernBertModel"]),
        embedding
    ));
    assert!(!model_matches_profile(
        &candidate("qwen2", &["Qwen2Model"]),
        embedding
    ));
    assert!(!model_matches_profile(
        &candidate("modernbert", &["ModernBertForMaskedLM"]),
        embedding
    ));

    let missing_config = parse_model_search_candidate(&serde_json::json!({
        "id": "owner/no-config",
        "downloads": 1,
        "likes": 0,
        "tags": ["mlx", "feature-extraction"],
        "pipeline_tag": "feature-extraction",
        "lastModified": "",
        "gated": false
    }))
    .expect("search candidate");
    assert!(!model_matches_profile(&missing_config, embedding));

    let filtered = finalize_profile_search(
        vec![
            candidate("qwen2", &["Qwen2Model"]),
            candidate("bert", &["BertModel"]),
        ],
        embedding,
        10,
        false,
    );
    assert_eq!(filtered.models.len(), 1);
    assert_eq!(filtered.models[0].id, "owner/bert");
    assert!(
        serde_json::to_value(&filtered.models[0])
            .expect("serialized public card")
            .get("config")
            .is_none(),
        "private Hub config must not be serialized into HfModelCard"
    );

    let mut unsupported_duplicate = candidate("qwen2", &["Qwen2Model"]);
    unsupported_duplicate.card.id = "owner/duplicate".to_string();
    let mut supported_duplicate = candidate("bert", &["BertModel"]);
    supported_duplicate.card.id = "owner/duplicate".to_string();
    let filtered = finalize_profile_search(
        vec![unsupported_duplicate, supported_duplicate],
        embedding,
        10,
        false,
    );
    assert_eq!(
        filtered.models.len(),
        1,
        "an invalid duplicate from one pipeline route must not hide the valid candidate"
    );
}

#[test]
fn search_filters_text_only_models_from_mlx_vision_results() {
    let vision = capability_profile("mlx", HfModelTask::Vision).unwrap();
    let candidate = |id: &str, config: Option<serde_json::Value>| {
        let mut value = serde_json::json!({
            "id": id,
            "downloads": 1,
            "likes": 0,
            "tags": ["mlx", "image-text-to-text"],
            "pipeline_tag": "image-text-to-text",
            "lastModified": "",
            "gated": false
        });
        if let Some(config) = config {
            value["config"] = config;
        }
        parse_model_search_candidate(&value).expect("search candidate")
    };

    let multimodal = candidate(
        "owner/vision",
        Some(serde_json::json!({
            "architectures": ["LlavaForConditionalGeneration"],
            "vision_config": {}
        })),
    );
    let tagged_text_only = candidate(
        "owner/text-only",
        Some(serde_json::json!({
            "architectures": ["LlamaForCausalLM"]
        })),
    );
    let missing_config = candidate("owner/missing-config", None);

    assert!(model_matches_profile(&multimodal, vision));
    assert!(!model_matches_profile(&tagged_text_only, vision));
    assert!(!model_matches_profile(&missing_config, vision));
    let filtered = finalize_profile_search(
        vec![tagged_text_only, missing_config, multimodal],
        vision,
        10,
        false,
    );
    assert_eq!(filtered.models.len(), 1);
    assert_eq!(filtered.models[0].id, "owner/vision");
    assert!(profile_requires_search_config(vision));
    assert!(profile_requires_family_narrowing(vision));
}

#[test]
fn parse_model_card_gated_string() {
    // HF API sometimes returns "gated": "auto" instead of bool
    let json = serde_json::json!({
        "id": "meta-llama/Llama-3-8B",
        "downloads": 100,
        "likes": 5,
        "tags": [],
        "lastModified": "",
        "gated": "auto"
    });
    let card = parse_model_card(&json).expect("should parse");
    assert!(card.gated, "gated: 'auto' should be treated as gated=true");
}

#[test]
fn narrowed_search_filters_a_larger_pool_before_requested_truncation() {
    let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
    assert_eq!(hf_search_candidate_limit(profile, 2), 100);
    assert_eq!(
        hf_search_candidate_limit(
            capability_profile("mlx", HfModelTask::Embedding).unwrap(),
            2
        ),
        100
    );
    assert_eq!(
        hf_search_candidate_limit(capability_profile("mlx", HfModelTask::Chat).unwrap(), 2),
        2
    );

    let card = |id: &str| HfModelSearchCandidate {
        card: HfModelCard {
            id: id.to_string(),
            author: "owner".to_string(),
            name: id.to_string(),
            downloads: 1.0,
            likes: 0,
            tags: vec![
                "mlx".to_string(),
                "automatic-speech-recognition".to_string(),
            ],
            last_modified: String::new(),
            gated: false,
            revision: None,
        },
        config: None,
    };
    let candidates = vec![
        card("owner/not-whispr-a"),
        card("owner/not-whispr-b"),
        card("owner/whisper-one"),
        card("owner/whisper-two"),
        card("owner/whisper-three"),
    ];
    assert_eq!(
        compatible_profile_model_count(&candidates, profile, usize::MAX),
        3
    );
    let filtered = finalize_profile_search(candidates, profile, 2, false);

    assert_eq!(filtered.models.len(), 2);
    assert_eq!(filtered.models[0].id, "owner/whisper-one");
    assert_eq!(filtered.models[1].id, "owner/whisper-three");
    assert!(filtered.has_more);

    let exact = vec![card("owner/whisper-only")];
    assert!(!finalize_profile_search(exact.clone(), profile, 1, false).has_more);
    assert!(finalize_profile_search(exact, profile, 1, true).has_more);
}

#[test]
fn bounded_search_rounds_continue_only_for_missing_family_matches() {
    assert!(should_fetch_another_search_round(
        true, 2, 3, true, true, 2, 8,
    ));
    assert!(!should_fetch_another_search_round(
        true, 3, 3, true, true, 2, 8,
    ));
    assert!(!should_fetch_another_search_round(
        true, 2, 3, true, true, 8, 8,
    ));
    assert!(!should_fetch_another_search_round(
        false, 2, 3, true, true, 1, 8,
    ));
}

#[test]
fn pinned_config_preflight_enforces_runtime_format_contracts() {
    let mlx_vision = capability_profile("mlx", HfModelTask::Vision).unwrap();
    assert!(profile_requires_config_preflight(mlx_vision));
    assert!(validate_profile_config(
        mlx_vision,
        &serde_json::json!({
            "architectures": ["LlavaForConditionalGeneration"],
            "vision_config": {}
        })
    )
    .is_ok());
    assert!(validate_profile_config(
        mlx_vision,
        &serde_json::json!({"architectures": ["LlamaForCausalLM"]})
    )
    .is_err());

    let mlx_embedding = capability_profile("mlx", HfModelTask::Embedding).unwrap();
    assert!(profile_requires_config_preflight(mlx_embedding));
    for config in [
        serde_json::json!({
            "model_type": "bert",
            "architectures": ["BertModel"]
        }),
        serde_json::json!({
            "model_type": "xlm-roberta",
            "architectures": ["XLMRobertaModel"]
        }),
        serde_json::json!({
            "model_type": "qwen3",
            "architectures": ["Qwen3Model"]
        }),
        serde_json::json!({
            "model_type": "gemma3_text",
            "architectures": ["Gemma3TextModel"]
        }),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": ["ModernBertModel"]
        }),
    ] {
        assert!(
            validate_profile_config(mlx_embedding, &config).is_ok(),
            "expected pinned MLX embedding config to be accepted: {config}"
        );
    }
    for config in [
        serde_json::json!({
            "model_type": "qwen2",
            "architectures": ["Qwen2Model"]
        }),
        serde_json::json!({
            "model_type": "modernbert",
            "architectures": ["ModernBertForMaskedLM"]
        }),
        serde_json::json!({"model_type": "modernbert"}),
        serde_json::json!({"architectures": ["BertModel"]}),
        serde_json::json!({"model_type": "unknown"}),
    ] {
        assert!(
            validate_profile_config(mlx_embedding, &config).is_err(),
            "expected unsupported MLX embedding config to be rejected: {config}"
        );
    }

    let mflux = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
    assert!(profile_requires_config_preflight(mflux));
    assert!(validate_profile_config(
        mflux,
        &serde_json::json!({
            "_class_name": "FluxPipeline",
            "model_type": "flux-rectified-flow",
            "original_model": "black-forest-labs/FLUX.1-dev",
            "quantization": {"method": "mflux", "bits": 4}
        })
    )
    .is_ok());
    assert!(validate_profile_config(
        mflux,
        &serde_json::json!({
            "_class_name": "FluxPipeline",
            "model_type": "flux-rectified-flow",
            "original_model": "black-forest-labs/FLUX.1-Krea-dev",
            "quantization": {"method": "mflux", "bits": 4}
        })
    )
    .is_err());

    let awq = capability_profile("vllm", HfModelTask::Chat).unwrap();
    assert!(profile_requires_config_preflight(awq));
    assert!(validate_profile_config(
        awq,
        &serde_json::json!({"quantization_config": {"quant_method": "AWQ"}})
    )
    .is_ok());
    assert!(validate_profile_config(
        awq,
        &serde_json::json!({"quantization_config": {"quant_method": "gptq"}})
    )
    .is_err());
    assert!(validate_profile_config(awq, &serde_json::json!({})).is_err());

    let ordinary_mlx = capability_profile("mlx", HfModelTask::Chat).unwrap();
    assert!(!profile_requires_config_preflight(ordinary_mlx));
    assert!(validate_profile_config(ordinary_mlx, &serde_json::json!({})).is_ok());
}

#[test]
fn huggingface_lfs_oid_is_parsed_as_sha256() {
    let entry: HfTreeEntryWire = serde_json::from_value(serde_json::json!({
        "type": "file",
        "oid": "6e741018b927a409553a13979af2f9590676997f",
        "size": 123,
        "lfs": {
            "oid": "5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30",
            "size": 123,
            "pointerSize": 136
        },
        "path": "model.gguf"
    }))
    .unwrap();
    assert_eq!(
        entry.lfs.and_then(|lfs| lfs.sha256).as_deref(),
        Some("5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30")
    );
}

#[test]
fn capability_matrix_is_exact_and_has_no_duplicate_engine_tasks() {
    type ExpectedCapability<'a> = (
        &'a str,
        HfModelTask,
        &'a str,
        &'a [&'a str],
        &'a str,
        HfArtifactLayout,
    );
    let expected: &[ExpectedCapability<'_>] = &[
        (
            "llamacpp",
            HfModelTask::Chat,
            "LLM",
            &["text-generation"],
            "gguf",
            HfArtifactLayout::GgufVariants,
        ),
        (
            "llamacpp",
            HfModelTask::Vision,
            "LLM",
            &["image-text-to-text"],
            "gguf",
            HfArtifactLayout::GgufVariants,
        ),
        (
            "llamacpp",
            HfModelTask::Embedding,
            "Embedding",
            &["feature-extraction", "sentence-similarity"],
            "gguf",
            HfArtifactLayout::GgufVariants,
        ),
        (
            "mlx",
            HfModelTask::Chat,
            "LLM",
            &["text-generation"],
            "mlx",
            HfArtifactLayout::Directory,
        ),
        (
            "mlx",
            HfModelTask::Vision,
            "LLM",
            &["image-text-to-text"],
            "mlx",
            HfArtifactLayout::Directory,
        ),
        (
            "mlx",
            HfModelTask::Embedding,
            "Embedding",
            &["feature-extraction", "sentence-similarity"],
            "mlx",
            HfArtifactLayout::Directory,
        ),
        (
            "mlx",
            HfModelTask::Stt,
            "STT",
            &["automatic-speech-recognition"],
            "mlx",
            HfArtifactLayout::Directory,
        ),
        (
            "mlx",
            HfModelTask::Diffusion,
            "Diffusion",
            &["text-to-image", "image-to-image"],
            "mflux",
            HfArtifactLayout::Directory,
        ),
        (
            "vllm",
            HfModelTask::Chat,
            "LLM",
            &["text-generation"],
            "awq",
            HfArtifactLayout::Directory,
        ),
        (
            "vllm",
            HfModelTask::Vision,
            "LLM",
            &["image-text-to-text"],
            "awq",
            HfArtifactLayout::Directory,
        ),
    ];
    assert_eq!(HF_CAPABILITY_PROFILES.len(), expected.len());

    let mut seen = std::collections::HashSet::new();
    for (engine, task, category, pipeline_tags, format_tag, layout) in expected.iter().copied()
    {
        assert!(
            seen.insert((engine, task)),
            "duplicate expected capability for {engine}/{task:?}"
        );
        let profile = capability_profile(engine, task)
            .unwrap_or_else(|| panic!("missing capability for {engine}/{task:?}"));
        assert_eq!(profile.category, category);
        assert_eq!(profile.pipeline_tags, pipeline_tags);
        assert_eq!(profile.format_tag, format_tag);
        assert_eq!(profile.layout, layout);
    }

    let all_tasks = [
        HfModelTask::Chat,
        HfModelTask::Vision,
        HfModelTask::Embedding,
        HfModelTask::Stt,
        HfModelTask::Diffusion,
        HfModelTask::Tts,
    ];
    for engine in ["llamacpp", "mlx", "vllm", "ollama", "none", "unknown"] {
        for task in all_tasks {
            assert_eq!(
                capability_profile(engine, task).is_some(),
                seen.contains(&(engine, task)),
                "unexpected capability result for {engine}/{task:?}"
            );
        }
    }
}

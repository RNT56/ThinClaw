use super::*;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn prefetch_provider_context_uses_only_the_active_provider() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-prefetch-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
    let zep_recalls = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    strict_scoping: true,
                    hits: vec![ProviderMemoryHit {
                        provider: "honcho".to_string(),
                        summary: "Remembered preference".to_string(),
                        score: Some(0.91),
                        provenance: serde_json::json!({"id": "honcho:1"}),
                    }],
                    recalls: Arc::clone(&honcho_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    strict_scoping: true,
                    hits: vec![ProviderMemoryHit {
                        provider: "zep".to_string(),
                        summary: "Should not be used".to_string(),
                        score: Some(0.32),
                        provenance: serde_json::json!({"id": "zep:1"}),
                    }],
                    recalls: Arc::clone(&zep_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let context = orchestrator
        .prefetch_provider_context(
            &provider_access(user_id, "actor-1"),
            "summarize my preferences",
            3,
        )
        .await
        .expect("active provider should return prefetch context");

    assert_eq!(context.provider, "honcho");
    assert_eq!(context.context_refs, vec!["honcho:1"]);
    assert!(context.rendered_context.contains("honcho"));
    assert_eq!(
        honcho_recalls.lock().expect("honcho recall log").len(),
        1,
        "the selected provider should be queried exactly once"
    );
    assert!(
        zep_recalls.lock().expect("zep recall log").is_empty(),
        "inactive providers must not be queried"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn unhealthy_active_provider_fails_closed_for_prefetch_and_tool_surface() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-health-gating-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
    let zep_recalls = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    strict_scoping: true,
                    hits: vec![ProviderMemoryHit {
                        provider: "honcho".to_string(),
                        summary: "Should not be recalled".to_string(),
                        score: Some(0.11),
                        provenance: serde_json::json!({"id": "honcho:down"}),
                    }],
                    recalls: Arc::clone(&honcho_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status(
                        "honcho",
                        ProviderReadiness::Unhealthy,
                        false,
                        Some("provider health check failed"),
                    ),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    strict_scoping: true,
                    hits: vec![ProviderMemoryHit {
                        provider: "zep".to_string(),
                        summary: "Inactive backup".to_string(),
                        score: Some(0.88),
                        provenance: serde_json::json!({"id": "zep:1"}),
                    }],
                    recalls: Arc::clone(&zep_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let statuses = orchestrator.provider_health(user_id).await;
    let active = statuses
        .iter()
        .find(|status| status.provider == "honcho")
        .expect("active provider status");
    assert!(active.active, "honcho should be marked active");
    assert_eq!(active.readiness, ProviderReadiness::Unhealthy);

    assert!(
        orchestrator
            .prefetch_provider_context(
                &provider_access(user_id, "actor-1"),
                "remember my preferences",
                3,
            )
            .await
            .is_none(),
        "unhealthy providers should not surface prompt recall"
    );
    assert!(
        orchestrator
            .provider_recall(
                &provider_access(user_id, "actor-1"),
                "remember my preferences",
                3,
            )
            .await
            .is_err(),
        "unhealthy providers should not execute recall calls"
    );
    assert!(
        orchestrator
            .provider_tool_extensions(&provider_access(user_id, "actor-1"))
            .await
            .is_empty(),
        "tool extensions should disappear when the active provider is unhealthy"
    );
    assert!(
        honcho_recalls.lock().expect("honcho recall log").is_empty(),
        "prefetch/recall must fail closed before dispatching to an unhealthy provider"
    );
    assert!(
        zep_recalls.lock().expect("zep recall log").is_empty(),
        "inactive backups must not be used automatically"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn export_provider_payload_uses_only_ready_active_provider() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-export-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_exports = Arc::new(Mutex::new(Vec::new()));
    let zep_exports = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    strict_scoping: true,
                    hits: Vec::new(),
                    recalls: Arc::new(Mutex::new(Vec::new())),
                    exports: Arc::clone(&honcho_exports),
                    health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    strict_scoping: true,
                    hits: Vec::new(),
                    recalls: Arc::new(Mutex::new(Vec::new())),
                    exports: Arc::clone(&zep_exports),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let provider = orchestrator
        .export_provider_payload(
            &provider_access(user_id, "actor-1"),
            &serde_json::json!({"content": "prefers concise docs"}),
        )
        .await
        .expect("export should use active provider");

    assert_eq!(provider, "honcho");
    let exports = honcho_exports.lock().expect("honcho export log");
    assert_eq!(exports.len(), 1);
    assert_eq!(
        exports[0].0,
        provider_access(user_id, "actor-1").provider_subject_id()
    );
    assert_eq!(exports[0].1["content"], "prefers concise docs");
    assert_eq!(exports[0].1["_thinclaw_scope"]["subject_id"], exports[0].0);
    assert!(
        zep_exports.lock().expect("zep export log").is_empty(),
        "inactive providers must not receive explicit exports"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn provider_exports_isolate_actors_and_share_only_the_exact_group_scope() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-scope-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let exports = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![Arc::new(TestMemoryProvider {
                name: "honcho",
                strict_scoping: true,
                hits: Vec::new(),
                recalls: Arc::new(Mutex::new(Vec::new())),
                exports: Arc::clone(&exports),
                health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
            })],
        )),
    };

    let alice = provider_access(user_id, "alice");
    let bob = provider_access(user_id, "bob");
    let group_scope = uuid::Uuid::new_v4();
    let group_alice = thinclaw_identity::AccessContext {
        principal_id: user_id.to_string(),
        actor_id: "alice".to_string(),
        conversation_scope_id: group_scope,
        conversation_kind: ConversationKind::Group,
        channel: "test".to_string(),
    };
    let group_bob = thinclaw_identity::AccessContext {
        actor_id: "bob".to_string(),
        ..group_alice.clone()
    };

    for access in [&alice, &bob, &group_alice, &group_bob] {
        orchestrator
            .export_provider_payload(access, &serde_json::json!({"content": "scope probe"}))
            .await
            .expect("scoped export");
    }

    let exports = exports.lock().expect("export log");
    assert_ne!(
        exports[0].0, exports[1].0,
        "sibling actors must be isolated"
    );
    assert_eq!(
        exports[2].0, exports[3].0,
        "members of the same group use the conversation subject"
    );
    assert_ne!(exports[0].0, exports[2].0, "direct and group memory differ");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn provider_without_strict_subject_scoping_is_denied_every_memory_surface() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-unscoped-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let recalls = Arc::new(Mutex::new(Vec::new()));
    let exports = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![Arc::new(TestMemoryProvider {
                name: "honcho",
                strict_scoping: false,
                hits: Vec::new(),
                recalls: Arc::clone(&recalls),
                exports: Arc::clone(&exports),
                health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
            })],
        )),
    };
    let access = provider_access(user_id, "actor-1");

    assert!(
        orchestrator
            .prefetch_provider_context(&access, "probe", 3)
            .await
            .is_none()
    );
    assert!(
        orchestrator
            .provider_recall(&access, "probe", 3)
            .await
            .is_err()
    );
    assert!(
        orchestrator
            .export_provider_payload(&access, &serde_json::json!({"content": "probe"}))
            .await
            .is_err()
    );
    assert!(
        orchestrator
            .provider_tool_extensions(&access)
            .await
            .is_empty()
    );
    assert!(recalls.lock().expect("recall log").is_empty());
    assert!(exports.lock().expect("export log").is_empty());
}

#[test]
fn provider_hit_parser_handles_memory_service_and_vector_shapes() {
    let mem0_hits = parse_provider_hits(
        serde_json::json!({
            "results": [
                {"id": "m1", "memory": "likes terse changelogs", "score": 0.88}
            ]
        }),
        "mem0",
    );
    assert_eq!(mem0_hits[0].summary, "likes terse changelogs");
    assert_eq!(mem0_hits[0].score, Some(0.88));

    let chroma_hits = parse_provider_hits(
        serde_json::json!({
            "ids": [["doc-1"]],
            "documents": [["uses qdrant for high-recall vector search"]],
            "distances": [[0.12]],
            "metadatas": [[{"source": "test"}]]
        }),
        "chroma",
    );
    assert_eq!(
        chroma_hits[0].summary,
        "uses qdrant for high-recall vector search"
    );
    assert_eq!(chroma_hits[0].score, Some(0.12));

    let qdrant_hits = parse_provider_hits(
        serde_json::json!({
            "result": {
                "points": [
                    {
                        "id": "point-1",
                        "score": 0.77,
                        "payload": {"text": "keeps OpenMemory local"}
                    }
                ]
            }
        }),
        "qdrant",
    );
    assert_eq!(qdrant_hits[0].summary, "keeps OpenMemory local");
    assert_eq!(qdrant_hits[0].score, Some(0.77));
}

#[tokio::test]
async fn configured_http_memory_providers_recall_export_and_apply_auth() {
    let server = spawn_mock_provider_server().await;

    for (provider_name, provider, auth_header, auth_value) in configured_provider_cases() {
        let settings = configured_provider_settings(provider_name, &server.base_url, true);
        let hits = provider
            .recall(&settings, "user-123", "what do you remember?", 2)
            .await
            .unwrap_or_else(|err| panic!("{provider_name} recall failed: {err}"));
        assert_eq!(
            hits.len(),
            1,
            "{provider_name} should return one recall hit"
        );
        assert!(
            hits[0].summary.to_ascii_lowercase().contains(
                provider_name
                    .strip_suffix("_http")
                    .unwrap_or(provider_name)
                    .split('_')
                    .next()
                    .unwrap()
            ),
            "{provider_name} recall should parse provider-specific response"
        );

        provider
            .export_turn(
                &settings,
                "user-123",
                &serde_json::json!({"content": "prefers direct answers"}),
            )
            .await
            .unwrap_or_else(|err| panic!("{provider_name} export failed: {err}"));

        let requests = server.requests();
        let provider_requests = requests
            .iter()
            .filter(|request| {
                request.path.contains(provider_name)
                    || (provider_name == "custom_http" && request.path.contains("/custom/"))
                    || (provider_name == "letta" && request.path.contains("/letta/"))
                    || (provider_name == "chroma" && request.path.contains("/embed"))
                    || (provider_name == "qdrant" && request.path.contains("/embed"))
            })
            .collect::<Vec<_>>();
        assert!(
            provider_requests.iter().any(|request| request
                .headers
                .get(auth_header)
                .is_some_and(|value| value == auth_value)),
            "{provider_name} should send configured provider auth"
        );
    }

    let requests = server.requests();
    let mem0_recall = requests
        .iter()
        .find(|request| request.path == "/mem0/search")
        .expect("mem0 recall request");
    assert_eq!(mem0_recall.method, "POST");
    assert_eq!(mem0_recall.body.as_ref().unwrap()["user_id"], "user-123");
    assert_eq!(mem0_recall.body.as_ref().unwrap()["top_k"], 2);

    let letta_recall = requests
        .iter()
        .find(|request| request.path == "/letta/agent-123/search")
        .expect("letta recall request");
    assert_eq!(letta_recall.method, "GET");

    let chroma_upsert = requests
        .iter()
        .find(|request| request.path == "/chroma/collection-123/upsert")
        .expect("chroma export request");
    assert_eq!(
        chroma_upsert.body.as_ref().unwrap()["documents"][0],
        "prefers direct answers"
    );

    let qdrant_upsert = requests
        .iter()
        .find(|request| request.path == "/qdrant/memories/points")
        .expect("qdrant export request");
    assert_eq!(qdrant_upsert.method, "PUT");
    assert_eq!(
        qdrant_upsert.body.as_ref().unwrap()["points"][0]["payload"]["user_id"],
        "user-123"
    );
}

#[tokio::test]
async fn disabled_http_memory_providers_are_off_and_do_not_call_out() {
    let server = spawn_mock_provider_server().await;

    for (provider_name, provider, _, _) in configured_provider_cases() {
        let settings = configured_provider_settings(provider_name, &server.base_url, false);
        let health = provider.health(&settings).await;
        assert_eq!(
            health.readiness,
            ProviderReadiness::Disabled,
            "{provider_name} should report disabled health"
        );
        assert!(
            provider
                .recall(&settings, "user-123", "ignored", 2)
                .await
                .unwrap()
                .is_empty(),
            "{provider_name} disabled recall should be empty"
        );
        provider
            .export_turn(
                &settings,
                "user-123",
                &serde_json::json!({"content": "ignored"}),
            )
            .await
            .unwrap_or_else(|err| panic!("{provider_name} disabled export failed: {err}"));
    }

    assert!(
        server.requests().is_empty(),
        "disabled providers should not perform health, recall, or export HTTP requests"
    );
}

#[tokio::test]
async fn http_memory_providers_surface_recall_and_export_failures() {
    let server = spawn_mock_provider_server().await;

    for (provider_name, provider, _, _) in configured_provider_cases() {
        let mut settings = configured_provider_settings(provider_name, &server.base_url, true);
        let provider_settings = settings.providers.provider_mut(provider_name);
        match provider_name {
            "custom_http" => {
                provider_settings.config.insert(
                    "recall_url".to_string(),
                    format!("{}/fail", server.base_url),
                );
                provider_settings
                    .config
                    .insert("sync_url".to_string(), format!("{}/fail", server.base_url));
            }
            "letta" => {
                provider_settings
                    .config
                    .insert("search_path".to_string(), "/fail".to_string());
                provider_settings
                    .config
                    .insert("sync_path".to_string(), "/fail".to_string());
            }
            _ => {
                provider_settings
                    .config
                    .insert("search_path".to_string(), "/fail".to_string());
                provider_settings
                    .config
                    .insert("query_path".to_string(), "/fail".to_string());
                provider_settings
                    .config
                    .insert("sync_path".to_string(), "/fail".to_string());
            }
        }

        let recall_err = provider
            .recall(&settings, "user-123", "fail please", 2)
            .await
            .expect_err("recall should fail");
        assert!(
            recall_err.contains("500") || recall_err.contains("mock failure"),
            "{provider_name} recall failure should include HTTP failure context: {recall_err}"
        );

        let export_err = provider
            .export_turn(
                &settings,
                "user-123",
                &serde_json::json!({"content": "fail please"}),
            )
            .await
            .expect_err("export should fail");
        assert!(
            export_err.contains("500") || export_err.contains("mock failure"),
            "{provider_name} export failure should include HTTP failure context: {export_err}"
        );
    }
}

#[tokio::test]
async fn vector_provider_health_requires_embedding_wiring() {
    let mut settings = LearningSettings::default();
    let mut qdrant = crate::settings::LearningProviderSettings {
        enabled: true,
        ..crate::settings::LearningProviderSettings::default()
    };
    qdrant
        .config
        .insert("collection".to_string(), "memories".to_string());
    *settings.providers.provider_mut("qdrant") = qdrant;

    let status = QdrantProvider.health(&settings).await;
    assert_eq!(status.readiness, ProviderReadiness::NotConfigured);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("embedding_url")),
        "vector memory providers should report missing embedding wiring"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn positive_feedback_promotes_generated_skill_and_updates_candidate_proposal() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "generated-skill-positive-feedback";
    let created_at = Utc::now();
    let skill_name = "workflow-generated-positive";
    let skill_content = generated_skill_test_content(skill_name);
    let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert learning candidate");

    let user_dir = tempfile::tempdir().expect("temporary user dir for generated skill registry");
    let installed_dir =
        tempfile::tempdir().expect("temporary installed dir for generated skill registry");
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(installed_dir.path().to_path_buf()),
    ));
    let orchestrator =
        LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

    orchestrator
        .submit_feedback(
            user_id,
            "skill",
            skill_name,
            "helpful",
            Some("this saved time"),
            None,
        )
        .await
        .expect("positive feedback should activate generated skill");

    assert!(
        registry.read().await.has(skill_name),
        "positive feedback should install the generated skill"
    );

    let persisted = db
        .list_learning_candidates(user_id, Some("skill"), None, 10)
        .await
        .expect("list learning candidates")
        .into_iter()
        .find(|entry| entry.id == candidate.id)
        .expect("updated candidate");
    assert_eq!(
        persisted
            .proposal
            .get("lifecycle_status")
            .and_then(|value| value.as_str()),
        Some("active")
    );
    assert_eq!(
        persisted
            .proposal
            .get("activation_reason")
            .and_then(|value| value.as_str()),
        Some("explicit_positive_feedback")
    );
    assert_eq!(
        persisted
            .proposal
            .get("last_feedback")
            .and_then(|value| value.get("verdict"))
            .and_then(|value| value.as_str()),
        Some("helpful")
    );
    assert!(
        persisted
            .proposal
            .get("state_history")
            .and_then(|value| value.as_array())
            .is_some_and(|entries| entries.len() >= 2),
        "candidate proposal should retain lifecycle history on the canonical record"
    );

    let versions = db
        .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
        .await
        .expect("list learning artifact versions");
    let active_version = versions
        .iter()
        .find(|version| version.status == "active")
        .expect("active artifact version");
    assert_eq!(active_version.candidate_id, Some(candidate.id));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn negative_feedback_rolls_back_generated_skill_and_updates_candidate_proposal() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "generated-skill-negative-feedback";
    let created_at = Utc::now();
    let skill_name = "workflow-generated-negative";
    let skill_content = generated_skill_test_content(skill_name);
    let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert learning candidate");

    let user_dir = tempfile::tempdir().expect("temporary user dir for generated skill registry");
    let installed_dir =
        tempfile::tempdir().expect("temporary installed dir for generated skill registry");
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(installed_dir.path().to_path_buf()),
    ));
    registry
        .write()
        .await
        .install_skill(&skill_content)
        .await
        .expect("preinstall generated skill");
    let orchestrator =
        LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

    orchestrator
        .submit_feedback(
            user_id,
            "skill",
            skill_name,
            "reject",
            Some("this introduced drift"),
            None,
        )
        .await
        .expect("negative feedback should update generated skill lifecycle");

    assert!(
        !registry.read().await.has(skill_name),
        "negative feedback should remove the installed generated skill"
    );

    let persisted = db
        .list_learning_candidates(user_id, Some("skill"), None, 10)
        .await
        .expect("list learning candidates")
        .into_iter()
        .find(|entry| entry.id == candidate.id)
        .expect("updated candidate");
    assert_eq!(
        persisted
            .proposal
            .get("lifecycle_status")
            .and_then(|value| value.as_str()),
        Some("rolled_back")
    );
    assert_eq!(
        persisted
            .proposal
            .get("last_feedback")
            .and_then(|value| value.get("verdict"))
            .and_then(|value| value.as_str()),
        Some("reject")
    );
    assert!(
        persisted
            .proposal
            .get("rolled_back_at")
            .and_then(|value| value.as_str())
            .is_some(),
        "candidate proposal should record rollback timing"
    );

    let versions = db
        .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
        .await
        .expect("list learning artifact versions");
    let rollback_version = versions
        .iter()
        .find(|version| version.status == "rolled_back")
        .expect("rollback artifact version");
    assert_eq!(rollback_version.candidate_id, Some(candidate.id));
}

#[test]
fn custom_http_provider_parses_common_recall_shapes() {
    let hits = parse_custom_http_hits(
        serde_json::json!({
            "results": [
                { "id": "m1", "summary": "prefers concise answers", "score": 0.82 },
                { "id": "m2", "content": "likes examples" },
                { "id": "m3", "summary": "" }
            ]
        }),
        "custom_http",
    );
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].provider, "custom_http");
    assert_eq!(hits[0].score, Some(0.82));
    assert_eq!(hits[1].summary, "likes examples");
}

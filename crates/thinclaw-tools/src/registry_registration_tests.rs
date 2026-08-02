use super::*;

#[tokio::test]
async fn built_in_identity_is_single_assignment() {
    let registry = ToolRegistry::new();
    assert!(
        registry
            .register_builtin(Arc::new(EchoTool))
            .await
            .changed()
    );
    assert!(
        !registry
            .register_builtin(Arc::new(EchoTool))
            .await
            .accepted()
    );
    assert_eq!(registry.count(), 1);
}

#[test]
fn synchronous_startup_rejects_duplicate_identity() {
    let registry = ToolRegistry::new();
    assert!(registry.register_sync(Arc::new(EchoTool)).changed());
    assert!(!registry.register_sync(Arc::new(EchoTool)).accepted());
    assert_eq!(registry.count(), 1);
}

#[test]
fn static_catalog_is_complete_unique_and_reserved() {
    assert_eq!(STATIC_TOOL_CATALOG.len(), 124);
    assert_eq!(PROTECTED_TOOL_NAMES.len(), 124);
    let unique = PROTECTED_TOOL_NAMES.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), 124);
}

#[test]
fn dynamic_collisions_reject_and_same_source_rebinds_explicitly() {
    let registry = ToolRegistry::new();
    let first: Arc<dyn Tool> = Arc::new(crate::builtin::EchoTool);
    let reserved = registry.register_request(RegistrationRequest::new(
        first,
        ToolOrigin::UserTool,
        "user/a",
    ));
    assert!(matches!(reserved, RegistrationOutcome::Rejected { .. }));
    let spoofed_builtin = registry.register_request(RegistrationRequest::new(
        Arc::new(crate::builtin::EchoTool),
        ToolOrigin::Core,
        "builtin/echo",
    ));
    assert!(matches!(
        spoofed_builtin,
        RegistrationOutcome::Rejected { .. }
    ));

    struct DynamicEcho;
    #[async_trait::async_trait]
    impl Tool for DynamicEcho {
        fn name(&self) -> &str {
            "dynamic_echo"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _arguments: serde_json::Value,
            _ctx: &thinclaw_types::JobContext,
        ) -> Result<thinclaw_tools_core::ToolOutput, thinclaw_tools_core::ToolError> {
            Ok(thinclaw_tools_core::ToolOutput::text(
                "ok",
                std::time::Duration::ZERO,
            ))
        }
    }

    let inserted = registry.register_request(RegistrationRequest::new(
        Arc::new(DynamicEcho),
        ToolOrigin::UserTool,
        "user/a",
    ));
    assert!(matches!(
        inserted,
        RegistrationOutcome::Inserted { revision: 1 }
    ));
    let conflict = registry.register_request(RegistrationRequest::new(
        Arc::new(DynamicEcho),
        ToolOrigin::Mcp,
        "mcp/a",
    ));
    assert!(matches!(conflict, RegistrationOutcome::Rejected { .. }));
    let rebound = registry.register_request(
        RegistrationRequest::new(Arc::new(DynamicEcho), ToolOrigin::UserTool, "user/a").replacing(),
    );
    assert!(matches!(
        rebound,
        RegistrationOutcome::Rebound { revision: 2 }
    ));
}

#[test]
fn batch_registration_is_all_or_nothing_and_snapshot_is_monotonic() {
    struct Named(&'static str);
    #[async_trait::async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _arguments: serde_json::Value,
            _ctx: &thinclaw_types::JobContext,
        ) -> Result<thinclaw_tools_core::ToolOutput, thinclaw_tools_core::ToolError> {
            Ok(thinclaw_tools_core::ToolOutput::text(
                "ok",
                std::time::Duration::ZERO,
            ))
        }
    }
    let registry = ToolRegistry::new();
    registry.register_request(RegistrationRequest::new(
        Arc::new(Named("taken")),
        ToolOrigin::UserTool,
        "user/a",
    ));
    let result = registry.register_batch(vec![
        RegistrationRequest::new(Arc::new(Named("new")), ToolOrigin::Mcp, "mcp/server"),
        RegistrationRequest::new(Arc::new(Named("taken")), ToolOrigin::Mcp, "mcp/server"),
    ]);
    assert!(result.is_err());
    assert_eq!(registry.count(), 1);
    let inserted = registry
        .register_batch(vec![
            RegistrationRequest::new(Arc::new(Named("new-one")), ToolOrigin::Mcp, "mcp/server"),
            RegistrationRequest::new(Arc::new(Named("new-two")), ToolOrigin::Mcp, "mcp/server"),
        ])
        .expect("atomic insert");
    assert!(
        inserted
            .iter()
            .all(|outcome| matches!(outcome, RegistrationOutcome::Inserted { revision: 2 }))
    );
    assert_eq!(registry.snapshot().revision, 2);
    let reconciled = registry
        .reconcile_source(
            ToolOrigin::Mcp,
            "mcp/server",
            vec![
                RegistrationRequest::new(Arc::new(Named("new-two")), ToolOrigin::Mcp, "mcp/server")
                    .replacing(),
                RegistrationRequest::new(
                    Arc::new(Named("new-three")),
                    ToolOrigin::Mcp,
                    "mcp/server",
                ),
            ],
        )
        .expect("atomic reconcile");
    assert!(reconciled.iter().all(|outcome| matches!(
        outcome,
        RegistrationOutcome::Inserted { revision: 3 }
            | RegistrationOutcome::Rebound { revision: 3 }
    )));
    let reconciled_snapshot = registry.snapshot();
    assert_eq!(reconciled_snapshot.revision, 3);
    assert!(
        !reconciled_snapshot
            .identities
            .iter()
            .any(|identity| identity.name == "new-one")
    );
    let before = registry.snapshot();
    let sealed = registry.seal_startup().expect("descriptor parity");
    assert!(sealed.sealed);
    assert_eq!(before.revision, sealed.revision);
    let advanced = registry.advance_capability_revision();
    assert_eq!(advanced.revision, sealed.revision + 1);
    assert_eq!(advanced.identities, sealed.identities);
    assert!(advanced.sealed);
}

#[test]
fn every_origin_collision_pair_requires_exact_owner_and_explicit_rebind() {
    struct Named(String);
    #[async_trait::async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _arguments: serde_json::Value,
            _ctx: &thinclaw_types::JobContext,
        ) -> Result<thinclaw_tools_core::ToolOutput, thinclaw_tools_core::ToolError> {
            Ok(thinclaw_tools_core::ToolOutput::text(
                "ok",
                std::time::Duration::ZERO,
            ))
        }
    }

    assert_eq!(ALL_TOOL_ORIGINS.len(), 20);
    for (left_index, left) in ALL_TOOL_ORIGINS.iter().copied().enumerate() {
        for (right_index, right) in ALL_TOOL_ORIGINS.iter().copied().enumerate() {
            let registry = ToolRegistry::new();
            let name = format!("collision_{left_index}_{right_index}");
            assert!(
                registry
                    .register_request(RegistrationRequest::new(
                        Arc::new(Named(name.clone())),
                        left,
                        "source/left",
                    ))
                    .changed()
            );
            let outcome = registry.register_request(RegistrationRequest::new(
                Arc::new(Named(name)),
                right,
                "source/right",
            ));
            assert!(matches!(outcome, RegistrationOutcome::Rejected { .. }));
            assert_eq!(registry.count(), 1);
        }
    }
}

#[test]
fn concurrent_registration_never_omits_unrelated_tools() {
    struct Named(String);
    #[async_trait::async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _arguments: serde_json::Value,
            _ctx: &thinclaw_types::JobContext,
        ) -> Result<thinclaw_tools_core::ToolOutput, thinclaw_tools_core::ToolError> {
            Ok(thinclaw_tools_core::ToolOutput::text(
                "ok",
                std::time::Duration::ZERO,
            ))
        }
    }

    let registry = Arc::new(ToolRegistry::new());
    let workers = (0..32)
        .map(|index| {
            let registry = Arc::clone(&registry);
            std::thread::spawn(move || {
                let name = format!("contention_{index}");
                registry.register_request(RegistrationRequest::new(
                    Arc::new(Named(name)),
                    ToolOrigin::UserTool,
                    format!("user/{index}"),
                ))
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        assert!(worker.join().expect("registration thread").changed());
    }
    assert_eq!(registry.count(), 32);
    assert_eq!(registry.snapshot().revision, 32);
}

#[test]
fn concurrent_capability_mutations_receive_distinct_revisions() {
    let registry = Arc::new(ToolRegistry::new());
    registry.register_sync(Arc::new(EchoTool));
    registry.seal_startup().expect("seal populated registry");

    let workers = (0..32)
        .map(|_| {
            let registry = Arc::clone(&registry);
            std::thread::spawn(move || registry.advance_capability_revision().revision)
        })
        .collect::<Vec<_>>();
    let mut revisions = workers
        .into_iter()
        .map(|worker| worker.join().expect("capability mutation thread"))
        .collect::<Vec<_>>();
    revisions.sort_unstable();

    assert_eq!(revisions, (2..=33).collect::<Vec<_>>());
    assert_eq!(registry.snapshot().revision, 33);
}

#[test]
fn empty_registry_cannot_be_sealed() {
    assert_eq!(
        ToolRegistry::new().seal_startup(),
        Err(RegistrySealError::EmptyRegistry)
    );
}

#[test]
fn ignored_startup_collision_prevents_seal() {
    let registry = ToolRegistry::new();
    assert!(registry.register_sync(Arc::new(EchoTool)).changed());
    assert!(!registry.register_sync(Arc::new(EchoTool)).accepted());
    assert!(matches!(
        registry.seal_startup(),
        Err(RegistrySealError::RegistrationFailures { .. })
    ));
}

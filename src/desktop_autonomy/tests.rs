use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn session_manager_limits_single_concurrency() {
    let manager = DesktopSessionManager::new(1);
    let lease = manager.acquire("main").await.expect("first lease");
    assert_eq!(lease.session_id(), "main");
}

#[test]
fn trim_failed_canaries_keeps_recent_entries() {
    let mut entries = vec![Utc::now() - chrono::Duration::hours(25), Utc::now()];
    trim_failed_canaries(&mut entries);
    assert_eq!(entries.len(), 1);
}

#[test]
fn rollback_history_moves_back_without_oscillating_to_newer_build() {
    let manifest = |build_id: &str, seconds: i64| BuildManifest {
        build_id: build_id.to_string(),
        user_id: "user".to_string(),
        proposal_id: "proposal".to_string(),
        title: build_id.to_string(),
        created_at: Utc::now() + chrono::Duration::seconds(seconds),
        promoted: true,
        checks: Vec::new(),
        metadata: serde_json::json!({}),
    };
    let manifests = vec![manifest("c", 3), manifest("b", 2), manifest("a", 1)];
    let history = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    let (target, history) =
        super::rollout_helpers::rollback_target_and_history(&manifests, &history, Some("c"));
    assert_eq!(target.as_deref(), Some("b"));
    let (target, history) =
        super::rollout_helpers::rollback_target_and_history(&manifests, &history, Some("b"));
    assert_eq!(target.as_deref(), Some("a"));
    assert_eq!(history, vec!["a"]);
}

#[test]
fn build_identifiers_reject_path_components() {
    assert!(super::rollout_helpers::valid_build_id("20260101-abcd_1234"));
    assert!(!super::rollout_helpers::valid_build_id("../escape"));
    assert!(!super::rollout_helpers::valid_build_id("name/child"));
    assert!(!super::rollout_helpers::valid_build_id(""));
}

#[test]
fn bootstrap_reason_helper_covers_dedicated_user_branches() {
    assert_eq!(
        dedicated_bootstrap_blocking_reason(false, false, false),
        "requires_privileged_bootstrap"
    );
    assert_eq!(dedicated_bootstrap_blocking_reason(false, true, true), "");
    assert_eq!(
        dedicated_bootstrap_blocking_reason(true, true, false),
        "needs_target_user_login"
    );
    assert_eq!(dedicated_bootstrap_blocking_reason(true, true, true), "");
}

#[test]
fn shell_single_quote_handles_embedded_quotes() {
    assert_eq!(shell_single_quote("plain"), "'plain'");
    assert_eq!(shell_single_quote("a'b"), "'a'\"'\"'b'");
}

#[test]
fn bridge_and_permission_reports_fail_closed() {
    assert!(!bridge_report_passed(&serde_json::json!({})));
    assert!(!bridge_report_passed(&serde_json::Value::Null));
    assert!(bridge_report_passed(&serde_json::json!({"ok": true})));

    assert!(!permissions_report_passed(&serde_json::json!({})));
    assert!(!permissions_report_passed(&serde_json::json!({
        "platform": "macos",
        "accessibility": true,
        "screen_recording": false,
        "calendar": "authorized",
    })));
    assert!(permissions_report_passed(&serde_json::json!({
        "platform": "macos",
        "accessibility": true,
        "screen_recording": true,
        "calendar": "authorized",
    })));
    assert!(!permissions_report_passed(&serde_json::json!({
        "platform": "linux",
        "accessibility": true,
        "screen_recording": true,
        "calendar": "missing",
        "ocr": "available",
    })));
}

#[test]
fn validate_numbers_payload_requires_normalized_fields() {
    let err = validate_numbers_payload(
        "run_table_action",
        &serde_json::json!({
            "table": "Table 1",
            "table_action": "add_column_after",
        }),
    )
    .expect_err("missing column_index should fail");
    assert!(err.contains("column_index"));
}

#[test]
fn canary_manifest_and_report_round_trip() {
    let manifest = DesktopCanaryManifest {
        build_id: "build-123".to_string(),
        proposal_id: "proposal-123".to_string(),
        report_path: PathBuf::from("/tmp/canary-report.json"),
        shadow_home: PathBuf::from("/tmp/shadow-home"),
        session_id: "desktop-main-session".to_string(),
        fixture_paths: DesktopFixturePaths {
            calendar_title: "ThinClaw Canary".to_string(),
            numbers_doc: Some(PathBuf::from("/tmp/canary.numbers")),
            pages_doc: Some(PathBuf::from("/tmp/canary.pages")),
            textedit_doc: Some(PathBuf::from("/tmp/canary.txt")),
            export_dir: Some(PathBuf::from("/tmp/exports")),
        },
    };
    let encoded = serde_json::to_string(&manifest).expect("serialize manifest");
    let decoded: DesktopCanaryManifest =
        serde_json::from_str(&encoded).expect("deserialize manifest");
    assert_eq!(decoded.build_id, manifest.build_id);

    let report = DesktopCanaryReport {
        build_id: manifest.build_id.clone(),
        generated_at: Utc::now(),
        passed: true,
        fixture_paths: manifest.fixture_paths.clone(),
        checks: vec![passed_check(
            "bridge_health",
            None,
            serde_json::json!({"ok": true}),
        )],
    };
    let report_encoded = serde_json::to_string(&report).expect("serialize report");
    let report_decoded: DesktopCanaryReport =
        serde_json::from_str(&report_encoded).expect("deserialize report");
    assert!(report_decoded.passed);
    assert_eq!(report_decoded.checks.len(), 1);
}

#[test]
fn copy_fixture_path_supports_package_directories() {
    let temp = tempdir().expect("tempdir");
    let src = temp.path().join("source.pages");
    let nested = src.join("Data");
    std::fs::create_dir_all(&nested).expect("create source package");
    std::fs::write(src.join("Index.xml"), "<doc />").expect("write package file");
    std::fs::write(nested.join("payload.txt"), "hello").expect("write nested file");

    let dst = temp.path().join("copy.pages");
    copy_fixture_path(&src, &dst).expect("copy package dir");

    assert!(dst.join("Index.xml").exists());
    assert_eq!(
        std::fs::read_to_string(dst.join("Data").join("payload.txt"))
            .expect("read copied nested file"),
        "hello"
    );
}

#[cfg(unix)]
#[test]
fn copy_fixture_path_rejects_symlinks() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("source.pages");
    std::fs::create_dir(&source).expect("create source package");
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, "secret").expect("write outside file");
    std::os::unix::fs::symlink(&outside, source.join("payload.txt"))
        .expect("create fixture symlink");

    let error = copy_fixture_path(&source, &temp.path().join("copy.pages"))
        .expect_err("fixture symlink must be rejected");
    assert!(error.contains("symlink"));
}

#[test]
fn bootstrap_report_serializes_extended_fields() {
    let report = AutonomyBootstrapReport {
        passed: false,
        health: serde_json::json!({"ok": true}),
        permissions: serde_json::json!({"accessibility": false}),
        seeded_skills: vec![PathBuf::from("/tmp/skill.md")],
        seeded_routines: vec!["daily_desktop_heartbeat".to_string()],
        launch_agent_path: Some(PathBuf::from("/tmp/test.plist")),
        launch_agent_written: true,
        launch_agent_loaded: false,
        fixture_paths: DesktopFixturePaths {
            calendar_title: "ThinClaw Canary".to_string(),
            ..Default::default()
        },
        session_ready: false,
        blocking_reason: Some("needs_target_user_login".to_string()),
        dedicated_user_keychain_label: Some("ThinClaw Desktop Autonomy/tester".to_string()),
        one_time_login_secret: Some("secret".to_string()),
        notes: vec!["note".to_string()],
    };
    let encoded = serde_json::to_string(&report).expect("serialize bootstrap report");
    assert!(encoded.contains("needs_target_user_login"));
    let decoded: AutonomyBootstrapReport =
        serde_json::from_str(&encoded).expect("deserialize bootstrap report");
    assert_eq!(decoded.fixture_paths.calendar_title, "ThinClaw Canary");
    assert_eq!(decoded.one_time_login_secret.as_deref(), Some("secret"));
}

#[test]
fn bridge_spec_matches_current_host_backend() {
    let spec = DesktopBridgeSpec::current();
    match spec.backend {
        DesktopBridgeBackend::MacOsSwift => {
            assert_eq!(spec.filename, MACOS_SIDECAR_FILENAME);
            assert!(spec.source.contains("ThinClawDesktopBridge"));
        }
        DesktopBridgeBackend::WindowsPowerShell => {
            assert_eq!(spec.filename, WINDOWS_SIDECAR_FILENAME);
            assert!(spec.source.contains("Invoke-Numbers"));
        }
        DesktopBridgeBackend::LinuxPython => {
            assert_eq!(spec.filename, LINUX_SIDECAR_FILENAME);
            assert!(spec.source.contains("invoke_numbers"));
        }
        DesktopBridgeBackend::Unsupported => {
            assert!(spec.source.is_empty());
        }
    }
}

#[test]
fn runtime_evidence_adds_platform_and_providers() {
    let manager = DesktopAutonomyManager::new(
        crate::config::DesktopAutonomyConfig {
            enabled: true,
            profile: crate::settings::DesktopAutonomyProfile::RecklessDesktop,
            deployment_mode: crate::settings::DesktopDeploymentMode::WholeMachineAdmin,
            target_username: None,
            desktop_max_concurrent_jobs: 1,
            desktop_action_timeout_secs: 30,
            capture_evidence: true,
            emergency_stop_path: PathBuf::from("/tmp/stop"),
            pause_on_bootstrap_failure: true,
            kill_switch_hotkey: "ctrl+option+command+period".to_string(),
        },
        None,
        None,
    );
    let evidence = manager.attach_runtime_evidence(
        "numbers_open_write_read_export",
        serde_json::json!({"export_path": "/tmp/out.csv"}),
    );
    assert_eq!(
        evidence.get("platform").and_then(|value| value.as_str()),
        Some(manager.platform_label())
    );
    assert!(evidence.get("providers").is_some());
    assert_eq!(
        evidence
            .get("bridge_backend")
            .and_then(|value| value.as_str()),
        Some(manager.bridge_backend().as_str())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shadow_canary_process_reads_fake_runner_output() {
    let temp = tempdir().expect("tempdir");
    let report_path = temp.path().join("canary-report.json");
    let binary_path = temp.path().join("fake-runner.sh");
    let manifest_path = report_path.with_file_name("canary-manifest.json");
    let fixture_paths = DesktopFixturePaths {
        calendar_title: "ThinClaw Canary".to_string(),
        ..Default::default()
    };
    let expected_report = DesktopCanaryReport {
        build_id: "build-123".to_string(),
        generated_at: Utc::now(),
        passed: true,
        fixture_paths: fixture_paths.clone(),
        checks: [
            "bridge_health",
            "permissions",
            "apps_list",
            "calendar_crud",
            "numbers_open_write_read_export",
            "pages_open_insert_find_export",
            "generic_ui_textedit_fallback",
        ]
        .into_iter()
        .map(|name| passed_check(name, None, serde_json::json!({"ok": true})))
        .collect(),
    };
    let report_json = serde_json::to_string(&expected_report).expect("serialize fake report");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" != \"autonomy-shadow-canary\" ]; then exit 2; fi\nprintf '%s\\n' {}\n",
        shell_single_quote(&report_json),
    );
    std::fs::write(&binary_path, script).expect("write fake runner");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&binary_path)
        .expect("metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&binary_path, perms).expect("chmod");

    let manifest = DesktopCanaryManifest {
        build_id: "build-123".to_string(),
        proposal_id: "proposal-123".to_string(),
        report_path: report_path.clone(),
        shadow_home: temp.path().join("shadow-home"),
        session_id: "desktop-main-session".to_string(),
        fixture_paths,
    };
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    let manager = DesktopAutonomyManager::new(
        crate::config::DesktopAutonomyConfig {
            enabled: true,
            profile: crate::settings::DesktopAutonomyProfile::RecklessDesktop,
            deployment_mode: crate::settings::DesktopDeploymentMode::WholeMachineAdmin,
            target_username: None,
            desktop_max_concurrent_jobs: 1,
            desktop_action_timeout_secs: 30,
            capture_evidence: true,
            emergency_stop_path: temp.path().join("stop"),
            pause_on_bootstrap_failure: true,
            kill_switch_hotkey: "ctrl+option+command+period".to_string(),
        },
        None,
        None,
    );
    let report = manager
        .run_shadow_canary_process(&binary_path, &manifest)
        .await
        .expect("fake canary report");
    assert!(report.passed);
    assert_eq!(report.build_id, "build-123");
}

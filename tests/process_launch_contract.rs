use std::collections::HashSet;

use serde_json::Value;

#[test]
fn all_manifest_entries_have_enforceable_descriptor_contract() {
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/process_launch_manifest.json"))
            .expect("process manifest JSON");
    let launches = manifest["launches"].as_array().expect("launch array");
    assert_eq!(
        manifest["launch_count"].as_u64(),
        Some(launches.len() as u64)
    );
    assert_eq!(launches.len(), 212);

    let mut identities = HashSet::new();
    let mut proofs = HashSet::new();
    let mut dynamic_environment_count = 0;
    for launch in launches {
        let id = launch["id"].as_str().expect("launch id");
        assert!(identities.insert(id), "duplicate launch identity {id}");
        let proof = launch["proof_id"].as_str().expect("proof id");
        assert!(
            proof
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
            "invalid proof id {proof}"
        );
        assert!(proofs.insert(proof), "duplicate proof id {proof}");
        assert_eq!(launch["classification"], "production");
        assert_eq!(launch["child_environment"], "exact_reviewed");
        assert!(!launch["owner"].as_str().expect("source owner").is_empty());
        assert!(launch["rust_module"].as_str().is_some_and(
            |owner| owner.starts_with("thinclaw") || owner.starts_with("tauri_app_lib")
        ));
        assert!(
            !launch["program"]
                .as_str()
                .expect("program policy")
                .is_empty()
        );
        let environment_schema = launch["environment_schema"]
            .as_str()
            .expect("environment schema");
        if environment_schema != "literal_keys_only" {
            dynamic_environment_count += 1;
        }
        assert!(
            launch["credential_slots"]
                .as_array()
                .expect("credential slots")
                .iter()
                .all(
                    |slot| slot["name"].as_str().is_some_and(|value| !value.is_empty())
                        && slot["purpose"]
                            .as_str()
                            .is_some_and(|value| !value.is_empty())
                        && slot["sink"].as_str().is_some_and(|value| !value.is_empty())
                )
        );
        assert_eq!(launch["callsite_digest"].as_str().map(str::len), Some(64));
        assert!(launch["source_line"].as_u64().is_some_and(|line| line > 0));
        assert!(matches!(
            launch["execution_policy"].as_str(),
            Some(
                "bounded_owned" | "owned_lifecycle" | "reaped_host_integration" | "caller_mediated"
            )
        ));
        assert!(launch["io_policy"]["stdout_limit"].as_u64().is_some());
        assert!(
            launch["lifetime_policy"]["reap_on_drop"]
                .as_bool()
                .is_some()
        );
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(launch["source"].as_str().expect("source path"));
        let text = std::fs::read_to_string(source).expect("manifest source exists");
        assert!(
            text.contains(id),
            "source no longer contains launch id {id}"
        );
    }
    assert_eq!(dynamic_environment_count, 8);
}

#[test]
fn runtime_and_test_process_manifests_are_byte_identical() {
    assert_eq!(
        include_bytes!("fixtures/process_launch_manifest.json"),
        include_bytes!("../crates/thinclaw-platform/src/process_launch_manifest.json")
    );
}

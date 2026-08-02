use std::collections::HashSet;

use serde_json::Value;

#[test]
fn credential_consumer_lifecycles_are_complete_and_secret_safe() {
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/credential_consumer_manifest.json"))
            .expect("credential manifest JSON");
    let candidates = manifest["candidates"].as_array().expect("candidate array");
    assert_eq!(
        manifest["candidate_count"].as_u64(),
        Some(candidates.len() as u64)
    );
    assert!(candidates.len() >= 300);

    let mut identities = HashSet::new();
    let mut proofs = HashSet::new();
    for candidate in candidates {
        let id = candidate["id"].as_str().expect("candidate id");
        assert!(identities.insert(id), "duplicate credential candidate {id}");
        let proof = candidate["proof_id"].as_str().expect("proof id");
        assert!(proofs.insert(proof), "duplicate proof id {proof}");
        assert!(
            proof
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        assert!(matches!(
            candidate["disposition"].as_str(),
            Some(
                "source_bound"
                    | "bootstrap_direct"
                    | "ephemeral_internal"
                    | "protocol_sensitive"
                    | "deliberate_reveal"
                    | "non_secret_semantic"
            )
        ));
        let lifecycle = candidate["lifecycle"].as_object().expect("lifecycle");
        assert_eq!(
            lifecycle.keys().map(String::as_str).collect::<HashSet<_>>(),
            HashSet::from(["persistence", "presentation", "resolution"])
        );
        if candidate["disposition"] == "non_secret_semantic" {
            assert!(
                !candidate["rust_type"]
                    .as_str()
                    .expect("Rust type")
                    .contains("SecretString")
            );
        }
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(candidate["source"].as_str().expect("source"));
        let text = std::fs::read_to_string(source).expect("candidate source exists");
        assert!(
            text.contains(candidate["field"].as_str().expect("field")),
            "source no longer contains {id}"
        );
    }
}

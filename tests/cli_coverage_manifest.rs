use std::collections::{BTreeSet, HashSet};

use clap::CommandFactory;
use serde_json::Value;
use thinclaw::cli::Cli;

fn canonical_leaves(command: &clap::Command, prefix: &[String], output: &mut Vec<String>) {
    for child in command
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
    {
        let mut path = prefix.to_vec();
        path.push(child.get_name().to_string());
        if child
            .get_subcommands()
            .any(|grandchild| !grandchild.is_hide_set())
        {
            canonical_leaves(child, &path, output);
        } else {
            output.push(path.join(" "));
        }
    }
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("entity array")
        .iter()
        .map(|entry| entry.as_str().expect("string entity").to_string())
        .collect()
}

#[test]
fn manifest_entities_match_generated_runtime_metadata() {
    let manifest: Value = serde_json::from_str(include_str!("fixtures/cli_contract_manifest.json"))
        .expect("CLI contract manifest");
    let entities = &manifest["entities"];

    let inventory = strings(&entities["inventory"]);
    assert_eq!(inventory.len(), 95);
    assert_eq!(inventory.first().map(String::as_str), Some("INV-01"));
    assert_eq!(inventory.last().map(String::as_str), Some("INV-95"));

    let tools = thinclaw_tools::STATIC_TOOL_CATALOG
        .iter()
        .map(|tool| format!("tool:{}", tool.name))
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["static_tools"]), tools);
    assert_eq!(tools.len(), 124);

    let channels = thinclaw::channels::catalog::static_channel_catalog()
        .into_iter()
        .map(|channel| format!("channel:{}:{}", channel.id, channel.variant))
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["channels"]), channels);

    let setup_steps = thinclaw_app::ALL_SETUP_WIZARD_STEP_IDS
        .iter()
        .map(|step| format!("setup-step:{step:?}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["setup_steps"]), setup_steps);
    assert_eq!(setup_steps.len(), 27);
    let setup_phases = thinclaw_app::ALL_SETUP_WIZARD_PHASE_IDS
        .iter()
        .map(|phase| format!("setup-phase:{phase:?}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["setup_phases"]), setup_phases);
    assert_eq!(setup_phases.len(), 10);

    let mut leaves = Vec::new();
    canonical_leaves(&Cli::command(), &[], &mut leaves);
    let expected_leaves = leaves
        .into_iter()
        .map(|path| {
            thinclaw_types::canonical_cli_leaf_effect(&path)
                .unwrap_or_else(|error| panic!("{error}"));
            format!("leaf:{path}")
        })
        .collect::<BTreeSet<_>>();
    let manifest_leaves = entities["canonical_leaves"]
        .as_array()
        .expect("leaf entities")
        .iter()
        .map(|entry| entry["id"].as_str().expect("leaf id").to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(manifest_leaves, expected_leaves);

    let process: Value =
        serde_json::from_str(include_str!("fixtures/process_launch_manifest.json"))
            .expect("process manifest");
    let process_ids = process["launches"]
        .as_array()
        .expect("launches")
        .iter()
        .map(|launch| format!("process:{}", launch["id"].as_str().expect("id")))
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["process_launches"]), process_ids);

    let credentials: Value =
        serde_json::from_str(include_str!("fixtures/credential_consumer_manifest.json"))
            .expect("credential manifest");
    let credential_ids = credentials["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .map(|candidate| {
            format!(
                "credential:{}",
                candidate["id"].as_str().expect("candidate id")
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(strings(&entities["credential_consumers"]), credential_ids);
}

#[test]
fn every_entity_has_one_exact_discoverable_proof_claim() {
    let manifest: Value = serde_json::from_str(include_str!("fixtures/cli_contract_manifest.json"))
        .expect("CLI contract manifest");
    let mut entities = HashSet::new();
    for entries in manifest["entities"].as_object().expect("entities").values() {
        for entry in entries.as_array().expect("entity group") {
            let id = entry
                .as_str()
                .or_else(|| entry["id"].as_str())
                .expect("entity id");
            assert!(entities.insert(id), "duplicate entity {id}");
        }
    }
    let mut covered = HashSet::new();
    let mut proof_ids = HashSet::new();
    for proof in manifest["proofs"].as_array().expect("proofs") {
        let proof_id = proof["proof_id"].as_str().expect("proof id");
        assert!(proof_ids.insert(proof_id));
        assert!(
            proof_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        assert!(!proof["target"].as_str().expect("target").is_empty());
        let test = proof["test"].as_str().expect("test");
        assert!(!test.is_empty() && !test.contains('*'));
        for entity in proof["entities"].as_array().expect("proof entities") {
            let entity = entity.as_str().expect("proof entity");
            assert!(entities.contains(entity), "unknown entity {entity}");
            assert!(covered.insert(entity), "duplicate proof claim {entity}");
        }
    }
    assert_eq!(covered, entities);
}

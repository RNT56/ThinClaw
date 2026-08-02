use std::collections::HashSet;

use clap::CommandFactory;
use thinclaw::cli::Cli;

fn leaves(command: &clap::Command, prefix: &[String], output: &mut Vec<String>) {
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
            leaves(child, &path, output);
        } else {
            output.push(path.join(" "));
        }
    }
}

#[test]
fn canonical_leaf_inventory_is_unique_and_complete() {
    let command = Cli::command();
    let mut paths = Vec::new();
    leaves(&command, &[], &mut paths);
    assert!(
        paths.len() >= 100,
        "unexpectedly small canonical CLI: {}",
        paths.len()
    );
    assert_eq!(paths.len(), paths.iter().collect::<HashSet<_>>().len());
    assert!(
        paths
            .iter()
            .any(|path| path == "setup" || path.starts_with("setup "))
    );
    assert!(paths.iter().any(|path| path.starts_with("runtime web ")));
    assert!(paths.iter().all(|path| !path.starts_with("onboard")));
}

#[test]
fn inventory_contract_is_exact() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cli_contract_manifest.json"))
            .expect("CLI contract manifest");
    let inventory = manifest["entities"]["inventory"]
        .as_array()
        .expect("inventory entities");
    assert_eq!(inventory.len(), 95);
    assert_eq!(
        inventory.first().and_then(|value| value.as_str()),
        Some("INV-01")
    );
    assert_eq!(
        inventory.last().and_then(|value| value.as_str()),
        Some("INV-95")
    );
}

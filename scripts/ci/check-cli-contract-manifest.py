#!/usr/bin/env python3
"""Generate and verify the no-wildcard CLI overhaul proof manifest."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/fixtures/cli_contract_manifest.json"
PROOF_ID = re.compile(r"^[A-Za-z0-9_-]+$")


def run_surface_export() -> dict[str, object]:
    environment = os.environ.copy()
    environment.setdefault("CARGO_INCREMENTAL", "0")
    completed = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "--example", "export-cli-surface"],
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"CLI surface export failed:\n{completed.stderr}")
    return json.loads(completed.stdout)


def load_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def expected_manifest() -> dict[str, object]:
    surface = run_surface_export()
    process = load_json(ROOT / "tests/fixtures/process_launch_manifest.json")
    credentials = load_json(ROOT / "tests/fixtures/credential_consumer_manifest.json")

    leaves = []
    for leaf in surface["canonical_leaves"]:
        leaves.append(
            {
                "id": f"leaf:{leaf['path']}",
                "path": leaf["path"],
                "effect": leaf["effect"],
            }
        )
    leaves.sort(key=lambda item: item["id"])
    tools = sorted(f"tool:{item['name']}" for item in surface["static_tools"])
    dynamic = sorted(f"dynamic-origin:{origin}" for origin in surface["dynamic_tool_origins"])
    channels = sorted(
        f"channel:{item['id']}:{item['variant']}" for item in surface["channels"]
    )
    setup_steps = sorted(f"setup-step:{item['id']}" for item in surface["setup_steps"])
    setup_phases = [f"setup-phase:{item['id']}" for item in surface["setup_phases"]]
    setup_profiles = [
        "setup-profile:balanced",
        "setup-profile:local-private",
        "setup-profile:builder-coding",
        "setup-profile:channel-first",
        "setup-profile:remote",
        "setup-profile:pi-os-lite-64",
        "setup-profile:custom",
    ]
    readiness_profiles = [
        "readiness-profile:server",
        "readiness-profile:remote",
        "readiness-profile:desktop",
        "readiness-profile:pi-os-lite-64",
        "readiness-profile:all-features",
    ]
    build_profiles = [
        "build-profile:light-default",
        "build-profile:edge",
        "build-profile:full",
        "build-profile:all-features",
        "build-profile:desktop",
        "build-profile:minimal-libsql",
        "build-profile:minimal-postgres",
        "build-profile:compat-repl",
        "build-profile:compat-web-gateway",
        "build-profile:compat-timezones",
        "build-profile:compat-all-empty",
        "build-profile:delta-browser",
        "build-profile:delta-nostr",
        "build-profile:delta-bundled-wasm",
    ]
    process_entities = sorted(f"process:{item['id']}" for item in process["launches"])
    credential_entities = sorted(
        f"credential:{item['id']}" for item in credentials["candidates"]
    )
    inventory = [f"INV-{index:02d}" for index in range(1, 96)]

    groups = {
        "inventory": inventory,
        "canonical_leaves": leaves,
        "static_tools": tools,
        "dynamic_origins": dynamic,
        "channels": channels,
        "setup_steps": setup_steps,
        "setup_phases": setup_phases,
        "setup_profiles": setup_profiles,
        "readiness_profiles": readiness_profiles,
        "build_profiles": build_profiles,
        "process_launches": process_entities,
        "credential_consumers": credential_entities,
    }
    leaf_ids = [item["id"] for item in leaves]
    proofs = [
        {
            "proof_id": "CLI-inventory-contract",
            "target": "cli_contract",
            "test": "inventory_contract_is_exact",
            "entities": inventory,
        },
        {
            "proof_id": "CLI-canonical-leaf-contract",
            "target": "cli_contract",
            "test": "canonical_leaf_inventory_is_unique_and_complete",
            "entities": leaf_ids + setup_profiles + readiness_profiles + build_profiles,
        },
        {
            "proof_id": "CLI-tool-registry-contract",
            "target": "tool_registry_contract",
            "test": "static_catalog_identity_and_seal_contract",
            "entities": tools,
        },
        {
            "proof_id": "CLI-dynamic-origin-contract",
            "target": "tool_registry_contract",
            "test": "dynamic_origin_vocabulary_is_closed",
            "entities": dynamic,
        },
        {
            "proof_id": "CLI-channel-catalog-contract",
            "target": "cli_coverage_manifest",
            "test": "manifest_entities_match_generated_runtime_metadata",
            "entities": channels,
        },
        {
            "proof_id": "CLI-setup-navigation-contract",
            "target": "cli_setup_contract",
            "test": "legacy_steps_have_exact_target_sections",
            "entities": setup_steps + setup_phases,
        },
        {
            "proof_id": "CLI-process-launch-contract",
            "target": "process_launch_contract",
            "test": "all_manifest_entries_have_enforceable_descriptor_contract",
            "entities": process_entities,
        },
        {
            "proof_id": "CLI-credential-lifecycle-contract",
            "target": "cli_safety_contract",
            "test": "credential_consumer_lifecycles_are_complete_and_secret_safe",
            "entities": credential_entities,
        },
    ]
    return {"schema_version": 1, "entities": groups, "proofs": proofs}


def all_entity_ids(manifest: dict[str, object]) -> set[str]:
    result: set[str] = set()
    for name, entries in manifest["entities"].items():
        for entry in entries:
            identity = entry["id"] if isinstance(entry, dict) else entry
            if identity in result:
                raise ValueError(f"duplicate entity id {identity} in {name}")
            result.add(identity)
    return result


def validate_proofs(manifest: dict[str, object]) -> list[str]:
    problems: list[str] = []
    entities = all_entity_ids(manifest)
    covered: set[str] = set()
    proofs: set[str] = set()
    for proof in manifest["proofs"]:
        proof_id = proof["proof_id"]
        if not PROOF_ID.fullmatch(proof_id) or proof_id in proofs:
            problems.append(f"invalid or duplicate proof id: {proof_id}")
        proofs.add(proof_id)
        if not proof["target"] or not proof["test"] or "*" in proof["test"]:
            problems.append(f"proof {proof_id} lacks an exact test target/name")
        for entity in proof["entities"]:
            if entity not in entities:
                problems.append(f"proof {proof_id} references unknown entity {entity}")
            if entity in covered:
                problems.append(f"entity is claimed by multiple proofs: {entity}")
            covered.add(entity)
    missing = sorted(entities - covered)
    if missing:
        problems.append(f"unproved entities: {', '.join(missing[:20])}")
    return problems


def discover_tests(manifest: dict[str, object]) -> list[str]:
    problems: list[str] = []
    by_target: dict[str, set[str]] = {}
    for proof in manifest["proofs"]:
        by_target.setdefault(proof["target"], set()).add(proof["test"])
    for target, expected in sorted(by_target.items()):
        completed = subprocess.run(
            [
                "cargo",
                "test",
                "--locked",
                "--test",
                target,
                "--",
                "--list",
                "--format",
                "terse",
            ],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            problems.append(f"could not discover target {target}: {completed.stderr.strip()}")
            continue
        discovered = {
            line.split(":", 1)[0]
            for line in completed.stdout.splitlines()
            if line.endswith(": test")
        }
        for test in sorted(expected - discovered):
            problems.append(f"proof test is not discoverable: {target}::{test}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--skip-discovery", action="store_true")
    args = parser.parse_args()
    try:
        expected = expected_manifest()
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    problems = validate_proofs(expected)
    if args.write:
        MANIFEST.write_text(json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    elif not MANIFEST.exists():
        problems.append(f"missing {MANIFEST.relative_to(ROOT)}")
    else:
        current = load_json(MANIFEST)
        if current != expected:
            problems.append(
                "CLI contract manifest drift (run scripts/ci/check-cli-contract-manifest.py --write)"
            )
    if not args.skip_discovery and not problems:
        problems.extend(discover_tests(expected))
    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    print(
        "CLI contract manifest verified: "
        f"{len(all_entity_ids(expected))} exact entities, {len(expected['proofs'])} proofs"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

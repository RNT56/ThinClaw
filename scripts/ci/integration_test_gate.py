#!/usr/bin/env python3
"""Execute every hermetic integration target and audit external classifications."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "scripts/ci/integration-tests.json"


class ContractError(RuntimeError):
    pass


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot read integration manifest {path}: {error}") from error
    if not isinstance(value, dict):
        raise ContractError("integration manifest root must be an object")
    return value


def discover_targets(tests_dir: Path) -> set[str]:
    return {path.stem for path in tests_dir.glob("*.rs") if path.is_file()}


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list) or not all(isinstance(item, str) and item for item in value):
        raise ContractError(f"{field} must be a list of non-empty strings")
    if len(value) != len(set(value)):
        raise ContractError(f"{field} contains duplicate targets")
    return value


def _classifications(value: Any, field: str) -> list[dict[str, str]]:
    if not isinstance(value, list):
        raise ContractError(f"{field} must be a list")
    result: list[dict[str, str]] = []
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            raise ContractError(f"{field}[{index}] must be an object")
        normalized: dict[str, str] = {}
        for key in ("target", "reason", "workflow", "evidence"):
            field_value = item.get(key)
            if not isinstance(field_value, str) or not field_value.strip():
                raise ContractError(f"{field}[{index}].{key} must be a non-empty string")
            normalized[key] = field_value
        result.append(normalized)
    targets = [item["target"] for item in result]
    if len(targets) != len(set(targets)):
        raise ContractError(f"{field} contains duplicate targets")
    return result


def validate_manifest(
    manifest: dict[str, Any],
    *,
    repo_root: Path = REPO_ROOT,
    tests_dir: Path | None = None,
) -> list[str]:
    if manifest.get("schema_version") != 1:
        raise ContractError("integration manifest schema_version must be 1")
    features = manifest.get("features")
    if not isinstance(features, str) or not features.strip():
        raise ContractError("integration manifest features must be a non-empty string")

    hermetic = _string_list(manifest.get("hermetic"), "hermetic")
    external = _classifications(manifest.get("external"), "external")
    ignored_external = _classifications(
        manifest.get("ignored_external", []), "ignored_external"
    )

    external_targets = [item["target"] for item in external]
    overlap = sorted(set(hermetic) & set(external_targets))
    if overlap:
        raise ContractError(f"targets cannot be both hermetic and external: {overlap}")

    discovered = discover_targets(tests_dir or repo_root / "tests")
    classified = set(hermetic) | set(external_targets)
    missing = sorted(discovered - classified)
    stale = sorted(classified - discovered)
    if missing or stale:
        raise ContractError(
            f"integration target classification drift: missing={missing}, stale={stale}"
        )

    for field, entries in (("external", external), ("ignored_external", ignored_external)):
        for item in entries:
            if item["target"] not in discovered:
                raise ContractError(f"{field} target does not exist: {item['target']}")
            workflow = repo_root / item["workflow"]
            try:
                workflow_text = workflow.read_text(encoding="utf-8")
            except OSError as error:
                raise ContractError(f"cannot read classified workflow {workflow}: {error}") from error
            if item["evidence"] not in workflow_text:
                raise ContractError(
                    f"{field} target {item['target']} lacks workflow evidence "
                    f"{item['evidence']!r} in {item['workflow']}"
                )

    return hermetic


def cargo_command(manifest: dict[str, Any], hermetic: list[str]) -> list[str]:
    command = [
        "cargo",
        "test",
        "--locked",
        "--workspace",
        "--features",
        str(manifest["features"]),
    ]
    for target in hermetic:
        command.extend(("--test", target))
    command.extend(("--", "--nocapture", "--test-threads=2"))
    return command


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("check", "run", "print-command"))
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = load_manifest(args.manifest)
    hermetic = validate_manifest(manifest)
    command = cargo_command(manifest, hermetic)
    if args.mode == "check":
        print(
            f"Integration target contract verified: {len(hermetic)} hermetic, "
            f"{len(manifest['external'])} externally provisioned"
        )
        return 0
    if args.mode == "print-command":
        print(" ".join(command))
        return 0
    subprocess.run(command, cwd=REPO_ROOT, check=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ContractError as error:
        raise SystemExit(f"integration test contract error: {error}") from error

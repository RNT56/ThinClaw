#!/usr/bin/env python3
"""Fail-closed helpers for the Rust -> OpenAPI -> Swift generated contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
ROOT_OPENAPI = REPO_ROOT / "clients/openapi/thinclaw-gateway.openapi.json"
IOS_ROOT = REPO_ROOT / "apps/ios"
VENDORED_OPENAPI = IOS_ROOT / "Packages/ThinClawAPI/openapi/openapi.json"
GENERATOR_CONFIG = IOS_ROOT / "Packages/ThinClawAPI/openapi/openapi-generator-config.yaml"
GENERATION_MANIFEST = IOS_ROOT / "Packages/ThinClawAPI/openapi/generated-client-manifest.json"
GENERATED_DIR = IOS_ROOT / "Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"
MISE_CONFIG = IOS_ROOT / "mise.toml"
GENERATOR_KEY = "spm:apple/swift-openapi-generator"
REQUIRED_SWIFT_OUTPUTS = ("Client.swift", "Types.swift")


class ContractError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    try:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        raise ContractError(f"required generated-contract file is missing: {path} ({error})") from error
    return digest


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot read JSON contract {path}: {error}") from error


def check_rust_snapshot(committed: Path, generated: Path) -> None:
    if read_json(committed) != read_json(generated):
        raise ContractError(
            "Rust-generated OpenAPI differs from the committed snapshot; regenerate the snapshot"
        )


def generator_version(mise_config: Path = MISE_CONFIG) -> str:
    try:
        text = mise_config.read_text(encoding="utf-8")
    except OSError as error:
        raise ContractError(f"cannot read pinned Swift OpenAPI generator from {mise_config}: {error}") from error
    match = re.search(
        rf'^\s*"{re.escape(GENERATOR_KEY)}"\s*=\s*"([^"]+)"\s*$',
        text,
        flags=re.MULTILINE,
    )
    if match is None:
        raise ContractError(
            f"cannot read pinned Swift OpenAPI generator from {mise_config}: missing {GENERATOR_KEY}"
        )
    version = match.group(1)
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ContractError(
            f"{GENERATOR_KEY} must be pinned to an exact semantic version, got {version!r}"
        )
    return version


def swift_outputs(generated_dir: Path) -> dict[str, str]:
    for required in REQUIRED_SWIFT_OUTPUTS:
        path = generated_dir / required
        if not path.is_file() or path.stat().st_size == 0:
            raise ContractError(f"required generated Swift output is missing or empty: {path}")
    outputs = {
        path.name: sha256_file(path)
        for path in sorted(generated_dir.glob("*.swift"))
        if path.is_file()
    }
    if set(outputs) != set(REQUIRED_SWIFT_OUTPUTS):
        raise ContractError(
            "generated Swift output set drifted: "
            f"expected={list(REQUIRED_SWIFT_OUTPUTS)}, actual={sorted(outputs)}"
        )
    return outputs


def build_swift_manifest(
    *,
    root_openapi: Path = ROOT_OPENAPI,
    vendored_openapi: Path = VENDORED_OPENAPI,
    generated_dir: Path = GENERATED_DIR,
    generator_config: Path = GENERATOR_CONFIG,
    mise_config: Path = MISE_CONFIG,
) -> dict[str, Any]:
    source_hash = sha256_file(root_openapi)
    vendored_hash = sha256_file(vendored_openapi)
    if source_hash != vendored_hash:
        raise ContractError(
            "vendored OpenAPI differs from the authoritative committed OpenAPI snapshot"
        )
    return {
        "schema_version": 1,
        "generator": "apple/swift-openapi-generator",
        "generator_version": generator_version(mise_config),
        "source_openapi_sha256": source_hash,
        "vendored_openapi_sha256": vendored_hash,
        "generator_config_sha256": sha256_file(generator_config),
        "outputs": swift_outputs(generated_dir),
    }


def write_swift_manifest(path: Path = GENERATION_MANIFEST, **kwargs: Any) -> None:
    manifest = build_swift_manifest(**kwargs)
    path.parent.mkdir(parents=True, exist_ok=True)
    rendered = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as handle:
        handle.write(rendered)
        temporary = Path(handle.name)
    os.replace(temporary, path)


def check_swift_contract(path: Path = GENERATION_MANIFEST, **kwargs: Any) -> None:
    committed = read_json(path)
    expected = build_swift_manifest(**kwargs)
    if committed != expected:
        raise ContractError(
            "OpenAPI-to-Swift generation manifest drifted; run apps/ios/scripts/generate-api.sh"
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    check_json = subparsers.add_parser("check-json")
    check_json.add_argument("--committed", type=Path, required=True)
    check_json.add_argument("--generated", type=Path, required=True)
    subparsers.add_parser("check-swift")
    subparsers.add_parser("write-swift-manifest")
    subparsers.add_parser("generator-version")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.command == "check-json":
        check_rust_snapshot(args.committed, args.generated)
    elif args.command == "check-swift":
        check_swift_contract()
    elif args.command == "write-swift-manifest":
        write_swift_manifest()
    elif args.command == "generator-version":
        print(generator_version())
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ContractError as error:
        raise SystemExit(f"generated contract error: {error}") from error

#!/usr/bin/env python3
"""Validate ThinClaw's root-only release automation contract."""

from __future__ import annotations

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ACTION_SHA = "5c625bfb5d1ff62eadeeb3772007f7f66fdcf071"


def cargo_version() -> str:
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    package = manifest.split("[package]", 1)[1]
    match = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE)
    if match is None:
        raise SystemExit("root Cargo.toml package version is missing")
    return match.group(1)


def locked_package_version(path: Path, name: str) -> str:
    content = path.read_text(encoding="utf-8")
    for package in content.split("[[package]]")[1:]:
        package_name = re.search(r'^name\s*=\s*"([^"]+)"', package, re.MULTILINE)
        if package_name is None or package_name.group(1) != name:
            continue
        version = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE)
        if version is None:
            raise SystemExit(f"{path.relative_to(ROOT)} package {name} has no version")
        return version.group(1)
    raise SystemExit(f"{path.relative_to(ROOT)} has no {name} package")


def main() -> int:
    config = json.loads(
        (ROOT / "release-please-config.json").read_text(encoding="utf-8")
    )
    manifest = json.loads(
        (ROOT / ".release-please-manifest.json").read_text(encoding="utf-8")
    )
    workflow = (ROOT / ".github/workflows/release-please.yml").read_text(
        encoding="utf-8"
    )

    packages = config.get("packages", {})
    if set(packages) != {"."}:
        raise SystemExit("Release Please must manage only the root package")

    root = packages["."]
    expected = {
        "release-type": "rust",
        "package-name": "thinclaw",
        "changelog-path": "CHANGELOG.md",
        "include-component-in-tag": False,
        "include-v-in-tag": True,
        "extra-files": [
            {
                "type": "toml",
                "path": "apps/desktop/backend/Cargo.lock",
                "jsonpath": '$.package[?(@.name.value=="thinclaw")].version',
            }
        ],
    }
    mismatches = [
        f"{key}={root.get(key)!r} (expected {value!r})"
        for key, value in expected.items()
        if root.get(key) != value
    ]
    if mismatches:
        raise SystemExit("invalid root release policy: " + ", ".join(mismatches))

    version = cargo_version()
    if manifest != {".": version}:
        raise SystemExit(
            f"release manifest {manifest!r} must match root Cargo version {version}"
        )

    for lockfile in [ROOT / "Cargo.lock", ROOT / "apps/desktop/backend/Cargo.lock"]:
        locked_version = locked_package_version(lockfile, "thinclaw")
        if locked_version != version:
            raise SystemExit(
                f"{lockfile.relative_to(ROOT)} thinclaw version {locked_version} "
                f"must match root Cargo version {version}"
            )

    required_workflow_fragments = [
        f"googleapis/release-please-action@{ACTION_SHA}",
        "config-file: release-please-config.json",
        "manifest-file: .release-please-manifest.json",
        "if: steps.release.outputs.prs_created == 'true'",
        "RELEASE_PR: ${{ steps.release.outputs.pr }}",
        'gh workflow run ci.yml --repo "$GITHUB_REPOSITORY"',
        'gh workflow run release.yml --repo "$GITHUB_REPOSITORY"',
        "RELEASE_TAG: ${{ steps.release.outputs.tag_name }}",
    ]
    missing = [item for item in required_workflow_fragments if item not in workflow]
    if missing:
        raise SystemExit("release workflow is missing: " + ", ".join(missing))

    print(
        f"Release automation: root thinclaw v{version}, immutable action, "
        "protected CI and artifact dispatch"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

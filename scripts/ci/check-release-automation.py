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


def cargo_package_version(path: Path) -> str:
    manifest = path.read_text(encoding="utf-8")
    try:
        package = manifest.split("[package]", 1)[1].split("\n[", 1)[0]
    except IndexError as error:
        raise SystemExit(f"{path.relative_to(ROOT)} has no [package] table") from error
    match = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE)
    if match is None:
        raise SystemExit(f"{path.relative_to(ROOT)} package version is missing")
    return match.group(1)


def validate_dist_binaries() -> tuple[str, ...]:
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    package = manifest.split("[package]", 1)[1].split("\n[", 1)[0]
    if not re.search(r"^autobins\s*=\s*false\s*$", package, re.MULTILINE):
        raise SystemExit(
            "root Cargo.toml must disable automatic binary discovery so developer "
            "utilities cannot silently enter release artifacts"
        )

    binaries = tuple(
        sorted(
            match.group(1)
            for section in manifest.split("[[bin]]")[1:]
            if (
                match := re.search(
                    r'^name\s*=\s*"([^"]+)"', section.split("[[", 1)[0], re.MULTILINE
                )
            )
        )
    )
    if not binaries:
        raise SystemExit("root Cargo.toml declares no release binaries")

    wix = (ROOT / "wix/main.wxs").read_text(encoding="utf-8")
    wix_binaries = tuple(sorted(set(re.findall(r"Name='([^']+)\.exe'", wix))))
    if wix_binaries != binaries:
        raise SystemExit(
            "wix/main.wxs release binaries are stale: "
            f"{wix_binaries!r} (expected {binaries!r}); run "
            "`dist generate --mode=msi` with the pinned cargo-dist version"
        )

    return binaries


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
    artifact_workflow = (ROOT / ".github/workflows/release.yml").read_text(
        encoding="utf-8"
    )
    desktop_release_contract = artifact_workflow + (
        ROOT / "apps/desktop/scripts/collect_macos_release_artifacts.sh"
    ).read_text(encoding="utf-8")

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
                "type": "json",
                "path": "apps/desktop/package.json",
                "jsonpath": "$.version",
            },
            {
                "type": "json",
                "path": "apps/desktop/package-lock.json",
                "jsonpath": "$.version",
            },
            {
                "type": "json",
                "path": "apps/desktop/package-lock.json",
                "jsonpath": "$['packages'][''].version",
            },
            {
                "type": "toml",
                "path": "apps/desktop/backend/Cargo.toml",
                "jsonpath": "$.package.version",
            },
            {
                "type": "toml",
                "path": "apps/desktop/backend/Cargo.lock",
                "jsonpath": '$.package[?(@.name.value=="thinclaw-desktop")].version',
            },
            {
                "type": "toml",
                "path": "apps/desktop/backend/Cargo.lock",
                "jsonpath": '$.package[?(@.name.value=="thinclaw")].version',
            },
            {
                "type": "json",
                "path": "apps/desktop/backend/tauri.conf.json",
                "jsonpath": "$.version",
            },
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

    desktop_package = json.loads(
        (ROOT / "apps/desktop/package.json").read_text(encoding="utf-8")
    )
    desktop_package_lock = json.loads(
        (ROOT / "apps/desktop/package-lock.json").read_text(encoding="utf-8")
    )
    desktop_tauri = json.loads(
        (ROOT / "apps/desktop/backend/tauri.conf.json").read_text(encoding="utf-8")
    )
    desktop_versions = {
        "apps/desktop/package.json": desktop_package.get("version"),
        "apps/desktop/package-lock.json": desktop_package_lock.get("version"),
        "apps/desktop/package-lock.json root package": desktop_package_lock
        .get("packages", {})
        .get("", {})
        .get("version"),
        "apps/desktop/backend/Cargo.toml": cargo_package_version(
            ROOT / "apps/desktop/backend/Cargo.toml"
        ),
        "apps/desktop/backend/Cargo.lock": locked_package_version(
            ROOT / "apps/desktop/backend/Cargo.lock", "thinclaw-desktop"
        ),
        "apps/desktop/backend/tauri.conf.json": desktop_tauri.get("version"),
    }
    drifted = {
        path: desktop_version
        for path, desktop_version in desktop_versions.items()
        if desktop_version != version
    }
    if drifted:
        raise SystemExit(
            f"Desktop versions must match root Cargo version {version}: {drifted!r}"
        )

    release_binaries = validate_dist_binaries()

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

    desktop_release_fragments = [
        "build-desktop-macos:",
        "runs-on: macos-15",
        "APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}",
        "APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}",
        "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
        "npm run setup:all",
        "npm run validate:packaging",
        "npm run tauri:build:llamacpp",
        'VERIFY_APPLE_ARTIFACTS: "1"',
        "bash scripts/collect_macos_release_artifacts.sh",
        "flags=.*runtime",
        "context:primary-signature",
        "name: artifacts-desktop-macos",
        "needs.build-desktop-macos.result == 'success'",
    ]
    missing_desktop = [
        item for item in desktop_release_fragments if item not in desktop_release_contract
    ]
    if missing_desktop:
        raise SystemExit(
            "Desktop artifact workflow is missing: " + ", ".join(missing_desktop)
        )

    print(
        f"Release automation: root thinclaw v{version}, immutable action, "
        f"protected CI, synchronized Desktop versioning, and artifact dispatch, "
        f"binaries {', '.join(release_binaries)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

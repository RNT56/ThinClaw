#!/usr/bin/env python3
"""Validate ThinClaw's root-only release automation contract."""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ACTION_SHA = "45996ed1f6d02564a971a2fa1b5860e934307cf7"


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


def validate_google_oauth_build_configurator() -> None:
    script = ROOT / "scripts/ci/configure-google-oauth-build.sh"

    def run(client_id: str | None, client_secret: str | None):
        environment = os.environ.copy()
        for name in (
            "THINCLAW_RELEASE_GOOGLE_CLIENT_ID",
            "THINCLAW_RELEASE_GOOGLE_CLIENT_SECRET",
            "THINCLAW_GOOGLE_CLIENT_ID",
            "THINCLAW_GOOGLE_CLIENT_SECRET",
        ):
            environment.pop(name, None)
        if client_id is not None:
            environment["THINCLAW_RELEASE_GOOGLE_CLIENT_ID"] = client_id
        if client_secret is not None:
            environment["THINCLAW_RELEASE_GOOGLE_CLIENT_SECRET"] = client_secret

        with tempfile.NamedTemporaryFile() as github_env:
            environment["GITHUB_ENV"] = github_env.name
            result = subprocess.run(
                ["bash", str(script)],
                cwd=ROOT,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            github_env.seek(0)
            exported = github_env.read().decode("utf-8")
        return result, exported

    byok, byok_exports = run(None, None)
    if byok.returncode != 0 or byok_exports:
        raise SystemExit("Google OAuth release configurator must allow BYOK builds")

    for client_id, client_secret in (("client-id", None), (None, "client-secret")):
        partial, partial_exports = run(client_id, client_secret)
        if partial.returncode == 0 or partial_exports:
            raise SystemExit(
                "Google OAuth release configurator must reject partial credential pairs"
            )

    complete, complete_exports = run("client-id", "client-secret")
    expected_exports = (
        "THINCLAW_GOOGLE_CLIENT_ID=client-id\n"
        "THINCLAW_GOOGLE_CLIENT_SECRET=client-secret\n"
    )
    if complete.returncode != 0 or complete_exports != expected_exports:
        raise SystemExit(
            "Google OAuth release configurator did not export a complete build pair"
        )
    if "client-id" in complete.stdout or "client-secret" in complete.stdout:
        raise SystemExit("Google OAuth release configurator prints credential values")

    whitespace, whitespace_exports = run("client id", "client-secret")
    if whitespace.returncode == 0 or whitespace_exports:
        raise SystemExit(
            "Google OAuth release configurator must reject credential whitespace"
        )


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
    credential_workflow = (
        ROOT / ".github/workflows/release-credential-gate.yml"
    ).read_text(encoding="utf-8")
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
    if config.get("draft") is not True:
        raise SystemExit("Release Please must create draft releases for staged publication")
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
            {
                "type": "json",
                "path": "release/extension-registry.json",
                "jsonpath": "$.release_version",
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
    extension_registry = json.loads(
        (ROOT / "release/extension-registry.json").read_text(encoding="utf-8")
    )
    if extension_registry.get("release_version") != version:
        raise SystemExit(
            "extension registry release_version must match root Cargo version"
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
    validate_google_oauth_build_configurator()

    required_workflow_fragments = [
        f"googleapis/release-please-action@{ACTION_SHA}",
        "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349",
        "app-id: ${{ secrets.RELEASE_APP_ID }}",
        "private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}",
        "token: ${{ steps.app-token.outputs.token }}",
        "config-file: release-please-config.json",
        "manifest-file: .release-please-manifest.json",
        "if: steps.release.outputs.prs_created == 'true'",
        "RELEASE_PR: ${{ steps.release.outputs.pr }}",
        "continue-on-error: true",
        "recover_tag:",
        "no exact draft release was recoverable",
        'gh workflow run release.yml --repo "$GITHUB_REPOSITORY"',
        "-f mode=publish",
        "-f promote_latest=true",
        "prepare-extension-registry:",
        "scripts/ci/build_extension_bundles.sh",
        "scripts/ci/extension_registry.py compare-bundles",
        "scripts/ci/extension_registry.py prepare",
        'git push origin "HEAD:$RELEASE_BRANCH"',
    ]
    missing = [item for item in required_workflow_fragments if item not in workflow]
    if missing:
        raise SystemExit("release workflow is missing: " + ", ".join(missing))

    forbidden_workflow_fragments = [
        'gh workflow run ci.yml --repo "$GITHUB_REPOSITORY"',
        "token: ${{ secrets.GITHUB_TOKEN }}",
    ]
    forbidden = [
        item for item in forbidden_workflow_fragments if item in workflow
    ]
    if forbidden:
        raise SystemExit(
            "release workflow dispatches CI that cannot satisfy the PR gate: "
            + ", ".join(forbidden)
        )

    credential_workflow_fragments = [
        "name: Release Credential Gate",
        "workflow_run:",
        "workflows: [CI]",
        "workflow_dispatch:",
        "pull-requests: read",
        "statuses: write",
        "release-please--branches--main--components--thinclaw",
        "HEAD_REPOSITORY",
        "commits/$HEAD_SHA/pulls",
        "RELEASE_APP_ID: ${{ secrets.RELEASE_APP_ID }}",
        "RELEASE_APP_PRIVATE_KEY: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}",
        "RELEASE_APP_ID must be numeric",
        "RELEASE_APP_PRIVATE_KEY must be a PEM private key",
        "statuses/$HEAD_SHA",
        "context='Release Credentials'",
    ]
    missing_credentials = [
        item
        for item in credential_workflow_fragments
        if item not in credential_workflow
    ]
    if missing_credentials:
        raise SystemExit(
            "release credential gate is missing: "
            + ", ".join(missing_credentials)
        )
    if "actions/checkout" in credential_workflow:
        raise SystemExit(
            "release credential gate must not execute pull-request-controlled code"
        )

    desktop_release_fragments = [
        "build-desktop-macos:",
        "runs-on: macos-15",
        "APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}",
        "APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}",
        "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
        "THINCLAW_DESKTOP_ENGINE: mlx",
        "TAURI_TARGET_TRIPLE: aarch64-apple-darwin",
        'MACOSX_DEPLOYMENT_TARGET: "14.0"',
        "npm run setup:all",
        "npm run validate:packaging",
        "npm run tauri:build:mlx",
        "bash scripts/verify_macos_mlx_bundle.sh backend/target/release/bundle/macos",
        'VERIFY_APPLE_ARTIFACTS: "1"',
        "bash scripts/collect_macos_release_artifacts.sh",
        "flags=.*runtime",
        "context:primary-signature",
        "name: artifacts-desktop-macos",
        "if: ${{ github.event_name == 'workflow_call' }}",
    ]
    missing_desktop = [
        item for item in desktop_release_fragments if item not in desktop_release_contract
    ]
    if missing_desktop:
        raise SystemExit(
            "Desktop artifact workflow is missing: " + ", ".join(missing_desktop)
        )

    staged_release_fragments = [
        "mode:",
        "backfill",
        "dry-run",
        "environment: release-production",
        "release/non-apple-assets.json",
        "scripts/ci/release_control.py assemble",
        "scripts/ci/stage_release_assets.sh",
        "push: false",
        "outputs: type=oci",
        "publish-container-images:",
        "publish-release:",
        "promote-stable:",
        "--draft=false --latest=false",
        "refusing to overwrite it",
        "Legacy extension registry policy is grandfathered only for the exact v0.16.0 backfill.",
        ".release-control/scripts/ci/extension_registry.py",
        ".release-control/scripts/ci/build_extension_bundles.sh",
        "compare-bundles",
        "verify-bundles",
    ]
    missing_staging = [
        item for item in staged_release_fragments if item not in artifact_workflow
    ]
    if missing_staging:
        raise SystemExit(
            "staged core release workflow is missing: " + ", ".join(missing_staging)
        )
    forbidden_publication = [
        "--clobber",
        "push: true",
        "--steps=upload --steps=release",
        "needs.build-desktop-macos.result == 'success'",
    ]
    unsafe = [item for item in forbidden_publication if item in artifact_workflow]
    if unsafe:
        raise SystemExit(
            "staged core release workflow contains unsafe early publication: "
            + ", ".join(unsafe)
        )
    if "update-registry-checksums:" in artifact_workflow:
        raise SystemExit(
            "release workflow must not mutate registry metadata after tag creation"
        )

    oauth_release_step = "run: bash scripts/ci/configure-google-oauth-build.sh"
    if artifact_workflow.count(oauth_release_step) != 3:
        raise SystemExit(
            "optional Google OAuth client must be configured in the host, Desktop, "
            "and edge release build jobs"
        )
    for secret_fragment in (
        "THINCLAW_RELEASE_GOOGLE_CLIENT_ID: ${{ secrets.THINCLAW_GOOGLE_CLIENT_ID }}",
        "THINCLAW_RELEASE_GOOGLE_CLIENT_SECRET: ${{ secrets.THINCLAW_GOOGLE_CLIENT_SECRET }}",
    ):
        if artifact_workflow.count(secret_fragment) != 3:
            raise SystemExit(
                "official binary jobs are missing optional Google OAuth secret wiring: "
                + secret_fragment
            )
    for gate_fragment in (
        "THINCLAW_GOOGLE_CLIENT_ID: ${{ secrets.THINCLAW_GOOGLE_CLIENT_ID }}",
        "THINCLAW_GOOGLE_CLIENT_SECRET: ${{ secrets.THINCLAW_GOOGLE_CLIENT_SECRET }}",
        "Optional ThinClaw Google OAuth credentials must both be configured or both be absent",
    ):
        if gate_fragment not in credential_workflow:
            raise SystemExit(
                "release credential gate is missing optional Google OAuth validation: "
                + gate_fragment
            )

    print(
        f"Release automation: root thinclaw v{version}, immutable action, "
        f"GitHub-App PR automation, recoverable draft staging, "
        f"synchronized Desktop versioning, optional official Google OAuth client, "
        f"atomic non-Apple publication and immutable artifact dispatch, "
        f"binaries {', '.join(release_binaries)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

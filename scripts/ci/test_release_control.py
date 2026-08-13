#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "release_control", ROOT / "scripts/ci/release_control.py"
)
assert SPEC is not None and SPEC.loader is not None
release_control = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_control)


class ReleaseControlTests(unittest.TestCase):
    def test_plan_rejects_version_drift(self) -> None:
        with self.assertRaises(release_control.ReleaseContractError):
            release_control.plan("v0.16.1", "publish", "0.16.0", False)

    def test_prerelease_cannot_promote_latest(self) -> None:
        with self.assertRaises(release_control.ReleaseContractError):
            release_control.plan("v0.16.1-rc.1", "publish", "0.16.1-rc.1", True)

    def test_backfill_is_explicit_and_publishable(self) -> None:
        result = release_control.plan("v0.16.0", "backfill", "0.16.0", False)
        self.assertTrue(result["publishing"])
        self.assertTrue(result["backfill"])
        self.assertFalse(result["promote_latest"])

    def test_asset_contract_rejects_deferred_apple_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "required_exact": ["core.tar.gz"],
                        "required_globs": [],
                        "forbidden_globs": ["*.dmg"],
                    }
                ),
                encoding="utf-8",
            )
            (root / "core.tar.gz").write_bytes(b"core")
            (root / "desktop.dmg").write_bytes(b"deferred")
            with self.assertRaises(release_control.ReleaseContractError):
                release_control.validate_assets(root, contract)

    def test_asset_contract_accepts_complete_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "required_exact": ["core.tar.gz"],
                        "required_globs": [{"pattern": "plugin-*.tar.gz", "minimum": 2}],
                        "forbidden_globs": ["*.dmg"],
                    }
                ),
                encoding="utf-8",
            )
            for name in ("core.tar.gz", "plugin-a.tar.gz", "plugin-b.tar.gz"):
                (root / name).write_bytes(name.encode())
            result = release_control.validate_assets(root, contract)
            self.assertEqual(result["asset_count"], 4)  # contract.json is also present

    def test_assembly_emits_only_the_non_apple_contract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            output = root / "output"
            source.mkdir()
            for target in release_control.LINUX_TARGETS:
                (source / f"thinclaw-{target}.tar.gz").write_bytes(target.encode())
                (source / f"thinclaw-edge-{target}.tar.gz").write_bytes(
                    f"edge-{target}".encode()
                )
            (source / "thinclaw-x86_64-pc-windows-msvc.tar.gz").write_bytes(b"win")
            (source / "thinclaw-x86_64-pc-windows-msvc.msi").write_bytes(b"msi")
            (source / "thinclaw-installer.sh").write_bytes(b"#!/bin/sh\n")
            (source / "thinclaw-installer.ps1").write_bytes(b"installer")
            (source / "notarized-desktop.dmg").write_bytes(b"deferred")
            for index in range(28):
                (source / f"extension-{index:02}-wasm32-wasip2.tar.gz").write_bytes(
                    str(index).encode()
                )

            result = release_control.assemble(
                source,
                output,
                "v0.16.1",
                ROOT / "release/non-apple-assets.json",
            )
            self.assertGreaterEqual(result["asset_count"], 42)
            self.assertFalse((output / "notarized-desktop.dmg").exists())
            self.assertTrue((output / "release-inventory.json").exists())

            environment = os.environ.copy()
            environment.update(
                {
                    "RELEASE_TAG": "v0.16.1",
                    "RELEASE_MODE": "dry-run",
                    "RELEASE_COMMIT": "0" * 40,
                    "RELEASE_ASSET_DIR": str(output),
                    "RELEASE_NOTES_FILE": "/dev/null",
                    "RELEASE_DRY_RUN": "1",
                }
            )
            staged = subprocess.run(
                ["bash", "scripts/ci/stage_release_assets.sh"],
                cwd=ROOT,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(staged.returncode, 0, staged.stderr)
            self.assertIn("dry-run: would stage", staged.stdout)


if __name__ == "__main__":
    unittest.main()

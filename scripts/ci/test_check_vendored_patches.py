from __future__ import annotations

import datetime as dt
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check-vendored-patches.py")
SPEC = importlib.util.spec_from_file_location("check_vendored_patches", SCRIPT)
assert SPEC and SPEC.loader
patches = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(patches)


class VendoredPatchCheckTests(unittest.TestCase):
    def write_patch(self, root: Path) -> dict:
        patch_root = root / "patches" / "sample"
        patch_root.mkdir(parents=True)
        (patch_root / "Cargo.toml").write_text('[package]\nname = "sample"\nversion = "1.0.1"\n')
        (patch_root / "THINCLAW-PATCH.md").write_text("# patch\n")
        (root / "Cargo.toml").write_text(
            '[package]\nname = "host"\nversion = "1.0.0"\n'
            '[patch.crates-io]\nsample = { path = "patches/sample" }\n'
        )
        return {
            "schema_version": 1,
            "patches": [
                {
                    "id": "sample",
                    "package": "sample",
                    "local_path": "patches/sample",
                    "uses": [{"manifest": "Cargo.toml", "package": "sample"}],
                    "owner": "@owner",
                    "base": {
                        "source": "crates.io",
                        "version": "1.0.0",
                        "revision": "abc",
                        "checksum": "0" * 64,
                    },
                    "rationale": "required",
                    "upstream": [{"kind": "issue", "status": "open", "url": "https://example.test/1"}],
                    "removal_condition": "upgrade and test",
                    "security_update_process": "review and rebase",
                    "review_on": "2026-09-15",
                    "diff_sha256": "0" * 64,
                    "changed_paths": [],
                }
            ],
        }

    def test_valid_inventory_covers_discovered_patch(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inventory = self.write_patch(root)
            patches.validate_inventory(root, inventory, dt.date(2026, 8, 13))

    def test_unrecorded_patch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inventory = self.write_patch(root)
            inventory["patches"] = []
            with self.assertRaisesRegex(patches.PatchError, "inventory mismatch"):
                patches.validate_inventory(root, inventory, dt.date(2026, 8, 13))

    def test_expired_review_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inventory = self.write_patch(root)
            inventory["patches"][0]["review_on"] = "2026-08-12"
            with self.assertRaisesRegex(patches.PatchError, "review expired"):
                patches.validate_inventory(root, inventory, dt.date(2026, 8, 13))

    def test_fingerprint_is_stable_and_detects_source_change(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            base = root / "base"
            patched = root / "patched"
            base.mkdir()
            patched.mkdir()
            (base / "src.rs").write_text("one\n")
            (patched / "src.rs").write_text("two\n")
            first = patches.diff_fingerprint(base, patched)
            second = patches.diff_fingerprint(base, patched)
            self.assertEqual(first, second)
            self.assertEqual(first[1], ["src.rs"])
            (patched / "src.rs").write_text("three\n")
            self.assertNotEqual(first[0], patches.diff_fingerprint(base, patched)[0])


if __name__ == "__main__":
    unittest.main()

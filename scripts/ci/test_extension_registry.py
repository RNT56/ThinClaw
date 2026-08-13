import gzip
import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("extension_registry.py")
SPEC = importlib.util.spec_from_file_location("extension_registry", MODULE_PATH)
assert SPEC and SPEC.loader
registry = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = registry
SPEC.loader.exec_module(registry)


class ExtensionRegistryTests(unittest.TestCase):
    def fixture(self):
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        source = root / "tools-src/example"
        source.mkdir(parents=True)
        (root / "registry/tools").mkdir(parents=True)
        (root / "registry/channels").mkdir(parents=True)
        (root / "release").mkdir()
        (root / ".github").mkdir()
        (root / "Cargo.toml").write_text(
            '[package]\nname = "fixture"\nversion = "0.16.1"\n', encoding="utf-8"
        )
        (source / "Cargo.toml").write_text(
            '[package]\nname = "example-tool"\nversion = "0.1.0"\n', encoding="utf-8"
        )
        (source / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "example-tool"\nversion = "0.1.0"\n',
            encoding="utf-8",
        )
        (source / "example.capabilities.json").write_text('{"type":"tool"}\n', encoding="utf-8")
        manifest = {
            "name": "example",
            "version": "0.1.0",
            "artifacts": {"wasm32-wasip2": {"url": None, "sha256": None}},
            "source": {
                "dir": "tools-src/example",
                "capabilities": "example.capabilities.json",
                "crate_name": "example-tool",
            },
        }
        registry.write_json(root / "registry/tools/example.json", manifest)
        config = {
            "schema_version": 1,
            "repository": "owner/repo",
            "release_version": "0.16.1",
            "strict_from": "v0.16.1",
            "target": "wasm32-wasip2",
            "cargo_component_version": "0.21.1",
            "prepared": None,
        }
        registry.write_json(root / "release/extension-registry.json", config)
        extensions = registry.load_extensions(root)
        (root / ".github/dependabot.yml").write_text(
            "version: 2\nupdates:\n" + registry.dependabot_block(root, extensions) + "\n",
            encoding="utf-8",
        )
        bundles = root / "bundles"
        bundles.mkdir()
        wasm = root / "example.wasm"
        wasm.write_bytes(b"\x00asm-fixture")
        registry.package_bundle(
            wasm,
            source / "example.capabilities.json",
            bundles / "example-wasm32-wasip2.tar.gz",
            "example",
        )
        return temporary, root, bundles

    def test_deterministic_packaging_and_comparison(self):
        temporary, root, first = self.fixture()
        with temporary:
            second = root / "second"
            second.mkdir()
            registry.package_bundle(
                root / "example.wasm",
                root / "tools-src/example/example.capabilities.json",
                second / "example-wasm32-wasip2.tar.gz",
                "example",
            )
            self.assertEqual(
                (first / "example-wasm32-wasip2.tar.gz").read_bytes(),
                (second / "example-wasm32-wasip2.tar.gz").read_bytes(),
            )
            self.assertTrue(registry.compare_bundles(root, first, second)["reproducible"])

    def test_strict_release_requires_prepared_registry(self):
        temporary, root, _ = self.fixture()
        with temporary:
            with self.assertRaisesRegex(registry.RegistryError, "requires a prepared"):
                registry.check_policy(root, "v0.16.1")
            result = registry.check_policy(root, "v0.16.0")
            self.assertFalse(result["strict"])

    def test_prepare_updates_immutable_url_hash_and_verifies_tag_build(self):
        temporary, root, bundles = self.fixture()
        with temporary:
            registry.prepare(root, "v0.16.1", bundles)
            manifest = registry.read_json(root / "registry/tools/example.json")
            artifact = manifest["artifacts"]["wasm32-wasip2"]
            self.assertEqual(
                artifact["url"],
                "https://github.com/owner/repo/releases/download/"
                "v0.16.1/example-wasm32-wasip2.tar.gz",
            )
            self.assertRegex(artifact["sha256"], r"^[0-9a-f]{64}$")
            self.assertTrue(registry.verify_bundles(root, "v0.16.1", bundles)["verified"])

    def test_source_or_bundle_drift_fails_after_prepare(self):
        temporary, root, bundles = self.fixture()
        with temporary:
            registry.prepare(root, "v0.16.1", bundles)
            (root / "tools-src/example/Cargo.toml").write_text(
                '[package]\nname = "example-tool"\nversion = "0.1.1"\n', encoding="utf-8"
            )
            with self.assertRaisesRegex(registry.RegistryError, "sources changed"):
                registry.check_policy(root, "v0.16.1")

    def test_missing_lock_and_dependabot_root_fail(self):
        temporary, root, _ = self.fixture()
        with temporary:
            (root / "tools-src/example/Cargo.lock").unlink()
            with self.assertRaisesRegex(registry.RegistryError, "committed lockfile"):
                registry.check_policy(root, "v0.16.0")
            (root / "tools-src/example/Cargo.lock").write_text(
                '[[package]]\nname = "example-tool"\n', encoding="utf-8"
            )
            (root / ".github/dependabot.yml").write_text("version: 2\nupdates: []\n", encoding="utf-8")
            with self.assertRaisesRegex(registry.RegistryError, "Dependabot"):
                registry.check_policy(root, "v0.16.0")

    def test_unregistered_source_root_fails(self):
        temporary, root, _ = self.fixture()
        with temporary:
            orphan = root / "channels-src/orphan"
            orphan.mkdir(parents=True)
            (orphan / "Cargo.toml").write_text("[package]\nname='orphan'\n", encoding="utf-8")
            with self.assertRaisesRegex(registry.RegistryError, "unregistered"):
                registry.check_policy(root, "v0.16.0")

    def test_nondeterministic_gzip_header_fails(self):
        temporary, root, bundles = self.fixture()
        with temporary:
            path = bundles / "example-wasm32-wasip2.tar.gz"
            raw = bytearray(path.read_bytes())
            raw[4:8] = (1234).to_bytes(4, "little")
            path.write_bytes(raw)
            with self.assertRaisesRegex(registry.RegistryError, "gzip header"):
                registry.bundle_inventory(root, bundles)


if __name__ == "__main__":
    unittest.main()

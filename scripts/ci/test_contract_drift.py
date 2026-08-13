import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("contract_drift.py")
SPEC = importlib.util.spec_from_file_location("contract_drift", MODULE_PATH)
assert SPEC and SPEC.loader
drift = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(drift)


class ContractDriftTests(unittest.TestCase):
    def test_rust_schema_drift_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            committed = root / "committed.json"
            generated = root / "generated.json"
            committed.write_text('{"openapi":"3.1.0"}\n', encoding="utf-8")
            generated.write_text('{"openapi":"3.1.0"}\n', encoding="utf-8")
            drift.check_rust_snapshot(committed, generated)
            generated.write_text('{"openapi":"3.1.1"}\n', encoding="utf-8")
            with self.assertRaisesRegex(drift.ContractError, "Rust-generated OpenAPI"):
                drift.check_rust_snapshot(committed, generated)

    def swift_fixture(self, root: Path):
        root_openapi = root / "root.json"
        vendored_openapi = root / "vendored.json"
        generator_config = root / "generator.yaml"
        mise_config = root / "mise.toml"
        generated_dir = root / "Generated"
        manifest = root / "manifest.json"
        generated_dir.mkdir()
        root_openapi.write_text('{"openapi":"3.1.0"}\n', encoding="utf-8")
        vendored_openapi.write_text(root_openapi.read_text(), encoding="utf-8")
        generator_config.write_text("generate:\n  - types\n  - client\n", encoding="utf-8")
        mise_config.write_text(
            '[tools]\n"spm:apple/swift-openapi-generator" = "1.7.0"\n',
            encoding="utf-8",
        )
        (generated_dir / "Client.swift").write_text("public struct Client {}\n", encoding="utf-8")
        (generated_dir / "Types.swift").write_text("public enum Types {}\n", encoding="utf-8")
        kwargs = {
            "root_openapi": root_openapi,
            "vendored_openapi": vendored_openapi,
            "generated_dir": generated_dir,
            "generator_config": generator_config,
            "mise_config": mise_config,
        }
        drift.write_swift_manifest(manifest, **kwargs)
        return manifest, kwargs

    def test_openapi_drift_fails_until_swift_is_regenerated(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest, kwargs = self.swift_fixture(Path(temporary))
            drift.check_swift_contract(manifest, **kwargs)
            kwargs["root_openapi"].write_text('{"openapi":"3.1.1"}\n', encoding="utf-8")
            with self.assertRaisesRegex(drift.ContractError, "vendored OpenAPI"):
                drift.check_swift_contract(manifest, **kwargs)

    def test_generated_swift_mutation_and_missing_files_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest, kwargs = self.swift_fixture(Path(temporary))
            client = kwargs["generated_dir"] / "Client.swift"
            client.write_text("public struct StaleClient {}\n", encoding="utf-8")
            with self.assertRaisesRegex(drift.ContractError, "generation manifest drifted"):
                drift.check_swift_contract(manifest, **kwargs)
            client.unlink()
            with self.assertRaisesRegex(drift.ContractError, "missing or empty"):
                drift.check_swift_contract(manifest, **kwargs)

    def test_generator_version_must_be_exact(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / "mise.toml"
            config.write_text(
                '[tools]\n"spm:apple/swift-openapi-generator" = "latest"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(drift.ContractError, "exact semantic version"):
                drift.generator_version(config)


if __name__ == "__main__":
    unittest.main()

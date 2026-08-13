import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("integration_test_gate.py")
SPEC = importlib.util.spec_from_file_location("integration_test_gate", MODULE_PATH)
assert SPEC and SPEC.loader
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class IntegrationTestGateTests(unittest.TestCase):
    def fixture(self):
        temp = tempfile.TemporaryDirectory()
        root = Path(temp.name)
        tests = root / "tests"
        workflows = root / ".github/workflows"
        tests.mkdir(parents=True)
        workflows.mkdir(parents=True)
        (tests / "hermetic.rs").write_text("#[test]\nfn works() {}\n", encoding="utf-8")
        (tests / "external.rs").write_text("#[test]\nfn works() {}\n", encoding="utf-8")
        (workflows / "ci.yml").write_text("run: --test external\n", encoding="utf-8")
        manifest = {
            "schema_version": 1,
            "features": "full",
            "hermetic": ["hermetic"],
            "external": [
                {
                    "target": "external",
                    "reason": "service",
                    "workflow": ".github/workflows/ci.yml",
                    "evidence": "--test external",
                }
            ],
            "ignored_external": [],
        }
        return temp, root, tests, manifest

    def test_complete_classification_passes(self):
        temp, root, tests, manifest = self.fixture()
        with temp:
            self.assertEqual(
                gate.validate_manifest(manifest, repo_root=root, tests_dir=tests),
                ["hermetic"],
            )

    def test_new_unclassified_target_fails(self):
        temp, root, tests, manifest = self.fixture()
        with temp:
            (tests / "forgotten.rs").write_text("#[test]\nfn works() {}\n", encoding="utf-8")
            with self.assertRaisesRegex(gate.ContractError, "missing=\\['forgotten'\\]"):
                gate.validate_manifest(manifest, repo_root=root, tests_dir=tests)

    def test_external_target_without_workflow_evidence_fails(self):
        temp, root, tests, manifest = self.fixture()
        with temp:
            manifest["external"][0]["evidence"] = "missing command"
            with self.assertRaisesRegex(gate.ContractError, "lacks workflow evidence"):
                gate.validate_manifest(manifest, repo_root=root, tests_dir=tests)

    def test_command_is_locked_and_selects_each_hermetic_target(self):
        manifest = {"features": "full"}
        command = gate.cargo_command(manifest, ["one", "two"])
        self.assertEqual(command[:6], ["cargo", "test", "--locked", "--workspace", "--features", "full"])
        self.assertIn("one", command)
        self.assertIn("two", command)


if __name__ == "__main__":
    unittest.main()

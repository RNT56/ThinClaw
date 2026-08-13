#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "worker_image_contract", ROOT / "scripts/ci/worker_image_contract.py"
)
assert SPEC and SPEC.loader
contract = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = contract
SPEC.loader.exec_module(contract)

CONTRACT_FILES = (
    "Dockerfile.worker",
    "release/worker-image-lock.json",
    "release/worker-tools/package.json",
    "release/worker-tools/package-lock.json",
    ".github/workflows/worker-image.yml",
    "scripts/ci/test_worker_image_health.sh",
    "src/worker/health.rs",
    "src/worker/mod.rs",
    "src/cli/mod.rs",
    "src/async_main/command_dispatch.rs",
    "src/main.rs",
)


class WorkerImageContractTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        for relative in CONTRACT_FILES:
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        return temporary, root

    def mutate(self, root: Path, relative: str, old: str, new: str) -> None:
        path = root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def test_repository_contract_passes(self) -> None:
        result = contract.check(ROOT)
        self.assertEqual(result["base_images"], 3)
        self.assertGreaterEqual(result["npm_packages"], 10)

    def test_mutable_base_and_remote_bootstrap_fail(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            lock = json.loads((root / "release/worker-image-lock.json").read_text())
            exact = lock["base_images"]["builder"]
            self.mutate(root, "Dockerfile.worker", exact, "rust:1.94.0-bookworm")
            with self.assertRaisesRegex(contract.WorkerImageContractError, "locked builder"):
                contract.check(root)

            self.mutate(
                root,
                "Dockerfile.worker",
                "# Multi-stage Dockerfile",
                "RUN curl https://example.invalid/bootstrap | sh\n# Multi-stage Dockerfile",
            )
            with self.assertRaises(contract.WorkerImageContractError):
                contract.check(root)

    def test_worker_tool_version_and_integrity_drift_fail(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            package = root / "release/worker-tools/package.json"
            data = json.loads(package.read_text())
            data["dependencies"]["@openai/codex"] = "latest"
            package.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(contract.WorkerImageContractError, "versions drifted"):
                contract.check(root)

        temporary, root = self.fixture()
        with temporary:
            npm_lock = root / "release/worker-tools/package-lock.json"
            data = json.loads(npm_lock.read_text())
            data["packages"]["node_modules/@openai/codex"]["integrity"] = ""
            npm_lock.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(contract.WorkerImageContractError, "integrity"):
                contract.check(root)

    def test_startup_sentinel_and_missing_runtime_heartbeat_fail(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            self.mutate(
                root,
                "Dockerfile.worker",
                'CMD ["thinclaw", "worker-health",',
                "CMD test -f /tmp/.thinclaw-alive || exit 1 #",
            )
            with self.assertRaises(contract.WorkerImageContractError):
                contract.check(root)

        temporary, root = self.fixture()
        with temporary:
            self.mutate(
                root,
                "src/async_main/command_dispatch.rs",
                "Command::CodexBridge { .. }",
                "Command::CodexBridgeDisabled { .. }",
            )
            with self.assertRaisesRegex(contract.WorkerImageContractError, "CodexBridge"):
                contract.check(root)

    def test_scan_and_liveness_evidence_fail_closed(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            self.mutate(root, ".github/workflows/worker-image.yml", 'exit-code: "1"', 'exit-code: "0"')
            with self.assertRaisesRegex(contract.WorkerImageContractError, "exit-code"):
                contract.check(root)

        temporary, root = self.fixture()
        with temporary:
            self.mutate(
                root,
                "scripts/ci/test_worker_image_health.sh",
                "--signal STOP",
                "--signal TERM",
            )
            with self.assertRaisesRegex(contract.WorkerImageContractError, "STOP"):
                contract.check(root)

    def test_source_only_rollback_requires_auditable_explanation(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "release/worker-image-lock.json"
            data = json.loads(path.read_text())
            data["rollback"]["reason"] = "unknown"
            path.write_text(json.dumps(data), encoding="utf-8")
            with self.assertRaisesRegex(contract.WorkerImageContractError, "explain"):
                contract.check(root)


if __name__ == "__main__":
    unittest.main()

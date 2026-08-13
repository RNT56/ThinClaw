from __future__ import annotations

import pathlib
import subprocess
import tempfile
import unittest

from scripts import dev_hooks
from scripts.hooks import pre_commit


class CredentialScanTests(unittest.TestCase):
    def test_high_confidence_credentials_are_detected_without_echoing_value(self) -> None:
        secret = "AKIA" + "A" * 16
        findings = pre_commit.scan_content("config.env", f"KEY={secret}\n".encode())
        self.assertEqual([(finding.rule, finding.line) for finding in findings], [("aws-access-key", 1)])
        self.assertNotIn(secret, repr(findings))

    def test_reviewed_fixture_marker_is_line_scoped(self) -> None:
        secret = "ghp_" + "a" * 40
        content = f"fixture={secret}  # {pre_commit.ALLOW_MARKER}\nnext={secret}\n".encode()
        findings = pre_commit.scan_content("fixture.txt", content)
        self.assertEqual([(finding.rule, finding.line) for finding in findings], [("github-token", 2)])

    def test_binary_and_oversized_files_are_skipped(self) -> None:
        self.assertEqual(pre_commit.scan_content("image", b"\0AKIA" + b"A" * 16), [])
        self.assertEqual(
            pre_commit.scan_content("large", b"x" * (pre_commit.MAX_SCANNED_BYTES + 1)),
            [],
        )


class CheckPlanTests(unittest.TestCase):
    def test_rust_desktop_and_contract_changes_reuse_existing_checks(self) -> None:
        plan = pre_commit.check_plan(
            [
                "src/app.rs",
                "apps/desktop/frontend/src/App.tsx",
                "clients/openapi/thinclaw-gateway.openapi.json",
            ]
        )
        names = {check.name for check in plan}
        self.assertIn("Rust formatting", names)
        self.assertIn("Rust file-size guard", names)
        self.assertIn("Desktop command/type boundary", names)
        self.assertIn("OpenAPI/Swift generated drift", names)

    def test_docs_only_change_keeps_the_default_hook_fast(self) -> None:
        plan = pre_commit.check_plan(["README.md"])
        self.assertEqual([check.name for check in plan], ["staged whitespace"])


class InstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temp.name)
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def config(self, key: str) -> str | None:
        result = subprocess.run(
            ["git", "config", "--local", "--get", key],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )
        return result.stdout.strip() if result.returncode == 0 else None

    def test_install_and_uninstall_are_idempotent(self) -> None:
        dev_hooks.install(self.repo)
        dev_hooks.install(self.repo)
        self.assertEqual(self.config("core.hooksPath"), ".githooks")
        dev_hooks.uninstall(self.repo)
        dev_hooks.uninstall(self.repo)
        self.assertIsNone(self.config("core.hooksPath"))

    def test_force_install_restores_an_existing_hook_path(self) -> None:
        subprocess.run(
            ["git", "config", "--local", "core.hooksPath", "custom-hooks"],
            cwd=self.repo,
            check=True,
        )
        with self.assertRaises(RuntimeError):
            dev_hooks.install(self.repo)
        dev_hooks.install(self.repo, force=True)
        dev_hooks.uninstall(self.repo)
        self.assertEqual(self.config("core.hooksPath"), "custom-hooks")


if __name__ == "__main__":
    unittest.main()

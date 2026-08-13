from __future__ import annotations

import importlib.util
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import date
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("check-coverage.py")
SPEC = importlib.util.spec_from_file_location("check_coverage", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
check_coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_coverage)


class CoverageDiffTests(unittest.TestCase):
    def test_parse_changed_lines_uses_new_file_line_numbers(self) -> None:
        diff = """\
diff --git a/src/example.rs b/src/example.rs
--- a/src/example.rs
+++ b/src/example.rs
@@ -4,0 +5,2 @@
+first();
+second();
@@ -10 +12 @@
-old();
+replacement();
"""
        self.assertEqual(
            check_coverage.parse_changed_lines(diff),
            {
                ("src/example.rs", 5),
                ("src/example.rs", 6),
                ("src/example.rs", 12),
            },
        )

    def test_parse_moved_lines_tracks_hunks_and_ignores_regular_additions(self) -> None:
        green = "\x1b[1;32m"
        reset = "\x1b[m"
        diff = (
            "diff --git a/src/example.rs b/src/example.rs\n"
            "--- a/src/example.rs\n"
            "+++ b/src/example.rs\n"
            "@@ -20,2 +40,3 @@\n"
            f"{green}+{reset}{green}moved_one();{reset}\n"
            f"{green}+{reset}{green}moved_two();{reset}\n"
            "+new_logic();\n"
            "@@ -30 +50 @@\n"
            "-old();\n"
            f"{green}+{reset}{green}moved_three();{reset}\n"
        )
        self.assertEqual(
            check_coverage.parse_moved_lines(diff),
            {
                ("src/example.rs", 40),
                ("src/example.rs", 41),
                ("src/example.rs", 50),
            },
        )

    def test_is_moved_addition_only_reads_colour_before_diff_marker(self) -> None:
        self.assertTrue(check_coverage.is_moved_addition("\x1b[1;32m+\x1b[mline"))
        self.assertFalse(check_coverage.is_moved_addition("+\x1b[1;32mline"))
        self.assertFalse(check_coverage.is_moved_addition("+ordinary line"))

    def test_relocated_lines_match_informative_deletions_as_a_multiset(self) -> None:
        diff = """\
diff --git a/src/old.rs b/src/old.rs
--- a/src/old.rs
+++ b/src/old.rs
@@ -1,3 +0,0 @@
-pub(super) async fn persist_runtime_outcome() {
-pub(super) async fn persist_runtime_outcome() {
-}
diff --git a/src/new.rs b/src/new.rs
--- a/src/new.rs
+++ b/src/new.rs
@@ -0,0 +10,4 @@
+pub(super) async fn persist_runtime_outcome() {
+pub(super) async fn persist_runtime_outcome() {
+pub(super) async fn persist_runtime_outcome() {
+}
"""
        self.assertEqual(
            check_coverage.parse_relocated_lines(diff),
            {("src/new.rs", 10), ("src/new.rs", 11)},
        )

    def test_relocated_boundaries_include_only_immediate_neighbors(self) -> None:
        self.assertEqual(
            check_coverage.expand_relocated_boundaries({("src/new.rs", 10)}),
            {("src/new.rs", 9), ("src/new.rs", 10), ("src/new.rs", 11)},
        )

    def test_debt_manifest_is_digest_guarded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "src" / "lib.rs"
            source.parent.mkdir()
            source.write_text("one();\ntwo();\nthree();\n", encoding="utf-8")
            baseline = root / "coverage-debt.json"
            baseline.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "files": {
                            "src/lib.rs": {
                                "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                                "uncovered": ["1", "3"],
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            with mock.patch.object(check_coverage, "ROOT", root):
                debt, invalidated = check_coverage.load_coverage_debt(baseline)
                self.assertEqual(debt, {("src/lib.rs", 1), ("src/lib.rs", 3)})
                self.assertEqual(invalidated, [])

                source.write_text("changed();\ntwo();\nthree();\n", encoding="utf-8")
                debt, invalidated = check_coverage.load_coverage_debt(baseline)
                self.assertEqual(debt, set())
                self.assertEqual(invalidated, ["src/lib.rs"])

    def test_debt_size_counts_declared_ranges_including_stale_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            baseline = Path(directory) / "coverage-debt.json"
            baseline.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "files": {
                            "src/one.rs": {"uncovered": ["1-3", "8"]},
                            "src/two.rs": {"uncovered": ["4-5"]},
                        },
                    }
                ),
                encoding="utf-8",
            )
            self.assertEqual(check_coverage.coverage_debt_size(baseline), (2, 6))

    def test_ratchet_selects_active_floor_and_reaches_sixty_percent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            ratchet = Path(directory) / "coverage-ratchet.json"
            ratchet.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "schedule": [
                            {
                                "effective_on": "2026-01-01",
                                "project_min": 50,
                                "covered_lines_min": 100,
                            },
                            {
                                "effective_on": "2026-06-01",
                                "project_min": 60,
                                "covered_lines_min": 100,
                            },
                        ],
                        "debt": {
                            "max_files": 4,
                            "max_lines": 20,
                            "require_current": True,
                            "require_pruned": True,
                        },
                    }
                ),
                encoding="utf-8",
            )
            policy = check_coverage.load_coverage_ratchet(
                ratchet, date.fromisoformat("2026-04-01")
            )
            self.assertEqual(policy.project_min, 50)
            self.assertEqual(policy.covered_lines_min, 100)
            self.assertEqual(policy.effective_on, date.fromisoformat("2026-01-01"))
            self.assertTrue(policy.require_current_debt)
            self.assertTrue(policy.require_pruned_debt)

    def test_ratchet_rejects_decreasing_or_sub_target_schedule(self) -> None:
        base = {
            "version": 1,
            "debt": {
                "max_files": 0,
                "max_lines": 0,
                "require_current": True,
                "require_pruned": True,
            },
        }
        invalid_schedules = [
            [
                {
                    "effective_on": "2026-01-01",
                    "project_min": 60,
                    "covered_lines_min": 100,
                },
                {
                    "effective_on": "2026-02-01",
                    "project_min": 59,
                    "covered_lines_min": 100,
                },
            ],
            [
                {
                    "effective_on": "2026-01-01",
                    "project_min": 59,
                    "covered_lines_min": 100,
                }
            ],
        ]
        for schedule in invalid_schedules:
            with self.subTest(schedule=schedule), tempfile.TemporaryDirectory() as directory:
                ratchet = Path(directory) / "coverage-ratchet.json"
                ratchet.write_text(
                    json.dumps({**base, "schedule": schedule}), encoding="utf-8"
                )
                with self.assertRaises(ValueError):
                    check_coverage.load_coverage_ratchet(
                        ratchet, date.fromisoformat("2026-01-15")
                    )

    def test_cli_rejects_absolute_covered_line_regression(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "coverage.lcov"
            report.write_text(
                "SF:/fixture/src/lib.rs\nDA:1,1\nend_of_record\n",
                encoding="utf-8",
            )
            ratchet = root / "coverage-ratchet.json"
            ratchet.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "schedule": [
                            {
                                "effective_on": "2026-01-01",
                                "project_min": 0,
                                "covered_lines_min": 2,
                            },
                            {
                                "effective_on": "2027-01-01",
                                "project_min": 60,
                                "covered_lines_min": 2,
                            },
                        ],
                        "debt": {
                            "max_files": 0,
                            "max_lines": 0,
                            "require_current": True,
                            "require_pruned": True,
                        },
                    }
                ),
                encoding="utf-8",
            )
            debt = root / "coverage-debt.json"
            debt.write_text(
                json.dumps({"version": 1, "files": {}}), encoding="utf-8"
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    str(report),
                    "--ratchet",
                    str(ratchet),
                    "--debt-baseline",
                    str(debt),
                    "--as-of",
                    "2026-01-01",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("covered line count 1 is below 2", result.stderr)


if __name__ == "__main__":
    unittest.main()

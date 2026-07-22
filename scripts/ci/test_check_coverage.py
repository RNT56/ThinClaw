from __future__ import annotations

import importlib.util
import hashlib
import json
import tempfile
import unittest
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


if __name__ == "__main__":
    unittest.main()

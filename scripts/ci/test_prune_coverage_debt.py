from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("prune-coverage-debt.py")
SPEC = importlib.util.spec_from_file_location("prune_coverage_debt", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
prune_coverage_debt = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prune_coverage_debt)


class PruneCoverageDebtTests(unittest.TestCase):
    def test_compact_ranges_is_canonical(self) -> None:
        self.assertEqual(
            prune_coverage_debt.compact_ranges({8, 1, 2, 3, 6}),
            ["1-3", "6", "8"],
        )

    def test_ranges_round_trip(self) -> None:
        lines = prune_coverage_debt.parse_ranges(["1-3", "6", "8-9"])
        self.assertEqual(lines, {1, 2, 3, 6, 8, 9})
        self.assertEqual(
            prune_coverage_debt.parse_ranges(
                prune_coverage_debt.compact_ranges(lines)
            ),
            lines,
        )

    def test_historical_source_prefix_rebases_lcov(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "coverage.lcov"
            report.write_text(
                "SF:/ci/checkout/src/lib.rs\nDA:10,2\nend_of_record\n",
                encoding="utf-8",
            )
            coverage = prune_coverage_debt.parse_lcov(
                report, [Path("/ci/checkout")]
            )
            self.assertEqual(coverage, {("src/lib.rs", 10): 2})


if __name__ == "__main__":
    unittest.main()

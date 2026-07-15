#!/usr/bin/env python3
"""Enforce project and changed-line coverage directly from an LCOV report."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def parse_lcov(path: Path) -> dict[tuple[str, int], int]:
    coverage: dict[tuple[str, int], int] = {}
    source: str | None = None
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if raw_line.startswith("SF:"):
            candidate = Path(raw_line[3:])
            try:
                source = candidate.resolve().relative_to(ROOT).as_posix()
            except ValueError:
                source = candidate.as_posix()
        elif raw_line.startswith("DA:") and source is not None:
            line_text, hits_text, *_ = raw_line[3:].split(",")
            key = (source, int(line_text))
            coverage[key] = max(coverage.get(key, 0), int(hits_text))
    if not coverage:
        raise ValueError(f"{path} contains no executable line records")
    return coverage


def changed_lines(base: str) -> set[tuple[str, int]]:
    if not base or set(base) == {"0"}:
        return set()
    result = subprocess.run(
        [
            "git",
            "diff",
            "--unified=0",
            "--no-color",
            base,
            "--",
            "*.rs",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    current_file: str | None = None
    changed: set[tuple[str, int]] = set()
    for line in result.stdout.splitlines():
        if line.startswith("+++ b/"):
            current_file = line[6:]
            continue
        match = HUNK.match(line)
        if match is None or current_file is None:
            continue
        start = int(match.group(1))
        length = int(match.group(2) or "1")
        changed.update((current_file, number) for number in range(start, start + length))

    # `git diff <base>` includes committed, staged, and unstaged tracked
    # changes, but Git intentionally omits untracked files. Include new Rust
    # sources explicitly so a local pre-commit run cannot silently report 0/0.
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "--", "*.rs"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    for relative in untracked.stdout.splitlines():
        path = ROOT / relative
        if not path.is_file():
            continue
        line_count = len(path.read_text(encoding="utf-8").splitlines())
        changed.update((relative, number) for number in range(1, line_count + 1))
    return changed


def percentage(covered: int, total: int) -> float:
    return 100.0 if total == 0 else covered * 100.0 / total


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("lcov", type=Path)
    parser.add_argument("--base", default="")
    parser.add_argument("--project-min", type=float, default=38.0)
    parser.add_argument("--patch-min", type=float, default=70.0)
    args = parser.parse_args()

    coverage = parse_lcov(args.lcov)
    project_covered = sum(hits > 0 for hits in coverage.values())
    project_total = len(coverage)
    project_pct = percentage(project_covered, project_total)
    print(
        f"Project line coverage: {project_pct:.2f}% "
        f"({project_covered}/{project_total}, minimum {args.project_min:.2f}%)"
    )

    failures: list[str] = []
    if project_pct < args.project_min:
        failures.append(
            f"project coverage {project_pct:.2f}% is below {args.project_min:.2f}%"
        )

    if args.base and set(args.base) != {"0"}:
        changed = changed_lines(args.base)
        coverable_changed = changed.intersection(coverage)
        patch_covered = sum(coverage[line] > 0 for line in coverable_changed)
        patch_total = len(coverable_changed)
        patch_pct = percentage(patch_covered, patch_total)
        print(
            f"Patch line coverage: {patch_pct:.2f}% "
            f"({patch_covered}/{patch_total}, minimum {args.patch_min:.2f}%)"
        )
        if patch_total and patch_pct < args.patch_min:
            uncovered = sorted(line for line in coverable_changed if coverage[line] == 0)
            print("Uncovered changed lines:", file=sys.stderr)
            for source, line_number in uncovered:
                print(f"  {source}:{line_number}", file=sys.stderr)
            failures.append(
                f"patch coverage {patch_pct:.2f}% is below {args.patch_min:.2f}%"
            )
    else:
        print("Patch line coverage: skipped (no base revision for this event)")

    if failures:
        for failure in failures:
            print(f"coverage gate failed: {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

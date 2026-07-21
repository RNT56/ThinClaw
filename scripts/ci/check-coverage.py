#!/usr/bin/env python3
"""Enforce project and changed-line coverage directly from an LCOV report."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")
ANSI_GREEN = re.compile(r"\x1b\[(?:[0-9;]*;)?32(?:;[0-9;]*)?m")
DEBT_RANGE = re.compile(r"^(\d+)(?:-(\d+))?$")


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


def parse_changed_lines(diff: str) -> set[tuple[str, int]]:
    """Return every added line number from an uncoloured unified diff."""
    current_file: str | None = None
    changed: set[tuple[str, int]] = set()
    for line in diff.splitlines():
        if line.startswith("+++ b/"):
            current_file = line[6:]
            continue
        match = HUNK.match(line)
        if match is None or current_file is None:
            continue
        start = int(match.group(1))
        length = int(match.group(2) or "1")
        changed.update((current_file, number) for number in range(start, start + length))
    return changed


def is_moved_addition(raw_line: str) -> bool:
    """Whether Git marked an added diff line as relocated code."""
    plus = raw_line.find("+")
    return plus >= 0 and ANSI_GREEN.search(raw_line[:plus]) is not None


def parse_moved_lines(diff: str) -> set[tuple[str, int]]:
    """Return added lines marked by Git's native moved-block detector."""
    current_file: str | None = None
    new_line: int | None = None
    moved: set[tuple[str, int]] = set()

    for raw_line in diff.splitlines():
        line = ANSI_ESCAPE.sub("", raw_line)
        if line.startswith("+++ b/"):
            current_file = line[6:]
            new_line = None
            continue
        match = HUNK.match(line)
        if match is not None:
            new_line = int(match.group(1))
            continue
        if current_file is None or new_line is None:
            continue
        if line.startswith("+") and not line.startswith("+++"):
            if is_moved_addition(raw_line):
                moved.add((current_file, new_line))
            new_line += 1
        elif line.startswith(" "):
            new_line += 1
        elif line.startswith("-") or line.startswith("\\"):
            continue

    return moved


def parse_relocated_lines(diff: str) -> set[tuple[str, int]]:
    """Match informative one-line relocations that are below Git's block limit.

    Git's moved-block detector intentionally requires a substantive contiguous
    block. Module extraction also leaves isolated signatures and error branches
    around edited boundaries. Match those only when the exact normalized line
    was deleted elsewhere in the same patch, consume matches as a multiset, and
    require enough identifier content to avoid pairing braces or boilerplate.
    """
    deleted: Counter[str] = Counter()
    additions: list[tuple[str, int, str]] = []
    current_file: str | None = None
    new_line: int | None = None

    for line in diff.splitlines():
        if line.startswith("+++ b/"):
            current_file = line[6:]
            new_line = None
            continue
        match = HUNK.match(line)
        if match is not None:
            new_line = int(match.group(1))
            continue
        if line.startswith("-") and not line.startswith("---"):
            normalized = line[1:].strip()
            if sum(char.isalnum() or char == "_" for char in normalized) >= 20:
                deleted[normalized] += 1
            continue
        if (
            current_file is not None
            and new_line is not None
            and line.startswith("+")
            and not line.startswith("+++")
        ):
            normalized = line[1:].strip()
            additions.append((current_file, new_line, normalized))
            new_line += 1
        elif new_line is not None and line.startswith(" "):
            new_line += 1

    relocated: set[tuple[str, int]] = set()
    for source, line_number, normalized in additions:
        if deleted[normalized] <= 0:
            continue
        deleted[normalized] -= 1
        relocated.add((source, line_number))
    return relocated


def expand_relocated_boundaries(
    relocated: set[tuple[str, int]],
) -> set[tuple[str, int]]:
    """Include declaration/delimiter lines bordering a detected moved block."""
    expanded = set(relocated)
    for source, line_number in relocated:
        if line_number > 1:
            expanded.add((source, line_number - 1))
        expanded.add((source, line_number + 1))
    return expanded


def parse_debt_ranges(values: list[str]) -> set[int]:
    lines: set[int] = set()
    for value in values:
        match = DEBT_RANGE.fullmatch(value)
        if match is None:
            raise ValueError(f"invalid coverage debt range: {value!r}")
        start = int(match.group(1))
        end = int(match.group(2) or match.group(1))
        if start <= 0 or end < start:
            raise ValueError(f"invalid coverage debt range: {value!r}")
        lines.update(range(start, end + 1))
    return lines


def load_coverage_debt(path: Path) -> tuple[set[tuple[str, int]], list[str]]:
    """Load known uncovered debt for source files whose content is unchanged.

    Entries are line ranges guarded by an exact file digest. Any edit to a file
    invalidates all of its debt instead of letting new uncovered code consume an
    old numeric allowance.
    """
    payload = json.loads(path.read_text(encoding="utf-8"))
    if payload.get("version") != 1 or not isinstance(payload.get("files"), dict):
        raise ValueError(f"{path} is not a version 1 coverage debt manifest")

    debt: set[tuple[str, int]] = set()
    invalidated: list[str] = []
    for source, record in payload["files"].items():
        if not isinstance(source, str) or not isinstance(record, dict):
            raise ValueError(f"{path} contains an invalid file record")
        relative = Path(source)
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"{path} contains an unsafe source path: {source!r}")
        expected_digest = record.get("sha256")
        ranges = record.get("uncovered")
        if (
            not isinstance(expected_digest, str)
            or re.fullmatch(r"[0-9a-f]{64}", expected_digest) is None
            or not isinstance(ranges, list)
            or not all(isinstance(value, str) for value in ranges)
        ):
            raise ValueError(f"{path} contains an invalid debt entry for {source}")
        source_path = ROOT / relative
        if not source_path.is_file():
            invalidated.append(source)
            continue
        actual_digest = hashlib.sha256(source_path.read_bytes()).hexdigest()
        if actual_digest != expected_digest:
            invalidated.append(source)
            continue
        line_count = len(source_path.read_text(encoding="utf-8").splitlines())
        line_numbers = parse_debt_ranges(ranges)
        if line_numbers and max(line_numbers) > line_count:
            raise ValueError(f"{path} contains an out-of-range line for {source}")
        debt.update((source, line_number) for line_number in line_numbers)
    return debt, invalidated


def git_diff(base: str, *, detect_moves: bool) -> str:
    command = ["git"]
    if detect_moves:
        # Use unique colours for moved additions while leaving ordinary added
        # lines uncoloured. `blocks` requires a substantive exact match and
        # `allow-indentation-change` recognizes module extraction/reindentation.
        command.extend(
            [
                "-c",
                "color.diff.old=normal",
                "-c",
                "color.diff.new=normal",
                "-c",
                "color.diff.oldMoved=bold red",
                "-c",
                "color.diff.newMoved=bold green",
                "-c",
                "color.diff.oldMovedAlternative=bold red",
                "-c",
                "color.diff.newMovedAlternative=bold green",
            ]
        )
    command.extend(
        [
            "diff",
            "--unified=0",
            "--no-ext-diff",
            "--color=always" if detect_moves else "--no-color",
        ]
    )
    if detect_moves:
        command.extend(
            [
                "--color-moved=blocks",
                "--color-moved-ws=allow-indentation-change",
            ]
        )
    command.extend(
        [
            base,
            "--",
            "*.rs",
        ]
    )
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def changed_lines(base: str) -> tuple[set[tuple[str, int]], int]:
    if not base or set(base) == {"0"}:
        return set(), 0
    plain_diff = git_diff(base, detect_moves=False)
    changed = parse_changed_lines(plain_diff)
    moved = expand_relocated_boundaries(
        parse_moved_lines(git_diff(base, detect_moves=True))
        | parse_relocated_lines(plain_diff)
    ).intersection(changed)
    changed.difference_update(moved)

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
    return changed, len(moved)


def percentage(covered: int, total: int) -> float:
    return 100.0 if total == 0 else covered * 100.0 / total


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("lcov", type=Path)
    parser.add_argument("--base", default="")
    parser.add_argument("--project-min", type=float, default=38.0)
    parser.add_argument("--patch-min", type=float, default=70.0)
    parser.add_argument("--debt-baseline", type=Path)
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
        changed, moved_count = changed_lines(args.base)
        print(f"Relocated Rust lines excluded from patch coverage: {moved_count}")
        coverable_changed = changed.intersection(coverage)
        debt: set[tuple[str, int]] = set()
        invalidated_debt_files: list[str] = []
        if args.debt_baseline is not None:
            try:
                debt, invalidated_debt_files = load_coverage_debt(args.debt_baseline)
            except (OSError, ValueError, json.JSONDecodeError) as error:
                print(f"coverage gate failed: invalid debt baseline: {error}", file=sys.stderr)
                return 1
        baselined = {
            line
            for line in coverable_changed.intersection(debt)
            if coverage[line] == 0
        }
        if args.debt_baseline is not None:
            print(f"Known uncovered-line debt excluded: {len(baselined)}")
            print(f"Coverage debt files invalidated by source changes: {len(invalidated_debt_files)}")
        eligible_changed = coverable_changed.difference(baselined)
        patch_covered = sum(coverage[line] > 0 for line in eligible_changed)
        patch_total = len(eligible_changed)
        patch_pct = percentage(patch_covered, patch_total)
        print(
            f"Patch line coverage: {patch_pct:.2f}% "
            f"({patch_covered}/{patch_total}, minimum {args.patch_min:.2f}%)"
        )
        if patch_total and patch_pct < args.patch_min:
            uncovered = sorted(line for line in eligible_changed if coverage[line] == 0)
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

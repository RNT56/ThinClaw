#!/usr/bin/env python3
"""Retire stale or newly covered entries from the coverage-debt manifest.

This command is deliberately one-way: it can remove debt but cannot add or
rebaseline it. Source edits retire the entire file allowance; newly covered
lines retire only those lines. Review the resulting JSON diff and commit it
with the tests or refactor that paid down the debt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def parse_ranges(values: list[str]) -> set[int]:
    lines: set[int] = set()
    for value in values:
        start_text, separator, end_text = value.partition("-")
        start = int(start_text)
        end = int(end_text) if separator else start
        if start <= 0 or end < start:
            raise ValueError(f"invalid coverage debt range: {value!r}")
        lines.update(range(start, end + 1))
    return lines


def compact_ranges(lines: set[int]) -> list[str]:
    if not lines:
        return []
    ordered = sorted(lines)
    ranges: list[str] = []
    start = previous = ordered[0]
    for line in ordered[1:]:
        if line == previous + 1:
            previous = line
            continue
        ranges.append(str(start) if start == previous else f"{start}-{previous}")
        start = previous = line
    ranges.append(str(start) if start == previous else f"{start}-{previous}")
    return ranges


def parse_lcov(path: Path, source_prefixes: list[Path]) -> dict[tuple[str, int], int]:
    coverage: dict[tuple[str, int], int] = {}
    source: str | None = None
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if raw_line.startswith("SF:"):
            candidate = Path(raw_line[3:])
            source = candidate.as_posix()
            for prefix in [ROOT, *source_prefixes]:
                try:
                    source = candidate.relative_to(prefix).as_posix()
                    break
                except ValueError:
                    continue
        elif raw_line.startswith("DA:") and source is not None:
            line_text, hits_text, *_ = raw_line[3:].split(",")
            key = (source, int(line_text))
            coverage[key] = max(coverage.get(key, 0), int(hits_text))
    return coverage


def prune(
    payload: dict[str, object], coverage: dict[tuple[str, int], int]
) -> tuple[dict[str, object], int, int]:
    files = payload.get("files")
    if payload.get("version") != 1 or not isinstance(files, dict):
        raise ValueError("not a version 1 coverage debt manifest")

    stale_files = 0
    covered_lines = 0
    next_files: dict[str, object] = {}
    for source, untyped_record in files.items():
        if not isinstance(source, str) or not isinstance(untyped_record, dict):
            raise ValueError("invalid coverage debt file record")
        record = dict(untyped_record)
        expected_digest = record.get("sha256")
        ranges = record.get("uncovered")
        if not isinstance(expected_digest, str) or not isinstance(ranges, list):
            raise ValueError(f"invalid coverage debt entry for {source}")
        source_path = ROOT / source
        if (
            not source_path.is_file()
            or hashlib.sha256(source_path.read_bytes()).hexdigest() != expected_digest
        ):
            stale_files += 1
            continue
        lines = parse_ranges(ranges)
        still_uncovered = {
            line for line in lines if coverage.get((source, line), 0) <= 0
        }
        covered_lines += len(lines) - len(still_uncovered)
        if still_uncovered:
            record["uncovered"] = compact_ranges(still_uncovered)
            next_files[source] = record

    next_payload = dict(payload)
    next_payload["files"] = next_files
    if next_files != files:
        next_payload["source_revision"] = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        next_payload["description"] = (
            "Digest-guarded, line-specific uncovered coverage debt. Source edits retire a "
            "file's allowance, and newly covered lines must be pruned; the manifest can only shrink."
        )
    return next_payload, stale_files, covered_lines


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("lcov", type=Path)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=ROOT / "scripts/ci/coverage-debt.json",
    )
    parser.add_argument(
        "--source-prefix",
        action="append",
        type=Path,
        default=[],
        help="checkout prefix embedded in a historical LCOV artifact",
    )
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()

    payload = json.loads(args.manifest.read_text(encoding="utf-8"))
    next_payload, stale_files, covered_lines = prune(
        payload, parse_lcov(args.lcov, args.source_prefix)
    )
    changed = next_payload != payload
    print(
        f"Coverage debt retirement: {stale_files} stale files, "
        f"{covered_lines} newly covered lines"
    )
    if not changed:
        print("Coverage debt manifest is current.")
        return 0
    if not args.write:
        print("Coverage debt manifest needs pruning; rerun with --write.")
        return 1

    temporary = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
    temporary.write_text(json.dumps(next_payload, indent=2) + "\n", encoding="utf-8")
    temporary.replace(args.manifest)
    print(f"Updated {args.manifest.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

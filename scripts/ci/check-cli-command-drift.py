#!/usr/bin/env python3
"""Reject deprecated ThinClaw CLI spellings in live sources and documentation."""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
EXCLUDED_PREFIXES = (
    "docs/cli-refinement/",
    "docs/remediation/",
)
EXCLUDED_FILES = {"CHANGELOG.md"}

LEGACY_ROOTS = (
    "onboard|cron|gateway|service|sessions|repo-projects|trajectory|pairing|"
    "tool|registry|mcp|memory|backup|browser|comfy|secrets|models|channels|"
    "identity|devices|experiments|logs|update|message send|reset"
)
CHECKS = (
    re.compile(rf"\bthinclaw\s+(?:{LEGACY_ROOTS})\b"),
    re.compile(r"\brun\s+--no-onboard\b"),
    re.compile(r"\bthinclaw\s+(?:doctor|status)\s+--profile\b"),
    re.compile(r"--show" + r"-token\b"),
)


def tracked_files() -> list[pathlib.Path]:
    output = subprocess.check_output(
        ["git", "ls-files", "-z"], cwd=ROOT
    )
    paths = []
    for raw in output.split(b"\0"):
        if not raw:
            continue
        relative = raw.decode("utf-8")
        if relative in EXCLUDED_FILES or relative.startswith(EXCLUDED_PREFIXES):
            continue
        paths.append(ROOT / relative)
    return paths


def main() -> int:
    findings: list[str] = []
    for path in tracked_files():
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        relative = path.relative_to(ROOT)
        for line_number, line in enumerate(text.splitlines(), start=1):
            if any(pattern.search(line) for pattern in CHECKS):
                findings.append(f"{relative}:{line_number}:{line.strip()}")

    if findings:
        print("Deprecated ThinClaw CLI spellings found:", file=sys.stderr)
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        print(
            "Use canonical command paths; historical records belong only in the excluded dossiers or changelog.",
            file=sys.stderr,
        )
        return 1

    print("CLI command drift check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

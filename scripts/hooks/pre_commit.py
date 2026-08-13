#!/usr/bin/env python3
"""Fast, deterministic checks used by the opt-in pre-commit hook and CI."""

from __future__ import annotations

import argparse
import dataclasses
import pathlib
import re
import shutil
import subprocess
import sys
from collections.abc import Iterable, Sequence


ROOT = pathlib.Path(__file__).resolve().parents[2]
MAX_SCANNED_BYTES = 4 * 1024 * 1024
ALLOW_MARKER = "thinclaw-secret-scan: allow"

# Intentionally high-confidence credential formats. Generic `password = ...`
# patterns create false positives in fixtures and encourage blanket ignores.
SECRET_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    (
        "private-key",
        re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----"),
    ),
    ("aws-access-key", re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")),
    ("github-token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{36,}\b")),
    ("github-fine-grained-token", re.compile(r"\bgithub_pat_[A-Za-z0-9_]{50,}\b")),
    ("slack-token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b")),
    ("stripe-live-key", re.compile(r"\b[rs]k_live_[A-Za-z0-9]{20,}\b")),
    ("google-api-key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
    ("openai-style-key", re.compile(r"\bsk-[A-Za-z0-9_-]{32,}\b")),
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    rule: str


@dataclasses.dataclass(frozen=True)
class Check:
    name: str
    command: tuple[str, ...]
    cwd: pathlib.Path = ROOT
    requires_desktop_modules: bool = False


def _git_bytes(arguments: Sequence[str], root: pathlib.Path = ROOT) -> bytes:
    return subprocess.check_output(["git", *arguments], cwd=root)


def selected_paths(*, all_tracked: bool, root: pathlib.Path = ROOT) -> list[str]:
    arguments = ["ls-files", "-z"] if all_tracked else [
        "diff",
        "--cached",
        "--name-only",
        "--diff-filter=ACMR",
        "-z",
    ]
    return [
        raw.decode("utf-8")
        for raw in _git_bytes(arguments, root).split(b"\0")
        if raw
    ]


def index_content(path: str, root: pathlib.Path = ROOT) -> bytes | None:
    try:
        return _git_bytes(["show", f":{path}"], root)
    except subprocess.CalledProcessError:
        return None


def scan_content(path: str, content: bytes) -> list[Finding]:
    if len(content) > MAX_SCANNED_BYTES or b"\0" in content:
        return []
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError:
        return []

    findings: list[Finding] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        if ALLOW_MARKER in line:
            continue
        for rule, pattern in SECRET_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(path, line_number, rule))
    return findings


def scan_paths(
    paths: Iterable[str],
    root: pathlib.Path = ROOT,
    *,
    all_tracked: bool = False,
) -> list[Finding]:
    findings: list[Finding] = []
    for path in paths:
        if all_tracked:
            candidate = root / path
            if candidate.is_symlink() or not candidate.is_file():
                continue
            try:
                content = candidate.read_bytes()
            except OSError:
                content = None
        else:
            # Hooks inspect the staged blob, not a potentially different
            # working-tree copy. The staged set is normally small, so one
            # bounded `git show` per path keeps this straightforward.
            content = index_content(path, root)
        if content is not None:
            findings.extend(scan_content(path, content))
    return findings


def check_plan(paths: Iterable[str], root: pathlib.Path = ROOT) -> list[Check]:
    changed = set(paths)
    rust_changed = any(
        path.endswith(".rs")
        or pathlib.PurePosixPath(path).name in {"Cargo.toml", "Cargo.lock"}
        for path in changed
    )
    desktop_changed = any(
        path.startswith("apps/desktop/")
        and pathlib.PurePosixPath(path).suffix
        in {".ts", ".tsx", ".js", ".mjs", ".json"}
        for path in changed
    )
    cli_surface_changed = any(
        path.startswith(("src/cli/", "crates/thinclaw-cli-contract/"))
        or path in {"docs/CLI_REFERENCE.md", "docs/SURFACES_AND_COMMANDS.md"}
        for path in changed
    )
    swift_contract_changed = any(
        path.startswith("apps/ios/Packages/ThinClawAPI/")
        or path == "clients/openapi/thinclaw-gateway.openapi.json"
        for path in changed
    )

    checks = [Check("staged whitespace", ("git", "diff", "--cached", "--check"), root)]
    if rust_changed:
        checks.extend(
            [
                Check("Rust formatting", ("cargo", "fmt", "--all", "--", "--check"), root),
                Check(
                    "Rust file-size guard",
                    ("bash", "scripts/ci/check-file-sizes.sh"),
                    root,
                ),
            ]
        )
    if desktop_changed:
        checks.append(
            Check(
                "Desktop command/type boundary",
                ("npm", "run", "lint:ts"),
                root / "apps/desktop",
                requires_desktop_modules=True,
            )
        )
    if cli_surface_changed:
        checks.append(
            Check(
                "CLI spelling drift",
                (sys.executable, "scripts/ci/check-cli-command-drift.py"),
                root,
            )
        )
    if swift_contract_changed:
        checks.append(
            Check(
                "OpenAPI/Swift generated drift",
                (sys.executable, "scripts/ci/contract_drift.py", "check-swift"),
                root,
            )
        )
    return checks


def _require_command(command: str) -> None:
    if shutil.which(command) is None:
        raise RuntimeError(f"required command is unavailable: {command}")


def run_checks(checks: Iterable[Check], root: pathlib.Path = ROOT) -> int:
    for check in checks:
        if check.requires_desktop_modules and not (root / "apps/desktop/node_modules").is_dir():
            print(
                "ThinClaw pre-commit: Desktop dependencies are missing; run "
                "'cd apps/desktop && npm ci'.",
                file=sys.stderr,
            )
            return 1
        try:
            _require_command(check.command[0])
        except RuntimeError as error:
            print(f"ThinClaw pre-commit: {error}", file=sys.stderr)
            return 1
        print(f"[pre-commit] {check.name}")
        completed = subprocess.run(check.command, cwd=check.cwd, check=False)
        if completed.returncode != 0:
            print(f"ThinClaw pre-commit: {check.name} failed.", file=sys.stderr)
            return completed.returncode
    return 0


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--all-tracked",
        action="store_true",
        help="scan every tracked file (CI mode) instead of staged files",
    )
    parser.add_argument(
        "--secret-scan-only",
        action="store_true",
        help="run only the high-confidence credential scan",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    paths = selected_paths(all_tracked=args.all_tracked)
    if not args.all_tracked and not paths:
        print("ThinClaw pre-commit: no staged files")
        return 0

    findings = scan_paths(paths, all_tracked=args.all_tracked)
    if findings:
        print("Potential committed credentials detected:", file=sys.stderr)
        for finding in findings:
            print(
                f"  {finding.path}:{finding.line}: {finding.rule} "
                f"(use '{ALLOW_MARKER}' only for a reviewed fixture)",
                file=sys.stderr,
            )
        return 1
    print(f"Credential scan passed ({len(paths)} file(s))")

    if args.secret_scan_only or args.all_tracked:
        return 0
    return run_checks(check_plan(paths))


if __name__ == "__main__":
    raise SystemExit(main())

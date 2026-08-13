#!/usr/bin/env python3
"""Install, remove, inspect, or execute ThinClaw's opt-in Git hooks."""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import sys
from collections.abc import Sequence


ROOT = pathlib.Path(__file__).resolve().parents[1]
HOOKS_PATH = ".githooks"
PREVIOUS_KEY = "thinclaw.previousHooksPath"

# Executing `python3 scripts/dev_hooks.py` places `scripts/`, rather than the
# repository root, on `sys.path`. Add the root so the same package import works
# for the executable and `python -m unittest` paths.
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def _git(repo: pathlib.Path, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *arguments],
        cwd=repo,
        check=check,
        text=True,
        capture_output=True,
    )


def _config_get(repo: pathlib.Path, key: str) -> str | None:
    result = _git(repo, "config", "--local", "--get", key, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def install(repo: pathlib.Path = ROOT, *, force: bool = False) -> None:
    current = _config_get(repo, "core.hooksPath")
    if current == HOOKS_PATH:
        print("ThinClaw hooks are already installed.")
        return
    if current and not force:
        raise RuntimeError(
            f"core.hooksPath is already {current!r}; rerun with '--force' to preserve and replace it"
        )
    if current:
        _git(repo, "config", "--local", PREVIOUS_KEY, current)
    _git(repo, "config", "--local", "core.hooksPath", HOOKS_PATH)
    print("Installed ThinClaw hooks. CI remains authoritative.")


def uninstall(repo: pathlib.Path = ROOT) -> None:
    current = _config_get(repo, "core.hooksPath")
    if current != HOOKS_PATH:
        print("ThinClaw hooks are not installed; no Git configuration changed.")
        return
    previous = _config_get(repo, PREVIOUS_KEY)
    if previous:
        _git(repo, "config", "--local", "core.hooksPath", previous)
        _git(repo, "config", "--local", "--unset", PREVIOUS_KEY)
        print(f"Removed ThinClaw hooks and restored core.hooksPath={previous!r}.")
    else:
        _git(repo, "config", "--local", "--unset", "core.hooksPath")
        print("Removed ThinClaw hooks.")


def status(repo: pathlib.Path = ROOT) -> int:
    current = _config_get(repo, "core.hooksPath")
    if current == HOOKS_PATH:
        print("ThinClaw hooks: installed")
        return 0
    print(f"ThinClaw hooks: not installed (core.hooksPath={current!r})")
    return 1


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    install_parser = subparsers.add_parser("install")
    install_parser.add_argument("--force", action="store_true")
    subparsers.add_parser("uninstall")
    subparsers.add_parser("status")
    subparsers.add_parser("run")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.command == "install":
            install(force=args.force)
        elif args.command == "uninstall":
            uninstall()
        elif args.command == "status":
            return status()
        elif args.command == "run":
            from scripts.hooks import pre_commit

            return pre_commit.main([])
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"ThinClaw hooks: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

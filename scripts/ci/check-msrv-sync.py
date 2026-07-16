#!/usr/bin/env python3
"""Fail when package MSRV and pinned developer, CI, or container toolchains drift."""

from pathlib import Path
import json
import re
import subprocess


root = Path(__file__).resolve().parents[2]
metadata = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
)
root_manifest = str((root / "Cargo.toml").resolve())
root_package = next(
    package
    for package in metadata["packages"]
    if str(Path(package["manifest_path"]).resolve()) == root_manifest
)
rust_version = str(root_package["rust_version"])

toolchain_text = (root / "rust-toolchain.toml").read_text(encoding="utf-8")
match = re.search(r'^\s*channel\s*=\s*"([^"]+)"\s*$', toolchain_text, re.MULTILINE)
if match is None:
    raise SystemExit("rust-toolchain.toml is missing a quoted toolchain.channel")
channel = match.group(1)
normalized_channel = channel.removesuffix(".0")

if normalized_channel != rust_version:
    raise SystemExit(
        "MSRV drift: Cargo.toml package.rust-version "
        f"is {rust_version}, rust-toolchain.toml channel is {channel}"
    )


def normalized_version(version: str) -> str:
    """Normalize exact patch-zero pins to Cargo's major.minor MSRV form."""

    return version.removesuffix(".0")


toolchain_errors: list[str] = []

workflow_pin_pattern = re.compile(
    r"^\s*toolchain:\s*([0-9]+\.[0-9]+(?:\.[0-9]+)?)\s*(?:#.*)?$",
    re.MULTILINE,
)
for workflow in sorted((root / ".github" / "workflows").glob("*.y*ml")):
    workflow_text = workflow.read_text(encoding="utf-8")
    for workflow_match in workflow_pin_pattern.finditer(workflow_text):
        pin = workflow_match.group(1)
        if normalized_version(pin) != rust_version:
            line = workflow_match.string.count("\n", 0, workflow_match.start()) + 1
            toolchain_errors.append(
                f"{workflow.relative_to(root)}:{line} pins Rust {pin}"
            )

    workflow_lines = workflow_text.splitlines()
    for index, line in enumerate(workflow_lines):
        if not line.lstrip().startswith("uses: dtolnay/rust-toolchain@"):
            continue

        uses_indent = len(line) - len(line.lstrip())
        with_indent: int | None = None
        has_toolchain_input = False
        for candidate in workflow_lines[index + 1 :]:
            stripped = candidate.lstrip()
            candidate_indent = len(candidate) - len(stripped)
            if stripped.startswith("- ") and candidate_indent < uses_indent:
                break
            if stripped == "with:" and candidate_indent == uses_indent:
                with_indent = candidate_indent
                continue
            if (
                with_indent is not None
                and stripped
                and candidate_indent <= with_indent
            ):
                with_indent = None
            if (
                with_indent is not None
                and stripped.startswith("toolchain:")
                and candidate_indent > with_indent
            ):
                has_toolchain_input = True
                break

        if not has_toolchain_input:
            toolchain_errors.append(
                f"{workflow.relative_to(root)}:{index + 1} is missing a toolchain input"
            )

container_pin_pattern = re.compile(
    r"^FROM\s+rust:([0-9]+\.[0-9]+(?:\.[0-9]+)?)(?:[-@\s]|$)", re.MULTILINE
)
for dockerfile in (root / "Dockerfile.worker", root / "docker" / "sandbox.Dockerfile"):
    dockerfile_text = dockerfile.read_text(encoding="utf-8")
    container_match = container_pin_pattern.search(dockerfile_text)
    if container_match is None:
        toolchain_errors.append(
            f"{dockerfile.relative_to(root)} is missing a numeric rust:<version> base image"
        )
        continue
    pin = container_match.group(1)
    if normalized_version(pin) != rust_version:
        toolchain_errors.append(f"{dockerfile.relative_to(root)} pins Rust {pin}")

if toolchain_errors:
    details = "\n".join(f"- {error}" for error in toolchain_errors)
    raise SystemExit(f"Rust toolchain configuration errors:\n{details}")

print(f"MSRV, CI, and container toolchains synchronized at Rust {rust_version}")

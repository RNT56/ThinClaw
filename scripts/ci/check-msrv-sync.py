#!/usr/bin/env python3
"""Fail when package MSRV and the pinned developer/CI toolchain drift."""

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

print(f"MSRV/toolchain synchronized at Rust {rust_version}")

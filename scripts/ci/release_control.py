#!/usr/bin/env python3
"""Pure planning and assembly helpers for the staged core-release workflow."""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "release/non-apple-assets.json"
TAG_RE = re.compile(
    r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$"
)
LINUX_TARGETS = (
    "aarch64-unknown-linux-gnu",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
)
WINDOWS_TARGET = "x86_64-pc-windows-msvc"


class ReleaseContractError(RuntimeError):
    """Raised when a release cannot satisfy its immutable contract."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def plan(tag: str, mode: str, root_version: str, promote_latest: bool) -> dict[str, object]:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise ReleaseContractError(
            f"core releases require an exact v-prefixed SemVer tag, got {tag!r}"
        )
    version = match.group("version")
    if version != root_version:
        raise ReleaseContractError(
            f"tag {tag} does not match the checked-out root version {root_version}"
        )
    if mode not in {"publish", "backfill", "dry-run"}:
        raise ReleaseContractError(f"unsupported release mode {mode!r}")

    prerelease = "-" in version.split("+", 1)[0]
    if prerelease and promote_latest:
        raise ReleaseContractError("prereleases may not promote stable latest aliases")

    return {
        "tag": tag,
        "version": version,
        "mode": mode,
        "publishing": mode != "dry-run",
        "backfill": mode == "backfill",
        "prerelease": prerelease,
        "promote_latest": promote_latest and not prerelease,
    }


def load_contract(path: Path) -> dict[str, object]:
    contract = json.loads(path.read_text(encoding="utf-8"))
    if contract.get("schema_version") != 1:
        raise ReleaseContractError(f"unsupported asset contract in {path}")
    return contract


def validate_assets(directory: Path, contract_path: Path) -> dict[str, object]:
    contract = load_contract(contract_path)
    names = sorted(path.name for path in directory.iterdir() if path.is_file())
    name_set = set(names)

    missing = sorted(set(contract["required_exact"]) - name_set)
    if missing:
        raise ReleaseContractError(f"missing required release assets: {', '.join(missing)}")

    for rule in contract["required_globs"]:
        matches = [name for name in names if fnmatch.fnmatch(name, rule["pattern"])]
        if len(matches) < rule["minimum"]:
            raise ReleaseContractError(
                f"asset pattern {rule['pattern']!r} matched {len(matches)} files; "
                f"requires at least {rule['minimum']}"
            )

    forbidden = sorted(
        name
        for name in names
        if any(fnmatch.fnmatch(name, pattern) for pattern in contract["forbidden_globs"])
    )
    if forbidden:
        raise ReleaseContractError(
            "Apple/deferred artifacts entered the core release: " + ", ".join(forbidden)
        )

    return {"asset_count": len(names), "assets": names}


def unique_candidate(source: Path, pattern: str, exclusions: tuple[str, ...]) -> Path:
    matches = sorted(
        path
        for path in source.rglob(pattern)
        if path.is_file() and not any(token in path.name for token in exclusions)
    )
    hashes: dict[str, Path] = {}
    for candidate in matches:
        hashes.setdefault(sha256(candidate), candidate)
    if len(hashes) != 1:
        rendered = ", ".join(str(path.relative_to(source)) for path in matches) or "none"
        raise ReleaseContractError(
            f"expected one unique artifact for {pattern!r}, found {len(hashes)}: {rendered}"
        )
    return next(iter(hashes.values()))


def copy_asset(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and sha256(source) != sha256(destination):
        raise ReleaseContractError(
            f"two different build artifacts resolve to release asset {destination.name}"
        )
    shutil.copy2(source, destination)


def write_checksums(output: Path, names: list[str], destination: str) -> None:
    lines = [f"{sha256(output / name)}  {name}" for name in sorted(names)]
    (output / destination).write_text("\n".join(lines) + "\n", encoding="utf-8")


def assemble(source: Path, output: Path, tag: str, contract_path: Path) -> dict[str, object]:
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)

    for target in LINUX_TARGETS:
        candidate = unique_candidate(
            source,
            f"*{target}*.tar.gz",
            ("-edge-", "-wasm32-wasip2"),
        )
        copy_asset(candidate, output / f"thinclaw-{target}.tar.gz")

    windows_archive = unique_candidate(
        source,
        f"*{WINDOWS_TARGET}*.tar.gz",
        ("-edge-", "-wasm32-wasip2"),
    )
    copy_asset(windows_archive, output / f"thinclaw-{WINDOWS_TARGET}.tar.gz")
    windows_msi = unique_candidate(source, f"*{WINDOWS_TARGET}*.msi", ())
    copy_asset(windows_msi, output / f"thinclaw-{WINDOWS_TARGET}.msi")

    for target in LINUX_TARGETS:
        name = f"thinclaw-edge-{target}.tar.gz"
        copy_asset(unique_candidate(source, name, ()), output / name)

    wasm = sorted(path for path in source.rglob("*-wasm32-wasip2.tar.gz") if path.is_file())
    for bundle in wasm:
        copy_asset(bundle, output / bundle.name)

    shell_installer = unique_candidate(source, "thinclaw-installer.sh", ())
    copy_asset(shell_installer, output / "thinclaw-installer.sh")
    (output / "thinclaw-installer.sh").chmod(0o755)
    windows_installer = unique_candidate(source, "thinclaw-installer.ps1", ())
    copy_asset(windows_installer, output / "thinclaw-installer.ps1")

    full_names = [
        f"thinclaw-{target}.tar.gz" for target in LINUX_TARGETS
    ] + [
        f"thinclaw-{WINDOWS_TARGET}.tar.gz",
        f"thinclaw-{WINDOWS_TARGET}.msi",
    ] + [bundle.name for bundle in wasm]
    edge_names = [f"thinclaw-edge-{target}.tar.gz" for target in LINUX_TARGETS]
    write_checksums(output, full_names, "checksums.txt")
    write_checksums(output, edge_names, "checksums-edge.txt")

    inventory = {
        "schema_version": 1,
        "tag": tag,
        "contract": str(contract_path.relative_to(ROOT)),
        "assets": [
            {"name": path.name, "sha256": sha256(path), "size": path.stat().st_size}
            for path in sorted(output.iterdir())
            if path.is_file() and path.name != "release-inventory.json"
        ],
    }
    (output / "release-inventory.json").write_text(
        json.dumps(inventory, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    validation = validate_assets(output, contract_path)
    return {**validation, "inventory": inventory}


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)

    plan_parser = commands.add_parser("plan")
    plan_parser.add_argument("--tag", required=True)
    plan_parser.add_argument("--mode", choices=("publish", "backfill", "dry-run"), required=True)
    plan_parser.add_argument("--root-version", required=True)
    plan_parser.add_argument("--promote-latest", action="store_true")

    assemble_parser = commands.add_parser("assemble")
    assemble_parser.add_argument("--source", type=Path, required=True)
    assemble_parser.add_argument("--output", type=Path, required=True)
    assemble_parser.add_argument("--tag", required=True)
    assemble_parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)

    validate_parser = commands.add_parser("validate")
    validate_parser.add_argument("--directory", type=Path, required=True)
    validate_parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "plan":
            result = plan(args.tag, args.mode, args.root_version, args.promote_latest)
        elif args.command == "assemble":
            result = assemble(args.source, args.output, args.tag, args.contract)
        else:
            result = validate_assets(args.directory, args.contract)
    except ReleaseContractError as error:
        print(f"release contract error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

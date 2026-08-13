#!/usr/bin/env python3
"""Fail closed when ThinClaw's local Cargo patches drift or outlive review."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import tarfile
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = ROOT / "patches" / "manifest.json"
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
IGNORED_NAMES = {
    ".cargo-checksum.json",
    ".cargo-ok",
    ".cargo_vcs_info.json",
    ".gitignore",
    ".gitlab-ci.yml",
    "Cargo.lock",
    "THINCLAW-PATCH.md",
    "appveyor.yml",
    "rustfmt.toml",
}
IGNORED_DIRS = {".git", ".github", "docker", "target"}


class PatchError(RuntimeError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_manifest(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise PatchError(f"cannot read patch manifest {path}: {exc}") from exc
    if value.get("schema_version") != 1 or not isinstance(value.get("patches"), list):
        raise PatchError("patch manifest must use schema_version 1 and contain patches[]")
    return value


def discover_path_patches(root: Path) -> set[tuple[str, str, str]]:
    found: set[tuple[str, str, str]] = set()
    for manifest in sorted(root.rglob("Cargo.toml")):
        relative = manifest.relative_to(root)
        if any(part in IGNORED_DIRS for part in relative.parts):
            continue
        try:
            lines = manifest.read_text(encoding="utf-8").splitlines()
        except OSError as exc:
            raise PatchError(f"cannot read {relative}: {exc}") from exc
        in_crates_io_patch = False
        for line in lines:
            stripped = line.strip()
            if stripped.startswith("["):
                in_crates_io_patch = bool(
                    re.fullmatch(r"\[patch\.crates-io\](?:\s*#.*)?", stripped)
                )
                continue
            if not in_crates_io_patch or not stripped or stripped.startswith("#"):
                continue
            assignment = re.match(r"^([A-Za-z0-9_-]+)\s*=\s*\{(.*)\}\s*(?:#.*)?$", stripped)
            if not assignment:
                raise PatchError(
                    f"{relative}: patch entries must use one-line inline tables for auditability"
                )
            path_match = re.search(r'(?:^|,)\s*path\s*=\s*"([^"]+)"', assignment.group(2))
            if not path_match:
                continue
            package = assignment.group(1)
            target = (manifest.parent / path_match.group(1)).resolve()
            try:
                target_relative = target.relative_to(root.resolve()).as_posix()
            except ValueError as exc:
                raise PatchError(f"{relative}: patch {package} escapes the repository") from exc
            found.add((relative.as_posix(), package, target_relative))
    return found


def inventory_path_patches(entries: list[dict]) -> set[tuple[str, str, str]]:
    found: set[tuple[str, str, str]] = set()
    for entry in entries:
        for use in entry.get("uses", []):
            found.add((use["manifest"], use["package"], entry["local_path"]))
    return found


def parse_review_date(value: object, patch_id: str) -> dt.date:
    if not isinstance(value, str):
        raise PatchError(f"{patch_id}: review_on must be an ISO date")
    try:
        return dt.date.fromisoformat(value)
    except ValueError as exc:
        raise PatchError(f"{patch_id}: invalid review_on date {value!r}") from exc


def validate_inventory(root: Path, inventory: dict, today: dt.date) -> None:
    entries = inventory["patches"]
    discovered = discover_path_patches(root)
    recorded = inventory_path_patches(entries)
    if discovered != recorded:
        missing = sorted(discovered - recorded)
        stale = sorted(recorded - discovered)
        raise PatchError(f"patch inventory mismatch; unrecorded={missing}, stale={stale}")

    ids: set[str] = set()
    local_paths: set[str] = set()
    required_text = (
        "owner",
        "rationale",
        "removal_condition",
        "security_update_process",
    )
    for entry in entries:
        patch_id = entry.get("id")
        if not isinstance(patch_id, str) or not patch_id or patch_id in ids:
            raise PatchError(f"invalid or duplicate patch id: {patch_id!r}")
        ids.add(patch_id)
        for field in required_text:
            if not isinstance(entry.get(field), str) or not entry[field].strip():
                raise PatchError(f"{patch_id}: {field} must be non-empty")

        local_path = entry.get("local_path")
        if not isinstance(local_path, str) or local_path in local_paths:
            raise PatchError(f"{patch_id}: invalid or duplicate local_path")
        local_paths.add(local_path)
        patch_root = root / local_path
        if not patch_root.is_dir() or not (patch_root / "Cargo.toml").is_file():
            raise PatchError(f"{patch_id}: local patch directory is missing")
        if not (patch_root / "THINCLAW-PATCH.md").is_file():
            raise PatchError(f"{patch_id}: THINCLAW-PATCH.md is required")

        base = entry.get("base", {})
        if base.get("source") != "crates.io":
            raise PatchError(f"{patch_id}: only checksum-pinned crates.io bases are supported")
        if not all(isinstance(base.get(k), str) and base[k] for k in ("version", "revision", "checksum")):
            raise PatchError(f"{patch_id}: base version, revision, and checksum are required")
        if not HEX_64.fullmatch(base["checksum"]):
            raise PatchError(f"{patch_id}: base checksum is not SHA-256")
        if not HEX_64.fullmatch(entry.get("diff_sha256", "")):
            raise PatchError(f"{patch_id}: diff_sha256 is not SHA-256")
        if not isinstance(entry.get("changed_paths"), list):
            raise PatchError(f"{patch_id}: changed_paths[] is required")

        upstream = entry.get("upstream")
        if not isinstance(upstream, list) or not upstream:
            raise PatchError(f"{patch_id}: at least one upstream issue, PR, commit, or release is required")
        for link in upstream:
            if not isinstance(link, dict) or not str(link.get("url", "")).startswith("https://"):
                raise PatchError(f"{patch_id}: invalid upstream link")
            if link.get("status") not in {"open", "merged", "released", "fixed-upstream", "tracking"}:
                raise PatchError(f"{patch_id}: upstream link has an invalid status")

        review_on = parse_review_date(entry.get("review_on"), patch_id)
        if review_on < today:
            raise PatchError(
                f"{patch_id}: dependency patch review expired on {review_on}; "
                "review upstream status and move or remove the patch"
            )


def ignored(relative: PurePosixPath) -> bool:
    return relative.name in IGNORED_NAMES or any(part in IGNORED_DIRS for part in relative.parts)


def tree_hashes(root: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for path in sorted(root.rglob("*")):
        if not path.is_file() and not path.is_symlink():
            continue
        relative = PurePosixPath(path.relative_to(root).as_posix())
        if ignored(relative):
            continue
        if path.is_symlink():
            content = b"symlink\0" + os.readlink(path).encode("utf-8")
        else:
            content = path.read_bytes()
        result[relative.as_posix()] = sha256_bytes(content)
    return result


def diff_fingerprint(base: Path, patched: Path) -> tuple[str, list[str]]:
    before = tree_hashes(base)
    after = tree_hashes(patched)
    changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
    digest = hashlib.sha256()
    for path in changed:
        digest.update(path.encode("utf-8"))
        digest.update(b"\0")
        digest.update(before.get(path, "-").encode("ascii"))
        digest.update(b"\0")
        digest.update(after.get(path, "-").encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest(), changed


def safely_extract(archive: Path, destination: Path, expected_root: str) -> Path:
    with tarfile.open(archive, "r:gz") as crate:
        members = crate.getmembers()
        for member in members:
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or ".." in relative.parts or not relative.parts:
                raise PatchError(f"unsafe path in crate archive: {member.name}")
            if relative.parts[0] != expected_root or member.issym() or member.islnk():
                raise PatchError(f"unexpected member in crate archive: {member.name}")
        crate.extractall(destination)
    extracted = destination / expected_root
    if not extracted.is_dir():
        raise PatchError(f"crate archive did not contain {expected_root}")
    return extracted


def download(url: str, destination: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "ThinClaw-patch-audit/1"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            destination.write_bytes(response.read())
    except OSError as exc:
        raise PatchError(f"failed to download {url}: {exc}") from exc


def verify_upstream(root: Path, entries: list[dict], print_digests: bool) -> None:
    with tempfile.TemporaryDirectory(prefix="thinclaw-patches-") as raw_temp:
        temp = Path(raw_temp)
        for entry in entries:
            package = entry["package"]
            version = entry["base"]["version"]
            url = f"https://static.crates.io/crates/{package}/{package}-{version}.crate"
            archive = temp / f"{package}-{version}.crate"
            download(url, archive)
            actual_checksum = sha256_bytes(archive.read_bytes())
            if actual_checksum != entry["base"]["checksum"]:
                raise PatchError(
                    f"{entry['id']}: crates.io checksum mismatch: "
                    f"expected {entry['base']['checksum']}, got {actual_checksum}"
                )
            extract_dir = temp / f"extract-{entry['id']}"
            extract_dir.mkdir()
            baseline = safely_extract(archive, extract_dir, f"{package}-{version}")
            digest, changed = diff_fingerprint(baseline, root / entry["local_path"])
            if print_digests:
                print(json.dumps({"id": entry["id"], "diff_sha256": digest, "changed_paths": changed}))
                continue
            if digest != entry["diff_sha256"] or changed != entry["changed_paths"]:
                raise PatchError(
                    f"{entry['id']}: vendored fork diff drifted; expected "
                    f"{entry['diff_sha256']} {entry['changed_paths']}, got {digest} {changed}"
                )
            print(f"Verified {entry['id']}: {len(changed)} changed runtime/package files")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--today", type=dt.date.fromisoformat, default=dt.date.today())
    parser.add_argument("--verify-upstream", action="store_true")
    parser.add_argument("--print-digests", action="store_true")
    args = parser.parse_args()
    try:
        inventory = load_manifest(args.manifest)
        validate_inventory(args.root.resolve(), inventory, args.today)
        if args.print_digests and not args.verify_upstream:
            raise PatchError("--print-digests requires --verify-upstream")
        if args.verify_upstream:
            verify_upstream(args.root.resolve(), inventory["patches"], args.print_digests)
    except PatchError as exc:
        print(f"vendored patch check failed: {exc}")
        return 1
    if not args.print_digests:
        print(f"Vendored patch inventory is current ({len(inventory['patches'])} patches).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

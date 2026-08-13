#!/usr/bin/env python3
"""Fail-closed extension dependency and release-artifact lifecycle."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


DEFAULT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_RELATIVE = Path("release/extension-registry.json")
DEPENDABOT_RELATIVE = Path(".github/dependabot.yml")
GENERATED_BEGIN = "  # BEGIN GENERATED EXTENSION CARGO ROOTS — scripts/ci/extension_registry.py"
GENERATED_END = "  # END GENERATED EXTENSION CARGO ROOTS"
TAG_RE = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+)$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class RegistryError(RuntimeError):
    """Raised when extension policy or immutable artifact state is invalid."""


@dataclass(frozen=True)
class Extension:
    kind: str
    manifest_path: Path
    name: str
    source_dir: Path
    capabilities: Path
    crate_name: str

    @property
    def cargo_manifest(self) -> Path:
        return self.source_dir / "Cargo.toml"

    @property
    def cargo_lock(self) -> Path:
        return self.source_dir / "Cargo.lock"


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RegistryError(f"cannot read JSON file {path}: {error}") from error


def write_json(path: Path, value: Any) -> None:
    rendered = json.dumps(value, indent=2, sort_keys=True) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as handle:
        handle.write(rendered)
        temporary = Path(handle.name)
    os.replace(temporary, path)


def safe_relative(root: Path, value: Any, field: str) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise RegistryError(f"{field} must be a non-empty repository-relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise RegistryError(f"{field} escapes the repository: {value!r}")
    resolved = (root / path).resolve()
    try:
        resolved.relative_to(root.resolve())
    except ValueError as error:
        raise RegistryError(f"{field} escapes the repository: {value!r}") from error
    return resolved


def load_config(root: Path) -> dict[str, Any]:
    config = read_json(root / CONFIG_RELATIVE)
    if not isinstance(config, dict) or config.get("schema_version") != 1:
        raise RegistryError("extension registry config schema_version must be 1")
    for field in (
        "repository",
        "release_version",
        "strict_from",
        "target",
        "cargo_component_version",
    ):
        if not isinstance(config.get(field), str) or not config[field]:
            raise RegistryError(f"extension registry config requires {field}")
    if TAG_RE.fullmatch(config["strict_from"]) is None:
        raise RegistryError("strict_from must be an exact v-prefixed release version")
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", config["release_version"]) is None:
        raise RegistryError("release_version must be an exact semantic version")
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", config["cargo_component_version"]) is None:
        raise RegistryError("cargo_component_version must be an exact semantic version")
    return config


def load_extensions(root: Path) -> list[Extension]:
    extensions: list[Extension] = []
    names: set[str] = set()
    source_dirs: set[Path] = set()
    for kind in ("channels", "tools"):
        registry_dir = root / "registry" / kind
        for path in sorted(registry_dir.glob("*.json")):
            document = read_json(path)
            if not isinstance(document, dict):
                raise RegistryError(f"registry manifest root must be an object: {path}")
            name = document.get("name")
            source = document.get("source")
            artifacts = document.get("artifacts")
            if not isinstance(name, str) or not name:
                raise RegistryError(f"registry manifest requires a name: {path}")
            if name in names:
                raise RegistryError(f"duplicate extension name: {name}")
            if not isinstance(source, dict):
                raise RegistryError(f"registry manifest requires source metadata: {path}")
            source_dir = safe_relative(root, source.get("dir"), f"{path}: source.dir")
            capabilities_name = source.get("capabilities")
            crate_name = source.get("crate_name")
            if not isinstance(capabilities_name, str) or not capabilities_name:
                raise RegistryError(f"registry manifest requires source.capabilities: {path}")
            if Path(capabilities_name).name != capabilities_name:
                raise RegistryError(f"capabilities filename must not contain directories: {path}")
            if not isinstance(crate_name, str) or not crate_name:
                raise RegistryError(f"registry manifest requires source.crate_name: {path}")
            if source_dir in source_dirs:
                raise RegistryError(f"duplicate extension source root: {source_dir}")
            if not isinstance(artifacts, dict) or "wasm32-wasip2" not in artifacts:
                raise RegistryError(f"registry manifest lacks wasm32-wasip2 artifact: {path}")
            names.add(name)
            source_dirs.add(source_dir)
            extensions.append(
                Extension(
                    kind=kind,
                    manifest_path=path,
                    name=name,
                    source_dir=source_dir,
                    capabilities=source_dir / capabilities_name,
                    crate_name=crate_name,
                )
            )
    if not extensions:
        raise RegistryError("no extension registry manifests were discovered")
    return extensions


def root_version(root: Path) -> str:
    text = (root / "Cargo.toml").read_text(encoding="utf-8")
    package = re.search(r"(?ms)^\[package\]\s+.*?^version\s*=\s*\"([^\"]+)\"", text)
    if package is None or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", package.group(1)) is None:
        raise RegistryError("root Cargo package requires an exact semantic version")
    return package.group(1)


def version_tuple(tag: str) -> tuple[int, int, int]:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise RegistryError(f"expected exact v-prefixed release tag, got {tag!r}")
    return tuple(int(part) for part in match.group("version").split("."))  # type: ignore[return-value]


def strict_for(tag: str, config: dict[str, Any]) -> bool:
    return version_tuple(tag) >= version_tuple(config["strict_from"])


def relative(root: Path, path: Path) -> str:
    return path.resolve().relative_to(root.resolve()).as_posix()


def dependabot_block(root: Path, extensions: list[Extension]) -> str:
    lines: list[str] = [GENERATED_BEGIN]
    for extension in sorted(extensions, key=lambda item: relative(root, item.source_dir)):
        directory = "/" + relative(root, extension.source_dir)
        lines.extend(
            [
                "  - package-ecosystem: cargo",
                f'    directory: "{directory}"',
                "    schedule:",
                "      interval: weekly",
                "    open-pull-requests-limit: 2",
                "",
            ]
        )
    lines.append(GENERATED_END)
    return "\n".join(lines)


def check_dependabot(root: Path, extensions: list[Extension]) -> None:
    path = root / DEPENDABOT_RELATIVE
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise RegistryError(f"cannot read Dependabot config {path}: {error}") from error
    expected = dependabot_block(root, extensions)
    if expected not in text:
        raise RegistryError(
            "Dependabot extension-root coverage drifted; run "
            "scripts/ci/extension_registry.py render-dependabot"
        )


def iter_source_files(root: Path, extensions: list[Extension]) -> Iterable[Path]:
    resolved_root = root.resolve()
    seen: set[Path] = set()
    roots = [extension.source_dir for extension in extensions]
    roots.extend(path for path in (root / "wit",) if path.exists())
    for optional in (root / "rust-toolchain.toml", root / ".cargo/config.toml"):
        if optional.is_file():
            roots.append(optional)
    for source_root in roots:
        candidates = [source_root] if source_root.is_file() else sorted(source_root.rglob("*"))
        for path in candidates:
            if not path.is_file() or "target" in path.resolve().relative_to(resolved_root).parts:
                continue
            resolved = path.resolve()
            if resolved not in seen:
                seen.add(resolved)
                yield resolved


def source_digest(root: Path, extensions: list[Extension], config: dict[str, Any]) -> str:
    digest = hashlib.sha256()
    digest.update(b"thinclaw-extension-registry-source-v1\0")
    digest.update(config["cargo_component_version"].encode())
    digest.update(b"\0")
    digest.update(config["target"].encode())
    digest.update(b"\0")
    for path in sorted(iter_source_files(root, extensions), key=lambda item: relative(root, item)):
        name = relative(root, path).encode()
        content = path.read_bytes()
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return digest.hexdigest()


def check_policy(root: Path, expected_tag: str | None = None) -> dict[str, Any]:
    config = load_config(root)
    extensions = load_extensions(root)
    registered_manifests = {extension.cargo_manifest.resolve() for extension in extensions}
    discovered_manifests = {
        path.resolve()
        for pattern in ("channels-src/*/Cargo.toml", "tools-src/*/Cargo.toml")
        for path in root.glob(pattern)
        if path.is_file()
    }
    if registered_manifests != discovered_manifests:
        missing = sorted(relative(root, path) for path in discovered_manifests - registered_manifests)
        stale = sorted(relative(root, path) for path in registered_manifests - discovered_manifests)
        raise RegistryError(
            f"extension source classification drift: unregistered={missing}, stale={stale}"
        )
    for extension in extensions:
        for path, description in (
            (extension.cargo_manifest, "Cargo manifest"),
            (extension.cargo_lock, "committed lockfile"),
            (extension.capabilities, "capabilities file"),
        ):
            if not path.is_file() or path.stat().st_size == 0:
                raise RegistryError(f"missing {description} for {extension.name}: {path}")
        lock_text = extension.cargo_lock.read_text(encoding="utf-8")
        if f'name = "{extension.crate_name}"' not in lock_text:
            raise RegistryError(
                f"lockfile does not contain root package {extension.crate_name}: "
                f"{extension.cargo_lock}"
            )
    check_dependabot(root, extensions)
    ci_workflow = root / ".github/workflows/ci.yml"
    if ci_workflow.is_file():
        ci_text = ci_workflow.read_text(encoding="utf-8")
        for fragment in (
            "extension-dependency-policy:",
            "extension_registry.py list-manifests",
            "cargo deny --locked --manifest-path",
        ):
            if fragment not in ci_text:
                raise RegistryError(f"extension CI policy wiring is missing: {fragment}")
        pinned_scripts = {
            root / "scripts/mac-deploy.sh": (
                f'CARGO_COMPONENT_VERSION="{config["cargo_component_version"]}"',
            ),
            root / "src/registry/artifacts.rs": (
                f'const CARGO_COMPONENT_VERSION: &str = "{config["cargo_component_version"]}";',
                '["component", "build", "--locked"]',
            ),
        }
        for path, fragments in pinned_scripts.items():
            text = path.read_text(encoding="utf-8")
            for fragment in fragments:
                if fragment not in text:
                    raise RegistryError(
                        f"extension toolchain pin or locked build drifted in {relative(root, path)}"
                    )

    current_tag = expected_tag or f"v{root_version(root)}"
    if config["release_version"] != root_version(root):
        raise RegistryError(
            "extension registry release_version does not match the root Cargo version"
        )
    is_strict = strict_for(current_tag, config)
    prepared = config.get("prepared")
    if is_strict:
        if not isinstance(prepared, dict):
            raise RegistryError(
                f"{current_tag} requires a prepared extension registry (strict from "
                f"{config['strict_from']})"
            )
        if prepared.get("tag") != current_tag:
            raise RegistryError(
                f"prepared registry tag {prepared.get('tag')!r} does not match {current_tag}"
            )
        if current_tag != f"v{root_version(root)}":
            raise RegistryError(
                f"registry tag {current_tag} does not match root version {root_version(root)}"
            )
        expected_source = source_digest(root, extensions, config)
        if prepared.get("source_sha256") != expected_source:
            raise RegistryError("extension sources changed after registry artifacts were prepared")
        bundles = prepared.get("bundles")
        if not isinstance(bundles, dict) or set(bundles) != {item.name for item in extensions}:
            raise RegistryError("prepared registry bundle inventory is incomplete or stale")
        for extension in extensions:
            record = bundles[extension.name]
            if not isinstance(record, dict) or not SHA256_RE.fullmatch(str(record.get("sha256", ""))):
                raise RegistryError(f"prepared bundle hash is invalid for {extension.name}")
            if not isinstance(record.get("size"), int) or record["size"] <= 0:
                raise RegistryError(f"prepared bundle size is invalid for {extension.name}")
            manifest = read_json(extension.manifest_path)
            artifact = manifest["artifacts"][config["target"]]
            filename = f"{extension.name}-{config['target']}.tar.gz"
            expected_url = (
                f"https://github.com/{config['repository']}/releases/download/"
                f"{current_tag}/{filename}"
            )
            if artifact.get("url") != expected_url:
                raise RegistryError(f"immutable artifact URL drifted for {extension.name}")
            if artifact.get("sha256") != record["sha256"]:
                raise RegistryError(f"registry hash drifted for {extension.name}")
    return {
        "extension_count": len(extensions),
        "strict": is_strict,
        "tag": current_tag,
        "source_sha256": source_digest(root, extensions, config),
    }


def bundle_paths(directory: Path, extensions: list[Extension], target: str) -> dict[str, Path]:
    expected = {
        extension.name: directory / f"{extension.name}-{target}.tar.gz"
        for extension in extensions
    }
    actual = {path for path in directory.glob(f"*-{target}.tar.gz") if path.is_file()}
    missing = sorted(path.name for path in expected.values() if path not in actual)
    extra = sorted(path.name for path in actual - set(expected.values()))
    if missing or extra:
        raise RegistryError(f"bundle inventory drift: missing={missing}, extra={extra}")
    return expected


def inspect_bundle(path: Path, extension: Extension) -> None:
    raw = path.read_bytes()
    if len(raw) < 10 or raw[:2] != b"\x1f\x8b" or int.from_bytes(raw[4:8], "little") != 0:
        raise RegistryError(f"bundle gzip header is not deterministic: {path}")
    try:
        with gzip.open(path, "rb") as compressed:
            with tarfile.open(fileobj=compressed, mode="r:") as archive:
                members = archive.getmembers()
                expected_names = [
                    f"{extension.name}.capabilities.json",
                    f"{extension.name}.wasm",
                ]
                if sorted(member.name for member in members) != expected_names:
                    raise RegistryError(f"bundle member inventory drifted: {path}")
                for member in members:
                    if not member.isfile() or member.mtime != 0 or member.uid != 0 or member.gid != 0:
                        raise RegistryError(f"bundle metadata is not deterministic: {path}:{member.name}")
                capabilities = archive.extractfile(f"{extension.name}.capabilities.json")
                wasm = archive.extractfile(f"{extension.name}.wasm")
                if capabilities is None or capabilities.read() != extension.capabilities.read_bytes():
                    raise RegistryError(f"bundled capabilities do not match source: {path}")
                if wasm is None or len(wasm.read()) == 0:
                    raise RegistryError(f"bundle contains empty WASM: {path}")
    except (gzip.BadGzipFile, tarfile.TarError, OSError) as error:
        raise RegistryError(f"invalid extension bundle {path}: {error}") from error


def package_bundle(wasm: Path, capabilities: Path, output: Path, name: str) -> None:
    """Create a byte-reproducible gzip-compressed ustar archive."""
    if not wasm.is_file() or wasm.stat().st_size == 0:
        raise RegistryError(f"cannot package missing or empty WASM file: {wasm}")
    if not capabilities.is_file() or capabilities.stat().st_size == 0:
        raise RegistryError(f"cannot package missing or empty capabilities file: {capabilities}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as zipped:
            with tarfile.open(fileobj=zipped, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for archive_name, source in sorted(
                    (
                        (f"{name}.wasm", wasm),
                        (f"{name}.capabilities.json", capabilities),
                    )
                ):
                    content = source.read_bytes()
                    info = tarfile.TarInfo(archive_name)
                    info.size = len(content)
                    info.mode = 0o644
                    info.uid = 0
                    info.gid = 0
                    info.uname = "root"
                    info.gname = "root"
                    info.mtime = 0
                    archive.addfile(info, io.BytesIO(content))


def bundle_inventory(root: Path, directory: Path) -> dict[str, dict[str, Any]]:
    config = load_config(root)
    extensions = load_extensions(root)
    paths = bundle_paths(directory, extensions, config["target"])
    inventory: dict[str, dict[str, Any]] = {}
    for extension in extensions:
        path = paths[extension.name]
        inspect_bundle(path, extension)
        inventory[extension.name] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "size": path.stat().st_size,
        }
    return inventory


def compare_bundles(root: Path, first: Path, second: Path) -> dict[str, Any]:
    first_inventory = bundle_inventory(root, first)
    second_inventory = bundle_inventory(root, second)
    if first_inventory != second_inventory:
        changed = sorted(
            name
            for name in set(first_inventory) | set(second_inventory)
            if first_inventory.get(name) != second_inventory.get(name)
        )
        raise RegistryError(f"clean extension builds are not reproducible: {changed}")
    return {"bundle_count": len(first_inventory), "reproducible": True}


def prepare(root: Path, tag: str, directory: Path) -> dict[str, Any]:
    config = load_config(root)
    extensions = load_extensions(root)
    if not strict_for(tag, config):
        raise RegistryError(f"refusing to prepare legacy registry tag {tag}")
    if tag != f"v{root_version(root)}":
        raise RegistryError(f"preparation tag {tag} does not match root version {root_version(root)}")
    check_dependabot(root, extensions)
    inventory = bundle_inventory(root, directory)
    for extension in extensions:
        manifest = read_json(extension.manifest_path)
        filename = f"{extension.name}-{config['target']}.tar.gz"
        manifest["artifacts"][config["target"]] = {
            "url": (
                f"https://github.com/{config['repository']}/releases/download/{tag}/{filename}"
            ),
            "sha256": inventory[extension.name]["sha256"],
        }
        write_json(extension.manifest_path, manifest)
    config["prepared"] = {
        "tag": tag,
        "source_sha256": source_digest(root, extensions, config),
        "bundles": inventory,
    }
    write_json(root / CONFIG_RELATIVE, config)
    check_policy(root, tag)
    return {"tag": tag, "bundle_count": len(inventory), "prepared": True}


def verify_bundles(root: Path, tag: str, directory: Path) -> dict[str, Any]:
    policy = check_policy(root, tag)
    inventory = bundle_inventory(root, directory)
    if policy["strict"]:
        prepared = load_config(root)["prepared"]["bundles"]
        if inventory != prepared:
            changed = sorted(name for name in inventory if inventory[name] != prepared.get(name))
            raise RegistryError(f"tag-time bundles differ from committed registry hashes: {changed}")
    return {**policy, "bundle_count": len(inventory), "verified": True}


def replace_dependabot_block(root: Path, rendered: str) -> None:
    path = root / DEPENDABOT_RELATIVE
    text = path.read_text(encoding="utf-8")
    pattern = re.compile(
        rf"^\s*{re.escape(GENERATED_BEGIN.strip())}$.*?^\s*{re.escape(GENERATED_END.strip())}$",
        flags=re.MULTILINE | re.DOTALL,
    )
    if pattern.search(text):
        updated = pattern.sub(rendered, text)
    else:
        updated = text.rstrip() + "\n\n" + rendered + "\n"
    path.write_text(updated, encoding="utf-8")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    root.add_argument("--repo-root", type=Path, default=DEFAULT_ROOT)
    commands = root.add_subparsers(dest="command", required=True)

    check = commands.add_parser("check-policy")
    check.add_argument("--expected-tag")

    commands.add_parser("list-manifests")
    commands.add_parser("tool-version")
    commands.add_parser("render-dependabot")
    commands.add_parser("write-dependabot")

    compare = commands.add_parser("compare-bundles")
    compare.add_argument("--first", type=Path, required=True)
    compare.add_argument("--second", type=Path, required=True)

    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--tag", required=True)
    prepare_parser.add_argument("--bundles", type=Path, required=True)

    verify = commands.add_parser("verify-bundles")
    verify.add_argument("--tag", required=True)
    verify.add_argument("--bundles", type=Path, required=True)

    package = commands.add_parser("package-bundle")
    package.add_argument("--wasm", type=Path, required=True)
    package.add_argument("--capabilities", type=Path, required=True)
    package.add_argument("--output", type=Path, required=True)
    package.add_argument("--name", required=True)
    return root


def main() -> int:
    args = parser().parse_args()
    root = args.repo_root.resolve()
    try:
        extensions = load_extensions(root)
        if args.command == "check-policy":
            result: Any = check_policy(root, args.expected_tag)
        elif args.command == "list-manifests":
            for extension in extensions:
                print(relative(root, extension.cargo_manifest))
            return 0
        elif args.command == "tool-version":
            print(load_config(root)["cargo_component_version"])
            return 0
        elif args.command == "render-dependabot":
            print(dependabot_block(root, extensions))
            return 0
        elif args.command == "write-dependabot":
            replace_dependabot_block(root, dependabot_block(root, extensions))
            result = {"extension_count": len(extensions), "dependabot_updated": True}
        elif args.command == "compare-bundles":
            result = compare_bundles(root, args.first, args.second)
        elif args.command == "prepare":
            result = prepare(root, args.tag, args.bundles)
        elif args.command == "package-bundle":
            package_bundle(args.wasm, args.capabilities, args.output, args.name)
            result = {"bundle": str(args.output), "packaged": True}
        else:
            result = verify_bundles(root, args.tag, args.bundles)
    except RegistryError as error:
        print(f"extension registry error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

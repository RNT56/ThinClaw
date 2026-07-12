#!/usr/bin/env python3
"""Audit production prompt construction against Prompt System V2 governance."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTRY_PATH = ROOT / "docs" / "prompt-registry.json"
SEARCH_ROOTS = ("src", "crates", "apps", "assets")
EXTENSIONS = {".rs", ".ts", ".tsx", ".swift", ".rhai", ".md"}
PROMPT_MARKERS = re.compile(
    r"ChatMessage::system|system_prompt\s*=|with_system_prompt|\.preamble\(|"
    r"[\"']role[\"']\s*:\s*[\"']system[\"']|IMAGE_PROMPT_ENHANCE_SYSTEM_PROMPT|"
    r"HEARTBEAT_OK|ROUTINE_OK"
)
REQUIRED_FIELDS = {
    "id", "owner", "consumer", "authority", "lifetime", "input_provenance",
    "output_contract", "token_policy", "parser", "model_capabilities", "test_owner",
}
FORBIDDEN = {
    r"(?<!Flow)ApprovalDecision::Approve\b": "LLM security triage must never authorize execution",
    r"\.contains\(\"HEARTBEAT_OK\"\)": "heartbeat results must use exact typed parsing",
    r"\.contains\(LIGHTWEIGHT_ROUTINE_OK_SENTINEL\)": "routine results must use exact typed parsing",
    r"Start your response with a clear thought": "hidden reasoning must not be requested",
    r"`broadcast` capability": "broadcast is not a registered tool name",
    r"tool_search` to find and install": "discovery and installation must be distinct",
}


def production_files() -> list[Path]:
    files: list[Path] = []
    for root_name in SEARCH_ROOTS:
        root = ROOT / root_name
        if not root.exists():
            continue
        for path in root.rglob("*"):
            rel = path.relative_to(ROOT).as_posix()
            if not path.is_file() or path.suffix not in EXTENSIONS:
                continue
            if any(part in {"target", "node_modules", "generated"} for part in path.parts):
                continue
            if path.name.endswith(".snap") or "/tests/" in f"/{rel}/" or path.name == "tests.rs":
                continue
            files.append(path)
    return files


def main() -> int:
    registry = json.loads(REGISTRY_PATH.read_text())
    entries = registry.get("entries", [])
    errors: list[str] = []
    if registry.get("contract_version") != "v2":
        errors.append("prompt registry contract_version must be v2")
    ids: set[str] = set()
    covered_paths: set[str] = set()
    prefixes: list[str] = []
    for entry in entries:
        missing = REQUIRED_FIELDS - entry.keys()
        if missing:
            errors.append(f"registry entry {entry.get('id', '<missing>')} lacks {sorted(missing)}")
        entry_id = entry.get("id")
        if entry_id in ids:
            errors.append(f"duplicate registry entry id: {entry_id}")
        ids.add(entry_id)
        covered_paths.update(entry.get("paths", []))
        prefixes.extend(entry.get("path_prefixes", []))

    candidates: list[str] = []
    files = production_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        text = path.read_text(errors="replace")
        if PROMPT_MARKERS.search(text):
            candidates.append(rel)
            if rel not in covered_paths and not any(rel.startswith(prefix) for prefix in prefixes):
                errors.append(f"unregistered prompt-bearing file: {rel}")
        if path.suffix in {".rs", ".ts", ".tsx"}:
            for pattern, reason in FORBIDDEN.items():
                if re.search(pattern, text):
                    errors.append(f"{rel}: forbidden prompt pattern {pattern!r}: {reason}")

    if not candidates:
        errors.append("prompt audit found no production prompt sites")
    if errors:
        print("Prompt System V2 audit failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"Prompt System V2 audit passed: {len(candidates)} prompt-bearing files covered "
        f"by {len(entries)} registry entries."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

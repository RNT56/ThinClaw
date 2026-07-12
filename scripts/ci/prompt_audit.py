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
    r"ChatMessage::(?:system|immutable_policy|trusted_prompt)|system_prompt\s*=|"
    r"with_system_prompt|\.preamble\(|"
    r"[\"']role[\"']\s*:\s*[\"']system[\"']|IMAGE_PROMPT_ENHANCE_SYSTEM_PROMPT|"
    r"HEARTBEAT_OK|ROUTINE_OK|BOOT_SEQUENCE"
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
    "\u200b": "prompt-bearing source must not contain invisible zero-width characters",
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
    prompt_owners: dict[str, str] = {}
    for entry in entries:
        missing = REQUIRED_FIELDS - entry.keys()
        if missing:
            errors.append(f"registry entry {entry.get('id', '<missing>')} lacks {sorted(missing)}")
        entry_id = entry.get("id")
        if entry_id in ids:
            errors.append(f"duplicate registry entry id: {entry_id}")
        ids.add(entry_id)
        prompt_paths = entry.get("prompt_paths")
        if not isinstance(prompt_paths, list):
            errors.append(f"registry entry {entry_id} must declare prompt_paths")
            continue
        for prompt_path in prompt_paths:
            previous = prompt_owners.get(prompt_path)
            if previous is not None:
                errors.append(
                    f"prompt path {prompt_path} is owned by both {previous} and {entry_id}"
                )
            prompt_owners[prompt_path] = entry_id
            if not (ROOT / prompt_path).is_file():
                errors.append(f"registered prompt path does not exist: {prompt_path}")

    candidates: list[str] = []
    files = production_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        text = path.read_text(errors="replace")
        if PROMPT_MARKERS.search(text):
            candidates.append(rel)
            if rel not in prompt_owners:
                errors.append(f"unregistered prompt-bearing file: {rel}")
        if path.suffix in {".rs", ".ts", ".tsx"}:
            for pattern, reason in FORBIDDEN.items():
                if re.search(pattern, text):
                    errors.append(f"{rel}: forbidden prompt pattern {pattern!r}: {reason}")

    if not candidates:
        errors.append("prompt audit found no production prompt sites")
    stale_paths = sorted(set(prompt_owners) - set(candidates))
    for path in stale_paths:
        errors.append(f"registered prompt path no longer contains a prompt marker: {path}")

    # The interactive dispatcher has a stronger contract than registry
    # ownership alone: PromptStack policy and source segments must compile
    # together at request time, and runtime directives must use typed authority.
    invariant_patterns = {
        "src/llm/reasoning.rs": (
            "build_conversation_policy_stack(context).into_segment(",
            "PromptCompiler::new().push(policy)",
            "Prompt V2 compilation failed closed",
            "last_prompt_compilation",
        ),
        "src/agent/dispatcher/prompt_context.rs": (
            "with_prompt_contract(prompt_source_segments, prompt_budget)",
            "dispatcher_prompt_assembly(&prompt_materials)",
        ),
        "crates/thinclaw-agent/src/prompt_assembly.rs": (
            'push_required_policy("transcript_guidance"',
            "PromptTrust::UntrustedData",
        ),
        "crates/thinclaw-llm-core/src/provider.rs": (
            "pub fn immutable_policy(",
            "pub fn prompt_authority(",
        ),
    }
    for rel, patterns in invariant_patterns.items():
        text = (ROOT / rel).read_text(errors="replace")
        for pattern in patterns:
            if pattern not in text:
                errors.append(f"{rel}: missing canonical Prompt V2 invariant {pattern!r}")

    dispatcher_root = ROOT / "src" / "agent" / "dispatcher"
    for path in production_files():
        if dispatcher_root not in path.parents:
            continue
        text = path.read_text(errors="replace")
        if "ChatMessage::system(" in text:
            errors.append(
                f"{path.relative_to(ROOT).as_posix()}: untyped dispatcher system message; "
                "use immutable_policy or trusted_prompt"
            )
    if errors:
        print("Prompt System V2 audit failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"Prompt System V2 audit passed: {len(candidates)} prompt-bearing files exactly owned "
        f"by {len(entries)} registry entries; canonical dispatcher invariants verified."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

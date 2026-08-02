#!/usr/bin/env python3
"""Guard the compiled desktop runtime-builder topology and bridge projection."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PARENT = ROOT / "apps/desktop/backend/src/thinclaw/runtime_builder.rs"
CHILD_DIR = ROOT / "apps/desktop/backend/src/thinclaw/runtime_builder"
RUNTIME_KEY = re.compile(r'=>\s*"([A-Z0-9_]+)"')
ALLOWED_BRIDGE_KEYS = {
    "AGENT_AUTO_APPROVE_TOOLS",
    "AGENT_THINKING_BUDGET_TOKENS",
    "AGENT_THINKING_ENABLED",
    "ALLOW_LOCAL_TOOLS",
    "ANTHROPIC_MODEL",
    "DATABASE_BACKEND",
    "HEARTBEAT_ENABLED",
    "HEARTBEAT_INTERVAL_SECS",
    "HEARTBEAT_NOTIFY_CHANNEL",
    "HEARTBEAT_NOTIFY_USER",
    "IRONCLAW_SAFE_BINS_ONLY",
    "LIBSQL_PATH",
    "LLM_BACKEND",
    "LLM_BASE_URL",
    "LLM_MODEL",
    "OPENAI_MODEL",
    "SCREEN_CAPTURE_ENABLED",
    "WORKSPACE_MODE",
    "WORKSPACE_ROOT",
}
SECRET_SHAPE = re.compile(r"(?:TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL)")


def main() -> int:
    problems: list[str] = []
    parent_text = PARENT.read_text(encoding="utf-8")
    child_files = sorted(CHILD_DIR.glob("*.rs")) if CHILD_DIR.exists() else []
    declared = set(re.findall(r"^mod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;", parent_text, re.MULTILINE))
    for child in child_files:
        if child.stem not in declared:
            problems.append(f"uncompiled runtime-builder source: {child.relative_to(ROOT)}")
    for module in declared:
        if not (CHILD_DIR / f"{module}.rs").is_file():
            problems.append(f"declared runtime-builder module is missing: {module}")

    all_builder_text = [parent_text, *(path.read_text(encoding="utf-8") for path in child_files)]
    build_inner_count = sum(
        len(re.findall(r"(?:pub\(crate\)\s+)?async\s+fn\s+build_inner\s*\(", text))
        for text in all_builder_text
    )
    if build_inner_count != 1:
        problems.append(f"expected one compiled build_inner definition, found {build_inner_count}")

    if "struct DesktopRuntimeInputs" not in parent_text:
        problems.append("desktop runtime inputs must use the closed typed projection")
    if "bridge_config.insert" in parent_text:
        problems.append("generic desktop bridge_config insertion is forbidden")
    keys = set(RUNTIME_KEY.findall(parent_text))
    unknown = sorted(keys - ALLOWED_BRIDGE_KEYS)
    missing = sorted(ALLOWED_BRIDGE_KEYS - keys)
    if unknown:
        problems.append(f"unreviewed desktop bridge keys: {unknown}")
    if missing:
        problems.append(f"desktop bridge allowlist drift (missing source keys): {missing}")
    secret_keys = sorted(
        key
        for key in keys
        if SECRET_SHAPE.search(key)
        and not key.endswith("_TOKENS")
        and key != "AGENT_THINKING_BUDGET_TOKENS"
    )
    if secret_keys:
        problems.append(f"secret-shaped desktop bridge keys are forbidden: {secret_keys}")

    bridge = ROOT / "apps/desktop/backend/src/thinclaw/runtime_bridge.rs"
    bridge_text = bridge.read_text(encoding="utf-8")
    if bridge_text.count("super::runtime_builder::build_inner(") != 1:
        problems.append("runtime bridge must delegate to the sole builder exactly once")

    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    print(
        "desktop runtime builder verified: one compiled assembly path, "
        f"{len(child_files)} compiled child modules, {len(keys)} non-secret bridge keys"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

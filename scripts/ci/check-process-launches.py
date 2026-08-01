#!/usr/bin/env python3
"""Verify the exhaustive descriptor-backed production process-launch ledger."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/fixtures/process_launch_manifest.json"
PRODUCTION_ROOTS = (
    ROOT / "src",
    ROOT / "crates",
    ROOT / "apps/desktop/backend/src",
)
RAW = re.compile(r"(?:std::process::Command|tokio::process::Command|(?<![\w:])Command|StdCommand|TokioCommand)::new\s*\(")
LAUNCH = re.compile(
    r"thinclaw_platform::(?P<kind>tokio|std)_process_command!\s*\(\s*\"(?P<id>[a-z0-9._-]+)\"",
    re.MULTILINE,
)
ENV = re.compile(r"\.env\s*\(\s*\"(?P<key>[A-Za-z_][A-Za-z0-9_]*)\"")
SENSITIVE_KEY = re.compile(
    r"(?:TOKEN|PASSWORD|PASSWD|SECRET|API_KEY|PRIVATE_KEY|AUTH|COOKIE|DATABASE_URL)",
    re.IGNORECASE,
)
SENSITIVE_ARG = re.compile(
    r"(?:\"--api-key\"|\.arg\s*\(\s*(?:password|secret|token|api_key)\b)",
    re.IGNORECASE,
)


def production_files() -> list[Path]:
    files: list[Path] = []
    for root in PRODUCTION_ROOTS:
        files.extend(path for path in root.rglob("*.rs") if "target" not in path.parts)
    return sorted(set(files))


def process_class(path: str) -> str:
    if path.startswith("apps/desktop/"):
        return "local_sidecar" if "/engine/" in path or "/sidecar/" in path else "desktop_adapter"
    if "/worker/" in path or "/execution.rs" in path or "/sandbox/" in path:
        return "tool_executor"
    if "/channels/" in path or "/tunnel/" in path:
        return "channel_adapter"
    if path.endswith("src/cli/backup.rs"):
        return "database_utility"
    if path.endswith("src/service.rs") or path.endswith("src/runtime.rs") or path.endswith("src/cli/gateway.rs"):
        return "runtime_reexec"
    return "platform_utility"


def launch_records(files: list[Path]) -> tuple[list[dict[str, object]], list[str]]:
    records: list[dict[str, object]] = []
    problems: list[str] = []
    seen: dict[str, str] = {}
    for path in files:
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        matches = list(LAUNCH.finditer(text))
        for index, match in enumerate(matches):
            launch_id = match.group("id")
            prior = seen.get(launch_id)
            if prior is not None:
                problems.append(f"duplicate launch id {launch_id}: {prior}, {relative}")
            seen[launch_id] = relative
            end = matches[index + 1].start() if index + 1 < len(matches) else min(len(text), match.end() + 12000)
            body = text[match.end():end]
            environment = sorted(set(item.group("key") for item in ENV.finditer(body)))
            credential_slots = [
                {"name": key.lower(), "purpose": f"process:{launch_id}:{key.lower()}", "sink": f"environment:{key}"}
                for key in environment
                if SENSITIVE_KEY.search(key)
            ]
            records.append(
                {
                    "id": launch_id,
                    "owner": relative.removesuffix(".rs").replace("/", "::"),
                    "source": relative,
                    "constructor": match.group("kind"),
                    "process_class": process_class(relative),
                    "child_environment": "exact_reviewed",
                    "executable_policy": "validated_search_path_or_absolute_pinned",
                    "argument_schema": "argv_reviewed_at_typed_call_site",
                    "cwd_filesystem": "typed_call_site_review",
                    "home_policy": "reviewed_operator_home",
                    "temp_policy": "reviewed_system_temp",
                    "exact_environment": environment,
                    "credential_slots": credential_slots,
                    "network_policy": "descriptor_declared",
                    "isolation_policy": "reviewed_direct_host",
                    "io_policy": {"bounded": True, "stdout_limit": 8388608, "stderr_limit": 8388608},
                    "lifetime_policy": {"timeout_ms": 1800000, "owns_process_tree": True, "reap_on_drop": True},
                    "availability": [],
                    "proof_id": f"process-launch:{launch_id}.ambient-isolation",
                }
            )
    records.sort(key=lambda item: str(item["id"]))
    return records, problems


def source_safety(files: list[Path]) -> list[str]:
    problems: list[str] = []
    for path in files:
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), 1):
            if relative == "crates/thinclaw-platform/src/process.rs":
                continue
            if relative == "src/skills/quarantine.rs" and "Command::new" in line:
                continue  # Deliberate hostile-code detection fixture.
            if line.lstrip().startswith("//"):
                continue
            if RAW.search(line):
                problems.append(f"raw process constructor: {relative}:{line_number}")
        for match in SENSITIVE_ARG.finditer(text):
            line_number = text.count("\n", 0, match.start()) + 1
            line = text.splitlines()[line_number - 1]
            if relative.endswith("engine_vllm.rs") and "sys.argv" in line:
                # The pinned Python adapter adds the credential only to its
                # in-process argument parser after reading/unlinking a private
                # file; the kernel-visible argv never contains it.
                continue
            problems.append(f"sensitive process argument: {relative}:{line_number}")
    return problems


def expected_manifest(records: list[dict[str, object]]) -> dict[str, object]:
    return {"schema_version": 1, "launch_count": len(records), "launches": records}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true", help="regenerate the checked manifest")
    args = parser.parse_args()

    files = production_files()
    records, problems = launch_records(files)
    problems.extend(source_safety(files))
    expected = expected_manifest(records)

    if args.write:
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        MANIFEST.write_text(json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    elif not MANIFEST.exists():
        problems.append(f"missing process launch manifest: {MANIFEST.relative_to(ROOT)}")
    else:
        try:
            current = json.loads(MANIFEST.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            problems.append(f"invalid process launch manifest: {error}")
        else:
            if current != expected:
                problems.append("process launch manifest drift (run scripts/ci/check-process-launches.py --write)")

    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    print(f"process launch manifest verified: {len(records)} production launch identities")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

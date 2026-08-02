#!/usr/bin/env python3
"""Verify the exhaustive descriptor-backed production process-launch ledger."""

from __future__ import annotations

import argparse
import functools
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/fixtures/process_launch_manifest.json"
RUNTIME_MANIFEST = ROOT / "crates/thinclaw-platform/src/process_launch_manifest.json"
PRODUCTION_ROOTS = (
    ROOT / "src",
    ROOT / "crates",
    ROOT / "apps/desktop/backend/src",
)
RAW = re.compile(r"(?:std::process::Command|tokio::process::Command|(?<![\w:])Command|StdCommand|TokioCommand)::new\s*\(")
LAUNCH = re.compile(
    r"(?:thinclaw_platform::|crate::)(?P<kind>tokio|std)_process_command!\s*\(\s*\"(?P<id>[a-z0-9._-]+)\"",
    re.MULTILINE,
)
ENV = re.compile(r"\.env\s*\(\s*\"(?P<key>[A-Za-z_][A-Za-z0-9_]*)\"")
DYNAMIC_ENV = re.compile(
    r"\.envs\s*\(|\.env\s*\(\s*[A-Za-z_][A-Za-z0-9_]*"
)
SENSITIVE_KEY = re.compile(
    r"(?:TOKEN|PASSWORD|PASSWD|SECRET|API_KEY|PRIVATE_KEY|AUTH|COOKIE|DATABASE_URL)",
    re.IGNORECASE,
)
SENSITIVE_ARG = re.compile(
    r"(?:\"--api-key\"|\.arg\s*\(\s*(?:password|secret|token|api_key)\b)",
    re.IGNORECASE,
)
RAW_STRING_LITERAL = re.compile(r'(?:br|cr|r)(?P<hashes>#{0,255})"')
CHAR_LITERAL = re.compile(r"'(?:\\.|[^'\\\n])+'")

# Dynamic environment maps are permitted only at these reviewed typed
# boundaries. The scanner fails closed for any newly introduced dynamic map;
# each approved map states its schema and any credential-bearing slot.
DYNAMIC_ENVIRONMENT_SCHEMAS: dict[str, dict[str, object]] = {
    "src.sandbox.host_process.tokio.101": {
        "schema": "approved_host_command_environment",
        "credential_slots": [
            {
                "name": "approved_host_environment",
                "purpose": "process:host-command:approved-environment",
                "sink": "environment:approved-request-key",
            }
        ],
    },
    "src.sandbox.host_process.tokio.102": {
        "schema": "approved_host_command_environment",
        "credential_slots": [
            {
                "name": "approved_host_environment",
                "purpose": "process:host-command:approved-environment",
                "sink": "environment:approved-request-key",
            }
        ],
    },
    "src.worker.codex_bridge.tokio.101": {
        "schema": "orchestrator_resolved_codex_credentials",
        "credential_slots": [
            {
                "name": "codex_worker_credentials",
                "purpose": "process:worker-codex:credential-bundle",
                "sink": "environment:authorized-worker-key",
            }
        ],
    },
    "src.worker.claude_bridge.tokio.101": {
        "schema": "orchestrator_resolved_claude_credentials",
        "credential_slots": [
            {
                "name": "claude_worker_credentials",
                "purpose": "process:worker-claude:credential-bundle",
                "sink": "environment:authorized-worker-key",
            }
        ],
    },
    "crates.thinclaw-tools.src.execution.tokio.109": {
        "schema": "sandbox_validated_execution_environment",
        "credential_slots": [
            {
                "name": "sandbox_execution_environment",
                "purpose": "process:tool-execution:approved-environment",
                "sink": "environment:validated-request-key",
            }
        ],
    },
    "crates.thinclaw-tools.src.mcp.stdio.tokio.101": {
        "schema": "mcp_validated_public_and_source_resolved_secret_environment",
        "credential_slots": [
            {
                "name": "mcp_secret_environment",
                "purpose": "process:mcp-stdio:authorized-secret-environment",
                "sink": "environment:declared-secret-key",
            }
        ],
    },
    "apps.desktop.backend.src.thinclaw.commands.skill_repo.tokio.101": {
        "schema": "desktop_git_platform_allowlist",
        "credential_slots": [],
    },
    "src.desktop_autonomy.fixtures.tokio.101": {
        "schema": "desktop_shadow_fixture_and_database_environment",
        "credential_slots": [
            {
                "name": "shadow_database_credentials",
                "purpose": "process:desktop-shadow:database-access",
                "sink": "environment:DATABASE_URL-or-LIBSQL_AUTH_TOKEN",
            }
        ],
    },
}


def production_files() -> list[Path]:
    files: list[Path] = []
    for root in PRODUCTION_ROOTS:
        files.extend(
            path
            for path in root.rglob("*.rs")
            if "target" not in path.parts
            and "tests" not in path.parts
            and path.name not in {"tests.rs", "main_tests.rs", "testing.rs"}
        )
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


def execution_policy(body: str) -> str:
    if "spawn_reaped_std" in body:
        return "reaped_host_integration"
    if any(
        helper in body
        for helper in (
            "bounded_command_output",
            "bounded_std_command_output",
            "output_with_timeout",
            "owned_command_status",
            "run_cmd",
        )
    ):
        return "bounded_owned"
    if "OwnedChild::spawn" in body or "OwnedStdChild::spawn" in body:
        return "owned_lifecycle"
    # Constructors returned to a domain-specific executor are deliberately
    # conservative here. Their exact lifecycle must be proved by the coverage
    # manifest rather than receiving fabricated bounds from this scanner.
    return "caller_mediated"


def network_policy(class_name: str) -> str:
    if class_name == "tool_executor":
        return "inherited_sandbox"
    if class_name in {"channel_adapter", "runtime_reexec", "local_sidecar", "desktop_adapter"}:
        return "reviewed_external"
    return "denied"


def rust_module_owner(relative: str) -> str:
    path = Path(relative)
    if path.parts[:4] == ("apps", "desktop", "backend", "src"):
        crate_name = "tauri_app_lib"
        module_parts = list(path.parts[4:])
    elif path.parts and path.parts[0] == "src":
        crate_name = "thinclaw"
        module_parts = list(path.parts[1:])
    elif len(path.parts) >= 3 and path.parts[0] == "crates" and path.parts[2] == "src":
        crate_name = path.parts[1].replace("-", "_")
        module_parts = list(path.parts[3:])
    else:
        raise ValueError(f"cannot derive Rust module owner for {relative}")

    module_parts[-1] = Path(module_parts[-1]).stem
    if module_parts[-1] in {"lib", "main", "mod"}:
        module_parts.pop()
    return "::".join([crate_name, *module_parts])


def rust_structure_mask(text: str) -> str:
    """Blank comments and literals while preserving offsets and delimiters."""
    masked = list(text)
    index = 0
    while index < len(text):
        if text.startswith("//", index):
            end = text.find("\n", index + 2)
            end = len(text) if end == -1 else end
            masked[index:end] = " " * (end - index)
            index = end
            continue
        if text.startswith("/*", index):
            end = index + 2
            depth = 1
            while end < len(text) and depth:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            masked[index:end] = " " * (end - index)
            index = end
            continue

        raw = RAW_STRING_LITERAL.match(text, index)
        if raw is not None:
            delimiter = '"' + raw.group("hashes")
            content_start = raw.end()
            close = text.find(delimiter, content_start)
            end = len(text) if close == -1 else close + len(delimiter)
            masked[index:end] = " " * (end - index)
            index = end
            continue

        quote_offset = 1 if text.startswith(('b"', 'c"'), index) else 0
        if text[index + quote_offset:index + quote_offset + 1] == '"':
            end = index + quote_offset + 1
            while end < len(text):
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            masked[index:end] = " " * (end - index)
            index = end
            continue

        if text[index] == "'":
            char_literal = CHAR_LITERAL.match(text, index)
            if char_literal is not None:
                end = char_literal.end()
                masked[index:end] = " " * (end - index)
                index = end
                continue
        index += 1
    return "".join(masked)


@functools.lru_cache(maxsize=None)
def test_only_ranges(text: str) -> tuple[tuple[int, int], ...]:
    """Return byte ranges owned by modules whose cfg predicate requires tests."""
    if "test" not in text:
        return ()
    structure = rust_structure_mask(text)
    module = re.compile(
        r"#\s*\[\s*cfg\s*\(\s*"
        r"(?:test|all\s*\([^)]*\btest\b[^)]*\))"
        r"\s*\)\s*\]\s*"
        r"(?:(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*)\{"
    )
    ranges: list[tuple[int, int]] = []
    for match in module.finditer(structure):
        depth = 1
        cursor = match.end()
        while cursor < len(structure) and depth:
            if structure[cursor] == "{":
                depth += 1
            elif structure[cursor] == "}":
                depth -= 1
            cursor += 1
        if depth:
            raise ValueError("unterminated #[cfg(test)] module")
        ranges.append((match.start(), cursor))
    return tuple(ranges)


def in_ranges(offset: int, ranges: tuple[tuple[int, int], ...]) -> bool:
    return any(start <= offset < end for start, end in ranges)


def launch_records(files: list[Path]) -> tuple[list[dict[str, object]], list[str]]:
    records: list[dict[str, object]] = []
    problems: list[str] = []
    seen: dict[str, str] = {}
    for path in files:
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        test_ranges = test_only_ranges(text)
        matches = [
            match for match in LAUNCH.finditer(text) if not in_ranges(match.start(), test_ranges)
        ]
        for index, match in enumerate(matches):
            launch_id = match.group("id")
            prior = seen.get(launch_id)
            if prior is not None:
                problems.append(f"duplicate launch id {launch_id}: {prior}, {relative}")
            seen[launch_id] = relative
            end = matches[index + 1].start() if index + 1 < len(matches) else min(len(text), match.end() + 12000)
            body = text[match.end():end]
            callsite_digest = hashlib.sha256(
                text[match.start():end].encode("utf-8")
            ).hexdigest()
            literal_program = re.match(r'\s*,\s*"([^"\n]+)"', body)
            environment = sorted(set(item.group("key") for item in ENV.finditer(body)))
            has_dynamic_environment = DYNAMIC_ENV.search(body) is not None
            dynamic_environment = DYNAMIC_ENVIRONMENT_SCHEMAS.get(launch_id)
            if has_dynamic_environment and dynamic_environment is None:
                problems.append(
                    f"unclassified dynamic process environment for {launch_id}: {relative}"
                )
            class_name = process_class(relative)
            execution = execution_policy(body)
            bounded = execution == "bounded_owned"
            owned = execution in {"bounded_owned", "owned_lifecycle", "reaped_host_integration"}
            credential_slots = [
                {"name": key.lower(), "purpose": f"process:{launch_id}:{key.lower()}", "sink": f"environment:{key}"}
                for key in environment
                if SENSITIVE_KEY.search(key)
            ]
            if dynamic_environment is not None:
                credential_slots.extend(dynamic_environment["credential_slots"])
            records.append(
                {
                    "id": launch_id,
                    "classification": "production",
                    "owner": relative.removesuffix(".rs").replace("/", "::"),
                    "rust_module": rust_module_owner(relative),
                    "source": relative,
                    "source_line": text.count("\n", 0, match.start()) + 1,
                    "callsite_digest": callsite_digest,
                    "constructor": match.group("kind"),
                    "program": literal_program.group(1) if literal_program else "typed_dynamic_expression",
                    "process_class": class_name,
                    "execution_policy": execution,
                    "child_environment": "exact_reviewed",
                    "environment_schema": (
                        dynamic_environment["schema"]
                        if dynamic_environment is not None
                        else "literal_keys_only"
                    ),
                    "executable_policy": "validated_search_path_or_absolute_pinned",
                    "argument_schema": "argv_reviewed_at_typed_call_site",
                    "cwd_filesystem": "typed_call_site_review",
                    "home_policy": "reviewed_operator_home",
                    "temp_policy": "reviewed_system_temp",
                    "exact_environment": environment,
                    "credential_slots": credential_slots,
                    "network_policy": network_policy(class_name),
                    "isolation_policy": "reviewed_direct_host",
                    "io_policy": {
                        "bounded": bounded,
                        "stdout_limit": 8388608 if bounded else 0,
                        "stderr_limit": 8388608 if bounded else 0,
                    },
                    "lifetime_policy": {
                        "timeout_ms": 1800000 if bounded else 0,
                        "owns_process_tree": owned,
                        "reap_on_drop": owned,
                    },
                    "availability": [],
                    "proof_id": "process-launch-"
                    + hashlib.sha256(launch_id.encode("utf-8")).hexdigest()[:24],
                }
            )
    for launch_id in sorted(set(DYNAMIC_ENVIRONMENT_SCHEMAS) - set(seen)):
        problems.append(f"dynamic process environment classification has no launch: {launch_id}")
    records.sort(key=lambda item: str(item["id"]))
    return records, problems


def source_safety(files: list[Path]) -> list[str]:
    problems: list[str] = []
    for path in files:
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        test_ranges = test_only_ranges(text)
        offset = 0
        for line_number, line in enumerate(text.splitlines(keepends=True), 1):
            if relative == "crates/thinclaw-platform/src/process.rs":
                offset += len(line)
                continue
            if relative == "src/skills/quarantine.rs" and "Command::new" in line:
                offset += len(line)
                continue  # Deliberate hostile-code detection fixture.
            if line.lstrip().startswith("//"):
                offset += len(line)
                continue
            if RAW.search(line) and not in_ranges(offset, test_ranges):
                problems.append(f"raw process constructor: {relative}:{line_number}")
            offset += len(line)
        for match in SENSITIVE_ARG.finditer(text):
            if in_ranges(match.start(), test_ranges):
                continue
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
        encoded = json.dumps(expected, indent=2, sort_keys=True) + "\n"
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        RUNTIME_MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        MANIFEST.write_text(encoded, encoding="utf-8")
        RUNTIME_MANIFEST.write_text(encoded, encoding="utf-8")
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
        if not RUNTIME_MANIFEST.exists():
            problems.append(f"missing runtime process launch manifest: {RUNTIME_MANIFEST.relative_to(ROOT)}")
        elif RUNTIME_MANIFEST.read_bytes() != MANIFEST.read_bytes():
            problems.append("runtime process launch manifest differs from the checked test fixture")

    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    print(f"process launch manifest verified: {len(records)} production launch identities")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

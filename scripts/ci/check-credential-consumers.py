#!/usr/bin/env python3
"""Check the exhaustive public sensitive-field candidate disposition ledger."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/fixtures/credential_consumer_manifest.json"
SOURCE_ROOTS = (
    ROOT / "src",
    ROOT / "crates",
    ROOT / "apps/desktop/backend/src",
)
PUBLIC_FIELD = re.compile(
    r"^\s*pub(?:\([^)]*\))?\s+(?P<field>[A-Za-z_][A-Za-z0-9_]*(?:token|secret|password|api_key|private_key|credential)[A-Za-z0-9_]*)\s*:\s*(?P<rust_type>[^,\n]+)",
    re.IGNORECASE | re.MULTILINE,
)
PROOF_ID = re.compile(r"^[A-Za-z0-9_-]+$")
ALLOWED_DISPOSITIONS = {
    "source_bound",
    "bootstrap_direct",
    "ephemeral_internal",
    "protocol_sensitive",
    "deliberate_reveal",
    "non_secret_semantic",
}


def production_files() -> list[Path]:
    files: list[Path] = []
    for root in SOURCE_ROOTS:
        files.extend(
            path
            for path in root.rglob("*.rs")
            if "target" not in path.parts
            and "tests" not in path.parts
            and path.name not in {"tests.rs", "main_tests.rs", "testing.rs"}
        )
    return sorted(set(files))


def disposition(
    source: str, field: str, rust_type: str
) -> tuple[str, str, dict[str, object]]:
    name = field.lower()
    normalized_type = re.sub(r"\s+", "", rust_type)
    scalar_type = normalized_type.removeprefix("Option<").removesuffix(">")
    numeric_or_bool = scalar_type in {
        "bool",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "f32",
        "f64",
    }
    semantic_markers = (
        "_count",
        "_limit",
        "_used",
        "_supported",
        "tokenizer",
        "budget_token",
        "cancellation_token",
        "progress_token",
        "awaiting_token",
        "oauth_credential_sync",
        "invalid_private_key",
        "needs_private_key",
    )
    if numeric_or_bool or name.startswith("has_") or name.endswith("_present") or any(
        marker in name for marker in semantic_markers
    ):
        return (
            "non_secret_semantic",
            "typed status, count, capability, or cancellation metadata",
            {
                "persistence": "ordinary_non_secret_metadata",
                "presentation": "allowed",
                "resolution": "not_applicable",
            },
        )
    if name in {"device_token", "push_token", "apns_token", "live_activity_start_token"}:
        return (
            "protocol_sensitive",
            "device delivery token carried only by its authenticated device service",
            {
                "persistence": "protocol_owned_bounded_storage",
                "presentation": "redacted",
                "resolution": "authenticated_device_transport",
            },
        )
    locator_type = any(
        marker in normalized_type
        for marker in (
            "SecretBinding",
            "SecretRef",
            "SecretSourceId",
            "CredentialBinding",
            "CredentialSourceId",
        )
    )
    locator_name = (
        name.endswith(("_secret_name", "_secret_env", "_secret_id", "_secret_source_id"))
        or name.endswith("_env")
        or name in {
            "required_secrets",
            "allowed_secrets",
            "stored_secrets",
            "rotated_secrets",
            "custom_secrets",
        }
    )
    if locator_type or locator_name:
        return (
            "source_bound",
            "purpose-bound secret binding, source identity, or environment slot name",
            {
                "persistence": "opaque_locator_only",
                "presentation": "identifier_only",
                "resolution": "authorized_purpose_bound_resolver",
            },
        )
    if source == "src/platform/gateway_access.rs" or (
        source == "src/desktop_autonomy/types.rs" and name == "one_time_login_secret"
    ):
        return (
            "deliberate_reveal",
            "guarded operator-facing one-use or explicitly confirmed reveal",
            {
                "persistence": "no_general_durable_persistence",
                "presentation": "confirmed_or_one_use_reveal_only",
                "resolution": "guarded_local_operator_boundary",
            },
        )
    if source == "crates/thinclaw-app/src/setup.rs" or source.startswith("src/setup/"):
        return (
            "bootstrap_direct",
            "volatile pre-database setup draft consumed only by the local Apply boundary",
            {
                "persistence": "volatile_draft_until_apply",
                "presentation": "masked",
                "resolution": "local_secure_setup_apply",
            },
        )
    protocol_source = any(
        marker in source
        for marker in (
            "/oauth.rs",
            "/mcp/auth.rs",
            "/native_lifecycle",
            "/wasm/router.rs",
            "/wasm/schema.rs",
            "/wasm/oauth.rs",
            "/channels/src/http.rs",
        )
    )
    protocol_name = any(
        marker in name
        for marker in (
            "access_token",
            "refresh_token",
            "registration_access_token",
            "webhook_secret",
            "signature_secret",
            "verify_token",
            "vapid_private_key",
        )
    )
    if protocol_source and protocol_name:
        return (
            "protocol_sensitive",
            "credential material owned by a bounded authenticated protocol adapter",
            {
                "persistence": "protocol_owned_bounded_storage",
                "presentation": "redacted",
                "resolution": "authenticated_protocol_boundary",
            },
        )
    return (
        "ephemeral_internal",
        "secret-bearing runtime value confined to its owning in-process service",
        {
            "persistence": "memory_only_or_legacy_migration_input",
            "presentation": "redacted",
            "resolution": "ephemeral_owning_service_only",
        },
    )


def proof_id(identity: str) -> str:
    digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:24]
    return f"credential-consumer-{digest}"


def records() -> list[dict[str, object]]:
    result: list[dict[str, object]] = []
    occurrences: dict[tuple[str, str], int] = {}
    for path in production_files():
        relative = path.relative_to(ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        for match in PUBLIC_FIELD.finditer(text):
            field = match.group("field")
            rust_type = " ".join(match.group("rust_type").split())
            key = (relative, field)
            index = occurrences.get(key, 0) + 1
            occurrences[key] = index
            kind, reason, lifecycle = disposition(relative, field, rust_type)
            identity = f"{relative}:{field}:{index}"
            result.append(
                {
                    "id": identity,
                    "source": relative,
                    "field": field,
                    "rust_type": rust_type,
                    "occurrence": index,
                    "disposition": kind,
                    "reason": reason,
                    "lifecycle": lifecycle,
                    "proof_id": proof_id(identity),
                }
            )
    return sorted(result, key=lambda record: str(record["id"]))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    candidates = records()
    problems: list[str] = []
    proof_ids: set[str] = set()
    for candidate in candidates:
        disposition_name = str(candidate["disposition"])
        if disposition_name not in ALLOWED_DISPOSITIONS:
            problems.append(f"unsupported disposition for {candidate['id']}: {disposition_name}")
        candidate_proof = str(candidate["proof_id"])
        if not PROOF_ID.fullmatch(candidate_proof) or candidate_proof in proof_ids:
            problems.append(f"invalid or duplicate proof id for {candidate['id']}: {candidate_proof}")
        proof_ids.add(candidate_proof)
        lifecycle = candidate.get("lifecycle")
        if not isinstance(lifecycle, dict) or set(lifecycle) != {
            "persistence",
            "presentation",
            "resolution",
        }:
            problems.append(f"incomplete lifecycle for {candidate['id']}")
        if disposition_name == "non_secret_semantic" and "SecretString" in str(candidate["rust_type"]):
            problems.append(f"secret-typed field cannot be exempted as semantic: {candidate['id']}")
        if disposition_name == "source_bound" and not (
            "Binding" in str(candidate["rust_type"])
            or "SecretRef" in str(candidate["rust_type"])
            or "SourceId" in str(candidate["rust_type"])
            or str(candidate["field"]).endswith(("_secret_name", "_secret_env", "_secret_id", "_secret_source_id", "_env"))
            or str(candidate["field"]) in {
                "required_secrets",
                "allowed_secrets",
                "stored_secrets",
                "rotated_secrets",
                "custom_secrets",
            }
        ):
            problems.append(f"source-bound field lacks an opaque locator type/name: {candidate['id']}")
        if disposition_name == "bootstrap_direct" and not (
            str(candidate["source"]) == "crates/thinclaw-app/src/setup.rs"
            or str(candidate["source"]).startswith("src/setup/")
        ):
            problems.append(f"bootstrap-direct field is outside local setup: {candidate['id']}")
    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    expected = {
        "schema_version": 1,
        "candidate_count": len(candidates),
        "candidates": candidates,
    }
    if args.write:
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        MANIFEST.write_text(json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    elif not MANIFEST.exists():
        print(f"error: missing {MANIFEST.relative_to(ROOT)}", file=sys.stderr)
        return 1
    else:
        try:
            current = json.loads(MANIFEST.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            print(f"error: invalid credential consumer manifest: {error}", file=sys.stderr)
            return 1
        if current != expected:
            print(
                "error: credential consumer manifest drift "
                "(run scripts/ci/check-credential-consumers.py --write)",
                file=sys.stderr,
            )
            return 1
    print(f"credential consumer manifest verified: {len(candidates)} classified candidates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

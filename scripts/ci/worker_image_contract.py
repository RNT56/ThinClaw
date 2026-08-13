#!/usr/bin/env python3
"""Fail closed when the worker image's immutable and liveness inputs drift."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DIGEST_IMAGE = re.compile(r"^[a-z0-9./_-]+:[^@\s]+@sha256:[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
PINNED_ACTION = re.compile(r"^\s*uses:\s*[^\s@]+@[0-9a-f]{40}(?:\s+#.*)?$", re.MULTILINE)


class WorkerImageContractError(RuntimeError):
    pass


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkerImageContractError(f"cannot read {path}: {error}") from error


def require(condition: bool, message: str) -> None:
    if not condition:
        raise WorkerImageContractError(message)


def check(root: Path = ROOT) -> dict[str, Any]:
    lock_path = root / "release/worker-image-lock.json"
    dockerfile_path = root / "Dockerfile.worker"
    package_path = root / "release/worker-tools/package.json"
    package_lock_path = root / "release/worker-tools/package-lock.json"
    workflow_path = root / ".github/workflows/worker-image.yml"
    health_script_path = root / "scripts/ci/test_worker_image_health.sh"
    source_paths = {
        "health": root / "src/worker/health.rs",
        "worker_mod": root / "src/worker/mod.rs",
        "cli": root / "src/cli/mod.rs",
        "dispatch": root / "src/async_main/command_dispatch.rs",
        "main": root / "src/main.rs",
    }

    lock = read_json(lock_path)
    package = read_json(package_path)
    package_lock = read_json(package_lock_path)
    require(lock.get("schema_version") == 1, "worker image lock schema must be 1")

    try:
        dockerfile = dockerfile_path.read_text(encoding="utf-8")
        workflow = workflow_path.read_text(encoding="utf-8")
        health_script = health_script_path.read_text(encoding="utf-8")
        sources = {name: path.read_text(encoding="utf-8") for name, path in source_paths.items()}
    except OSError as error:
        raise WorkerImageContractError(f"cannot read worker contract input: {error}") from error

    images = lock.get("base_images")
    require(isinstance(images, dict), "base_images must be an object")
    require(set(images) == {"builder", "worker_tools", "runtime"}, "base image roles drifted")
    for role, image in images.items():
        require(isinstance(image, str) and DIGEST_IMAGE.fullmatch(image) is not None,
                f"{role} base image is not tag-and-digest pinned")
        require(f"FROM {image}" in dockerfile, f"Dockerfile does not consume locked {role} image")

    from_lines = [line.strip() for line in dockerfile.splitlines() if line.startswith("FROM ")]
    require(len(from_lines) == 3, "worker Dockerfile must have exactly three build stages")
    require(all("@sha256:" in line for line in from_lines), "every worker FROM must pin a digest")
    for forbidden in ("@latest", "npm install -g", "curl ", "wget ", "sh.rustup.rs",
                      "cli.github.com/packages", "entrypoint.sh", ".thinclaw-alive"):
        require(forbidden not in dockerfile, f"worker Dockerfile contains forbidden input: {forbidden}")

    snapshot = lock.get("debian_snapshot")
    require(isinstance(snapshot, str) and re.fullmatch(r"[0-9]{8}T[0-9]{6}Z", snapshot) is not None,
            "Debian snapshot must be an exact UTC timestamp")
    require(dockerfile.count(f"/20260803T000000Z") == 2 and snapshot == "20260803T000000Z",
            "Dockerfile Debian snapshot drifted from the reviewed lock")
    require("Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg" in dockerfile,
            "Debian snapshot metadata must remain signature verified")

    tools = lock.get("worker_tools")
    require(isinstance(tools, dict), "worker_tools lock must be an object")
    expected_dependencies = {
        "@anthropic-ai/claude-code": tools.get("claude_code"),
        "@openai/codex": tools.get("codex"),
    }
    require(package.get("dependencies") == expected_dependencies,
            "worker tool package versions drifted from the worker lock")
    require(package.get("engines", {}).get("node") == tools.get("node"),
            "worker Node engine drifted from the worker lock")
    require(package_lock.get("lockfileVersion") == tools.get("npm_lockfile_version") == 3,
            "worker npm lockfile version drifted")
    root_package = package_lock.get("packages", {}).get("")
    require(isinstance(root_package, dict) and root_package.get("dependencies") == expected_dependencies,
            "worker npm lock root dependencies drifted")
    require(root_package.get("engines", {}).get("node") == tools.get("node"),
            "worker npm lock Node engine drifted")
    packages = package_lock.get("packages")
    require(isinstance(packages, dict) and len(packages) >= 10,
            "worker npm lock does not contain the platform packages")
    for name, metadata in packages.items():
        if name == "":
            continue
        require(isinstance(metadata, dict) and isinstance(metadata.get("version"), str),
                f"{name} lacks an exact locked version")
        require(isinstance(metadata.get("integrity"), str) and metadata["integrity"].startswith("sha512-"),
                f"{name} lacks npm integrity verification")
        require(str(metadata.get("resolved", "")).startswith("https://registry.npmjs.org/"),
                f"{name} is not fetched from the reviewed npm registry")
    require("npm ci --omit=dev --ignore-scripts" in dockerfile,
            "worker tools must install from the lock without package lifecycle scripts")
    require("claude-code-linux-${worker_arch}/claude" in dockerfile,
            "Claude native binary must be selected explicitly from its locked platform package")

    health = lock.get("health")
    require(isinstance(health, dict), "health lock must be an object")
    heartbeat_file = health.get("heartbeat_file")
    require(dockerfile.count(str(heartbeat_file)) == 1, "Docker health file drifted")
    require(
        f'HEALTHCHECK --interval={health.get("docker_interval_seconds")}s '
        f'--timeout={health.get("docker_timeout_seconds")}s '
        f'--start-period={health.get("docker_start_period_seconds")}s '
        f'--retries={health.get("docker_retries")}' in dockerfile,
        "Docker health timing drifted",
    )
    require(f'"--max-age", "{health.get("max_age_seconds")}"' in dockerfile,
            "Docker maximum heartbeat age drifted")
    require(f'DEFAULT_HEARTBEAT_FILE: &str = "{heartbeat_file}"' in sources["health"],
            "Rust heartbeat path drifted")
    require(f'Duration::from_secs({health.get("heartbeat_interval_seconds")})' in sources["health"],
            "Rust heartbeat interval drifted")
    require(f'Duration::from_secs({health.get("max_age_seconds")})' in sources["health"],
            "Rust heartbeat maximum age drifted")
    for proof in ("symlink_metadata", "NotRegularFile", "FutureTimestamp", "Stale"):
        require(proof in sources["health"], f"health checker lost fail-closed proof: {proof}")
    require("pub mod health;" in sources["worker_mod"], "worker health module is not exported")
    for command in ("worker-health", "worker-health-loop"):
        require(command in sources["cli"], f"missing internal {command} command")
    for runtime in ("Command::Worker { .. }", "Command::ClaudeBridge { .. }",
                    "Command::CodexBridge { .. }", "Command::NetworkRelay { .. }",
                    "Command::WorkerHealthLoop { .. }"):
        require(runtime in sources["dispatch"], f"heartbeat does not cover {runtime}")
    require("WorkerHeartbeat::start_default()" in sources["dispatch"],
            "worker dispatch does not start the event-loop heartbeat")
    require("Command::WorkerHealthLoop { .. }) => RuntimeCommandIntent::WorkerRuntime" in sources["main"],
            "health loop does not use the credential-free worker bootstrap")

    evidence = lock.get("evidence")
    require(isinstance(evidence, dict), "worker evidence lock must be an object")
    required_workflow = (
        "provenance: mode=max",
        "sbom: true",
        f'syft-version: v{evidence.get("syft")}',
        f'version: v{evidence.get("trivy")}',
        'severity: "HIGH,CRITICAL"',
        'exit-code: "1"',
        "ignore-unfixed: true",
        "test_worker_image_health.sh",
        "worker-image-build-metadata.json",
        "worker-image.cdx.json",
        "worker-image-trivy.sarif",
    )
    for fragment in required_workflow:
        require(fragment in workflow, f"worker workflow is missing evidence gate: {fragment}")
    uses_lines = [line for line in workflow.splitlines() if line.lstrip().startswith("uses:")]
    require(uses_lines and all(PINNED_ACTION.fullmatch(line) for line in uses_lines),
            "worker workflow actions must be full commit pins")
    require("secrets." not in workflow, "worker image validation must not consume credentials")
    for proof in ("worker-health-loop", "--active", "--signal STOP", "--signal CONT", "unhealthy"):
        require(proof in health_script, f"worker health smoke lacks proof: {proof}")

    rollback = lock.get("rollback")
    require(isinstance(rollback, dict), "worker rollback record must be an object")
    previous_digest = rollback.get("previous_image_digest")
    if previous_digest is None:
        require(rollback.get("status") == "source-only",
                "a missing predecessor digest must be explicitly source-only")
        require("No thinclaw-worker container package" in str(rollback.get("reason")),
                "source-only rollback must explain why no predecessor digest exists")
    else:
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", previous_digest) is not None,
                "previous worker image digest is invalid")
    require(COMMIT.fullmatch(str(rollback.get("previous_source_commit", ""))) is not None,
            "previous worker source commit is invalid")
    require(SHA256.fullmatch(str(rollback.get("previous_dockerfile_sha256", ""))) is not None,
            "previous worker Dockerfile checksum is invalid")

    return {
        "base_images": len(images),
        "npm_packages": len(packages) - 1,
        "heartbeat_modes": 5,
        "rollback": rollback.get("status"),
    }


def main() -> int:
    try:
        result = check(ROOT)
    except WorkerImageContractError as error:
        print(f"worker image contract failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env bash
# Deterministic Desktop dual-mode acceptance gate (TDO-030).
set -euo pipefail

manifest="apps/desktop/backend/Cargo.toml"

run_fixture() {
  local test_name="$1"
  local output
  output="$(cargo test --manifest-path "$manifest" --locked --lib "$test_name" -- --exact --nocapture 2>&1)"
  printf '%s\n' "$output"
  if ! grep -q 'test result: ok. 1 passed; 0 failed' <<<"$output"; then
    echo "Fixture acceptance did not execute exactly one passing test: $test_name" >&2
    exit 1
  fi
}

run_fixture 'thinclaw::bridge::tests::fixture_acceptance_local_route_contract'
run_fixture 'thinclaw::remote_proxy::tests::fixture_acceptance_remote_chat_and_session_routes'
run_fixture 'thinclaw::remote_proxy::tests::fixture_acceptance_remote_management_routes'

echo 'Desktop fixture acceptance passed for local and remote modes.'

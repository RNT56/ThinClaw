#!/usr/bin/env bash
# Deterministic Desktop dual-mode acceptance gate (TDO-030).
set -euo pipefail

manifest="apps/desktop/backend/Cargo.toml"

# These fixtures exercise bridge routing and proxy contracts only; they do not
# open the embedded runtime database. Build the Desktop crate against the
# PostgreSQL runtime profile so the Linux test linker does not combine the
# independent bundled SQLite implementations from libsql and sqlx. The libsql
# runtime itself is covered by the dedicated profile and DB-contract jobs.
desktop_test_features="llamacpp,runtime-postgres"

run_fixture() {
  local test_name="$1"
  local output
  local status=0
  if output="$(
    CARGO_PROFILE_TEST_DEBUG=0 \
      cargo test --manifest-path "$manifest" --locked --no-default-features \
        --features "$desktop_test_features" --lib "$test_name" -- --exact --nocapture 2>&1
  )"; then
    status=0
  else
    status=$?
  fi
  printf '%s\n' "$output"
  if (( status != 0 )); then
    echo "Fixture acceptance command failed: $test_name" >&2
    exit "$status"
  fi
  if ! grep -q 'test result: ok. 1 passed; 0 failed' <<<"$output"; then
    echo "Fixture acceptance did not execute exactly one passing test: $test_name" >&2
    exit 1
  fi
}

run_fixture 'thinclaw::bridge::tests::fixture_acceptance_local_route_contract'
run_fixture 'thinclaw::remote_proxy::tests::fixture_acceptance_remote_chat_and_session_routes'
run_fixture 'thinclaw::remote_proxy::tests::fixture_acceptance_remote_management_routes'

echo 'Desktop fixture acceptance passed for local and remote modes.'

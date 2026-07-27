#!/usr/bin/env bash
set -euo pipefail

SCRIPT="scripts/run_default_engine.sh"

expect_engine() {
  local expected="$1"
  shift
  local actual
  actual="$(env "$@" bash "$SCRIPT" print)"
  if [[ "$actual" != "$expected" ]]; then
    echo "Expected engine '$expected', got '$actual'" >&2
    exit 1
  fi
}

expect_engine mlx \
  THINCLAW_ENGINE_HOST_OS=Darwin \
  THINCLAW_ENGINE_HOST_ARCH=arm64
expect_engine llamacpp \
  THINCLAW_ENGINE_HOST_OS=Darwin \
  THINCLAW_ENGINE_HOST_ARCH=x86_64
expect_engine llamacpp \
  THINCLAW_ENGINE_HOST_OS=Linux \
  THINCLAW_ENGINE_HOST_ARCH=x86_64
expect_engine mlx \
  THINCLAW_ENGINE_HOST_OS=Linux \
  THINCLAW_ENGINE_HOST_ARCH=x86_64 \
  TAURI_TARGET_TRIPLE=aarch64-apple-darwin
expect_engine ollama \
  THINCLAW_ENGINE_HOST_OS=Darwin \
  THINCLAW_ENGINE_HOST_ARCH=arm64 \
  THINCLAW_DESKTOP_ENGINE=ollama
expect_engine llamacpp \
  THINCLAW_ENGINE_HOST_OS=Darwin \
  THINCLAW_ENGINE_HOST_ARCH=arm64 \
  THINCLAW_DESKTOP_ENGINE=llamacpp

if THINCLAW_DESKTOP_ENGINE=invalid bash "$SCRIPT" print >/dev/null 2>&1; then
  echo "Invalid engine override unexpectedly succeeded" >&2
  exit 1
fi

echo "Default desktop engine selection tests passed."

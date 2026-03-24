#!/usr/bin/env bash
# Build ThinClaw and all bundled channels.
#
# Run this before release or when channel sources have changed.
# The main binary bundles telegram.wasm via build.rs; it must exist.
#
# Usage:
#   ./scripts/build-all.sh               # Standard build (download-based extensions)
#   ./scripts/build-all.sh --bundled     # Air-gapped build (all WASM embedded in binary)

set -euo pipefail

cd "$(dirname "$0")/.."

EXTRA_FEATURES=""
if [[ "${1:-}" == "--bundled" ]]; then
    echo "Building with bundled-wasm feature (all WASM extensions embedded)..."
    EXTRA_FEATURES="--features bundled-wasm"
else
    echo "Building bundled channels..."
    if [ -d "channels-src/telegram" ]; then
        ./channels-src/telegram/build.sh
    fi
fi

echo ""
echo "Building ThinClaw..."
cargo build --release $EXTRA_FEATURES

echo ""
if [[ "${1:-}" == "--bundled" ]]; then
    echo "Done. Binary (bundled): target/release/thinclaw"
    echo "All WASM extensions are embedded — no network required for install."
else
    echo "Done. Binary: target/release/thinclaw"
fi

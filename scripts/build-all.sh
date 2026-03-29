#!/usr/bin/env bash
# Build ThinClaw and all bundled WASM channels.
#
# Run this before release or when channel sources have changed.
#
# Usage:
#   ./scripts/build-all.sh               # Standard build (download-based extensions)
#   ./scripts/build-all.sh --bundled     # Air-gapped build (all WASM embedded in binary)

set -euo pipefail

cd "$(dirname "$0")/.."

CHANNELS_DIR="channels-src"
DEPLOY_DIR="${HOME}/.thinclaw/channels"

# Generic WASM channel builder for channels without their own build.sh
build_wasm_channel() {
    local name="$1"
    local dir="${CHANNELS_DIR}/${name}"

    if [ ! -d "$dir" ]; then
        echo "  ⏭  ${name}: not found, skipping"
        return 0
    fi

    # Use channel's own build.sh if it exists
    if [ -f "${dir}/build.sh" ]; then
        echo "  🔨 ${name}: using build.sh"
        bash "${dir}/build.sh"
        return $?
    fi

    # Generic build: cargo build → wasm-tools component new → strip
    echo "  🔨 ${name}: cargo build + wasm-tools"
    (
        cd "$dir"
        cargo build --release --target wasm32-wasip2 2>&1

        local crate_name
        crate_name=$(grep '^name' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/' | tr '-' '_')
        local wasm_path="target/wasm32-wasip2/release/${crate_name}.wasm"

        if [ ! -f "$wasm_path" ]; then
            echo "  ❌ ${name}: WASM output not found at ${wasm_path}"
            return 1
        fi

        wasm-tools component new "$wasm_path" -o "${name}.wasm" 2>/dev/null \
            || cp "$wasm_path" "${name}.wasm"
        wasm-tools strip "${name}.wasm" -o "${name}.wasm"
        echo "  ✅ ${name}: $(du -h "${name}.wasm" | cut -f1)"
    )
}

# Deploy built WASM + capabilities to ~/.thinclaw/channels/
deploy_channel() {
    local name="$1"
    local dir="${CHANNELS_DIR}/${name}"

    mkdir -p "$DEPLOY_DIR"

    if [ -f "${dir}/${name}.wasm" ]; then
        cp "${dir}/${name}.wasm" "${DEPLOY_DIR}/"
    fi
    if [ -f "${dir}/${name}.capabilities.json" ]; then
        cp "${dir}/${name}.capabilities.json" "${DEPLOY_DIR}/"
    fi
}

EXTRA_FEATURES=""
if [[ "${1:-}" == "--bundled" ]]; then
    echo "Building with bundled-wasm feature (all WASM extensions embedded)..."
    EXTRA_FEATURES="--features bundled-wasm"
else
    echo "Building WASM channels..."
    echo ""

    for channel_dir in "${CHANNELS_DIR}"/*/; do
        channel=$(basename "$channel_dir")
        build_wasm_channel "$channel"
        deploy_channel "$channel"
    done

    echo ""
    echo "Deployed WASM channels to ${DEPLOY_DIR}/"
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
    echo "WASM channels deployed to: ${DEPLOY_DIR}/"
fi


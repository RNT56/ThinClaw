#!/usr/bin/env bash
# Build the Brave Search tool WASM component
#
# Prerequisites:
#   - cargo-component: cargo install cargo-component
#
# Output:
#   - brave-search.wasm - WASM component ready for deployment
#   - brave-search.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Brave Search tool WASM component..."

# Build using cargo-component (handles WIT component model automatically)
cargo component build --release

WASM_PATH="target/wasm32-wasip1/release/brave_search_tool.wasm"

if [ -f "$WASM_PATH" ]; then
    cp "$WASM_PATH" brave-search.wasm

    echo "Built: brave-search.wasm ($(du -h brave-search.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.thinclaw/tools"
    echo "  cp brave-search.wasm ~/.thinclaw/tools/brave-search.wasm"
    echo "  cp brave-search-tool.capabilities.json ~/.thinclaw/tools/brave-search.capabilities.json"
    echo ""
    echo "Then authenticate:"
    echo "  thinclaw tool auth brave-search"
    echo ""
    echo "Or set via environment variable:"
    echo "  export BRAVE_SEARCH_API_KEY=your-key-here"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi


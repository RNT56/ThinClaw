#!/usr/bin/env bash
# Vendors the committed gateway OpenAPI spec and regenerates the Swift client.
#
# The spec is produced by the Rust side (`cargo run --bin export-openapi -- generate`
# from the repo root) and committed at clients/openapi/thinclaw-gateway.openapi.json.
# Generated Swift is committed under Packages/ThinClawAPI/Sources/ThinClawAPI/Generated/
# so CI never needs the Rust toolchain.
set -euo pipefail

IOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$IOS_ROOT/../.." && pwd)"
SPEC_SRC="$REPO_ROOT/clients/openapi/thinclaw-gateway.openapi.json"
SPEC_DST="$IOS_ROOT/Packages/ThinClawAPI/openapi/openapi.json"
GEN_DIR="$IOS_ROOT/Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"
CONFIG="$IOS_ROOT/Packages/ThinClawAPI/openapi/openapi-generator-config.yaml"

if [[ ! -f "$SPEC_SRC" ]]; then
    echo "error: missing $SPEC_SRC — run 'cargo run --bin export-openapi -- generate' at the repo root" >&2
    exit 1
fi

cp "$SPEC_SRC" "$SPEC_DST"

if [[ ! -f "$CONFIG" ]]; then
    cat > "$CONFIG" <<'YAML'
generate:
  - types
  - client
accessModifier: public
YAML
fi

mkdir -p "$GEN_DIR"
if command -v mise >/dev/null 2>&1; then
    (cd "$IOS_ROOT" && mise exec -- swift-openapi-generator generate \
        --config "$CONFIG" \
        --output-directory "$GEN_DIR" \
        "$SPEC_DST")
else
    echo "error: mise not found — install mise, then 'mise install' in apps/ios" >&2
    exit 1
fi

echo "regenerated $GEN_DIR from $(basename "$SPEC_SRC")"

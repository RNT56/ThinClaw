#!/usr/bin/env bash
# Vendors the committed gateway OpenAPI spec and regenerates the Swift client.
#
# The spec is produced by the Rust side (`cargo run --example export-openapi -- generate`
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
    echo "error: missing $SPEC_SRC — run 'cargo run --example export-openapi -- generate' at the repo root" >&2
    exit 1
fi

cp "$SPEC_SRC" "$SPEC_DST"

if [[ ! -f "$CONFIG" ]]; then
    echo "error: missing $CONFIG — the generator config (modes, access level," >&2
    echo "       and the REST-only operation filter) is committed and must not" >&2
    echo "       be regenerated ad hoc; restore it from git" >&2
    exit 1
fi

mkdir -p "$GEN_DIR"
if command -v mise >/dev/null 2>&1; then
    # `mise exec --` puts the spm-backed generator binary on PATH (installs/
    # spm-apple-swift-openapi-generator/<ver>/bin/swift-openapi-generator).
    # The committed config selects modes (types+client), access level, naming
    # strategy, and the REST-only operation filter — keep those in the config,
    # not on the command line, so generation stays reproducible.
    (cd "$IOS_ROOT" && mise exec -- swift-openapi-generator generate \
        --config "$CONFIG" \
        --output-directory "$GEN_DIR" \
        "$SPEC_DST")
else
    echo "error: mise not found — install mise, then 'mise install' in apps/ios" >&2
    exit 1
fi

# The generated sources deliberately do not conform to the repo's .swift-format
# style (long operationId comments, generator indentation, unsorted @_spi
# imports). CI lints Packages/ recursively, so mark each generated file
# swift-format-ignore. Injecting the header here (rather than editing the files
# by hand) keeps generation the single source of truth: check-generated-drift.sh
# regenerates verbatim and the header is reproduced every time.
IGNORE_HEADER='// swift-format-ignore-file'
for f in "$GEN_DIR"/*.swift; do
    [[ -f "$f" ]] || continue
    if [[ "$(head -n 1 "$f")" != "$IGNORE_HEADER" ]]; then
        tmp="$(mktemp)"
        printf '%s\n' "$IGNORE_HEADER" >"$tmp"
        cat "$f" >>"$tmp"
        mv "$tmp" "$f"
    fi
done

echo "regenerated $GEN_DIR from $(basename "$SPEC_SRC")"

#!/usr/bin/env bash
# Fails if the committed generated Swift client is stale relative to the
# committed OpenAPI spec. Run by CI (ios.yml).
set -euo pipefail

IOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GEN_DIR="$IOS_ROOT/Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"

# Grace period: until the first generation lands, only READMEs live here.
if ! ls "$GEN_DIR"/*.swift >/dev/null 2>&1; then
    echo "no generated sources yet — skipping drift check (generation lands with M1)"
    exit 0
fi

"$IOS_ROOT/scripts/generate-api.sh"

# Only the paths generate-api.sh actually (re)writes count as drift: the
# vendored spec snapshot and the generated Swift sources. Hand-authored files in
# the same package (e.g. GatewayClient.swift convenience wrappers) must not trip
# this check — they are reviewed as normal source, not regenerated.
GENERATED_PATHS=(
    "Packages/ThinClawAPI/openapi/openapi.json"
    "Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"
)
if ! git -C "$IOS_ROOT" diff --quiet -- "${GENERATED_PATHS[@]}"; then
    echo "error: generated client is stale; run apps/ios/scripts/generate-api.sh and commit" >&2
    git -C "$IOS_ROOT" diff --stat -- "${GENERATED_PATHS[@]}" >&2
    exit 1
fi
echo "generated client is up to date"

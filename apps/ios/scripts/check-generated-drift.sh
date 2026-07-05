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

if ! git -C "$IOS_ROOT" diff --quiet -- "Packages/ThinClawAPI"; then
    echo "error: generated client is stale; run apps/ios/scripts/generate-api.sh and commit" >&2
    git -C "$IOS_ROOT" diff --stat -- "Packages/ThinClawAPI" >&2
    exit 1
fi
echo "generated client is up to date"

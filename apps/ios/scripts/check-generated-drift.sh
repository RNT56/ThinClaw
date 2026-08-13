#!/usr/bin/env bash
# Fails if the committed generated Swift client is stale relative to the
# committed OpenAPI spec. Run by CI (ios.yml).
set -euo pipefail

IOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GEN_DIR="$IOS_ROOT/Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"
REPO_ROOT="$(cd "$IOS_ROOT/../.." && pwd)"
CONTRACT_CHECKER="$REPO_ROOT/scripts/ci/contract_drift.py"

python3 "$CONTRACT_CHECKER" check-swift

"$IOS_ROOT/scripts/generate-api.sh"

# Only the paths generate-api.sh actually (re)writes count as drift: the
# vendored spec snapshot and the generated Swift sources. Hand-authored files in
# the same package (e.g. GatewayClient.swift convenience wrappers) must not trip
# this check — they are reviewed as normal source, not regenerated.
GENERATED_PATHS=(
    "Packages/ThinClawAPI/openapi/openapi.json"
    "Packages/ThinClawAPI/openapi/generated-client-manifest.json"
    "Packages/ThinClawAPI/Sources/ThinClawAPI/Generated"
)
if ! git -C "$IOS_ROOT" diff --quiet -- "${GENERATED_PATHS[@]}"; then
    echo "error: generated client is stale; run apps/ios/scripts/generate-api.sh and commit" >&2
    git -C "$IOS_ROOT" diff --stat -- "${GENERATED_PATHS[@]}" >&2
    exit 1
fi
echo "generated client is up to date"

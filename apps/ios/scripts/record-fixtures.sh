#!/usr/bin/env bash
# Dev-only: records a live gateway SSE stream into the ThinClawTransport test
# fixtures. Requires a locally running gateway (cargo run at the repo root)
# and GATEWAY_AUTH_TOKEN in the environment. Never run in CI — fixtures are
# committed and reviewed.
set -euo pipefail

GATEWAY_URL="${GATEWAY_URL:-http://127.0.0.1:3000}"
OUT="${1:?usage: record-fixtures.sh <output-name.sse> [seconds]}"
SECONDS_TO_RECORD="${2:-30}"

IOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="$IOS_ROOT/Packages/ThinClawTransport/Tests/ThinClawTransportTests/Fixtures"

: "${GATEWAY_AUTH_TOKEN:?set GATEWAY_AUTH_TOKEN}"

echo "recording $SECONDS_TO_RECORD s of $GATEWAY_URL/api/chat/events -> $FIXTURE_DIR/$OUT"
curl --silent --no-buffer --max-time "$SECONDS_TO_RECORD" \
    -H "Authorization: Bearer $GATEWAY_AUTH_TOKEN" \
    "$GATEWAY_URL/api/chat/events" > "$FIXTURE_DIR/$OUT" || true
wc -c "$FIXTURE_DIR/$OUT"

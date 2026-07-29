#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ROOT="${1:-backend/target/release/bundle/macos}"
DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$DESKTOP_DIR/engine-manifest.json"

shopt -s nullglob
APPS=("$BUNDLE_ROOT"/*.app)
shopt -u nullglob
if (( ${#APPS[@]} != 1 )); then
  echo "Expected exactly one macOS app in $BUNDLE_ROOT, found ${#APPS[@]}" >&2
  exit 1
fi

APP="${APPS[0]}"
UV="$APP/Contents/MacOS/uv"
INFO="$APP/Contents/Info.plist"
EXPECTED_UV="$(node -e 'process.stdout.write(require(process.argv[1]).uv.version)' "$MANIFEST")"
EXPECTED_MINIMUM="$(node -e 'process.stdout.write(require(process.argv[1]).engines.mlx.minimumMacosVersion)' "$MANIFEST")"

[[ -x "$UV" ]] || { echo "Packaged uv sidecar is missing or not executable: $UV" >&2; exit 1; }
[[ "$("$UV" --version)" == "uv $EXPECTED_UV"* ]] || {
  echo "Packaged uv does not match engine-manifest.json" >&2
  exit 1
}
[[ "$(lipo -archs "$UV")" == *arm64* ]] || { echo "Packaged uv is not arm64" >&2; exit 1; }
[[ ! -e "$APP/Contents/MacOS/llama-server" ]] || {
  echo "MLX bundle unexpectedly contains llama-server" >&2
  exit 1
}

ACTUAL_MINIMUM="$(plutil -extract LSMinimumSystemVersion raw "$INFO")"
[[ "$ACTUAL_MINIMUM" == "$EXPECTED_MINIMUM" ]] || {
  echo "Expected macOS minimum $EXPECTED_MINIMUM, got $ACTUAL_MINIMUM" >&2
  exit 1
}

codesign --verify --deep --strict "$APP"
echo "Verified MLX macOS app bundle: $APP"

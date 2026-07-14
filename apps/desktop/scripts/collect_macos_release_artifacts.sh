#!/usr/bin/env bash
set -euo pipefail

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DESKTOP_DIR"

: "${TAURI_RELEASE_VERSION:?TAURI_RELEASE_VERSION is required}"
: "${TAURI_RELEASE_TAG:?TAURI_RELEASE_TAG is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

BUNDLE_ROOT="${TAURI_BUNDLE_ROOT:-backend/target/release/bundle}"
OUTPUT_DIR="${TAURI_RELEASE_OUTPUT_DIR:-backend/target/release-artifacts}"
VERIFY_APPLE="${VERIFY_APPLE_ARTIFACTS:-0}"

find_one() {
  local label="$1"
  shift
  local value
  value="$(find "$@" -print -quit 2>/dev/null || true)"
  if [[ -z "$value" ]]; then
    echo "Missing $label under $BUNDLE_ROOT" >&2
    exit 1
  fi
  printf '%s\n' "$value"
}

APP="$(find_one 'macOS .app bundle' "$BUNDLE_ROOT/macos" -maxdepth 1 -type d -name '*.app')"
DMG="$(find_one 'notarized DMG' "$BUNDLE_ROOT/dmg" -maxdepth 1 -type f -name '*.dmg')"
UPDATER="$(find_one 'macOS updater archive' "$BUNDLE_ROOT/macos" -maxdepth 1 -type f -name '*.app.tar.gz')"
SIGNATURE="${UPDATER}.sig"
if [[ ! -s "$SIGNATURE" ]]; then
  echo "Missing or empty updater signature: $SIGNATURE" >&2
  exit 1
fi

if [[ "$VERIFY_APPLE" == "1" ]]; then
  [[ "$(uname -s)" == "Darwin" ]] || { echo 'Apple verification requires macOS.' >&2; exit 1; }
  codesign --verify --deep --strict --verbose=2 "$APP"
  CODESIGN_DETAILS="$(codesign --display --verbose=4 "$APP" 2>&1)"
  if ! grep -Eq 'flags=.*runtime' <<<"$CODESIGN_DETAILS"; then
    echo 'The app signature is missing the hardened-runtime flag.' >&2
    exit 1
  fi
  spctl --assess --type execute --verbose=4 "$APP"
  spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"
  xcrun stapler validate "$APP"
  xcrun stapler validate "$DMG"
fi

case "$(uname -m)" in
  arm64|aarch64) ARCH="aarch64" ;;
  x86_64|amd64) ARCH="x86_64" ;;
  *) echo "Unsupported release architecture: $(uname -m)" >&2; exit 1 ;;
esac

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"
cp "$DMG" "$OUTPUT_DIR/"
cp "$UPDATER" "$OUTPUT_DIR/"
cp "$SIGNATURE" "$OUTPUT_DIR/"
COPIED_UPDATER="$OUTPUT_DIR/$(basename "$UPDATER")"
COPIED_SIGNATURE="$OUTPUT_DIR/$(basename "$SIGNATURE")"

node scripts/create_tauri_update_manifest.mjs \
  --version "$TAURI_RELEASE_VERSION" \
  --tag "$TAURI_RELEASE_TAG" \
  --repository "$GITHUB_REPOSITORY" \
  --arch "$ARCH" \
  --artifact "$COPIED_UPDATER" \
  --signature "$COPIED_SIGNATURE" \
  --output "$OUTPUT_DIR/latest.json"

echo "Collected notarized Desktop release artifacts:"
find "$OUTPUT_DIR" -maxdepth 1 -type f -print

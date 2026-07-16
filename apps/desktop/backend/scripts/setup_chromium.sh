#!/usr/bin/env bash
set -euo pipefail

# Chromium snapshot validated for ThinClaw Desktop on 2026-07-14. Keep the
# revision and checksums together so a changed archive can never be installed
# silently. Environment overrides exist for deterministic fixture tests and
# deliberate release-operator upgrades.
DEFAULT_REVISION="1313161"
CHROMIUM_REVISION="${CHROMIUM_REVISION:-$DEFAULT_REVISION}"
TARGET_DIR="${CHROMIUM_TARGET_DIR:-backend/resources/chromium}"
ARCH="${CHROMIUM_ARCH:-$(uname -m)}"

case "$ARCH" in
  arm64|aarch64)
    DEFAULT_PLATFORM="Mac_Arm"
    DEFAULT_SHA256="98173187fab109a1e3431806811e95bff38e61682bbeaf8776733b5a378515ab"
    ;;
  x86_64|amd64)
    DEFAULT_PLATFORM="Mac"
    DEFAULT_SHA256="bb95467ac4b4097833f707e58f079926a9cece4824c700da335c38081c7a4e5b"
    ;;
  *)
    echo "Unsupported architecture for the bundled macOS Chromium snapshot: $ARCH" >&2
    exit 1
    ;;
esac

PLATFORM="${CHROMIUM_PLATFORM:-$DEFAULT_PLATFORM}"
ARCHIVE_NAME="${CHROMIUM_ARCHIVE_NAME:-chrome-mac.zip}"
DOWNLOAD_URL="${CHROMIUM_DOWNLOAD_URL:-https://storage.googleapis.com/chromium-browser-snapshots/${PLATFORM}/${CHROMIUM_REVISION}/${ARCHIVE_NAME}}"

if [[ "$CHROMIUM_REVISION" == "$DEFAULT_REVISION" && "$PLATFORM" == "$DEFAULT_PLATFORM" ]]; then
  EXPECTED_SHA256="${CHROMIUM_SHA256:-$DEFAULT_SHA256}"
elif [[ -n "${CHROMIUM_SHA256:-}" ]]; then
  EXPECTED_SHA256="$CHROMIUM_SHA256"
else
  echo "A custom Chromium revision/platform requires CHROMIUM_SHA256." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-chromium.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
ARCHIVE="$TMP_DIR/$ARCHIVE_NAME"
EXTRACT_DIR="$TMP_DIR/extracted"
mkdir -p "$EXTRACT_DIR" "$(dirname "$TARGET_DIR")"

echo "Downloading Chromium r${CHROMIUM_REVISION} for $PLATFORM..."
curl --fail --show-error --location --retry 3 --retry-all-errors \
  --output "$ARCHIVE" "$DOWNLOAD_URL"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA256="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
else
  ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
fi
if [[ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]]; then
  echo "Chromium archive checksum mismatch: expected $EXPECTED_SHA256, got $ACTUAL_SHA256" >&2
  exit 1
fi

unzip -q "$ARCHIVE" -d "$EXTRACT_DIR"
APP_PATH="$EXTRACT_DIR/chrome-mac/Chromium.app"
EXECUTABLE_PATH="$APP_PATH/Contents/MacOS/Chromium"
if [[ ! -f "$EXECUTABLE_PATH" ]]; then
  echo "Chromium archive is missing $EXECUTABLE_PATH" >&2
  exit 1
fi
chmod +x "$EXECUTABLE_PATH"

if [[ "$(uname -s)" == "Darwin" ]]; then
  xattr -cr "$APP_PATH"
fi

# Replace only after the archive has passed checksum and layout validation.
rm -rf "$TARGET_DIR"
mv "$EXTRACT_DIR" "$TARGET_DIR"

echo "Installed verified Chromium r${CHROMIUM_REVISION} at $TARGET_DIR/chrome-mac/Chromium.app"

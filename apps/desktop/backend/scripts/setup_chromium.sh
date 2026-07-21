#!/usr/bin/env bash
set -euo pipefail

# Chromium snapshot validated for ThinClaw Desktop on 2026-07-14. Keep the
# revision and checksums together so a changed archive can never be installed
# silently. Environment overrides exist for deterministic fixture tests and
# deliberate release-operator upgrades.
DEFAULT_REVISION="1313161"
CHROMIUM_REVISION="${CHROMIUM_REVISION:-$DEFAULT_REVISION}"
TARGET_DIR="${CHROMIUM_TARGET_DIR:-backend/resources/chromium}"
HOST_OS="$(uname -s)"
OS="${CHROMIUM_OS:-$HOST_OS}"
ARCH="${CHROMIUM_ARCH:-$(uname -m)}"

case "$OS:$ARCH" in
  Darwin:arm64|Darwin:aarch64)
    DEFAULT_PLATFORM="Mac_Arm"
    DEFAULT_ARCHIVE_NAME="chrome-mac.zip"
    DEFAULT_ARCHIVE_ROOT="chrome-mac"
    DEFAULT_EXECUTABLE_RELATIVE="Chromium.app/Contents/MacOS/Chromium"
    DEFAULT_SHA256="98173187fab109a1e3431806811e95bff38e61682bbeaf8776733b5a378515ab"
    ;;
  Darwin:x86_64|Darwin:amd64)
    DEFAULT_PLATFORM="Mac"
    DEFAULT_ARCHIVE_NAME="chrome-mac.zip"
    DEFAULT_ARCHIVE_ROOT="chrome-mac"
    DEFAULT_EXECUTABLE_RELATIVE="Chromium.app/Contents/MacOS/Chromium"
    DEFAULT_SHA256="bb95467ac4b4097833f707e58f079926a9cece4824c700da335c38081c7a4e5b"
    ;;
  Linux:x86_64|Linux:amd64)
    DEFAULT_PLATFORM="Linux_x64"
    DEFAULT_ARCHIVE_NAME="chrome-linux.zip"
    DEFAULT_ARCHIVE_ROOT="chrome-linux"
    DEFAULT_EXECUTABLE_RELATIVE="chrome"
    DEFAULT_SHA256="5c58e0e0e08e2e56ef34609195decc4898418a232c39d095db92e133facb3333"
    ;;
  MINGW*:x86_64|MINGW*:amd64|MSYS*:x86_64|MSYS*:amd64|CYGWIN*:x86_64|CYGWIN*:amd64|Windows_NT:x86_64|Windows_NT:amd64)
    DEFAULT_PLATFORM="Win_x64"
    DEFAULT_ARCHIVE_NAME="chrome-win.zip"
    DEFAULT_ARCHIVE_ROOT="chrome-win"
    DEFAULT_EXECUTABLE_RELATIVE="chrome.exe"
    DEFAULT_SHA256="55e23d3d24bc4fd7dd2fb9f9884d13f34b46574436fd9c25939dae3da0d68c0d"
    ;;
  MINGW*:arm64|MINGW*:aarch64|MSYS*:arm64|MSYS*:aarch64|CYGWIN*:arm64|CYGWIN*:aarch64|Windows_NT:arm64|Windows_NT:aarch64)
    DEFAULT_PLATFORM="Win_Arm64"
    DEFAULT_ARCHIVE_NAME="chrome-win.zip"
    DEFAULT_ARCHIVE_ROOT="chrome-win"
    DEFAULT_EXECUTABLE_RELATIVE="chrome.exe"
    DEFAULT_SHA256="c0edc4b2accfa961f517115e8d3c348ef7a4751e638e7481d3b92449b838f1c8"
    ;;
  *)
    echo "Unsupported host for the bundled Chromium snapshot: $OS/$ARCH" >&2
    exit 1
    ;;
esac

PLATFORM="${CHROMIUM_PLATFORM:-$DEFAULT_PLATFORM}"
ARCHIVE_NAME="${CHROMIUM_ARCHIVE_NAME:-$DEFAULT_ARCHIVE_NAME}"
ARCHIVE_ROOT="${CHROMIUM_ARCHIVE_ROOT:-$DEFAULT_ARCHIVE_ROOT}"
EXECUTABLE_RELATIVE="${CHROMIUM_EXECUTABLE_RELATIVE:-$DEFAULT_EXECUTABLE_RELATIVE}"
DOWNLOAD_URL="${CHROMIUM_DOWNLOAD_URL:-https://storage.googleapis.com/chromium-browser-snapshots/${PLATFORM}/${CHROMIUM_REVISION}/${ARCHIVE_NAME}}"

for relative_path in "$ARCHIVE_NAME" "$ARCHIVE_ROOT" "$EXECUTABLE_RELATIVE"; do
  if [[ -z "$relative_path" \
    || "$relative_path" == /* \
    || "$relative_path" == *\\* \
    || "/$relative_path/" == *"/../"* ]]; then
    echo "Chromium archive paths must be non-empty, relative, and traversal-free: $relative_path" >&2
    exit 1
  fi
done
if [[ "$ARCHIVE_NAME" == */* ]]; then
  echo "Chromium archive name must not contain a directory: $ARCHIVE_NAME" >&2
  exit 1
fi

if [[ "$CHROMIUM_REVISION" == "$DEFAULT_REVISION" \
  && "$PLATFORM" == "$DEFAULT_PLATFORM" \
  && "$ARCHIVE_NAME" == "$DEFAULT_ARCHIVE_NAME" \
  && "$ARCHIVE_ROOT" == "$DEFAULT_ARCHIVE_ROOT" \
  && "$EXECUTABLE_RELATIVE" == "$DEFAULT_EXECUTABLE_RELATIVE" ]]; then
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
INSTALL_ROOT="$EXTRACT_DIR/$ARCHIVE_ROOT"
EXECUTABLE_PATH="$INSTALL_ROOT/$EXECUTABLE_RELATIVE"
if [[ ! -f "$EXECUTABLE_PATH" ]]; then
  echo "Chromium archive is missing $EXECUTABLE_PATH" >&2
  exit 1
fi
chmod +x "$EXECUTABLE_PATH"

if [[ "$HOST_OS" == "Darwin" ]]; then
  xattr -cr "$INSTALL_ROOT"
fi

# Replace only after the archive has passed checksum and layout validation.
rm -rf "$TARGET_DIR"
mv "$EXTRACT_DIR" "$TARGET_DIR"

echo "Installed verified Chromium r${CHROMIUM_REVISION} for $PLATFORM at $TARGET_DIR/$ARCHIVE_ROOT/$EXECUTABLE_RELATIVE"

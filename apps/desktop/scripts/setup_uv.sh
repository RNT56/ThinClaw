#!/usr/bin/env bash
set -euo pipefail

# uv release validated by the ThinClaw MLX/vLLM matrix on 2026-07-13.
UV_VERSION="${UV_VERSION:-0.11.28}"
BIN_DIR="${BACKEND_BIN_DIR:-backend/bin}"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Darwin-arm64)
    PLATFORM="aarch64-apple-darwin"
    ASSET="uv-aarch64-apple-darwin.tar.gz"
    EXPECTED_SHA256="33540eb7c883ab857eff79bd5ac2aa31fe27b595abecb4a9c003a2c998447232"
    ;;
  Darwin-x86_64)
    PLATFORM="x86_64-apple-darwin"
    ASSET="uv-x86_64-apple-darwin.tar.gz"
    EXPECTED_SHA256="2ad79983127ffca7d77b77ce6a24278d7e4f7b817a1acf72fea5f8124b4aac5e"
    ;;
  Linux-x86_64)
    PLATFORM="x86_64-unknown-linux-gnu"
    ASSET="uv-x86_64-unknown-linux-gnu.tar.gz"
    EXPECTED_SHA256="e490a6464492183c5d4534a5527fb4440f7f2bb2f228162ad7e4afe076dc0224"
    ;;
  *)
    echo "Unsupported platform: $OS-$ARCH" >&2
    exit 1
    ;;
esac

if [[ "$UV_VERSION" != "0.11.28" && -z "${UV_SHA256:-}" ]]; then
  echo "Custom uv version '$UV_VERSION' requires UV_SHA256." >&2
  exit 1
fi
EXPECTED_SHA256="${UV_SHA256:-$EXPECTED_SHA256}"
DOWNLOAD_URL="${UV_DOWNLOAD_URL:-https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/${ASSET}}"
TARGET_NAME="uv-${PLATFORM}"

mkdir -p "$BIN_DIR"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-uv.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
ARCHIVE="$TMP_DIR/$ASSET"
EXTRACT_DIR="$TMP_DIR/extracted"
mkdir -p "$EXTRACT_DIR"

echo "Downloading uv $UV_VERSION for $PLATFORM..."
curl --fail --show-error --location --retry 3 --retry-all-errors \
  --output "$ARCHIVE" "$DOWNLOAD_URL"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA256="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
else
  ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
fi
if [[ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]]; then
  echo "uv archive checksum mismatch: expected $EXPECTED_SHA256, got $ACTUAL_SHA256" >&2
  exit 1
fi

tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
UV_BIN="$(find "$EXTRACT_DIR" -name uv -type f -print -quit)"
if [[ -z "$UV_BIN" ]]; then
  echo "uv binary not found in $ASSET" >&2
  exit 1
fi

STAGED_TARGET="$BIN_DIR/.${TARGET_NAME}.new"
cp "$UV_BIN" "$STAGED_TARGET"
chmod +x "$STAGED_TARGET"
"$STAGED_TARGET" --version | grep -F "uv $UV_VERSION" >/dev/null
mv -f "$STAGED_TARGET" "$BIN_DIR/$TARGET_NAME"
echo "Installed verified uv $UV_VERSION sidecar at $BIN_DIR/$TARGET_NAME"

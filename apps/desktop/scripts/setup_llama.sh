#!/usr/bin/env bash
set -euo pipefail

# llama.cpp release validated by the ThinClaw engine matrix on 2026-07-13.
DEFAULT_TAG="b9988"
TAG_NAME="${1:-${LLAMA_CPP_VERSION:-$DEFAULT_TAG}}"
BIN_DIR="${BACKEND_BIN_DIR:-backend/bin}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Darwin-arm64)
    ASSET_SUFFIX="bin-macos-arm64"
    TARGET_NAME="llama-server-aarch64-apple-darwin"
    DEFAULT_SHA256="dd338e5123ef8f075410584d72c3a7437fdbe43a5550bc2c851cd978b1e1d815"
    ;;
  Darwin-x86_64)
    ASSET_SUFFIX="bin-macos-x64"
    TARGET_NAME="llama-server-x86_64-apple-darwin"
    DEFAULT_SHA256="206fc960d9800cd428162db6b489ae496a7b86435ca503322567d5b880824b55"
    ;;
  Linux-x86_64)
    ASSET_SUFFIX="bin-ubuntu-x64"
    TARGET_NAME="llama-server-x86_64-unknown-linux-gnu"
    DEFAULT_SHA256="b4352042ad5b8d73b8bf55487f2fc73c783b86f8d8938e42a86cd4dd27f19b66"
    ;;
  MINGW*-x86_64|CYGWIN*-x86_64|MSYS*-x86_64)
    ASSET_SUFFIX="bin-win-cpu-x64"
    TARGET_NAME="llama-server-x86_64-pc-windows-msvc.exe"
    DEFAULT_SHA256="6a7d350cbe89136a95ecb2ab48b6ad8bc8d22103fd50d0b2aa7ad7db59d34980"
    ;;
  *)
    echo "Unsupported platform: $OS-$ARCH" >&2
    exit 1
    ;;
esac

ASSET_NAME="${LLAMA_ASSET_NAME:-llama-${TAG_NAME}-${ASSET_SUFFIX}.tar.gz}"
if [[ "$OS" == MINGW* || "$OS" == CYGWIN* || "$OS" == MSYS* ]]; then
  ASSET_NAME="${LLAMA_ASSET_NAME:-llama-${TAG_NAME}-${ASSET_SUFFIX}.zip}"
fi

if [[ "$TAG_NAME" == "$DEFAULT_TAG" ]]; then
  EXPECTED_SHA256="${LLAMA_SHA256:-$DEFAULT_SHA256}"
elif [[ -n "${LLAMA_SHA256:-}" ]]; then
  EXPECTED_SHA256="$LLAMA_SHA256"
else
  echo "Custom llama.cpp tag '$TAG_NAME' requires LLAMA_SHA256." >&2
  echo "Set LLAMA_ASSET_NAME too when that release uses a different archive name." >&2
  exit 1
fi

DOWNLOAD_URL="${LLAMA_DOWNLOAD_URL:-https://github.com/ggml-org/llama.cpp/releases/download/${TAG_NAME}/${ASSET_NAME}}"
mkdir -p "$BIN_DIR"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-llama.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
ARCHIVE="$TMP_DIR/$ASSET_NAME"
EXTRACT_DIR="$TMP_DIR/extracted"
mkdir -p "$EXTRACT_DIR"

echo "Downloading llama.cpp $TAG_NAME for $OS-$ARCH..."
curl --fail --show-error --location --retry 3 --retry-all-errors \
  --output "$ARCHIVE" "$DOWNLOAD_URL"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA256="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
else
  ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
fi
if [[ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]]; then
  echo "llama.cpp archive checksum mismatch: expected $EXPECTED_SHA256, got $ACTUAL_SHA256" >&2
  exit 1
fi

case "$ASSET_NAME" in
  *.tar.gz) tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR" ;;
  *.zip) unzip -q "$ARCHIVE" -d "$EXTRACT_DIR" ;;
  *) echo "Unsupported llama.cpp archive: $ASSET_NAME" >&2; exit 1 ;;
esac

SERVER_NAME="llama-server"
[[ "$TARGET_NAME" == *.exe ]] && SERVER_NAME="llama-server.exe"
FOUND_BIN="$(find "$EXTRACT_DIR" -name "$SERVER_NAME" -type f -print -quit)"
if [[ -z "$FOUND_BIN" ]]; then
  echo "llama-server binary not found in $ASSET_NAME" >&2
  exit 1
fi

# The official archives dynamically link their adjacent runtime libraries.
# Copy every platform library before smoke-testing the staged server.
while IFS= read -r library; do
  # Dereference release-archive symlinks so every SONAME expected by the
  # executable is a real bundled resource after Tauri packaging.
  cp -L "$library" "$BIN_DIR/$(basename "$library")"
done < <(find "$EXTRACT_DIR" \( -type f -o -type l \) \( -name '*.dylib' -o -name '*.so' -o -name '*.so.*' -o -name '*.dll' \))

STAGED_TARGET="$BIN_DIR/.${TARGET_NAME}.new"
cp "$FOUND_BIN" "$STAGED_TARGET"
if [[ "$TARGET_NAME" != *.exe ]]; then
  chmod +x "$STAGED_TARGET"
fi

if [[ "$OS" == "Darwin" ]] && command -v install_name_tool >/dev/null 2>&1; then
  install_name_tool -add_rpath "@executable_path" "$STAGED_TARGET" 2>/dev/null || true
fi

if [[ "$OS" == "Darwin" ]]; then
  DYLD_LIBRARY_PATH="$BIN_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" "$STAGED_TARGET" --version >/dev/null
elif [[ "$OS" == "Linux" ]]; then
  LD_LIBRARY_PATH="$BIN_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$STAGED_TARGET" --version >/dev/null
else
  "$STAGED_TARGET" --version >/dev/null
fi

mv -f "$STAGED_TARGET" "$BIN_DIR/$TARGET_NAME"
echo "Installed verified llama.cpp $TAG_NAME sidecar at $BIN_DIR/$TARGET_NAME"

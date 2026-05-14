#!/usr/bin/env bash
# ==========================================================================
# setup_uv.sh — download `uv` binary for bundling as a Tauri sidecar
#
# `uv` is a fast Python package manager used by the MLX and vLLM engines
# to create isolated Python environments and install mlx_lm / vllm.
#
# Tauri sidecar naming convention: bin/uv-<target-triple>
# e.g. bin/uv-aarch64-apple-darwin
# ==========================================================================
set -euo pipefail

UV_VERSION="${UV_VERSION:-0.4.30}"   # Pin to a known-good release

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS-$ARCH" in
  Darwin-arm64)
    PLATFORM="aarch64-apple-darwin"
    ASSET="uv-aarch64-apple-darwin.tar.gz"
    ;;
  Darwin-x86_64)
    PLATFORM="x86_64-apple-darwin"
    ASSET="uv-x86_64-apple-darwin.tar.gz"
    ;;
  Linux-x86_64)
    PLATFORM="x86_64-unknown-linux-gnu"
    ASSET="uv-x86_64-unknown-linux-gnu.tar.gz"
    ;;
  *)
    echo "Unsupported platform: $OS-$ARCH"
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/${ASSET}"
TARGET_NAME="uv-${PLATFORM}"

mkdir -p backend/bin

echo "Downloading uv ${UV_VERSION} for ${PLATFORM}..."
curl -fsSL -o "backend/bin/${ASSET}" "${DOWNLOAD_URL}"

echo "Extracting..."
mkdir -p "backend/bin/uv-temp"
tar -xzf "backend/bin/${ASSET}" -C "backend/bin/uv-temp"

# The archive contains uv and uvx binaries in the root or a subdir
UV_BIN=$(find "backend/bin/uv-temp" -name "uv" -type f | head -n 1)
if [ -z "$UV_BIN" ]; then
  echo "Error: uv binary not found in archive"
  exit 1
fi

echo "Moving uv binary to backend/bin/${TARGET_NAME}"
mv "$UV_BIN" "backend/bin/${TARGET_NAME}"
chmod +x "backend/bin/${TARGET_NAME}"

# Cleanup
rm "backend/bin/${ASSET}"
rm -rf "backend/bin/uv-temp"

echo "Done! uv sidecar: backend/bin/${TARGET_NAME}"

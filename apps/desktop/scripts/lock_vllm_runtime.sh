#!/usr/bin/env bash
set -euo pipefail

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DESKTOP_DIR"

MANIFEST="$DESKTOP_DIR/engine-manifest.json"
read_manifest() {
  node -e '
    const manifest = require(process.argv[1]);
    const value = process.argv[2].split(".").reduce((current, key) => current[key], manifest);
    process.stdout.write(String(value));
  ' "$MANIFEST" "$1"
}

PYTHON_VERSION="$(read_manifest engines.vllm.pythonVersion)"
RESOLUTION_CUTOFF="$(read_manifest engines.vllm.resolutionCutoff)"
TORCH_BACKEND="$(read_manifest engines.vllm.torchBackend)"
UV_VERSION="$(read_manifest uv.version)"

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) UV="$DESKTOP_DIR/backend/bin/uv-aarch64-apple-darwin" ;;
  Darwin:x86_64) UV="$DESKTOP_DIR/backend/bin/uv-x86_64-apple-darwin" ;;
  Linux:x86_64) UV="$DESKTOP_DIR/backend/bin/uv-x86_64-unknown-linux-gnu" ;;
  *) echo "No reviewed uv binary is available for this lock host." >&2; exit 1 ;;
esac

if [[ ! -x "$UV" ]] || ! "$UV" --version | grep -F "uv $UV_VERSION" >/dev/null; then
  bash scripts/setup_uv.sh
fi

UV_NO_CONFIG=1 "$UV" pip compile runtime/vllm/requirements.in \
  --quiet \
  --output-file runtime/vllm/requirements.lock \
  --python-platform x86_64-manylinux_2_31 \
  --python-version "$PYTHON_VERSION" \
  --torch-backend "$TORCH_BACKEND" \
  --exclude-newer "$RESOLUTION_CUTOFF" \
  --only-binary :all: \
  --generate-hashes \
  --no-annotate \
  --custom-compile-command "npm run lock:vllm-runtime"

echo "Updated runtime/vllm/requirements.lock for Linux x86_64 $TORCH_BACKEND."

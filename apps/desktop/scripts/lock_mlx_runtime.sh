#!/usr/bin/env bash
set -euo pipefail

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DESKTOP_DIR"

[[ "$(uname -s):$(uname -m)" == "Darwin:arm64" ]] || {
  echo "The MLX lock must be generated on macOS arm64." >&2
  exit 1
}

MANIFEST="$DESKTOP_DIR/engine-manifest.json"
read_manifest() {
  node -e '
    const manifest = require(process.argv[1]);
    const value = process.argv[2].split(".").reduce((current, key) => current[key], manifest);
    process.stdout.write(String(value));
  ' "$MANIFEST" "$1"
}

PYTHON_VERSION="$(read_manifest python.version)"
RESOLUTION_CUTOFF="$(read_manifest engines.mlx.resolutionCutoff)"
UV_VERSION="$(read_manifest uv.version)"
UV="$DESKTOP_DIR/backend/bin/uv-aarch64-apple-darwin"

if [[ ! -x "$UV" ]] || ! "$UV" --version | grep -F "uv $UV_VERSION" >/dev/null; then
  bash scripts/setup_uv.sh
fi

LOCK_CACHE="$DESKTOP_DIR/backend/target/mlx-lock-cache"
PYTHON_DIR="$LOCK_CACHE/python"
UV_CACHE_DIR="$LOCK_CACHE/uv"
export UV_NO_CONFIG=1
export UV_CACHE_DIR
export UV_PYTHON_INSTALL_DIR="$PYTHON_DIR"

"$UV" python install "$PYTHON_VERSION" --install-dir "$PYTHON_DIR" --no-bin
PYTHON="$("$UV" python find "$PYTHON_VERSION" --managed-python)"
"$UV" pip compile runtime/mlx/requirements.in \
  --quiet \
  --output-file runtime/mlx/requirements.lock \
  --python "$PYTHON" \
  --exclude-newer "$RESOLUTION_CUTOFF" \
  --only-binary :all: \
  --generate-hashes \
  --no-annotate \
  --custom-compile-command "npm run lock:mlx-runtime"

echo "Updated runtime/mlx/requirements.lock for Python $PYTHON_VERSION."

#!/usr/bin/env bash
set -euo pipefail

ACTION="${1:-print}"
HOST_OS="${THINCLAW_ENGINE_HOST_OS:-$(uname -s)}"
HOST_ARCH="${THINCLAW_ENGINE_HOST_ARCH:-$(uname -m)}"
TARGET_TRIPLE="${TAURI_TARGET_TRIPLE:-${TARGET:-}}"
ENGINE="${THINCLAW_DESKTOP_ENGINE:-}"
DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE_MANIFEST="${THINCLAW_ENGINE_MANIFEST:-$DESKTOP_DIR/engine-manifest.json}"
[[ -f "$ENGINE_MANIFEST" ]] || { echo "Missing engine manifest: $ENGINE_MANIFEST" >&2; exit 1; }

if [[ -z "$TARGET_TRIPLE" ]]; then
  case "$HOST_OS:$HOST_ARCH" in
    Darwin:arm64) TARGET_TRIPLE="aarch64-apple-darwin" ;;
    Darwin:x86_64) TARGET_TRIPLE="x86_64-apple-darwin" ;;
    Linux:x86_64) TARGET_TRIPLE="x86_64-unknown-linux-gnu" ;;
    MINGW*:*|MSYS*:*|CYGWIN*:*) TARGET_TRIPLE="x86_64-pc-windows-msvc" ;;
    *) echo "No reviewed desktop target for $HOST_OS:$HOST_ARCH" >&2; exit 1 ;;
  esac
fi

if [[ -z "$ENGINE" ]]; then
  ENGINE="$(node -e '
    const manifest = require(process.argv[1]);
    const engine = manifest.hostDefaults[process.argv[2]];
    if (!engine) process.exit(1);
    process.stdout.write(engine);
  ' "$ENGINE_MANIFEST" "$TARGET_TRIPLE")" || {
    echo "engine-manifest.json has no default for $TARGET_TRIPLE" >&2
    exit 1
  }
fi

case "$ENGINE" in
  llamacpp|mlx|vllm|ollama)
    ;;
  *)
    echo "Unknown THINCLAW_DESKTOP_ENGINE value: $ENGINE" >&2
    echo "Valid desktop engines: llamacpp, mlx, vllm, ollama" >&2
    exit 1
    ;;
esac

if [[ "$ENGINE" != "ollama" ]] && ! node -e '
  const manifest = require(process.argv[1]);
  process.exit(manifest.engines[process.argv[2]]?.supportedTargets?.includes(process.argv[3]) ? 0 : 1);
' "$ENGINE_MANIFEST" "$ENGINE" "$TARGET_TRIPLE"; then
  echo "Engine '$ENGINE' is unsupported for target '$TARGET_TRIPLE'" >&2
  exit 1
fi

case "$ACTION" in
  print)
    printf '%s\n' "$ENGINE"
    ;;
  setup)
    echo "Preparing ThinClaw Desktop engine: $ENGINE"
    case "$ENGINE" in
      llamacpp)
        bash scripts/setup_llama.sh
        ;;
      mlx|vllm)
        bash scripts/setup_uv.sh
        ;;
      ollama)
        # Ollama is managed by the user and does not require a bundled engine sidecar.
        ;;
    esac
    INCLUDE_CHROMIUM="${INCLUDE_CHROMIUM:-1}" \
      bash scripts/generate_tauri_overrides.sh "$ENGINE"
    npm run validate:sidecars
    ;;
  dev|build)
    echo "Using ThinClaw Desktop engine: $ENGINE"
    exec npm run "tauri:${ACTION}:${ENGINE}"
    ;;
  *)
    echo "Unknown action: $ACTION" >&2
    echo "Valid actions: print, setup, dev, build" >&2
    exit 1
    ;;
esac

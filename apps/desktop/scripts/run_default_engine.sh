#!/usr/bin/env bash
set -euo pipefail

ACTION="${1:-print}"
HOST_OS="${THINCLAW_ENGINE_HOST_OS:-$(uname -s)}"
HOST_ARCH="${THINCLAW_ENGINE_HOST_ARCH:-$(uname -m)}"
TARGET_TRIPLE="${TAURI_TARGET_TRIPLE:-${TARGET:-}}"
ENGINE="${THINCLAW_DESKTOP_ENGINE:-}"

if [[ -z "$ENGINE" ]]; then
  if [[ "$TARGET_TRIPLE" == "aarch64-apple-darwin" ]] \
    || [[ "$HOST_OS" == "Darwin" && "$HOST_ARCH" == "arm64" ]]; then
    ENGINE="mlx"
  else
    ENGINE="llamacpp"
  fi
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

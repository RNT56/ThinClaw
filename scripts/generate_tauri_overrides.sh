#!/usr/bin/env bash
# ==========================================================================
# generate_tauri_overrides.sh
#
# Generates a tauri.conf.json override that strips or includes sidecar
# binaries based on the active inference engine.
#
# Usage:
#   bash scripts/generate_tauri_overrides.sh <engine>
#
# Where <engine> is one of: llamacpp, mlx, vllm, ollama, none
#
# Output: writes backend/tauri.override.json (merge with --config flag)
# ==========================================================================
set -euo pipefail

ENGINE="${1:-llamacpp}"

echo "Generating Tauri config override for engine: $ENGINE"

case "$ENGINE" in
  llamacpp)
    # Include all native sidecars
    cat > backend/tauri.override.json <<'EOF'
{
  "bundle": {
    "externalBin": [
      "bin/llama-server",
      "bin/whisper",
      "bin/whisper-server",
      "bin/sd",
      "bin/tts"
    ],
    "resources": [
      "bin/*.dylib",
      "bin/*.metal",
      "resources/chromium"
    ]
  }
}
EOF
    ;;

  mlx|vllm)
    # Python-based engines — no llama-server needed, but bundle uv
    cat > backend/tauri.override.json <<'EOF'
{
  "bundle": {
    "externalBin": [
      "bin/uv",
      "bin/whisper",
      "bin/whisper-server",
      "bin/tts"
    ],
    "resources": [
      "bin/libwhisper*.dylib",
      "resources/chromium"
    ]
  }
}
EOF
    ;;

  ollama)
    # Ollama manages its own models/binaries — minimal sidecar set
    cat > backend/tauri.override.json <<'EOF'
{
  "bundle": {
    "externalBin": [
      "bin/whisper",
      "bin/whisper-server",
      "bin/tts"
    ],
    "resources": [
      "bin/libwhisper*.dylib",
      "resources/chromium"
    ]
  }
}
EOF
    ;;

  none)
    # Cloud-only build — minimal bundle
    cat > backend/tauri.override.json <<'EOF'
{
  "bundle": {
    "externalBin": [],
    "resources": [
      "resources/chromium"
    ]
  }
}
EOF
    ;;

  *)
    echo "Unknown engine: $ENGINE"
    echo "Valid engines: llamacpp, mlx, vllm, ollama, none"
    exit 1
    ;;
esac

echo "Override written to backend/tauri.override.json"
echo "Use: tauri build --config backend/tauri.override.json"

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
STRICT_SIDECARS="${STRICT_SIDECARS:-1}"
INCLUDE_CHROMIUM="${INCLUDE_CHROMIUM:-auto}"
DISABLE_UPDATER_ARTIFACTS="${DISABLE_UPDATER_ARTIFACTS:-0}"
TARGET_TRIPLE="${TAURI_TARGET_TRIPLE:-${TARGET:-}}"
RELEASE_VERSION="${TAURI_RELEASE_VERSION:-}"
ENGINE_MANIFEST="${THINCLAW_ENGINE_MANIFEST:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/engine-manifest.json}"
[[ -f "$ENGINE_MANIFEST" ]] || { echo "Missing engine manifest: $ENGINE_MANIFEST" >&2; exit 1; }

if [[ -n "$RELEASE_VERSION" ]] && ! [[ "$RELEASE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "TAURI_RELEASE_VERSION must be a SemVer without a leading v: $RELEASE_VERSION" >&2
  exit 1
fi

detect_target_triple() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Darwin:arm64) echo "aarch64-apple-darwin" ;;
    Darwin:x86_64) echo "x86_64-apple-darwin" ;;
    Linux:aarch64|Linux:arm64) echo "aarch64-unknown-linux-gnu" ;;
    Linux:x86_64) echo "x86_64-unknown-linux-gnu" ;;
    MINGW*:*|MSYS*:*|CYGWIN*:*) echo "x86_64-pc-windows-msvc" ;;
    *) echo "" ;;
  esac
}

if [[ -z "$TARGET_TRIPLE" ]]; then
  TARGET_TRIPLE="$(detect_target_triple)"
fi

if [[ "$ENGINE" == "llamacpp" || "$ENGINE" == "mlx" || "$ENGINE" == "vllm" ]]; then
  if ! node -e '
    const manifest = require(process.argv[1]);
    const engine = process.argv[2];
    const target = process.argv[3];
    if (!manifest.engines[engine]?.supportedTargets?.includes(target)) process.exit(1);
  ' "$ENGINE_MANIFEST" "$ENGINE" "$TARGET_TRIPLE"; then
    echo "Engine '$ENGINE' is not supported for target '${TARGET_TRIPLE:-unknown}' by engine-manifest.json" >&2
    exit 1
  fi
fi

MLX_MINIMUM_MACOS="$(node -e '
  const manifest = require(process.argv[1]);
  process.stdout.write(manifest.engines.mlx.minimumMacosVersion);
' "$ENGINE_MANIFEST")"

EXTERNAL_BINS=()
RESOURCES=()
MISSING_SIDECARS=()

sidecar_exists() {
  local entry="$1"
  local base="${entry#bin/}"
  [[ -n "$TARGET_TRIPLE" ]] && [[ -f "backend/bin/${base}-${TARGET_TRIPLE}" || -f "backend/bin/${base}-${TARGET_TRIPLE}.exe" ]]
}

add_required_sidecar() {
  local entry="$1"
  EXTERNAL_BINS+=("$entry")
  if ! sidecar_exists "$entry"; then
    MISSING_SIDECARS+=("$entry")
  fi
}

add_optional_sidecar() {
  local entry="$1"
  if sidecar_exists "$entry"; then
    EXTERNAL_BINS+=("$entry")
  else
    echo "Skipping optional sidecar not present for ${TARGET_TRIPLE:-unknown target}: $entry"
  fi
}

add_resource_if_present() {
  local pattern="$1"
  local matches=()
  if [[ "$pattern" == *['*''?''[']* ]]; then
    shopt -s nullglob
    matches=(backend/$pattern)
    shopt -u nullglob
  elif [[ -e "backend/$pattern" ]]; then
    matches=("backend/$pattern")
  fi
  if (( ${#matches[@]} > 0 )) || [[ "$STRICT_SIDECARS" == "1" ]]; then
    RESOURCES+=("$pattern")
  fi
}

add_optional_resource() {
  local pattern="$1"
  local matches=()
  if [[ "$pattern" == *['*''?''[']* ]]; then
    shopt -s nullglob
    matches=(backend/$pattern)
    shopt -u nullglob
  elif [[ -e "backend/$pattern" ]]; then
    matches=("backend/$pattern")
  fi
  if (( ${#matches[@]} > 0 )); then
    RESOURCES+=("$pattern")
  else
    echo "Skipping optional resource not present: $pattern"
  fi
}

add_platform_resources() {
  case "$TARGET_TRIPLE" in
    *apple-darwin)
      add_resource_if_present "bin/*.dylib"
      # Recent official llama.cpp macOS archives embed the Metal shaders and
      # no longer ship standalone .metal sources. Bundle them when an older or
      # custom archive provides them, but do not make them a packaging
      # requirement.
      add_optional_resource "bin/*.metal"
      ;;
    *linux*)
      add_resource_if_present "bin/*.so"
      add_resource_if_present "bin/*.so.*"
      ;;
    *windows*)
      add_resource_if_present "bin/*.dll"
      ;;
  esac
}

write_override() {
  {
    printf '{\n'
    if [[ -n "$RELEASE_VERSION" ]]; then
      printf '  "version": "%s",\n' "$RELEASE_VERSION"
    fi
    printf '  "bundle": {\n    "externalBin": ['
    for i in "${!EXTERNAL_BINS[@]}"; do
      [[ "$i" != "0" ]] && printf ','
      printf '\n      "%s"' "${EXTERNAL_BINS[$i]}"
    done
    (( ${#EXTERNAL_BINS[@]} > 0 )) && printf '\n    ' || printf '\n    '
    printf '],\n    "resources": ['
    for i in "${!RESOURCES[@]}"; do
      [[ "$i" != "0" ]] && printf ','
      printf '\n      "%s"' "${RESOURCES[$i]}"
    done
    printf '\n    ]'
    if [[ "$DISABLE_UPDATER_ARTIFACTS" == "1" || "$DISABLE_UPDATER_ARTIFACTS" == "true" || "$DISABLE_UPDATER_ARTIFACTS" == "yes" ]]; then
      printf ',\n    "createUpdaterArtifacts": false'
    fi
    if [[ "$ENGINE" == "mlx" && "$TARGET_TRIPLE" == "aarch64-apple-darwin" ]]; then
      printf ',\n    "macOS": {\n      "entitlements": "Entitlements.plist",\n      "minimumSystemVersion": "%s"\n    }' "$MLX_MINIMUM_MACOS"
    fi
    printf '\n  }\n}\n'
  } > backend/tauri.override.json
}

echo "Generating Tauri config override for engine: $ENGINE target: ${TARGET_TRIPLE:-unknown}"

RESOURCES+=("../../../deploy/**/*")
case "$INCLUDE_CHROMIUM" in
  1|true|yes)
    add_resource_if_present "resources/chromium"
    ;;
  auto)
    add_optional_resource "resources/chromium"
    ;;
  0|false|no)
    ;;
  *)
    echo "Unknown INCLUDE_CHROMIUM value: $INCLUDE_CHROMIUM"
    echo "Valid values: auto, 1, 0"
    exit 1
    ;;
esac

case "$ENGINE" in
  llamacpp)
    add_required_sidecar "bin/llama-server"
    add_resource_if_present "bin/llama-runtime-${TARGET_TRIPLE}.json"
    add_optional_sidecar "bin/whisper"
    add_optional_sidecar "bin/whisper-server"
    add_optional_sidecar "bin/sd"
    add_optional_sidecar "bin/tts"
    add_platform_resources
    ;;

  mlx|vllm)
    add_required_sidecar "bin/uv"
    add_optional_sidecar "bin/whisper"
    add_optional_sidecar "bin/whisper-server"
    add_optional_sidecar "bin/tts"
    ;;

  ollama)
    add_optional_sidecar "bin/whisper"
    add_optional_sidecar "bin/whisper-server"
    add_optional_sidecar "bin/tts"
    ;;

  none)
    # Cloud-only build: no bundled native sidecars.
    ;;

  *)
    echo "Unknown engine: $ENGINE"
    echo "Valid engines: llamacpp, mlx, vllm, ollama, none"
    exit 1
    ;;
esac

if (( ${#MISSING_SIDECARS[@]} > 0 )); then
  echo "Missing required sidecars for ${TARGET_TRIPLE:-unknown target}: ${MISSING_SIDECARS[*]}" >&2
  echo "Run npm run setup:ai or the engine-specific setup script before a native sidecar build." >&2
  if [[ "$STRICT_SIDECARS" == "1" ]]; then
    exit 1
  fi
fi

write_override

echo "Override written to backend/tauri.override.json"
echo "Use: tauri build --config backend/tauri.override.json"

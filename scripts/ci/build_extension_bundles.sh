#!/usr/bin/env bash
set -euo pipefail

tool_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repo_root="${THINCLAW_REPO_ROOT:-$tool_root}"
repo_root="$(cd "$repo_root" && pwd)"
checker="${THINCLAW_EXTENSION_CHECKER:-$tool_root/scripts/ci/extension_registry.py}"
output_arg="${1:?usage: build_extension_bundles.sh OUTPUT_DIRECTORY}"
mkdir -p "$output_arg"
output="$(cd "$output_arg" && pwd)"
target="wasm32-wasip2"

rm -f "$output"/*-"$target".tar.gz "$output"/checksums-wasm.txt

while IFS= read -r manifest; do
  source_dir="${manifest%/Cargo.toml}"
  registry_manifest="$({
    for candidate in "$repo_root"/registry/tools/*.json "$repo_root"/registry/channels/*.json; do
      [[ "$(jq -r '.source.dir' "$candidate")/Cargo.toml" == "$manifest" ]] && printf '%s\n' "$candidate"
    done
  } | head -n 1)"
  [[ -n "$registry_manifest" ]] || {
    echo "No registry manifest found for $source_dir" >&2
    exit 1
  }
  name="$(jq -er '.name' "$registry_manifest")"
  crate_name="$(jq -er '.source.crate_name' "$registry_manifest")"
  capabilities="$(jq -er '.source.capabilities' "$registry_manifest")"
  build_args=(component build --release --manifest-path "$repo_root/$manifest")
  if [[ "${THINCLAW_LEGACY_UNLOCKED:-0}" != "1" ]]; then
    build_args=(component build --locked --release --manifest-path "$repo_root/$manifest")
  fi
  cargo "${build_args[@]}"
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    if [[ "$CARGO_TARGET_DIR" == /* ]]; then
      build_target="$CARGO_TARGET_DIR"
    else
      build_target="$repo_root/$CARGO_TARGET_DIR"
    fi
  else
    build_target="$repo_root/$source_dir/target"
  fi
  wasm=""
  for wasm_target in wasm32-wasip2 wasm32-wasip1 wasm32-wasi; do
    candidate="$build_target/$wasm_target/release/${crate_name//-/_}.wasm"
    if [[ -s "$candidate" ]]; then
      wasm="$candidate"
      break
    fi
  done
  [[ -s "$wasm" ]] || {
    echo "Missing WASM output for $name under $build_target" >&2
    exit 1
  }
  python3 "$checker" --repo-root "$repo_root" package-bundle \
    --name "$name" \
    --wasm "$wasm" \
    --capabilities "$repo_root/$source_dir/$capabilities" \
    --output "$output/$name-$target.tar.gz"
done < <(python3 "$checker" --repo-root "$repo_root" list-manifests)

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$output" && sha256sum *-"$target".tar.gz | sort -k2 > checksums-wasm.txt)
else
  (cd "$output" && shasum -a 256 *-"$target".tar.gz | sort -k2 > checksums-wasm.txt)
fi
if [[ "${THINCLAW_EXTENSION_BUILD_ONLY:-0}" != "1" ]]; then
  python3 "$checker" --repo-root "$repo_root" verify-bundles \
    --tag "${RELEASE_TAG:-v$(awk -F'"' '/^version = "/ { print $2; exit }' "$repo_root/Cargo.toml")}" \
    --bundles "$output"
fi

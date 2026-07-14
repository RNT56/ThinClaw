#!/usr/bin/env bash
set -euo pipefail

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-release-artifacts.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT
mkdir -p "$TMP_ROOT/bundle/macos/ThinClaw Desktop.app" "$TMP_ROOT/bundle/dmg"
printf 'dmg fixture\n' > "$TMP_ROOT/bundle/dmg/ThinClaw Desktop_0.16.0_aarch64.dmg"
printf 'updater fixture\n' > "$TMP_ROOT/bundle/macos/ThinClaw Desktop.app.tar.gz"
printf 'signed fixture\n' > "$TMP_ROOT/bundle/macos/ThinClaw Desktop.app.tar.gz.sig"

TAURI_RELEASE_VERSION="0.16.0" \
TAURI_RELEASE_TAG="v0.16.0" \
TAURI_BUNDLE_ROOT="$TMP_ROOT/bundle" \
TAURI_RELEASE_OUTPUT_DIR="$TMP_ROOT/output" \
GITHUB_REPOSITORY="RNT56/ThinClaw" \
  bash "$DESKTOP_DIR/scripts/collect_macos_release_artifacts.sh" >/dev/null

test -s "$TMP_ROOT/output/ThinClaw Desktop_0.16.0_aarch64.dmg"
test -s "$TMP_ROOT/output/ThinClaw Desktop.app.tar.gz"
test -s "$TMP_ROOT/output/ThinClaw Desktop.app.tar.gz.sig"
node - "$TMP_ROOT/output/latest.json" <<'NODE'
const fs = require('fs');
const manifest = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const platform = process.arch === 'arm64' ? 'darwin-aarch64' : 'darwin-x86_64';
if (manifest.version !== '0.16.0' || !manifest.platforms[platform]) process.exit(1);
if (manifest.platforms[platform].signature !== 'signed fixture') process.exit(1);
NODE

echo "macOS release artifact collection fixture passed."

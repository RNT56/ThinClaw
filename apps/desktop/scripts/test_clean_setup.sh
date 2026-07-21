#!/usr/bin/env bash
set -euo pipefail

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-clean-setup.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT
FIXTURES="$TMP_ROOT/fixtures"
TEST_APP="$TMP_ROOT/app"
mkdir -p \
  "$FIXTURES/llama" \
  "$FIXTURES/chromium-mac/chrome-mac/Chromium.app/Contents/MacOS" \
  "$FIXTURES/chromium-linux/chrome-linux" \
  "$FIXTURES/chromium-windows/chrome-win"

cp "$DESKTOP_DIR/scripts/setup_llama.sh" "$TMP_ROOT/setup_llama.sh"
cp "$DESKTOP_DIR/backend/scripts/setup_chromium.sh" "$TMP_ROOT/setup_chromium.sh"
mkdir -p "$TEST_APP/scripts" "$TEST_APP/backend/scripts"
cp "$DESKTOP_DIR/scripts/generate_tauri_overrides.sh" "$TEST_APP/scripts/"
cp "$DESKTOP_DIR/scripts/check_sidecar_budgets.mjs" "$TEST_APP/scripts/"
cp "$DESKTOP_DIR/sidecar-budgets.json" "$TEST_APP/"

printf '#!/usr/bin/env bash\necho "llama fixture 1.0"\n' > "$FIXTURES/llama/llama-server"
chmod +x "$FIXTURES/llama/llama-server"
tar -czf "$FIXTURES/llama-fixture.tar.gz" -C "$FIXTURES/llama" llama-server

printf '#!/usr/bin/env bash\necho "Chromium macOS fixture"\n' > "$FIXTURES/chromium-mac/chrome-mac/Chromium.app/Contents/MacOS/Chromium"
printf '#!/usr/bin/env bash\necho "Chromium Linux fixture"\n' > "$FIXTURES/chromium-linux/chrome-linux/chrome"
printf '#!/usr/bin/env bash\necho "Chromium Windows fixture"\n' > "$FIXTURES/chromium-windows/chrome-win/chrome.exe"
chmod +x \
  "$FIXTURES/chromium-mac/chrome-mac/Chromium.app/Contents/MacOS/Chromium" \
  "$FIXTURES/chromium-linux/chrome-linux/chrome" \
  "$FIXTURES/chromium-windows/chrome-win/chrome.exe"
(cd "$FIXTURES/chromium-mac" && zip -qr "$FIXTURES/chromium-mac.zip" chrome-mac)
(cd "$FIXTURES/chromium-linux" && zip -qr "$FIXTURES/chromium-linux.zip" chrome-linux)
(cd "$FIXTURES/chromium-windows" && zip -qr "$FIXTURES/chromium-windows.zip" chrome-win)

checksum() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

LLAMA_SHA="$(checksum "$FIXTURES/llama-fixture.tar.gz")"
CHROMIUM_MAC_SHA="$(checksum "$FIXTURES/chromium-mac.zip")"
CHROMIUM_LINUX_SHA="$(checksum "$FIXTURES/chromium-linux.zip")"
CHROMIUM_WINDOWS_SHA="$(checksum "$FIXTURES/chromium-windows.zip")"

cd "$TEST_APP"
BACKEND_BIN_DIR="$TEST_APP/backend/bin" \
LLAMA_ASSET_NAME="llama-fixture.tar.gz" \
LLAMA_DOWNLOAD_URL="file://$FIXTURES/llama-fixture.tar.gz" \
LLAMA_SHA256="$LLAMA_SHA" \
  bash "$TMP_ROOT/setup_llama.sh"

CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/chromium" \
CHROMIUM_OS="Darwin" \
CHROMIUM_ARCH="arm64" \
CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-mac.zip" \
CHROMIUM_SHA256="$CHROMIUM_MAC_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh"

CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/chromium-linux-test" \
CHROMIUM_OS="Linux" \
CHROMIUM_ARCH="x86_64" \
CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-linux.zip" \
CHROMIUM_SHA256="$CHROMIUM_LINUX_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh"

CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/chromium-mac-x64-test" \
CHROMIUM_OS="Darwin" \
CHROMIUM_ARCH="x86_64" \
CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-mac.zip" \
CHROMIUM_SHA256="$CHROMIUM_MAC_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh"

CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/chromium-windows-test" \
CHROMIUM_OS="MINGW64_NT-10.0" \
CHROMIUM_ARCH="x86_64" \
CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-windows.zip" \
CHROMIUM_SHA256="$CHROMIUM_WINDOWS_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh"

CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/chromium-windows-arm-test" \
CHROMIUM_OS="Windows_NT" \
CHROMIUM_ARCH="aarch64" \
CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-windows.zip" \
CHROMIUM_SHA256="$CHROMIUM_WINDOWS_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh"

INSTALLED_LLAMA="$(find backend/bin -maxdepth 1 -type f -name 'llama-server-*' -print -quit)"
TEST_TARGET="${INSTALLED_LLAMA##*/llama-server-}"
TEST_TARGET="${TEST_TARGET%.exe}"
if [[ -z "$INSTALLED_LLAMA" || -z "$TEST_TARGET" ]]; then
  echo "Clean setup did not install a target-suffixed llama-server." >&2
  exit 1
fi

TAURI_TARGET_TRIPLE="$TEST_TARGET" TAURI_RELEASE_VERSION="0.16.0-rc.1+build.7" INCLUDE_CHROMIUM=1 \
  bash scripts/generate_tauri_overrides.sh llamacpp
node -e 'const c = require("./backend/tauri.override.json"); if (c.version !== "0.16.0-rc.1+build.7") process.exit(1)'
TAURI_TARGET_TRIPLE="$TEST_TARGET" \
  node scripts/check_sidecar_budgets.mjs --config backend/tauri.override.json

test -x "$INSTALLED_LLAMA"
test -x backend/resources/chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium
test -x backend/resources/chromium-linux-test/chrome-linux/chrome
test -x backend/resources/chromium-mac-x64-test/chrome-mac/Chromium.app/Contents/MacOS/Chromium
test -x backend/resources/chromium-windows-test/chrome-win/chrome.exe
test -x backend/resources/chromium-windows-arm-test/chrome-win/chrome.exe
if grep -Eq 'bin/(whisper|whisper-server|sd|tts)' backend/tauri.override.json; then
  echo "Clean setup unexpectedly bundled an optional media sidecar." >&2
  exit 1
fi

if CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/rejected" \
  CHROMIUM_OS="Darwin" \
  CHROMIUM_ARCH="arm64" \
  CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-mac.zip" \
  CHROMIUM_SHA256="$(printf '0%.0s' {1..64})" \
  bash "$TMP_ROOT/setup_chromium.sh" >/dev/null 2>&1; then
  echo "Chromium setup accepted an invalid checksum." >&2
  exit 1
fi

if CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/rejected-path" \
  CHROMIUM_OS="Darwin" \
  CHROMIUM_ARCH="arm64" \
  CHROMIUM_ARCHIVE_ROOT="../escape" \
  CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-mac.zip" \
  CHROMIUM_SHA256="$CHROMIUM_MAC_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh" >/dev/null 2>&1; then
  echo "Chromium setup accepted an unsafe archive root." >&2
  exit 1
fi

if CHROMIUM_TARGET_DIR="$TEST_APP/backend/resources/rejected-host" \
  CHROMIUM_OS="Linux" \
  CHROMIUM_ARCH="aarch64" \
  CHROMIUM_DOWNLOAD_URL="file://$FIXTURES/chromium-linux.zip" \
  CHROMIUM_SHA256="$CHROMIUM_LINUX_SHA" \
  bash "$TMP_ROOT/setup_chromium.sh" >/dev/null 2>&1; then
  echo "Chromium setup accepted an unsupported Linux ARM64 host." >&2
  exit 1
fi

if TAURI_TARGET_TRIPLE="$TEST_TARGET" \
  node scripts/check_sidecar_budgets.mjs --config ../outside.json >/dev/null 2>&1; then
  echo "Sidecar budget check accepted a config path outside the desktop root." >&2
  exit 1
fi

mv backend/tauri.override.json "$TMP_ROOT/outside-config.json"
ln -s "$TMP_ROOT/outside-config.json" backend/tauri.override.json
if TAURI_TARGET_TRIPLE="$TEST_TARGET" \
  node scripts/check_sidecar_budgets.mjs --config backend/tauri.override.json >/dev/null 2>&1; then
  echo "Sidecar budget check accepted a config symlink outside the desktop root." >&2
  exit 1
fi
rm backend/tauri.override.json
cp "$TMP_ROOT/outside-config.json" backend/tauri.override.json

node <<'NODE'
const fs = require('node:fs');
const path = 'backend/tauri.override.json';
const config = JSON.parse(fs.readFileSync(path, 'utf8'));
config.bundle.externalBin = ['bin/../../escape'];
fs.writeFileSync(path, JSON.stringify(config));
NODE
if TAURI_TARGET_TRIPLE="$TEST_TARGET" \
  node scripts/check_sidecar_budgets.mjs --config backend/tauri.override.json >/dev/null 2>&1; then
  echo "Sidecar budget check accepted an undeclared external binary." >&2
  exit 1
fi
cp "$TMP_ROOT/outside-config.json" backend/tauri.override.json

if TAURI_TARGET_TRIPLE='../escape' \
  node scripts/check_sidecar_budgets.mjs --config backend/tauri.override.json >/dev/null 2>&1; then
  echo "Sidecar budget check accepted an unsafe target triple." >&2
  exit 1
fi

echo "Clean-machine setup fixture passed with verified downloads, required sidecars, and size budgets."

#!/usr/bin/env bash
# Validate the Desktop packaging/platform contract without requiring release secrets.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OVERRIDE_PATH="backend/tauri.override.json"
BACKUP_PATH=""
if [[ -f "$OVERRIDE_PATH" ]]; then
  BACKUP_PATH="$(mktemp)"
  cp "$OVERRIDE_PATH" "$BACKUP_PATH"
fi

cleanup() {
  if [[ -n "$BACKUP_PATH" && -f "$BACKUP_PATH" ]]; then
    cp "$BACKUP_PATH" "$OVERRIDE_PATH"
    rm -f "$BACKUP_PATH"
  else
    rm -f "$OVERRIDE_PATH"
  fi
}
trap cleanup EXIT

echo "== Tauri metadata =="
npm run tauri -- info

echo "== Static bundle contract =="
node <<'NODE'
const fs = require('fs');

function fail(message) {
  console.error(message);
  process.exit(1);
}

const config = JSON.parse(fs.readFileSync('backend/tauri.conf.json', 'utf8'));
if (config.productName !== 'ThinClaw Desktop') fail(`Unexpected productName: ${config.productName}`);
if (config.identifier !== 'com.thinclaw.desktop') fail(`Unexpected bundle identifier: ${config.identifier}`);
if (!config.bundle?.active) fail('Bundle must be active for Desktop packaging.');
if (!config.bundle?.createUpdaterArtifacts) fail('Updater artifacts must be enabled.');
if (!config.bundle?.macOS?.entitlements) fail('macOS entitlements file is not configured.');
if (!config.plugins?.updater?.pubkey) fail('Updater public key is missing.');
if (!Array.isArray(config.plugins?.updater?.endpoints) || config.plugins.updater.endpoints.length === 0) {
  fail('Updater endpoint list is missing.');
}

const entitlements = fs.readFileSync('backend/Entitlements.plist', 'utf8');
for (const key of [
  'com.apple.security.app-sandbox',
  'com.apple.security.network.client',
  'com.apple.security.network.server',
  'com.apple.security.device.audio-input',
  'com.apple.security.files.user-selected.read-write',
]) {
  if (!entitlements.includes(`<key>${key}</key>`)) fail(`Missing entitlement: ${key}`);
}

const info = fs.readFileSync('backend/Info.plist', 'utf8');
if (!info.includes('NSMicrophoneUsageDescription')) fail('Info.plist is missing microphone usage text.');

const cargo = fs.readFileSync('backend/Cargo.toml', 'utf8');
if (!cargo.includes('name = "thinclaw-desktop"')) fail('Cargo package name changed unexpectedly.');
if (!cargo.includes('security-framework = "3"')) fail('macOS Keychain dependency is missing.');

const keychain = fs.readFileSync('backend/src/openclaw/config/keychain.rs', 'utf8');
if (!keychain.includes('const SERVICE: &str = "com.thinclaw.desktop";')) {
  fail('Keychain service must match the bundle identifier.');
}

console.log('Bundle identity, updater metadata, entitlements, Info.plist, and keychain service are consistent.');
NODE

echo "== Engine override matrix =="
for engine in none ollama llamacpp mlx vllm; do
  echo "-- $engine"
  STRICT_SIDECARS=0 INCLUDE_CHROMIUM=auto bash scripts/generate_tauri_overrides.sh "$engine" >/tmp/thinclaw-override-"$engine".log
  ENGINE="$engine" node <<'NODE'
const fs = require('fs');
const engine = process.env.ENGINE;
const override = JSON.parse(fs.readFileSync('backend/tauri.override.json', 'utf8'));
const bins = override.bundle?.externalBin ?? [];
const resources = override.bundle?.resources ?? [];

function fail(message) {
  console.error(`${engine}: ${message}`);
  process.exit(1);
}

if (!resources.includes('../../../deploy/**/*')) fail('deploy resources are not bundled.');
if (engine === 'none' && bins.length !== 0) fail(`cloud build should not include sidecars: ${bins.join(', ')}`);
if (engine === 'llamacpp' && !bins.includes('bin/llama-server')) fail('llama.cpp build must declare llama-server sidecar.');
if ((engine === 'mlx' || engine === 'vllm') && !bins.includes('bin/uv')) fail(`${engine} build must declare uv sidecar.`);
if (engine === 'ollama' && bins.includes('bin/llama-server')) fail('ollama build must not bundle llama-server.');

console.log(`${engine}: externalBin=[${bins.join(', ')}], resources=[${resources.join(', ')}]`);
NODE
done

echo "== Focused platform tests =="
cargo test --manifest-path backend/Cargo.toml --locked openclaw::ironclaw_secrets::tests:: -- --test-threads=1
cargo test --manifest-path backend/Cargo.toml --locked personas::tests::legacy_scrappy_persona_aliases_to_thinclaw -- --test-threads=1
cargo test --manifest-path backend/Cargo.toml --locked cloud::providers::icloud::tests:: -- --test-threads=1
cargo test --manifest-path backend/Cargo.toml --locked cloud::migration::tests::test_validated_manifest_relative_path_rejects_traversal -- --test-threads=1

echo "Packaging readiness validation complete."

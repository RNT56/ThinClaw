#!/usr/bin/env bash
#
# archive.sh — local, credential-optional TestFlight archive + export helper.
#
# Mirrors the tag-triggered `archive` job in .github/workflows/ios.yml so an
# operator can cut a TestFlight build from their own machine with their own
# Apple Developer team. It is deliberately a no-op-with-guidance when the
# required credentials are absent: the repo ships no Apple team, so running
# this with an empty environment must not error.
#
# What it does (only when credentials are present):
#   1. tuist install + generate           -> ThinClaw.xcworkspace
#   2. xcodebuild archive                  -> build/ThinClaw.xcarchive
#   3. xcodebuild -exportArchive           -> build/export/ThinClaw.ipa
#   4. xcrun altool --upload-app           -> TestFlight (optional; --upload)
#
# Required credentials (environment variables):
#   DEVELOPMENT_TEAM              Apple Developer team id (10 chars, e.g. ABCDE12345)
#   APP_STORE_CONNECT_KEY_ID      App Store Connect API key id
#   APP_STORE_CONNECT_ISSUER_ID   App Store Connect API key issuer id
#   APP_STORE_CONNECT_KEY_P8      The .p8 private key, base64-encoded
#                                 (base64 -i AuthKey_XXXX.p8 | tr -d '\n')
#
# Alternatively DEVELOPMENT_TEAM can be supplied via Config/Signing.local.xcconfig.
#
# Usage:
#   apps/ios/scripts/archive.sh            # archive + export .ipa (no upload)
#   apps/ios/scripts/archive.sh --upload   # also upload to TestFlight
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELF="${SCRIPT_DIR}/$(basename "${BASH_SOURCE[0]}")"
IOS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${IOS_DIR}"

SCHEME="ThinClaw"
WORKSPACE="ThinClaw.xcworkspace"
BUILD_DIR="${IOS_DIR}/build"
ARCHIVE_PATH="${BUILD_DIR}/ThinClaw.xcarchive"
EXPORT_DIR="${BUILD_DIR}/export"
EXPORT_OPTIONS_SRC="${IOS_DIR}/Config/ExportOptions.plist"

UPLOAD=0
for arg in "$@"; do
  case "$arg" in
    --upload) UPLOAD=1 ;;
    -h|--help)
      sed -n '2,45p' "${SELF}"
      exit 0
      ;;
    *)
      echo "error: unknown argument '$arg' (see --help)" >&2
      exit 2
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Resolve the Apple team id: env var wins, else Signing.local.xcconfig.
# ---------------------------------------------------------------------------
SIGNING_LOCAL="${IOS_DIR}/Config/Signing.local.xcconfig"
if [ -z "${DEVELOPMENT_TEAM:-}" ] && [ -f "${SIGNING_LOCAL}" ]; then
  DEVELOPMENT_TEAM="$(sed -n 's/^[[:space:]]*DEVELOPMENT_TEAM[[:space:]]*=[[:space:]]*//p' "${SIGNING_LOCAL}" | tail -1 | tr -d '[:space:]')"
fi

# The example team placeholder is not a real team.
if [ "${DEVELOPMENT_TEAM:-}" = "YOURTEAMID" ]; then
  DEVELOPMENT_TEAM=""
fi

# ---------------------------------------------------------------------------
# Credential gate: if we cannot sign for distribution, print guidance + exit 0.
# ---------------------------------------------------------------------------
missing=()
[ -z "${DEVELOPMENT_TEAM:-}" ] && missing+=("DEVELOPMENT_TEAM")
[ -z "${APP_STORE_CONNECT_KEY_ID:-}" ] && missing+=("APP_STORE_CONNECT_KEY_ID")
[ -z "${APP_STORE_CONNECT_ISSUER_ID:-}" ] && missing+=("APP_STORE_CONNECT_ISSUER_ID")
[ -z "${APP_STORE_CONNECT_KEY_P8:-}" ] && missing+=("APP_STORE_CONNECT_KEY_P8")

if [ "${#missing[@]}" -ne 0 ]; then
  cat >&2 <<EOF
================================================================================
TestFlight archive skipped — Apple distribution credentials are not configured.
================================================================================
Missing: ${missing[*]}

ThinClaw ships no Apple Developer team, so this is expected for contributors.
To cut a TestFlight build, provide your own credentials, e.g.:

  export DEVELOPMENT_TEAM=ABCDE12345
  export APP_STORE_CONNECT_KEY_ID=XXXXXXXXXX
  export APP_STORE_CONNECT_ISSUER_ID=00000000-0000-0000-0000-000000000000
  export APP_STORE_CONNECT_KEY_P8="\$(base64 -i AuthKey_XXXXXXXXXX.p8 | tr -d '\\n')"
  apps/ios/scripts/archive.sh --upload

(DEVELOPMENT_TEAM may instead live in Config/Signing.local.xcconfig.)

Exiting 0 — this is a no-op, not a failure.
EOF
  exit 0
fi

echo "==> Apple team: ${DEVELOPMENT_TEAM}"

# ---------------------------------------------------------------------------
# Prepare a private, gitignored workspace for the .p8 key and export options.
# ---------------------------------------------------------------------------
PRIVATE_DIR="$(mktemp -d)"
cleanup() { rm -rf "${PRIVATE_DIR}"; }
trap cleanup EXIT

P8_PATH="${PRIVATE_DIR}/AuthKey_${APP_STORE_CONNECT_KEY_ID}.p8"
printf '%s' "${APP_STORE_CONNECT_KEY_P8}" | base64 --decode > "${P8_PATH}"
chmod 600 "${P8_PATH}"

# Substitute the real team id into a working copy of ExportOptions.plist.
EXPORT_OPTIONS="${PRIVATE_DIR}/ExportOptions.plist"
cp "${EXPORT_OPTIONS_SRC}" "${EXPORT_OPTIONS}"
plutil -replace teamID -string "${DEVELOPMENT_TEAM}" "${EXPORT_OPTIONS}"

# Ensure Signing.local.xcconfig carries the team for the Tuist-generated project.
mkdir -p "${IOS_DIR}/Config"
printf 'DEVELOPMENT_TEAM = %s\n' "${DEVELOPMENT_TEAM}" > "${SIGNING_LOCAL}"

# ---------------------------------------------------------------------------
# Generate the workspace and archive.
# ---------------------------------------------------------------------------
echo "==> tuist install && generate"
tuist install
tuist generate --no-open

rm -rf "${ARCHIVE_PATH}" "${EXPORT_DIR}"
mkdir -p "${BUILD_DIR}"

echo "==> xcodebuild archive"
xcodebuild archive \
  -workspace "${WORKSPACE}" \
  -scheme "${SCHEME}" \
  -destination 'generic/platform=iOS' \
  -archivePath "${ARCHIVE_PATH}" \
  -authenticationKeyPath "${P8_PATH}" \
  -authenticationKeyID "${APP_STORE_CONNECT_KEY_ID}" \
  -authenticationKeyIssuerID "${APP_STORE_CONNECT_ISSUER_ID}" \
  -allowProvisioningUpdates \
  DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM}"

echo "==> xcodebuild -exportArchive"
xcodebuild -exportArchive \
  -archivePath "${ARCHIVE_PATH}" \
  -exportOptionsPlist "${EXPORT_OPTIONS}" \
  -exportPath "${EXPORT_DIR}" \
  -authenticationKeyPath "${P8_PATH}" \
  -authenticationKeyID "${APP_STORE_CONNECT_KEY_ID}" \
  -authenticationKeyIssuerID "${APP_STORE_CONNECT_ISSUER_ID}" \
  -allowProvisioningUpdates

IPA_PATH="$(/usr/bin/find "${EXPORT_DIR}" -maxdepth 1 -name '*.ipa' | head -1)"
if [ -z "${IPA_PATH}" ]; then
  echo "error: no .ipa produced in ${EXPORT_DIR}" >&2
  exit 1
fi
echo "==> Exported ${IPA_PATH}"

# ---------------------------------------------------------------------------
# Optional upload to TestFlight.
# ---------------------------------------------------------------------------
if [ "${UPLOAD}" -eq 1 ]; then
  echo "==> xcrun altool --upload-app (TestFlight)"
  # altool resolves --apiKey against private-key search dirs, not a file path.
  # Point it at our private dir via API_PRIVATE_KEYS_DIR (the .p8 is already
  # named AuthKey_<key-id>.p8 there).
  API_PRIVATE_KEYS_DIR="${PRIVATE_DIR}" \
    xcrun altool --upload-app \
    --type ios \
    --file "${IPA_PATH}" \
    --apiKey "${APP_STORE_CONNECT_KEY_ID}" \
    --apiIssuer "${APP_STORE_CONNECT_ISSUER_ID}"
  echo "==> Uploaded to TestFlight."
else
  echo "==> Skipping upload (pass --upload to send to TestFlight)."
fi

echo "==> Done."

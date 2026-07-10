#!/usr/bin/env bash
#
# feature-tests.sh — run iOS-only package tests on a concrete supported
# simulator. Defaults to the newest installed iOS runtime (18 or newer); set
# IOS_RUNTIME_MAJOR to certify a specific release.
#
# Why this exists: the Feature packages under Packages/Features/* declare
# iOS-only support (no macOS), so `swift test` (which builds for the host Mac)
# cannot run them — the swift-test CI matrix skips them entirely. Their unit
# tests (e.g. FeatureOnboarding's DiscoveryStoreTests + OnboardingStoreTests)
# therefore need a real simulator destination. `xcodebuild test -scheme <Pkg>`
# builds the SPM package's generated scheme and runs its test target on the
# simulator, no Tuist workspace required.
#
# Behaviour:
#   * Auto-discovers every Packages/Features/* package that has a Tests/ target
#     plus iOS-only shared packages with tests.
#   * Picks a booted matching simulator, else the first available device.
#   * If no matching simulator/runtime is available, this is a SOFT no-op: it
#     prints a clear ::warning:: and exits 0 so contributors are never blocked
#     by runner image drift. Set FEATURE_TESTS_REQUIRE_SIM=1 to make a missing
#     simulator a hard failure instead. CI always enables this requirement.
#
# Usage:
#   apps/ios/scripts/feature-tests.sh
#   FEATURE_TESTS_REQUIRE_SIM=1 IOS_RUNTIME_MAJOR=18 apps/ios/scripts/feature-tests.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
FEATURES_DIR="${IOS_DIR}/Packages/Features"
REQUIRE_SIM="${FEATURE_TESTS_REQUIRE_SIM:-0}"
REQUESTED_RUNTIME="${IOS_RUNTIME_MAJOR:-}"

# GitHub Actions annotation helpers that degrade to plain echo locally.
warn() { echo "::warning::$*" 2>/dev/null || echo "WARN: $*"; }
note() { echo "::notice::$*" 2>/dev/null || echo "NOTE: $*"; }

# --- Discover feature packages that actually carry tests -------------------
PACKAGE_DIRS=()
for dir in "${FEATURES_DIR}"/*/; do
  if find "${dir}Tests" -name '*.swift' -type f >/dev/null 2>&1 \
    && [ -n "$(find "${dir}Tests" -name '*.swift' -type f 2>/dev/null)" ]; then
    PACKAGE_DIRS+=("${dir%/}")
  fi
done
for dir in "${IOS_DIR}/Packages/ThinClawWidgetKitShared"; do
  if [ -d "${dir}/Tests" ] && [ -n "$(find "${dir}/Tests" -name '*.swift' -type f 2>/dev/null)" ]; then
    PACKAGE_DIRS+=("${dir}")
  fi
done

if [ "${#PACKAGE_DIRS[@]}" -eq 0 ]; then
  note "No Feature packages have test targets yet; nothing to run."
  exit 0
fi
echo "==> iOS packages with tests: ${PACKAGE_DIRS[*]}"

# --- Find (and boot) a supported simulator --------------------------------
# Note: simctl JSON is captured to a variable first, then piped into python,
# so the python here-doc (its own stdin) does not swallow the JSON.
SIMCTL_JSON="$(xcrun simctl list devices available --json 2>/dev/null || echo '{}')"
DEVICE_UDID="$(
  printf '%s' "${SIMCTL_JSON}" | REQUESTED_RUNTIME="${REQUESTED_RUNTIME}" python3 -c '
import json, os, re, sys
data = json.load(sys.stdin)
requested = os.environ.get("REQUESTED_RUNTIME")
candidates = []
for runtime, devices in data.get("devices", {}).items():
    match = re.search(r"iOS-(\d+)", runtime)
    if not match:
        continue
    major = int(match.group(1))
    if major < 18 or (requested and major != int(requested)):
        continue
    for dev in devices:
        if not dev.get("isAvailable", True):
            continue
        # Prefer an iPhone; the store tests are UI-agnostic but a phone is the
        # canonical destination.
        if "iPad" in dev.get("name", ""):
            continue
        candidates.append((major, dev.get("state") == "Booted", dev["udid"]))
candidates.sort(key=lambda item: (item[0], item[1]), reverse=True)
print(candidates[0][2] if candidates else "")
'
)"

if [ -z "${DEVICE_UDID}" ]; then
  msg="No available supported iOS simulator${REQUESTED_RUNTIME:+ for iOS ${REQUESTED_RUNTIME}}; skipping iOS package tests."
  if [ "${REQUIRE_SIM}" = "1" ]; then
    echo "error: ${msg} (FEATURE_TESTS_REQUIRE_SIM=1)" >&2
    exit 1
  fi
  warn "${msg} Install the requested runtime to run them. Set FEATURE_TESTS_REQUIRE_SIM=1 to make this a hard failure."
  exit 0
fi

echo "==> Using supported iOS simulator udid: ${DEVICE_UDID}"
# Boot it if needed (idempotent; 'already booted' is not an error we care about).
xcrun simctl bootstatus "${DEVICE_UDID}" -b >/dev/null 2>&1 \
  || xcrun simctl boot "${DEVICE_UDID}" >/dev/null 2>&1 \
  || true

DESTINATION="platform=iOS Simulator,id=${DEVICE_UDID}"

# --- Run each package's tests ---------------------------------------------
FAILED=()
for pkg_dir in "${PACKAGE_DIRS[@]}"; do
  pkg="$(basename "${pkg_dir}")"
  echo ""
  echo "=============================================================="
  echo "==> xcodebuild test: ${pkg}"
  echo "=============================================================="
  if ( cd "${pkg_dir}" && xcodebuild test \
        -scheme "${pkg}" \
        -destination "${DESTINATION}" \
        -skipPackagePluginValidation \
        CODE_SIGNING_ALLOWED=NO ); then
    echo "==> ${pkg}: PASSED"
  else
    echo "==> ${pkg}: FAILED"
    FAILED+=("${pkg}")
  fi
done

echo ""
if [ "${#FAILED[@]}" -ne 0 ]; then
  echo "error: feature tests failed: ${FAILED[*]}" >&2
  exit 1
fi
echo "==> All iOS package tests passed."

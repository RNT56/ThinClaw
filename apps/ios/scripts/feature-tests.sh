#!/usr/bin/env bash
#
# feature-tests.sh — run the iOS-only Feature package test suites on a concrete
# iOS 26 simulator.
#
# Why this exists: the Feature packages under Packages/Features/* declare
# `.iOS(.v26)` only (no macOS), so `swift test` (which builds for the host Mac)
# cannot run them — the swift-test CI matrix skips them entirely. Their unit
# tests (e.g. FeatureOnboarding's DiscoveryStoreTests + OnboardingStoreTests)
# therefore need a real simulator destination. `xcodebuild test -scheme <Pkg>`
# builds the SPM package's generated scheme and runs its test target on the
# simulator, no Tuist workspace required.
#
# Behaviour:
#   * Auto-discovers every Packages/Features/* package that has a Tests/ target.
#   * Picks a booted iOS 26 simulator if one is booted, else the first available
#     iOS 26 device, and boots it.
#   * If NO iOS 26 simulator/runtime is available, this is a SOFT no-op: it
#     prints a clear ::warning:: and exits 0 so contributors are never blocked
#     by runner image drift. Set FEATURE_TESTS_REQUIRE_SIM=1 to make a missing
#     simulator a hard failure instead (recommended once the runner image is
#     known to always ship an iOS 26 runtime).
#
# Usage:
#   apps/ios/scripts/feature-tests.sh
#   FEATURE_TESTS_REQUIRE_SIM=1 apps/ios/scripts/feature-tests.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
FEATURES_DIR="${IOS_DIR}/Packages/Features"
REQUIRE_SIM="${FEATURE_TESTS_REQUIRE_SIM:-0}"

# GitHub Actions annotation helpers that degrade to plain echo locally.
warn() { echo "::warning::$*" 2>/dev/null || echo "WARN: $*"; }
note() { echo "::notice::$*" 2>/dev/null || echo "NOTE: $*"; }

# --- Discover feature packages that actually carry tests -------------------
PACKAGES=()
for dir in "${FEATURES_DIR}"/*/; do
  name="$(basename "${dir}")"
  if find "${dir}Tests" -name '*.swift' -type f >/dev/null 2>&1 \
    && [ -n "$(find "${dir}Tests" -name '*.swift' -type f 2>/dev/null)" ]; then
    PACKAGES+=("${name}")
  fi
done

if [ "${#PACKAGES[@]}" -eq 0 ]; then
  note "No Feature packages have test targets yet; nothing to run."
  exit 0
fi
echo "==> Feature packages with tests: ${PACKAGES[*]}"

# --- Find (and boot) an iOS 26 simulator ----------------------------------
# Prefer an already-booted iOS 26 device; otherwise the first available one.
# Note: simctl JSON is captured to a variable first, then piped into python,
# so the python here-doc (its own stdin) does not swallow the JSON.
SIMCTL_JSON="$(xcrun simctl list devices available --json 2>/dev/null || echo '{}')"
DEVICE_UDID="$(
  printf '%s' "${SIMCTL_JSON}" | python3 -c '
import json, sys
data = json.load(sys.stdin)
booted, first = None, None
for runtime, devices in data.get("devices", {}).items():
    if "iOS-26" not in runtime:
        continue
    for dev in devices:
        if not dev.get("isAvailable", True):
            continue
        # Prefer an iPhone; the store tests are UI-agnostic but a phone is the
        # canonical destination.
        if "iPad" in dev.get("name", ""):
            continue
        if first is None:
            first = dev["udid"]
        if dev.get("state") == "Booted" and booted is None:
            booted = dev["udid"]
print(booted or first or "")
'
)"

if [ -z "${DEVICE_UDID}" ]; then
  msg="No available iOS 26 simulator on this machine; skipping Feature package tests."
  if [ "${REQUIRE_SIM}" = "1" ]; then
    echo "error: ${msg} (FEATURE_TESTS_REQUIRE_SIM=1)" >&2
    exit 1
  fi
  warn "${msg} Install an iOS 26 runtime (Xcode 26) to run them. Set FEATURE_TESTS_REQUIRE_SIM=1 to make this a hard failure."
  exit 0
fi

echo "==> Using iOS 26 simulator udid: ${DEVICE_UDID}"
# Boot it if needed (idempotent; 'already booted' is not an error we care about).
xcrun simctl bootstatus "${DEVICE_UDID}" -b >/dev/null 2>&1 \
  || xcrun simctl boot "${DEVICE_UDID}" >/dev/null 2>&1 \
  || true

DESTINATION="platform=iOS Simulator,id=${DEVICE_UDID}"

# --- Run each package's tests ---------------------------------------------
FAILED=()
for pkg in "${PACKAGES[@]}"; do
  echo ""
  echo "=============================================================="
  echo "==> xcodebuild test: ${pkg}"
  echo "=============================================================="
  if ( cd "${FEATURES_DIR}/${pkg}" && xcodebuild test \
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
echo "==> All feature package tests passed: ${PACKAGES[*]}"

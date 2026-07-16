#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=simulator-support.sh
source "${SCRIPT_DIR}/simulator-support.sh"
cd "${IOS_DIR}"

SIMCTL_JSON="$(xcrun simctl list devices available --json)"
DEVICE_UDID="$(
  printf '%s' "${SIMCTL_JSON}" | python3 -c '
import json, re, sys
data = json.load(sys.stdin)
candidates = []
for runtime, devices in data.get("devices", {}).items():
    match = re.search(r"iOS-(\d+)", runtime)
    if not match or int(match.group(1)) < 18:
        continue
    for device in devices:
        if device.get("isAvailable", True) and "iPhone" in device.get("name", ""):
            candidates.append((int(match.group(1)), device.get("state") == "Booted", device["udid"]))
candidates.sort(key=lambda item: (item[0], item[1]), reverse=True)
print(candidates[0][2] if candidates else "")
'
)"

if [ -z "${DEVICE_UDID}" ]; then
  echo "error: no supported iOS simulator is installed" >&2
  exit 1
fi

wait_for_simulator_boot "${DEVICE_UDID}"

rm -rf build/app-tests.xcresult
xcodebuild test \
  -workspace ThinClaw.xcworkspace \
  -scheme ThinClaw \
  -destination "platform=iOS Simulator,id=${DEVICE_UDID}" \
  -resultBundlePath build/app-tests.xcresult \
  CODE_SIGNING_ALLOWED=NO

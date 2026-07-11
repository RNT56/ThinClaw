#!/usr/bin/env bash

# Wait for a simulator without allowing CoreSimulator to hold a CI job open.
# The caller is expected to run with `set -e` and treat a non-zero result as a
# hard test setup failure.
wait_for_simulator_boot() {
  local device_udid="$1"
  local timeout_seconds="${SIMULATOR_BOOT_TIMEOUT_SECONDS:-90}"

  DEVICE_UDID="${device_udid}" \
    SIMULATOR_BOOT_TIMEOUT_SECONDS="${timeout_seconds}" \
    python3 - <<'PY'
import os
import subprocess
import sys

udid = os.environ["DEVICE_UDID"]
raw_timeout = os.environ["SIMULATOR_BOOT_TIMEOUT_SECONDS"]

try:
    timeout = int(raw_timeout)
except ValueError:
    print(
        f"error: SIMULATOR_BOOT_TIMEOUT_SECONDS must be an integer, got {raw_timeout!r}",
        file=sys.stderr,
    )
    sys.exit(2)

if timeout <= 0:
    print("error: SIMULATOR_BOOT_TIMEOUT_SECONDS must be positive", file=sys.stderr)
    sys.exit(2)


def run_simctl(*args, command_timeout):
    try:
        return subprocess.run(
            ["xcrun", "simctl", *args],
            capture_output=True,
            check=False,
            text=True,
            timeout=command_timeout,
        )
    except subprocess.TimeoutExpired:
        return None


last_output = ""
for attempt in range(1, 3):
    # `boot` returns a non-zero status when the device is already booted. The
    # authoritative readiness result comes from the bounded `bootstatus` call.
    run_simctl("boot", udid, command_timeout=15)
    result = run_simctl("bootstatus", udid, "-b", command_timeout=timeout)
    if result is not None and result.returncode == 0:
        sys.exit(0)

    if result is None:
        last_output = f"bootstatus exceeded {timeout}s"
    else:
        last_output = (result.stderr or result.stdout).strip()
        if not last_output:
            last_output = f"bootstatus exited {result.returncode}"

    if attempt == 1:
        print(
            f"::warning::Simulator {udid} was not ready ({last_output}); "
            "resetting it and retrying once."
        )
        run_simctl("shutdown", udid, command_timeout=30)

print(
    f"error: simulator {udid} did not become ready after two bounded attempts: "
    f"{last_output}",
    file=sys.stderr,
)
sys.exit(1)
PY
}

#!/usr/bin/env bash
set -euo pipefail

image=${1:-thinclaw-worker:ci}
suffix=$$
idle_container="thinclaw-worker-health-idle-${suffix}"
active_container="thinclaw-worker-health-active-${suffix}"

cleanup() {
  docker container rm --force "$idle_container" "$active_container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_for_health() {
  local container=$1
  local expected=$2
  local attempts=${3:-45}
  local status
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    status=$(docker inspect --format '{{.State.Health.Status}}' "$container")
    if [[ "$status" == "$expected" ]]; then
      return 0
    fi
    sleep 2
  done
  docker inspect "$container"
  echo "container $container did not become $expected" >&2
  return 1
}

docker image inspect "$image" >/dev/null

# Idle liveness is credential-free and must become healthy from the event loop.
docker run --detach --name "$idle_container" "$image" worker-health-loop >/dev/null
wait_for_health "$idle_container" healthy
docker exec "$idle_container" thinclaw worker-health --max-age 20 >/dev/null
docker container rm --force "$idle_container" >/dev/null

# Active scheduling must remain healthy. Stopping PID 1 freezes the heartbeat,
# which must become unhealthy; resuming it must recover without recreation.
docker run --detach --name "$active_container" "$image" worker-health-loop --active >/dev/null
wait_for_health "$active_container" healthy
docker kill --signal STOP "$active_container" >/dev/null
wait_for_health "$active_container" unhealthy
if docker exec "$active_container" thinclaw worker-health --max-age 20 >/dev/null 2>&1; then
  echo "stopped worker unexpectedly retained a fresh heartbeat" >&2
  exit 1
fi
docker kill --signal CONT "$active_container" >/dev/null
wait_for_health "$active_container" healthy
docker exec "$active_container" thinclaw worker-health --max-age 20 >/dev/null

echo "worker image health contract passed for $image"

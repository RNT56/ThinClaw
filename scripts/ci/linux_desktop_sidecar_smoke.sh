#!/usr/bin/env bash
set -euo pipefail

SESSION_MODE="${1:-}"
if [[ -z "$SESSION_MODE" ]]; then
    echo "usage: $0 <gnome-x11|plasma-kwin-wayland|kde-wayland|openbox-x11>" >&2
    exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SIDECAR="$REPO_ROOT/desktop-sidecars/thinclaw_desktop_bridge.py"
TMP_DIR="$(mktemp -d)"
SESSION_PIDS=()

cleanup() {
    local pid=""
    for pid in "${SESSION_PIDS[@]}"; do
        kill "$pid" >/dev/null 2>&1 || true
        wait "$pid" >/dev/null 2>&1 || true
    done
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

export THINCLAW_HOME="$TMP_DIR/home"
export HOME="$TMP_DIR/home"
export NO_AT_BRIDGE=0
export GTK_MODULES="${GTK_MODULES:-gail:atk-bridge}"
mkdir -p "$THINCLAW_HOME"

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

require_command() {
    local command_name="$1"
    if ! command_exists "$command_name"; then
        echo "missing required command for $SESSION_MODE: $command_name" >&2
        exit 1
    fi
}

wait_for_path() {
    local path="$1"
    local label="$2"
    local attempt=""
    for attempt in $(seq 1 80); do
        if [[ -e "$path" ]]; then
            return 0
        fi
        sleep 0.1
    done
    echo "timed out waiting for $label at $path" >&2
    exit 1
}

start_x11_session() {
    local desktop="$1"
    export DISPLAY="${DISPLAY:-:99}"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP="$desktop"

    Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$TMP_DIR/xvfb.log" 2>&1 &
    SESSION_PIDS+=("$!")

    for _ in $(seq 1 80); do
        if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
            break
        fi
        sleep 0.1
    done
    xdpyinfo -display "$DISPLAY" >/dev/null

    openbox >"$TMP_DIR/openbox.log" 2>&1 &
    SESSION_PIDS+=("$!")
    sleep 1

    xterm -geometry 80x24+20+20 -T "ThinClaw CI Smoke" >"$TMP_DIR/xterm.log" 2>&1 &
    SESSION_PIDS+=("$!")
    sleep 1
}

wait_for_process() {
    local pattern="$1"
    local label="$2"
    local attempt=""
    for attempt in $(seq 1 100); do
        if pgrep -f "$pattern" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "timed out waiting for $label process matching $pattern" >&2
    return 1
}

start_plasma_kwin_wayland_session() {
    require_command kwin_wayland
    require_command plasmashell

    export XDG_SESSION_TYPE=wayland
    export XDG_CURRENT_DESKTOP=KDE:Plasma
    export KDE_FULL_SESSION=true
    export KDE_SESSION_VERSION="${KDE_SESSION_VERSION:-6}"
    export XDG_SESSION_DESKTOP=KDE
    export XDG_RUNTIME_DIR="$TMP_DIR/runtime"
    export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-kwin-thinclaw-ci}"
    export KWIN_COMPOSE=Q
    export QT_QPA_PLATFORM=wayland
    export QT_LOGGING_RULES="${QT_LOGGING_RULES:-kwin_*.warning=true}"
    mkdir -p "$XDG_RUNTIME_DIR"
    chmod 700 "$XDG_RUNTIME_DIR"

    local child_env="$TMP_DIR/kwin-child.env"
    local probe="$TMP_DIR/kwin-session-probe.sh"
    if command_exists xterm; then
        cat >"$probe" <<EOF
#!/usr/bin/env sh
env > "$child_env"
exec xterm -geometry 80x24+20+20 -T "ThinClaw CI Plasma Smoke"
EOF
    else
        cat >"$probe" <<EOF
#!/usr/bin/env sh
env > "$child_env"
sleep 120
EOF
    fi
    chmod +x "$probe"

    local -a apps=("plasmashell")
    apps+=("$probe")

    kwin_wayland \
        --virtual \
        --width 1280 \
        --height 800 \
        --socket "$WAYLAND_DISPLAY" \
        --xwayland \
        --no-lockscreen \
        --no-global-shortcuts \
        "${apps[@]}" \
        >"$TMP_DIR/kwin_wayland.log" 2>&1 &
    SESSION_PIDS+=("$!")

    wait_for_path "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" "KWin Wayland socket"
    wait_for_path "$child_env" "KWin child application environment"
    if grep -q '^DISPLAY=' "$child_env"; then
        export DISPLAY
        DISPLAY="$(sed -n 's/^DISPLAY=//p' "$child_env" | tail -n 1)"
    fi
    if ! wait_for_process "plasmashell" "Plasma shell"; then
        cat "$TMP_DIR/kwin_wayland.log" >&2 || true
        exit 1
    fi
    sleep 2
    if ! pgrep -f "plasmashell" >/dev/null 2>&1; then
        cat "$TMP_DIR/kwin_wayland.log" >&2 || true
        echo "Plasma shell exited before smoke checks could run" >&2
        exit 1
    fi
}

case "$SESSION_MODE" in
    gnome-x11)
        start_x11_session "GNOME"
        ;;
    openbox-x11)
        start_x11_session "openbox"
        ;;
    kde-wayland)
        start_plasma_kwin_wayland_session
        ;;
    plasma-kwin-wayland)
        start_plasma_kwin_wayland_session
        ;;
    *)
        echo "unsupported desktop smoke session: $SESSION_MODE" >&2
        exit 2
        ;;
esac

sidecar() {
    local command="$1"
    local payload="${2:-}"
    if [[ -z "$payload" ]]; then
        payload="{}"
    fi
    THINCLAW_DESKTOP_BRIDGE_PAYLOAD="$payload" python3 "$SIDECAR" "$command" </dev/null
}

assert_ok() {
    local json="$1"
    JSON="$json" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["JSON"])
if payload.get("ok") is not True:
    raise SystemExit(f"sidecar returned failure: {payload}")
PY
}

assert_health() {
    local json="$1"
    local mode="$2"
    JSON="$json" MODE="$mode" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["JSON"])
mode = os.environ["MODE"]
result = payload["result"]
caps = result["desktop_capabilities"]
if not caps["supported_desktop"]:
    raise SystemExit(f"desktop session was not detected as supported: {caps}")
if mode.endswith("-x11"):
    if caps["session_type"] != "x11" or not caps["display"]:
        raise SystemExit(f"expected X11 display/session, got {caps}")
    if "xdotool" not in caps["input_backends"]:
        raise SystemExit(f"expected xdotool input backend, got {caps}")
    if "wmctrl" not in caps["window_backends"]:
        raise SystemExit(f"expected wmctrl window backend, got {caps}")
elif mode in ("kde-wayland", "plasma-kwin-wayland"):
    if caps["session_type"] != "wayland" or not caps["wayland_display"]:
        raise SystemExit(f"expected Wayland display/session, got {caps}")
    if not {"kde", "plasma"}.intersection(set(caps["desktops"])):
        raise SystemExit(f"expected KDE/Plasma desktop tokens, got {caps}")
    if not caps["display"] or "xdotool" not in caps["input_backends"]:
        raise SystemExit(f"expected KWin XWayland display with xdotool input, got {caps}")
else:
    raise SystemExit(f"unhandled mode {mode}")
PY
}

assert_screen_capture() {
    local json="$1"
    local mode="$2"
    JSON="$json" MODE="$mode" python3 - <<'PY'
import json
import os

payload = json.loads(os.environ["JSON"])
mode = os.environ["MODE"]
result = payload["result"]
if mode in ("kde-wayland", "plasma-kwin-wayland") and result.get("backend") != "spectacle":
    raise SystemExit(f"expected KDE/Plasma screen capture through spectacle, got {result}")
PY
}

health="$(sidecar health '{}')"
echo "$health"
assert_ok "$health"
assert_health "$health" "$SESSION_MODE"

permissions="$(sidecar permissions '{}')"
echo "$permissions"
assert_ok "$permissions"

snapshot="$(sidecar ui '{"action":"snapshot"}')"
echo "$snapshot"
assert_ok "$snapshot"

windows="$(sidecar apps '{"action":"windows"}')"
echo "$windows"
assert_ok "$windows"

if [[ "$SESSION_MODE" == *"-x11" ]]; then
    click="$(sidecar ui '{"action":"click","x":24,"y":24}')"
    echo "$click"
    assert_ok "$click"
fi

type_text="$(sidecar ui '{"action":"type_text","text":"thinclaw-ci-desktop-smoke"}')"
echo "$type_text"
assert_ok "$type_text"

screen_path="$TMP_DIR/screen.png"
if sidecar screen "{\"action\":\"capture\",\"path\":\"$screen_path\"}" >"$TMP_DIR/screen.json"; then
    cat "$TMP_DIR/screen.json"
    assert_ok "$(cat "$TMP_DIR/screen.json")"
    assert_screen_capture "$(cat "$TMP_DIR/screen.json")" "$SESSION_MODE"
    test -s "$screen_path"
else
    cat "$TMP_DIR/screen.json" >&2 || true
    exit 1
fi

echo "ThinClaw Linux desktop sidecar smoke passed for $SESSION_MODE"

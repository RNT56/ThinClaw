#!/bin/sh

set -eu

fail() {
    printf 'ThinClaw container configuration error: %s\n' "$1" >&2
    exit 64
}

require_hex_32() {
    variable_name="$1"
    value="$2"

    [ "${#value}" -eq 64 ] || fail "$variable_name must be exactly 64 hexadecimal characters"
    case "$value" in
        *[!0-9A-Fa-f]*)
            fail "$variable_name must be exactly 64 hexadecimal characters"
            ;;
    esac
}

gateway_auth_token="${GATEWAY_AUTH_TOKEN:-}"
secrets_master_key="${SECRETS_MASTER_KEY:-}"

require_hex_32 GATEWAY_AUTH_TOKEN "$gateway_auth_token"
require_hex_32 SECRETS_MASTER_KEY "$secrets_master_key"
if [ "${DATABASE_BACKEND:-libsql}" = postgres ]; then
    require_hex_32 POSTGRES_PASSWORD "${POSTGRES_PASSWORD:-}"
fi

thinclaw_home="${THINCLAW_HOME:-/data/.thinclaw}"
workspace_root="${WORKSPACE_ROOT:-/workspace}"

case "$thinclaw_home" in
    /*) ;;
    *) fail "THINCLAW_HOME must be an absolute path" ;;
esac
case "$workspace_root" in
    /*) ;;
    *) fail "WORKSPACE_ROOT must be an absolute path" ;;
esac

mkdir -p "$thinclaw_home" "$workspace_root"
[ -w "$thinclaw_home" ] || fail "THINCLAW_HOME is not writable"
[ -w "$workspace_root" ] || fail "WORKSPACE_ROOT is not writable"

exec thinclaw "$@"

#!/bin/sh

set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
entrypoint="$repo_root/deploy/docker-entrypoint.sh"
test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

valid_gateway_token="$(printf '%64s' '' | tr ' ' a)"
valid_master_key="$(printf '%64s' '' | tr ' ' b)"

mkdir -p "$test_root/bin"
printf '%s\n' \
    '#!/bin/sh' \
    'printf "%s\n" "$*" > "$CAPTURE_FILE"' \
    > "$test_root/bin/thinclaw"
chmod +x "$test_root/bin/thinclaw"

expect_failure() {
    name="$1"
    shift
    if "$@" >"$test_root/$name.log" 2>&1; then
        printf 'expected %s to fail\n' "$name" >&2
        exit 1
    fi
}

expect_failure missing_gateway env \
    PATH="$test_root/bin:/usr/bin:/bin" \
    SECRETS_MASTER_KEY="$valid_master_key" \
    "$entrypoint" run

invalid_gateway_token='do-not-print-this-invalid-token'
expect_failure invalid_gateway env \
    PATH="$test_root/bin:/usr/bin:/bin" \
    GATEWAY_AUTH_TOKEN="$invalid_gateway_token" \
    SECRETS_MASTER_KEY="$valid_master_key" \
    "$entrypoint" run
if grep -Fq "$invalid_gateway_token" "$test_root/invalid_gateway.log"; then
    printf 'entrypoint leaked an invalid secret value\n' >&2
    exit 1
fi

expect_failure relative_state env \
    PATH="$test_root/bin:/usr/bin:/bin" \
    GATEWAY_AUTH_TOKEN="$valid_gateway_token" \
    SECRETS_MASTER_KEY="$valid_master_key" \
    THINCLAW_HOME=relative-state \
    "$entrypoint" run

expect_failure invalid_postgres_password env \
    PATH="$test_root/bin:/usr/bin:/bin" \
    GATEWAY_AUTH_TOKEN="$valid_gateway_token" \
    SECRETS_MASTER_KEY="$valid_master_key" \
    DATABASE_BACKEND=postgres \
    POSTGRES_PASSWORD=CHANGE_ME \
    "$entrypoint" run

capture_file="$test_root/arguments"
state_dir="$test_root/state/.thinclaw"
workspace_dir="$test_root/workspace"
env \
    PATH="$test_root/bin:/usr/bin:/bin" \
    CAPTURE_FILE="$capture_file" \
    GATEWAY_AUTH_TOKEN="$valid_gateway_token" \
    SECRETS_MASTER_KEY="$valid_master_key" \
    THINCLAW_HOME="$state_dir" \
    WORKSPACE_ROOT="$workspace_dir" \
    "$entrypoint" run --skip-setup-check

test -d "$state_dir"
test -d "$workspace_dir"
test "$(cat "$capture_file")" = 'run --skip-setup-check'

printf 'Docker entrypoint contract passed.\n'

#!/usr/bin/env bash

# Configure an optional distributor-owned Google OAuth client for an official
# binary build. The release workflow passes repository secrets through the
# THINCLAW_RELEASE_* input names; only a complete pair is promoted to the
# compile-time THINCLAW_GOOGLE_* names consumed by Rust's option_env! macros.

set -euo pipefail

client_id="${THINCLAW_RELEASE_GOOGLE_CLIENT_ID:-}"
client_secret="${THINCLAW_RELEASE_GOOGLE_CLIENT_SECRET:-}"

if [[ -z "$client_id" && -z "$client_secret" ]]; then
  echo "No ThinClaw-owned Google OAuth client is configured; this build remains BYOK."
  exit 0
fi

if [[ -z "$client_id" || -z "$client_secret" ]]; then
  echo "::error::THINCLAW_GOOGLE_CLIENT_ID and THINCLAW_GOOGLE_CLIENT_SECRET must both be configured or both be absent"
  exit 1
fi

if [[ "$client_id" =~ [[:space:]] || "$client_secret" =~ [[:space:]] ]]; then
  echo "::error::ThinClaw Google OAuth build credentials must not contain whitespace"
  exit 1
fi

if [[ -z "${GITHUB_ENV:-}" ]]; then
  echo "::error::GITHUB_ENV is required to configure release build credentials"
  exit 1
fi

# Do not print either value. GitHub masks repository secrets in logs, and this
# step only promotes the complete pair into the environment of later steps.
printf 'THINCLAW_GOOGLE_CLIENT_ID=%s\n' "$client_id" >> "$GITHUB_ENV"
printf 'THINCLAW_GOOGLE_CLIENT_SECRET=%s\n' "$client_secret" >> "$GITHUB_ENV"
echo "Configured the optional ThinClaw-owned Google OAuth client for this official build."

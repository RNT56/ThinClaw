# Line channel (`line`)

> **Production status: `production`**

LINE Messaging API webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`line.capabilities.json`](./line.capabilities.json).

## Authentication

**Auth: production-grade.** Inbound requests are verified with `hmac_sha256_base64_body` over the raw body against `X-Line-Signature`, keyed on `line_channel_secret` — LINE's native scheme.

## Setup

See the `setup.required_secrets` block in `line.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

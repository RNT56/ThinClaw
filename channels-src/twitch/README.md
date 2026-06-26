# Twitch channel (`twitch`)

> **Production status: `production`**

Twitch EventSub webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`twitch.capabilities.json`](./twitch.capabilities.json).

## Authentication

**Auth: production-grade.** Inbound requests are verified with `twitch_eventsub_hmac_sha256` over `message-id + timestamp + body` against `Twitch-Eventsub-Message-Signature`, plus the EventSub `webhook_callback_verification_pending` challenge.

## Setup

See the `setup.required_secrets` block in `twitch.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

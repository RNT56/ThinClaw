# Google Chat channel (`google_chat`)

> **Production status: `beta`**

Google Chat app webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`google_chat.capabilities.json`](./google_chat.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` only.** Google Chat events arrive with a Google-signed Bearer JWT in `Authorization` that must be verified against Google's public certs; this shim instead expects a shared `X-Webhook-Secret`.

## Setup

See the `setup.required_secrets` block in `google_chat.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

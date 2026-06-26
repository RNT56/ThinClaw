# Dingtalk channel (`dingtalk`)

> **Production status: `beta`**

DingTalk outgoing-robot webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`dingtalk.capabilities.json`](./dingtalk.capabilities.json).

## Authentication

**Auth caveat (beta): inbound auth is shared-secret `equals` only.** DingTalk signs callbacks with HMAC-SHA256 over `timestamp\nsecret` delivered as `timestamp`+`sign` query params; the `equals`/`X-Webhook-Secret` compare cannot validate a genuine DingTalk request. Deploy only behind a trusted ingress.

## Setup

See the `setup.required_secrets` block in `dingtalk.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

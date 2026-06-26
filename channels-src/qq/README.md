# QQ channel (`qq`)

> **Production status: `beta`**

QQ bot webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`qq.capabilities.json`](./qq.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` cannot validate QQ.** QQ bot webhooks use **Ed25519** signatures over the body. This is the cheapest shim to promote to production by reusing the Discord Ed25519 verification helper (an `Ed25519Body` host variant); until then treat inbound auth as a shared secret only.

## Setup

See the `setup.required_secrets` block in `qq.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

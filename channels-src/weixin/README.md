# Weixin channel (`weixin`)

> **Production status: `beta`**

WeChat Official Account webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`weixin.capabilities.json`](./weixin.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` only.** WeChat uses `signature` = SHA1(sort(token, timestamp, nonce)); this shim echoes `echostr` without verifying it and uses `equals` for inbound messages.

## Setup

See the `setup.required_secrets` block in `weixin.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

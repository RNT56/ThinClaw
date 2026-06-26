# Feishu Lark channel (`feishu_lark`)

> **Production status: `beta`**

Feishu / Lark event-callback webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`feishu_lark.capabilities.json`](./feishu_lark.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` against `verification_token` only.** Feishu uses a verification token plus (in encrypt mode) AES, and an `X-Lark-Signature` over timestamp+nonce+body — none of which this shim verifies.

## Setup

See the `setup.required_secrets` block in `feishu_lark.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

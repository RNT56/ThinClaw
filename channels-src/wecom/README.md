# Wecom channel (`wecom`)

> **Production status: `beta`**

WeCom / Enterprise WeChat webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`wecom.capabilities.json`](./wecom.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` only.** WeCom uses `msg_signature` (SHA1 over token+timestamp+nonce+encrypt) and AES; the GET `echostr` challenge is itself signature-gated on the real platform but echoed here unverified.

## Setup

See the `setup.required_secrets` block in `wecom.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

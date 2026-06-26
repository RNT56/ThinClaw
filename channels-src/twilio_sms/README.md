# Twilio SMS channel (`twilio_sms`)

> **Production status: `production`**

Twilio Programmable SMS webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`twilio_sms.capabilities.json`](./twilio_sms.capabilities.json).

## Authentication

**Auth: production-grade.** Inbound requests are verified with `twilio_request_signature` (URL + sorted params) against `X-Twilio-Signature`, keyed on `twilio_auth_token`.

## Setup

See the `setup.required_secrets` block in `twilio_sms.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

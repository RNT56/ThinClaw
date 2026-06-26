# Ms Teams channel (`ms_teams`)

> **Production status: `beta`**

Microsoft Teams / Bot Framework webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`ms_teams.capabilities.json`](./ms_teams.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` only.** Teams sends a signed JWT bearer that must be validated against the Bot Connector's OpenID metadata; `equals`/`X-Webhook-Secret` cannot perform this.

## Setup

See the `setup.required_secrets` block in `ms_teams.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

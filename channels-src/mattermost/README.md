# Mattermost channel (`mattermost`)

> **Production status: `beta`**

Mattermost outgoing-webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`mattermost.capabilities.json`](./mattermost.capabilities.json).

## Authentication

**Auth caveat (beta): `equals` only.** Mattermost outgoing webhooks carry a per-webhook `token` (commonly in the request body field `token`, not a header); `equals` against `X-Webhook-Secret` works only if the operator manually routes that token into the configured header.

## Setup

See the `setup.required_secrets` block in `mattermost.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

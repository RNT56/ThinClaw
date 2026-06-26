# Matrix channel (`matrix`)

> **Production status: `beta`**

Matrix webhook channel.

This is an `include!`-based thin shim over the shared webhook engine
(`channels-src/shared_webhook_channel/src/impl.rs`); its behavior is declared
entirely in [`matrix.capabilities.json`](./matrix.capabilities.json).

## Authentication

**Auth note (beta): route/proxy secret, not platform auth.** Matrix has no standard inbound webhook signature. `matrix_webhook_secret` is a ThinClaw **route/proxy secret** an operator places in front of the channel via `equals` — it is not Matrix platform authentication.

## Setup

See the `setup.required_secrets` block in `matrix.capabilities.json` for the
secrets the onboarding wizard prompts for, and `docs/CHANNEL_ARCHITECTURE.md`
for the shared webhook-channel model.

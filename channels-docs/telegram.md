# Telegram Channel Package

> Telegram is a packaged WASM channel in ThinClaw's channel runtime.

## What This Page Covers

This page is about the operator-facing Telegram setup path. For the architecture behind the native/WASM split, use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md).

## Runtime Shape

- channel type: WASM package
- transport shape: Telegram Bot API over webhook or polling-style host callbacks
- secret handling: bot credentials stay at the host boundary and are injected into the package runtime

## What Telegram Is Not

Telegram is not a native Rust channel in the current ThinClaw architecture. If another doc describes it that way, prefer the canonical architecture doc.

## Setup Notes

- create the bot with `@BotFather`
- configure the required Telegram credentials in ThinClaw's channel setup path
- ensure the user has started the bot before expecting inbound messages

## Behavior Highlights

- operator-facing messaging through the Telegram Bot API
- host-managed formatting and streaming behavior
- generated media replies use the host-side Telegram media helper and require a
  Bot API token that can send photos/documents to the target chat
- package-based deployment under the WASM channel runtime

## Related Docs

- [README.md](README.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [../docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md)

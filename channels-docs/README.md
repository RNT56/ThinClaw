# ThinClaw Channel Docs

This directory is the operator-facing index for channel and delivery surfaces.

For the canonical architecture model, start with [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md).

## Read This First

- The **gateway** is the operator control plane.
- ThinClaw has both **native channels** and **packaged WASM channels**.
- Discord has two paths: native Gateway delivery and packaged WASM interactions.

If you need the architectural why, use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md). If you need setup help, use the pages below.

## Operator Surfaces

| Surface | Runtime Shape | Page |
|---|---|---|
| Gateway UI and API | native | [gateway.md](gateway.md) |
| Generic HTTP ingress | native | [http.md](http.md) |
| Local REPL | native | [repl.md](repl.md) |

## Native Messaging Channels

| Channel | Notes | Page |
|---|---|---|
| Signal | signal-cli bridge | [signal.md](signal.md) |
| Nostr | relay-based native runtime | [nostr.md](nostr.md) |
| Gmail | native Gmail channel | [gmail.md](gmail.md) |
| iMessage | macOS-only local channel | [imessage.md](imessage.md) |
| Apple Mail | macOS-only mail channel | [apple-mail.md](apple-mail.md) |
| Discord Gateway | persistent native Discord path | [discord.md](discord.md) |

## Packaged WASM Channels

| Channel | Notes | Page |
|---|---|---|
| Telegram | packaged Bot API channel | [telegram.md](telegram.md) |
| Slack | packaged Events API channel | [slack.md](slack.md) |
| WhatsApp | packaged webhook channel | currently documented through architecture and registry references |
| Discord interactions | packaged slash-command / webhook path | [discord.md](discord.md) |
| Generic WASM runtime notes | runtime/operator context | [wasm.md](wasm.md) |

Packaged channels are distributed through the channel registry and loaded by the ThinClaw host runtime.

## Related Docs

- [../docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [../docs/EXTENSION_SYSTEM.md](../docs/EXTENSION_SYSTEM.md)

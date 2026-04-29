# ThinClaw Channel Docs

This directory is the operator-facing index for channel and delivery surfaces.

For the canonical architecture model, start with [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md).

## Read This First

- The **gateway** is the operator control plane.
- ThinClaw has both **native channels** and **packaged WASM channels**.
- Discord has two paths: native Gateway delivery and packaged WASM interactions.

If you need the architectural why, use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md). If you need setup help, use the pages below.

For shared identity vocabulary and cross-surface command names, also use:

- [../docs/IDENTITY_AND_PERSONALITY.md](../docs/IDENTITY_AND_PERSONALITY.md)
- [../docs/SURFACES_AND_COMMANDS.md](../docs/SURFACES_AND_COMMANDS.md)

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

## Native Lifecycle Placeholders

These surfaces are exposed as config-gated descriptors in channel status, but
do not start transports yet. They reserve the native lifecycle shape and must
route future inbound payloads through `IncomingEvent`, `mint_session_key`, and
`parse_slash_command`.

| Channel | Config flag | Notes |
|---|---|---|
| Matrix | `MATRIX_ENABLED` | rooms and DMs |
| Voice-call | `VOICE_CALL_ENABLED` | requires builds with `--features voice` before a real transport can run |
| APNs | `APNS_ENABLED` | device notification lifecycle |
| Browser push | `BROWSER_PUSH_ENABLED` | requires builds with `--features browser` before a real transport can run |

## Packaged WASM Channels

| Channel | Notes | Page |
|---|---|---|
| Telegram | packaged Bot API channel | [telegram.md](telegram.md) |
| Slack | packaged Events API channel | [slack.md](slack.md) |
| WhatsApp | packaged webhook channel | [whatsapp.md](whatsapp.md) |
| Discord interactions | packaged slash-command / webhook path | [discord.md](discord.md) |
| Mattermost | packaged webhook channel | registry metadata |
| Twilio SMS | packaged SMS webhook channel | registry metadata |
| DingTalk | packaged callback channel | registry metadata |
| Feishu/Lark | packaged event callback channel | registry metadata |
| WeCom | packaged enterprise callback channel | registry metadata |
| Weixin | packaged Official Account callback channel | registry metadata |
| QQ | packaged bot webhook channel | registry metadata |
| LINE | packaged Messaging API webhook channel | registry metadata |
| Google Chat | packaged app event channel | registry metadata |
| Microsoft Teams | packaged bot activity channel | registry metadata |
| Twitch | packaged EventSub/chat channel | registry metadata |
| Generic WASM runtime notes | runtime/operator context | [wasm.md](wasm.md) |

Packaged channels are distributed through the channel registry and loaded by the ThinClaw host runtime.
Channels marked "registry metadata" have registry package entries and capability filenames reserved; full operator setup pages should land with their channel implementations.

## Related Docs

- [../docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [../docs/EXTENSION_SYSTEM.md](../docs/EXTENSION_SYSTEM.md)

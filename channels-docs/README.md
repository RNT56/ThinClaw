# ThinClaw Channel Documentation

This directory contains setup and usage documentation for every channel
(input/output pathway) in ThinClaw.

Channels are how the agent receives messages and sends responses. Multiple
channels can be active simultaneously — the agent processes them all
through a unified `ChannelManager`.

## Channels

### Built-in Channels

| Channel | Transport | Platform | Config Trigger |
|---------|-----------|----------|----------------|
| [REPL / CLI](repl.md) | Terminal stdin/stdout | All | `CLI_ENABLED=true` (default) |
| [Web Gateway](gateway.md) | HTTP + SSE + WebSocket | All | `GATEWAY_ENABLED=true` (default) |
| [HTTP Webhook](http.md) | HTTP POST | All | `HTTP_PORT=8080` |

### Messaging Channels

| Channel | Transport | Platform | Config Trigger |
|---------|-----------|----------|----------------|
| [Signal](signal.md) | signal-cli daemon SSE + JSON-RPC | All | `SIGNAL_HTTP_URL=...` |
| [Telegram](telegram.md) | Telegram Bot API (long polling) | All | `TELEGRAM_BOT_TOKEN=...` |
| [Slack](slack.md) | Socket Mode (WebSocket) | All | `SLACK_BOT_TOKEN=...` |
| [Discord](discord.md) | Gateway WebSocket | All | `DISCORD_BOT_TOKEN=...` |
| WhatsApp | Cloud API webhook | All | `thinclaw extension install whatsapp` |
| [Nostr](nostr.md) | NIP-04 encrypted DMs | All | `NOSTR_PRIVATE_KEY=...` |

### Email Channels

| Channel | Transport | Platform | Config Trigger |
|---------|-----------|----------|----------------|
| [Gmail](gmail.md) | Google Pub/Sub pull | All | `GMAIL_ENABLED=true` |
| [Apple Mail](apple-mail.md) | Mail.app SQLite + AppleScript | macOS only | `APPLE_MAIL_ENABLED=true` |

### Native macOS Channels

| Channel | Transport | Platform | Config Trigger |
|---------|-----------|----------|----------------|
| [iMessage](imessage.md) | chat.db SQLite + AppleScript | macOS only | `IMESSAGE_ENABLED=true` |
| [Apple Mail](apple-mail.md) | Envelope Index + AppleScript | macOS only | `APPLE_MAIL_ENABLED=true` |

### Extensible Channels

| Channel | Transport | Platform | Config Trigger |
|---------|-----------|----------|----------------|
| [WASM Channels](wasm.md) | Plugin-defined (HTTP/polling) | All | `WASM_CHANNELS_ENABLED=true` (default) |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      ChannelManager                          │
│                                                              │
│  ┌──────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌──────────┐   │
│  │ REPL │ │ Signal │ │ Slack  │ │Gateway │ │ iMessage │   │
│  └──┬───┘ └───┬────┘ └───┬────┘ └───┬────┘ └────┬─────┘   │
│     │         │          │          │            │           │
│     └─────────┴──────────┴──────────┴────────────┘           │
│                          │                                    │
│                   select_all (futures)                        │
│                          │                                    │
│                          ▼                                    │
│                    MessageStream → Agent Loop                 │
└─────────────────────────────────────────────────────────────┘
```

## Message Formatting

LLMs output standard Markdown, but each platform uses its own formatting syntax.
ThinClaw automatically converts Markdown to platform-native formats:

| Channel | Output Format | How |
|---------|---------------|-----|
| **Telegram** | HTML | `markdown_to_telegram_html()` with `parse_mode=HTML` |
| **Slack** | Slack mrkdwn | `markdown_to_slack_mrkdwn()` — `**bold**`→`*bold*`, links→`<url\|text>` |
| **WhatsApp** | WhatsApp text | `markdown_to_whatsapp()` — `**bold**`→`*bold*`, links→`text (url)` |
| **Discord** | Markdown | Pass-through (Discord supports standard Markdown) |
| **REPL** | Terminal ANSI | `termimad` renders Markdown with ANSI colors |
| **Web Gateway** | Raw Markdown | Frontend renders Markdown via the browser |
| Signal, iMessage, Nostr | Plain text | No rich text API available |
| Gmail, Apple Mail | Plain text | Sent as `text/plain` |

Code blocks and inline code are always preserved unchanged across all converters.

## General Notes

- All channels implement the `Channel` trait with `start()`, `respond()`, `broadcast()`, `health_check()`, and `shutdown()`
- Messages from all channels are merged into a single `MessageStream` via `futures::stream::select_all`
- Each message carries metadata identifying its source channel, sender, and routing info
- Channel health is monitored via `ChannelHealthMonitor` with configurable intervals

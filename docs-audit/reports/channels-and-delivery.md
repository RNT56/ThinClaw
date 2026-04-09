# Channels and Delivery Report

## Executive Summary

ThinClaw’s actual channel model is hybrid, not uniform: native Rust channels handle persistent connections, local system access, and full `Channel` trait behavior, while WASM channels handle stateless webhook/polling integrations with credential isolation and hot reload. That split is explicit in [src/channels/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/mod.rs#L1), [src/channels/wasm/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/wasm/mod.rs#L1), and [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L1).

The main documentation drift is not that the project lacks channel docs; it is that the docs use inconsistent framing. `channels-docs/README.md` still reads like a flat channel catalog with old transport assumptions, while the code and registry clearly separate native runtime channels from packaged WASM channel artifacts. The result is that readers can easily miss the Discord interactions split, the WASM packaging model, and the fact that Telegram, Slack, Discord-interactions, and WhatsApp are shipped as registry-backed WASM channels.

## Actual Channel Model

ThinClaw exposes one unified `Channel` interface, but the implementation surfaces are split by transport and lifecycle requirements. Native channels are compiled into the binary and are used for REPL, HTTP webhook, the web gateway, Signal, Gmail, Nostr, iMessage, Apple Mail, and Discord Gateway. WASM channels are dynamically loaded from `~/.thinclaw/channels/` and are used for Telegram, WhatsApp, Slack, and Discord interactions. The WASM side is host-managed: the Rust host owns HTTP routing, polling, timing, credential injection, and hot reload.

The packaging model matters. The channel registry under [registry/channels/](/Users/vespian/coding/ThinClaw-main/registry/channels) shows Telegram, Slack, Discord, and WhatsApp as artifact-backed channel packages with `wasm32-wasip2` releases, which means the operator story is not just “turn on an env var.” It is “install a channel package, then configure its secret material and activation settings.”

## Channel Matrix: implementation + transport + doc status

| Channel | Implementation | Transport | Doc status |
|---|---|---|---|
| REPL / CLI | Native Rust | stdin/stdout | Mostly accurate in [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L14), but the index treats it as a generic built-in channel instead of a runtime mode. |
| Web Gateway | Native Rust | HTTP + SSE + WebSocket | Accurate in [channels-docs/gateway.md](/Users/vespian/coding/ThinClaw-main/channels-docs/gateway.md#L1), though it should be clearly positioned as the operator UI/control plane, not just a channel. |
| HTTP Webhook | Native Rust | HTTP POST | Mostly accurate in [channels-docs/http.md](/Users/vespian/coding/ThinClaw-main/channels-docs/http.md#L1); it should be described as part of the host webhook server. |
| Signal | Native Rust | signal-cli daemon HTTP/JSON-RPC + SSE | Mostly accurate in [channels-docs/signal.md](/Users/vespian/coding/ThinClaw-main/channels-docs/signal.md#L1). |
| Gmail | Native Rust | Pub/Sub pull | Present, but buried in the email section rather than surfaced as a core native channel. |
| Nostr | Native Rust | WebSocket relay | Present, but the index does not make the native-vs-WASM distinction explicit. |
| iMessage | Native Rust | SQLite polling + AppleScript | Accurate as a macOS-native channel, but the index should keep it in a macOS-only section. |
| Apple Mail | Native Rust | Envelope Index polling + AppleScript | Present twice in [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L31) and should be deduplicated. |
| Telegram | WASM channel | Webhook/polling over Bot API | Stale in [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L20) and [channels-docs/telegram.md](/Users/vespian/coding/ThinClaw-main/channels-docs/telegram.md#L1), which frame it as a generic long-polling bot instead of a packaged WASM channel. |
| WhatsApp | WASM channel | Cloud API webhook | Missing from the `channels-docs/README.md` index even though it is a first-class packaged channel in [registry/channels/whatsapp.json](/Users/vespian/coding/ThinClaw-main/registry/channels/whatsapp.json#L1). |
| Slack | WASM channel | Events API webhook | Stale in [channels-docs/slack.md](/Users/vespian/coding/ThinClaw-main/channels-docs/slack.md#L1) if read as a native transport guide; it should explicitly say it is a WASM channel package. |
| Discord Gateway | Native Rust | Gateway WebSocket + REST | The code supports this native channel, but `channels-docs/discord.md` only documents the Gateway side and omits the separate Discord interactions WASM package. |
| Discord interactions | WASM channel | Interactions webhook | Missing from `channels-docs/README.md` and should be documented separately or folded into Discord with a clear split. |

## Contradictions and Drift

- `channels-docs/README.md` collapses the whole system into one flat catalog and says Telegram is “Telegram Bot API (long polling)” and Slack is “Socket Mode (WebSocket),” which is too generic for the actual hybrid packaging model. The code explicitly splits native versus WASM channels in [src/channels/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/mod.rs#L1).
- The same index omits WhatsApp as a first-class linked doc even though it is a bundled WASM channel package in [registry/channels/whatsapp.json](/Users/vespian/coding/ThinClaw-main/registry/channels/whatsapp.json#L1).
- Discord is split across two implementation surfaces in code, but the docs only clearly cover the native Gateway side. That leaves the Discord interactions WASM path under-documented.
- `channels-docs/README.md` duplicates Apple Mail in both the “Email Channels” and “Native macOS Channels” sections.
- `docs/CHANNEL_ARCHITECTURE.md` is directionally correct, but it still uses a narrative tone that reads like implementation commentary rather than the canonical architecture reference.
- The channel registry is a better source of truth for installable WASM packages than the index text in `channels-docs/README.md`, because it captures the artifact-backed distribution model directly in [registry/channels/telegram.json](/Users/vespian/coding/ThinClaw-main/registry/channels/telegram.json#L1), [registry/channels/slack.json](/Users/vespian/coding/ThinClaw-main/registry/channels/slack.json#L1), [registry/channels/discord.json](/Users/vespian/coding/ThinClaw-main/registry/channels/discord.json#L1), and [registry/channels/whatsapp.json](/Users/vespian/coding/ThinClaw-main/registry/channels/whatsapp.json#L1).

## Canonical Doc Recommendations

`docs/CHANNEL_ARCHITECTURE.md` should be the canonical architecture reference for the native/WASM split, the transport decision rule, and the hot-reload model. `docs/BUILDING_CHANNELS.md` should remain the canonical internal guide for authoring new WASM channels.

`channels-docs/README.md` should stop trying to be the architecture doc. It should become an audience-first index that routes readers to per-channel setup pages, split by native channel, WASM channel, and operator surface. That index should also clearly call out which channels are bundled packages versus in-binary native channels.

`channels-docs/*.md` should remain the per-channel operator docs, but each page should explicitly state whether it is documenting a native Rust channel or a WASM package. Discord needs a split between Gateway and interactions. Telegram and Slack need their WASM packaging model surfaced up front. WhatsApp needs to be linked from the main index.

## Rewrite / Merge / Archive Recommendations

- Rewrite `channels-docs/README.md` into a concise channel map with canonical boundaries and links to the correct per-channel docs.
- Merge the duplicate Apple Mail rows into a single macOS-only entry.
- Add or surface a dedicated Discord interactions page or section so the WASM package is not invisible.
- Rewrite `channels-docs/telegram.md` and `channels-docs/slack.md` so they describe the actual WASM distribution model, not just the external service transport.
- Keep `channels-docs/gateway.md`, `channels-docs/http.md`, and `channels-docs/signal.md` as operator docs, but trim them to avoid duplicating the architecture story.
- Archive any old “one channel = one runtime shape” assumptions; they no longer match the code.

## Evidence Pointers

- [src/channels/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/mod.rs#L1)
- [src/channels/channel.rs](/Users/vespian/coding/ThinClaw-main/src/channels/channel.rs#L1)
- [src/channels/wasm/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/wasm/mod.rs#L1)
- [src/channels/wasm/channel_watcher.rs](/Users/vespian/coding/ThinClaw-main/src/channels/wasm/channel_watcher.rs#L1)
- [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L1)
- [docs/BUILDING_CHANNELS.md](/Users/vespian/coding/ThinClaw-main/docs/BUILDING_CHANNELS.md#L1)
- [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L1)
- [channels-docs/telegram.md](/Users/vespian/coding/ThinClaw-main/channels-docs/telegram.md#L1)
- [channels-docs/slack.md](/Users/vespian/coding/ThinClaw-main/channels-docs/slack.md#L1)
- [channels-docs/discord.md](/Users/vespian/coding/ThinClaw-main/channels-docs/discord.md#L1)
- [channels-docs/apple-mail.md](/Users/vespian/coding/ThinClaw-main/channels-docs/apple-mail.md#L1)
- [registry/channels/telegram.json](/Users/vespian/coding/ThinClaw-main/registry/channels/telegram.json#L1)
- [registry/channels/slack.json](/Users/vespian/coding/ThinClaw-main/registry/channels/slack.json#L1)
- [registry/channels/discord.json](/Users/vespian/coding/ThinClaw-main/registry/channels/discord.json#L1)
- [registry/channels/whatsapp.json](/Users/vespian/coding/ThinClaw-main/registry/channels/whatsapp.json#L1)
- [src/config/channels.rs](/Users/vespian/coding/ThinClaw-main/src/config/channels.rs#L1)
- [src/cli/channels.rs](/Users/vespian/coding/ThinClaw-main/src/cli/channels.rs#L1)

# Channel Architecture

ThinClaw uses a hybrid channel model.

Some delivery surfaces are compiled into the trusted Rust host. Others are packaged as WASM channels and loaded at runtime. The split is deliberate: it follows transport shape, lifecycle needs, local-system access, and trust-boundary requirements.

## The Short Version

- Use **native Rust channels** when the integration needs persistent connections, local filesystem access, or the full host `Channel` behavior.
- Use **WASM channels** when the integration is stateless, HTTP-driven, and benefits from credential isolation and hot reload.
- Treat the **gateway** as the operator control plane, not just another chat surface.

## Channel Inventory

| Surface | Implementation | Why |
|---|---|---|
| Gateway | native | operator UI, API, SSE, WebSocket, deep host integration |
| HTTP webhook | native | host-owned ingress and webhook handling |
| REPL | native | local process surface |
| Signal | native | long-lived bridge and daemon integration |
| Nostr | native | persistent relay connections |
| Gmail | native | Pub/Sub pull and OAuth-heavy host integration |
| iMessage | native | local `chat.db` access on macOS |
| Apple Mail | native | local mail index access on macOS |
| Discord Gateway | native | persistent Gateway connection |
| Telegram | WASM package | stateless Bot API path, host-managed credentials |
| Slack | WASM package | stateless Events API path, host-managed credentials |
| WhatsApp | WASM package | webhook-driven packaged channel |
| Discord interactions | WASM package | slash-command / webhook path |

The installable WASM packages are represented in `registry/channels/`. The host runtime loads them from `~/.thinclaw/channels/`.

## When ThinClaw Uses Native Channels

Native Rust is the right path when any of these are true:

1. The integration needs a persistent connection.
2. The integration needs local-system access.
3. The integration needs host-level `Channel` behavior directly.
4. The integration has stateful reconnection, token refresh, or complex lifecycle management that is simpler and safer in the trusted host runtime.

Examples:

- Discord Gateway
- Nostr
- Signal
- iMessage
- Apple Mail

## When ThinClaw Uses WASM Channels

WASM is the right path when all of these are true:

1. The integration is webhook-driven or otherwise stateless between callbacks.
2. The platform API is HTTP-based.
3. Credential isolation and package hot-reload are valuable.

Examples:

- Telegram
- Slack
- WhatsApp
- Discord interactions

## Why The Split Exists

This is not a leftover transition state.

ThinClaw intentionally uses different execution paths because they solve different problems:

- **Native channels** are best when the runtime must hold connection state or access the local machine.
- **WASM channels** are best when ThinClaw can keep credentials and policy in the host while letting the channel logic ship as a hot-loadable package.

That gives ThinClaw a cleaner trust model than pretending every integration is equivalent.

## Trust And Runtime Boundaries

- Native channels run in the trusted host runtime.
- WASM channels are package-based and host-managed.
- The host owns routing, policy, timing, and secret injection.
- A packaged channel should not be documented as if it were a native Rust transport just because the external service supports that transport.

For example:

- Telegram should be documented as a WASM channel package in ThinClaw.
- Slack should be documented as a WASM channel package in ThinClaw.
- Discord needs both paths documented clearly: native Gateway and packaged interactions.

## Formatting Guidance Ownership

Channel-specific formatting behavior belongs to the channel layer, not to generic prompt assembly.

- **Native channels** should expose platform guidance through `Channel::formatting_hints()`.
- **WASM channels** should declare platform guidance in their `*.capabilities.json` via `formatting_hints`.
- **Bundled WASM channels** may also ship a host-side fallback when a package omits explicit hints, but the package manifest remains the preferred source of truth.
- **Prompt assembly** should consume resolved hints injected by the channel/runtime path. It should not reintroduce channel-name switches inside `src/llm/reasoning.rs`.

Today the canonical lookup seam is `ChannelManager::formatting_hints_for()`. If you add or change channel-specific rendering behavior, update the owning native channel implementation or WASM manifest first so every surface sees the same guidance.

## Operator Docs

Use these pages for operator setup:

- [../channels-docs/README.md](../channels-docs/README.md)
- [../channels-docs/gateway.md](../channels-docs/gateway.md)
- [../channels-docs/telegram.md](../channels-docs/telegram.md)
- [../channels-docs/whatsapp.md](../channels-docs/whatsapp.md)
- [../channels-docs/slack.md](../channels-docs/slack.md)
- [../channels-docs/discord.md](../channels-docs/discord.md)

The operator docs should describe setup and usage. This page owns the architecture boundary.

## Building New Channels

If you are authoring a new channel:

- build a **native channel** when you need persistent or local capabilities
- build a **WASM channel** when you need stateless packaged delivery
- define formatting/rendering guidance on the channel itself (`Channel::formatting_hints()` for native, `formatting_hints` in `*.capabilities.json` for WASM)

See [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md) for the implementation guide.

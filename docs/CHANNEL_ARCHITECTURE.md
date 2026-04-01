# Channel Architecture

ThinClaw uses a **hybrid channel architecture**: some channels are native Rust modules compiled into the binary, while others are WASM components loaded at runtime from `~/.thinclaw/channels/`.

This is not a migration in progress — the two approaches serve different transport models with different requirements.

## Channel Inventory

| Channel    | Approach | Transport         | Why This Approach                              |
|------------|----------|-------------------|------------------------------------------------|
| Telegram   | WASM     | Webhook + Polling | Stateless, webhook-driven; credential isolation |
| WhatsApp   | WASM     | Webhook           | Stateless, webhook-driven; credential isolation |
| Slack       | WASM     | Events API webhook | Stateless, webhook-driven; credential isolation |
| Discord (interactions) | WASM | Interactions webhook | Stateless slash commands; credential isolation |
| Discord (Gateway)      | Native | WebSocket            | Requires persistent connection; full `Channel` trait |
| Signal     | Native   | HTTP bridge       | Long-polling HTTP bridge; binary IPC            |
| Gmail      | Native   | Pub/Sub pull      | Google Pub/Sub subscription; OAuth token mgmt   |
| Nostr      | Native   | WebSocket relay   | Persistent relay connections; NIP-04 crypto      |
| iMessage   | Native   | SQLite polling    | Local `chat.db` file access (macOS only)         |
| Apple Mail | Native   | Envelope Index    | Local filesystem access (macOS only)             |
| REPL/CLI   | Native   | stdin/stdout      | Process-local I/O                                |
| HTTP API   | Native   | HTTP server       | Inline with the webhook server                   |
| Gateway    | Native   | SSE + WebSocket   | WebUI real-time; deep integration with host state |

## When to Use WASM vs Native

### Use WASM when ALL of these are true:

1. **Webhook/polling transport** — The channel receives messages via stateless HTTP callbacks or periodic API polling. No persistent connection needed.

2. **Platform API is HTTP-only** — All interaction with the platform (send/receive/status) happens through REST API calls that the WASM host can proxy.

3. **Credential isolation matters** — The channel uses API tokens (bot tokens, access tokens) that should never be visible to the channel code. WASM channels use `{PLACEHOLDER}` syntax and the host substitutes credentials at request time.

### Use native Rust when ANY of these are true:

1. **Persistent connections** — Discord Gateway (WebSocket), Nostr relays (WebSocket), Signal (HTTP long-poll bridge). WASM modules are invoked per-callback; they cannot hold open connections between invocations.

2. **Local system access** — iMessage (`chat.db` SQLite), Apple Mail (Envelope Index files). WASM has no filesystem access.

3. **Full `Channel` trait features** — Streaming (`send_draft`), reactions (`react`), polls — these are host-level trait methods. WASM channels cannot implement them directly; the host must bridge them (adding complexity with no isolation benefit).

4. **Complex state management** — OAuth token refresh, connection reconnection with backoff, sequence tracking. While WASM has `workspace_read`/`workspace_write`, native Rust is far simpler for stateful protocols.

## Why Telegram and WhatsApp Are WASM

These two channels exemplify the ideal WASM use case:

### 1. Perfect Transport Fit

Both use **stateless webhook + REST API** patterns:

```
Telegram → POST /webhook/telegram → WASM on_http_request() → emit_message()
                                              ↓
Agent response → WASM on_respond() → HTTP POST api.telegram.org/sendMessage
```

The WASM callback model (`on_http_request`, `on_poll`, `on_respond`) maps 1:1 to the platform's interaction pattern. There is no "connection to maintain" between callbacks.

### 2. Credential Security

Bot tokens for Telegram and WhatsApp are **full-access tokens** — anyone with the token can impersonate the bot. WASM credential injection ensures:

- The WASM module **never sees** the raw token value
- URLs like `https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage` are populated by the host at request time
- Error messages are redacted to prevent token leakage in logs
- If a WASM module were malicious, it couldn't extract the token from its own code

A native Rust channel handles credentials via `SecretString` (zeroizable memory), but the token is still in-process and accessible to the channel code.

### 3. Dynamic Deployment & Hot-Reload

WASM channels are `.wasm` files loaded at runtime, with automatic hot-reload:

```bash
# Deploy a new channel — the channel watcher detects it automatically
cp telegram.wasm telegram.capabilities.json ~/.thinclaw/channels/
# Channel is loaded within ~3 seconds (no restart required)

# Update a channel — watcher detects mtime change, swaps it live
cp updated-telegram.wasm ~/.thinclaw/channels/telegram.wasm

# Remove a channel — watcher detects deletion, shuts it down
rm ~/.thinclaw/channels/telegram.wasm
```

The `ChannelWatcher` ([`src/channels/wasm/channel_watcher.rs`](../src/channels/wasm/channel_watcher.rs)) polls the channels directory every 3 seconds using mtime comparison (same approach as `ConfigWatcher`). Changes are debounced with a 1-second minimum interval.

This enables:
- Users building custom channel variants (e.g., different message formatting)
- Third-party channel distribution (install via file copy, not code merge)
- Separate channel build pipeline (faster iteration)
- **Zero-downtime updates** — channels are swapped live without restarting the agent

### 4. Crash Isolation

A bug in the Telegram WASM channel (e.g., a panic on malformed webhook JSON) is contained within the WASM sandbox. The host process continues running — other channels (Signal, Discord, etc.) are unaffected.

Native Rust: a panic in a channel's `async fn respond()` would be caught by the tokio runtime but could leave the channel in an inconsistent state.

## What WASM Cannot Do (and How We Handle It)

### Streaming (sendMessageDraft)

The `Channel::send_draft()` trait method operates at the **host level** — the dispatcher calls it on the `Channel` object during LLM streaming. WASM channels can't be called mid-stream because they use synchronous callbacks.

**Solution**: The WASM wrapper (`wrapper.rs`) implements `send_draft()` on the host side, making direct HTTP calls to `sendMessageDraft` using injected credentials. The WASM module is not involved in streaming at all.

```
Dispatcher → ChannelManager::send_draft() → WasmChannel::send_draft()
                                                    ↓ (host-side only)
                                              reqwest POST to Telegram API
```

This is an acceptable trade-off: streaming is a host-level concern (rate limiting, debouncing, accumulation), and the WASM module's job is message formatting and routing.

### Persistent WebSocket Connections

WASM channels are invoked per-event. They cannot hold a WebSocket connection open between callbacks. This is why Discord uses a **native** Gateway channel for real-time events and a separate WASM channel for slash command webhooks.

### Local File Access

WASM has no filesystem. Channels that read local files (iMessage, Apple Mail) must be native.

## Infrastructure Cost

The WASM sandbox infrastructure:

| Component          | Lines | Purpose                              |
|--------------------|-------|--------------------------------------|
| `wasm/wrapper.rs`  | 3,716 | Lifecycle, credentials, callbacks    |
| `wasm/host.rs`     | 1,524 | Host state, emit, workspace, HTTP    |
| `wasm/router.rs`   | 671   | Webhook routing to WASM channels     |
| `wasm/schema.rs`   | 588   | Capabilities file parsing            |
| `wasm/runtime.rs`  | 302   | WASM runtime initialization          |
| `wit/channel.wit`  | 367   | WIT interface contract               |
| **Total**          | **~7,200** | Shared by all WASM channels    |

This is amortized across Telegram (2,385 LOC), WhatsApp (1,448 LOC), Slack (962 LOC), and Discord interactions (744 LOC). The infrastructure also supports future third-party channels.

## Adding a New Channel

**Webhook/polling-based?** → Build a WASM channel. See [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md).

**Persistent connection / local access / full trait?** → Build a native Rust channel:
1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add `mod my_channel` to `src/channels/mod.rs`
4. Wire startup in `src/main.rs`

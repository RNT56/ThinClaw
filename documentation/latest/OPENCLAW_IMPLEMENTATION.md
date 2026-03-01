# OpenClaw Implementation — Comprehensive Technical Report

> **⚠️ HISTORICAL DOCUMENT — SUPERSEDED (2026-02-28)**
>
> The Node.js gateway architecture documented here has been **fully replaced** by **IronClaw** — a Rust library linked directly into the Tauri binary. For current documentation, see:
> - **`MICROSERVICES_AND_SIDECARS.md` §6** — IronClaw in-process engine architecture
> - **`TECHNICAL_ARCHITECTURE.md` §5** — Module structure, lifecycle, commands
> - **`ironclaw_library_roadmap.md`** — IronClaw library-side migration roadmap
> - **`ironclaw_phase1_spec.md`** / **`ironclaw_phase2_spec.md`** — Detailed implementation specs
> - **`tauri_dropin_spec.md`** — Complete integration specification (15 phases, all ✅)
>
> This document is preserved for historical reference. Sections on Keychain integration (§9), security model (§10), and frontend integration (§16-17) remain partially relevant.

> **Generated**: 2026-02-24
> **Scope**: ~~Full analysis of the OpenClaw integration within Scrappy~~ **(SUPERSEDED — see note above)**

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Architectural Overview](#2-architectural-overview)
3. [The OpenClawEngine (Node.js Sidecar)](#3-the-openclawengine-nodejs-sidecar)
4. [Rust Backend Module Structure](#4-rust-backend-module-structure)
5. [WebSocket Gateway Protocol](#5-websocket-gateway-protocol)
6. [WebSocket Client (`ws_client.rs`)](#6-websocket-client-ws_clientrs)
7. [Event Normalization Layer](#7-event-normalization-layer)
8. [Configuration System](#8-configuration-system)
9. [Identity & Key Management](#9-identity--key-management)
10. [Security Model](#10-security-model)
11. [Engine Configuration Generation](#11-engine-configuration-generation)
12. [Tauri Command Layer](#12-tauri-command-layer)
13. [MCP / IPC Request Handler](#13-mcp--ipc-request-handler)
14. [Fleet Management](#14-fleet-management)
15. [Remote Deployment](#15-remote-deployment)
16. [Frontend Integration (API Layer)](#16-frontend-integration-api-layer)
17. [Frontend Event Processing & UI Components](#17-frontend-event-processing--ui-components)
18. [Agent Boot Sequence](#18-agent-boot-sequence)
19. [Frontend Component Architecture](#19-frontend-component-architecture)
20. [Data Flow Diagrams](#20-data-flow-diagrams)
21. [File Inventory](#21-file-inventory)
22. [Known Issues & Technical Debt](#22-known-issues--technical-debt)

---

## 1. Executive Summary

**OpenClaw** is Scrappy's integrated agentic cockpit — a multi-channel AI agent platform powered by the **OpenClawEngine** ecosystem (an npm package: `openclaw@^2026.2.14`). It operates as a **Tauri sidecar** (Node.js process) managed by the Rust backend, communicating over a **WebSocket gateway protocol** with challenge/response authentication and Ed25519 cryptographic signing.

### Key Characteristics

| Aspect | Detail |
|--------|--------|
| **Engine Runtime** | Node.js v22+, bundled as Tauri sidecar |
| **Protocol** | WebSocket (Protocol v3), JSON-framed req/res/event |
| **Authentication** | Token + Ed25519 challenge/response signing |
| **Config Format** | `openclaw_engine.json` + `auth-profiles.json` + `identity.json` |
| **API Key Storage** | macOS Keychain (unified JSON blob, `com.schack.scrappy`) |
| **Gateway Port** | Default `18789`, dynamically allocated |
| **Deployment Modes** | Local (loopback), Remote (WS bridge), Docker, Ansible |
| **Supported Channels** | Slack (Socket Mode), Telegram (Bot API), WhatsApp (Web) |
| **Cloud Providers** | 18+ (Anthropic, OpenAI, Gemini, Groq, OpenRouter, xAI, Mistral, Venice, Together, Moonshot, MiniMax, NVIDIA, Qianfan, Xiaomi, Amazon Bedrock, Custom LLM) |
| **Local Inference** | llama.cpp/MLX/vLLM sidecar integration |
| **Tool System** | MCP-based with Rhai sandbox, skill manager, tool router |

---

## 2. Architectural Overview

```
┌────────────────────────────────────────────────────────────────────┐
│                         SCRAPPY APPLICATION                         │
│                                                                     │
│  ┌──────────────────────┐     ┌──────────────────────────────────┐ │
│  │   FRONTEND (React)   │     │     RUST BACKEND (Tauri)         │ │
│  │                      │     │                                  │ │
│  │  openclaw.ts          │◄───┤  openclaw/ module                │ │
│  │  use-openclaw-stream  │    │    ├── commands/                 │ │
│  │  OpenClawChatView     │    │    │   ├── mod.rs (Manager)      │ │
│  │  SettingsPages        │    │    │   ├── gateway.rs            │ │
│  │  FleetDashboard       │    │    │   ├── keys.rs               │ │
│  │                      │     │    │   ├── sessions.rs           │ │
│  │  Tauri Events:        │    │    │   ├── rpc.rs                │ │
│  │  "openclaw-event"     │    │    │   └── types.rs              │ │
│  │  "tool_event"         │    │    ├── config/                   │ │
│  │  "deploy-log/status"  │    │    │   ├── types.rs              │ │
│  └──────────┬───────────┘     │    │   ├── identity.rs           │ │
│             │ IPC (invoke)    │    │   ├── engine.rs              │ │
│             ▼                 │    │   └── keychain.rs            │ │
│  ┌──────────────────────┐     │    ├── ws_client.rs              │ │
│  │  Tauri Commands       │◄───┤    ├── frames.rs                 │ │
│  │  (openclaw_*)         │    │    ├── normalizer.rs             │ │
│  └──────────┬───────────┘     │    ├── ipc.rs (McpRequestHandler)│ │
│             │                 │    ├── fleet.rs                   │ │
│             ▼                 │    ├── deploy.rs                  │ │
│  ┌──────────────────────┐     │    └── extra_commands.rs          │ │
│  │  OpenClawWsClient     │    │                                  │ │
│  │  (WebSocket Actor)    │    └──────────────────────────────────┘ │
│  └──────────┬───────────┘                                         │
│             │ ws://127.0.0.1:18789                                │
│             ▼                                                     │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │              OPENCLAW ENGINE (Node.js Sidecar)                │ │
│  │                                                               │ │
│  │  main.js (wrapper) ──► openclaw npm package                   │ │
│  │                                                               │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌──────────────────────┐ │ │
│  │  │   Gateway    │  │  Agent      │  │  Channels            │ │ │
│  │  │   (WS srv)   │  │  Runtime    │  │  (Slack/Telegram/WA) │ │ │
│  │  └─────────────┘  └─────────────┘  └──────────────────────┘ │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌──────────────────────┐ │ │
│  │  │   Models     │  │  Sessions   │  │  Tools & Skills      │ │ │
│  │  │   (pi-ai)    │  │  Manager    │  │                      │ │ │
│  │  └─────────────┘  └─────────────┘  └──────────────────────┘ │ │
│  └──────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────┘
```

### Three-Layer Architecture

1. **Frontend Layer** — React/TypeScript UI that invokes Tauri commands and listens to Tauri events (`openclaw-event`, `tool_event`)
2. **Rust Backend Layer** — The `openclaw/` module that manages the engine lifecycle, generates configs, runs the WebSocket client, and bridges IPC
3. **Engine Layer** — The OpenClawEngine Node.js process (npm `openclaw` package) that runs the actual agent, manages sessions, and connects to external channels

---

## 3. The OpenClawEngine (Node.js Sidecar)

### Location
```
backend/openclaw-engine/
├── main.js              # Wrapper script that locates and launches openclaw
├── package.json         # Dependencies: openclaw@^2026.2.14
├── Dockerfile           # Docker image (node:22-alpine)
├── docker-compose.yml   # Docker Compose config
└── deploy-remote.sh     # Ansible-based remote deployment script
```

### How It Works

The `main.js` wrapper is the entry point spawned via Tauri's sidecar system:

1. **Discovery**: Searches for the `openclaw` npm package in `node_modules/` relative to the script directory
2. **Binary Resolution**: Reads `package.json.bin` to find the CLI entry point (e.g., `openclaw.mjs`)
3. **Spawn**: Uses `process.execPath` (the bundled Node binary) to execute the OpenClaw CLI with forwarded arguments
4. **Environment Logging**: Logs all `OPENCLAW_*`, `MOLTBOT_*`, `CLAWDBOT_*` environment variables for debugging

### Package Details

```json
{
  "name": "openclaw-engine",
  "dependencies": {
    "openclaw": "^2026.2.14"
  }
}
```

The engine is the official `openclaw` npm package, which provides:
- A WebSocket gateway server
- Agent runtime with LLM integration
- Session management and persistence
- Tool execution sandbox
- Channel connectors (Slack, Telegram, WhatsApp)
- mDNS discovery (disabled by default)

### Sidecar Spawn Process

The Rust backend spawns the engine via Tauri's shell sidecar system:

```rust
// In OpenClawManager::start_openclaw_engine_process()
let command = self.app.shell().sidecar("node")  // Bundled node binary
    .args(["main.js", "gateway", "run", "--port", "18789", "--force", "--verbose"])
    .env("OPENCLAW_STATE_DIR", state_dir)
    .env("OPENCLAW_CONFIG_PATH", config_path)
    .env("OPENCLAW_GATEWAY_TOKEN", auth_token)
    // ... 20+ environment variables
    .spawn();
```

### Spawn Modes

| Mode | CLI Arguments | Purpose |
|------|--------------|---------|
| **Gateway** | `gateway run --port PORT --allow-unconfigured --force --verbose` | Full gateway server (local mode) |
| **Node Host** | `node run --host HOST --port PORT --verbose` | Standalone node connecting to a remote gateway |

---

## 4. Rust Backend Module Structure

```
backend/src/openclaw/
├── mod.rs               # Module declarations and public re-exports
├── commands/            # Tauri command layer
│   ├── mod.rs           # OpenClawManager struct + process lifecycle
│   ├── gateway.rs       # Gateway start/stop/status/diagnostics/sync
│   ├── keys.rs          # API key management, secret toggles, cloud config
│   ├── sessions.rs      # Session CRUD, history, messaging, memory, workspace
│   ├── rpc.rs           # Thin WS RPC wrappers (cron, skills, config, canvas)
│   └── types.rs         # Response/input structs (OpenClawStatus, etc.)
├── config/              # Configuration management
│   ├── mod.rs           # Submodule declarations
│   ├── types.rs         # All config structs (OpenClawConfig, OpenClawEngineConfig, etc.)
│   ├── identity.rs      # Identity management, API key persistence, Keychain ops
│   ├── engine.rs        # Engine config generation, write_config(), deep_migrate()
│   └── keychain.rs      # macOS Keychain integration (unified JSON blob)
├── ws_client.rs         # WebSocket client actor with reconnection
├── frames.rs            # WS frame envelope types (req/res/event)
├── normalizer.rs        # Event normalization + LLM token sanitization
├── ipc.rs               # MCP request handler + TauriEventReporter
├── fleet.rs             # Fleet status, agent discovery, broadcast
├── deploy.rs            # Remote deployment via shell script
└── extra_commands.rs    # Profile switching, connection testing
```

### Key Types

| Type | File | Purpose |
|------|------|---------|
| `OpenClawManager` | `commands/mod.rs` | Central state: config, gateway process, node host process, WS handle |
| `OpenClawConfig` | `config/types.rs` | Config values including all API keys, gateway mode, profiles |
| `OpenClawEngineConfig` | `config/types.rs` | The JSON config written to `openclaw_engine.json` |
| `OpenClawWsClient` | `ws_client.rs` | WebSocket connection actor |
| `OpenClawWsHandle` | `ws_client.rs` | Cloneable handle for sending commands to the WS actor |
| `WsFrame` | `frames.rs` | Protocol envelope: `Req`, `Res`, `Event` |
| `UiEvent` | `normalizer.rs` | Stable UI event contract (19 variants) |
| `McpRequestHandler` | `ipc.rs` | Handles inbound RPC from engine → Rust (MCP tools) |

---

## 5. WebSocket Gateway Protocol

### Wire Format

All messages are JSON objects with a `type` discriminator:

```typescript
// Request (client → gateway OR gateway → client)
{ "type": "req", "id": "uuid", "method": "chat.send", "params": {...} }

// Response
{ "type": "res", "id": "uuid", "ok": true, "payload": {...} }
{ "type": "res", "id": "uuid", "ok": false, "error": { "code": "...", "message": "..." } }

// Event (gateway → client, push)
{ "type": "event", "event": "chat", "payload": {...}, "seq": 42 }
```

### Connection Handshake

1. **Client connects** to `ws://127.0.0.1:18789`
2. **Gateway sends** `connect.challenge` event with a random `nonce`
3. **Client signs** the challenge payload using Ed25519:
   ```
   payload = "v2|{deviceId}|cli|cli|operator|{scopes}|{signedAtMs}|{token}|{nonce}"
   signature = Ed25519.sign(payload, privateKey)
   ```
4. **Client sends** `connect` request with token, device info, public key, and signature
5. **Gateway responds** with protocol version confirmation

### Supported RPC Methods

| Method | Direction | Purpose |
|--------|-----------|---------|
| `connect` | Client → GW | Authentication handshake |
| `status` | Client → GW | Gateway health check |
| `sessions.list` | Client → GW | List all sessions |
| `sessions.delete` | Client → GW | Delete a session |
| `sessions.reset` | Client → GW | Clear session history |
| `chat.history` | Client → GW | Fetch message history |
| `chat.send` | Client → GW | Send a message |
| `chat.subscribe` | Client → GW | Subscribe to session events (no-op in current protocol) |
| `chat.abort` | Client → GW | Abort a running agent turn |
| `exec.approval.resolve` | Client → GW | Approve/deny tool execution |
| `cron.list` / `cron.run` / `cron.history` | Client → GW | Cron job management |
| `skills.list` / `skills.status` / `skills.update` / `skills.install` | Client → GW | Skill management |
| `config.schema` / `config.get` / `config.set` / `config.patch` | Client → GW | Engine configuration |
| `system.presence` | Client → GW | Get node/instance info |
| `logs.tail` | Client → GW | Tail engine logs |
| `update.run` | Client → GW | Trigger engine update |
| `web.login.whatsapp` / `web.login.telegram` | Client → GW | Initiate web login flows |
| `session.event` | Client → GW | Dispatch canvas events |
| `mcp.list_tools` | GW → Client | List available MCP tools |
| `mcp.search_tools` | GW → Client | Search tools by query |
| `mcp.call_tool` | GW → Client | Execute a tool on the host |
| `mcp.list_skills` | GW → Client | List available skills |
| `mcp.run_skill` | GW → Client | Execute a Rhai skill |
| `mcp.save_skill` | GW → Client | Persist a new skill |

### Event Types

| Event | Purpose |
|-------|---------|
| `connect.challenge` | Handshake nonce delivery |
| `chat` | Chat message events (delta, final, error, tool, run.status) |
| `agent` | Agent-specific events (tool, lifecycle, assistant, compaction) |
| `health` / `status` / `tick` / `ping` | Heartbeats (silently dropped) |
| `exec.approval.requested` | Tool approval gate |
| `exec.approval.resolved` | Approval resolution notification |
| `canvas` | Canvas content updates |
| `web.login.whatsapp` / `web.login.telegram` | Login flow events (QR codes) |
| `tool.start` / `tool.end` / `tool.output` / `tool.error` | Top-level tool lifecycle |

---

## 6. WebSocket Client (`ws_client.rs`)

### Architecture

The WS client is implemented as a **Tokio actor** pattern:

```
                        ┌──────────────────────┐
  Tauri Commands ──────►│  OpenClawWsHandle     │ (cloneable, sends commands via mpsc)
                        │  - rpc()              │
                        │  - sessions_list()    │
                        │  - chat_send()        │
                        │  - shutdown()         │
                        └──────────┬───────────┘
                                   │ mpsc::channel(32)
                                   ▼
                        ┌──────────────────────┐
                        │  OpenClawWsClient     │ (actor, owns WS connection)
                        │  - run_forever()      │
                        │  - run_once()         │
                        │  - handle_incoming()  │
                        │  - send_rpc()         │
                        └──────────┬───────────┘
                                   │ WebSocket
                                   ▼
                        ┌──────────────────────┐
                        │  OpenClawEngine GW    │
                        └──────────────────────┘
```

### Reconnection Strategy

- **Exponential backoff**: Starting at 250ms, doubling up to max 10s
- **Liveness monitoring**: The client checks an `AtomicBool` flag from the engine process; if the engine has exited, reconnection stops immediately
- **Connection attempt logging**: First 5 attempts logged as INFO (engine may still be booting), after that as ERROR
- **Automatic reset**: On successful connection, backoff resets to 250ms

### Event Routing

The client routes incoming frames in `handle_incoming_frame()`:

| Frame Type | Action |
|-----------|--------|
| `Res` | Resolves pending RPC request via `oneshot::Sender` |
| `Event` | Normalizes via `normalizer::normalize_event()` → forwards to `ui_tx` mpsc channel |
| `Req` (inbound from engine) | Delegates to `McpRequestHandler` for `mcp.*` methods |

### RPC Correlation

RPC requests use UUID-based correlation:
1. Generate `id = Uuid::new_v4()`
2. Store `(id, oneshot::Sender)` in `pending: HashMap`
3. Send `WsFrame::Req { id, method, params }`
4. When matching `WsFrame::Res` arrives, resolve the oneshot channel

---

## 7. Event Normalization Layer

### Purpose

The normalizer (`normalizer.rs`) insulates the frontend from protocol drift by converting raw upstream events into a **stable UI contract** (`UiEvent` enum with 16 variants).

### UiEvent Variants

```rust
pub enum UiEvent {
    Connected { protocol: u32 },
    Disconnected { reason: String },
    SessionList { sessions: Vec<UiSession> },
    History { session_key, messages, has_more, before },
    AssistantDelta { session_key, run_id, message_id, delta },
    AssistantInternal { session_key, run_id, message_id, text },
    AssistantSnapshot { session_key, run_id, message_id, text },
    AssistantFinal { session_key, run_id, message_id, text, usage },
    ToolUpdate { session_key, run_id, tool_name, status, input, output },
    RunStatus { session_key, run_id, status, error },
    ApprovalRequested { approval_id, session_key, tool_name, input },
    ApprovalResolved { approval_id, session_key, approved },
    Error { code, message, details },
    WebLogin { provider, qr_code, status },
    CanvasUpdate { session_key, run_id, content, content_type, url },
}
```

### LLM Token Sanitization

The normalizer includes a **compiled regex sanitizer** that strips leaked ChatML/Jinja template tokens before they reach the UI:

| Pattern | Example |
|---------|---------|
| ChatML markers | `<\|im_start\|>assistant`, `<\|im_end\|>` |
| Llama header blocks | `<\|start_header_id\|>role<\|end_header_id\|>` |
| End tokens | `<\|end\|>`, `<\|endoftext\|>`, `<\|eot_id\|>` |
| Thinking blocks | `<think>...</think>` |
| Bare role markers | `user>`, `assistant>` (line-only) |
| Excessive newlines | 3+ newlines → 2 |

This ensures ALL consumers (chat, fleet, live status) receive clean text regardless of the model's template format.

### Multi-Format Chat Protocol Support

The normalizer handles three different chat event formats:

1. **State-based (v2/v3)**: `payload.state = "delta" | "final" | "error"`
2. **Kind-based**: `payload.kind = "assistant.delta" | "assistant.final" | "tool" | "run.status"`
3. **Fallback heuristics**: Direct `payload.delta` or `payload.text` fields

### NO_REPL Filtering

Silent/internal replies containing `NO_REPL` are filtered out at the normalization layer, preventing internal agent communication from appearing in the UI.

---

## 8. Configuration System

### File Layout

```
~/Library/Application Support/scrappy cursor/OpenClaw/
├── identity.json              # Device identity + granted flags (NO API keys)
├── state/
│   ├── openclaw_engine.json   # Engine configuration
│   └── agents/
│       └── main/
│           ├── agent/
│           │   ├── auth-profiles.json    # API credentials for the engine
│           │   ├── agent.json            # Agent instructions
│           │   └── models.json           # Model config sync
│           ├── sessions/
│           │   ├── sessions.json         # Session index
│           │   └── *.jsonl               # Session history files
│           └── workspace/
│               ├── MEMORY.md             # Agent memory
│               ├── SOUL.md               # Agent personality
│               └── memory/               # Additional memory files
└── macOS Keychain: com.schack.scrappy/api_keys  # Encrypted API keys
```

### Configuration Flow

```
User Action (Frontend)
    │
    ▼
Tauri Command (e.g., openclaw_save_anthropic_key)
    │
    ├── 1. Update OpenClawConfig in memory
    ├── 2. Write API key to macOS Keychain
    ├── 3. Update identity.json (granted flag only)
    ├── 4. Regenerate openclaw_engine.json
    ├── 5. Write auth-profiles.json (runtime artifact, chmod 600)
    └── 6. Restart gateway if needed
```

### OpenClawConfig (Runtime State)

The `OpenClawConfig` struct holds ALL configuration in memory:

- 18+ API key fields (populated from Keychain at startup)
- Granted flags per provider
- Gateway mode (`local` / `remote`)
- Port, device ID, auth token
- Ed25519 key pair (private/public PEM)
- Remote URL and token
- Custom secrets array
- Cloud provider/model selection
- Feature flags (node_host, local_inference, expose_inference, etc.)
- Agent profiles for fleet management

### Secure Memory Cleanup

`OpenClawConfig` implements `Drop` with `zeroize`:

```rust
impl Drop for OpenClawConfig {
    fn drop(&mut self) {
        self.auth_token.zeroize();
        // Zeroize all 20+ API key Option<String> fields
        for secret in &mut self.custom_secrets {
            secret.value.zeroize();
        }
    }
}
```

---

## 9. Identity & Key Management

### Keychain Architecture (`keychain.rs`)

**Storage Model**: All API keys stored as a **single JSON object** in one macOS Keychain item:

```
Service:  com.schack.scrappy
Account:  api_keys
Password: {"anthropic":"sk-...","openai":"sk-...","huggingface":"hf_..."}
```

**Advantages**:
- Single `get_generic_password()` call on startup → single Keychain prompt
- Encrypted at rest by macOS (Secure Enclave / login password)
- Other processes cannot read without explicit Keychain access approval

**Supported Providers** (21 slots):
`anthropic`, `openai`, `openrouter`, `gemini`, `groq`, `brave`, `huggingface`, `xai`, `venice`, `together`, `moonshot`, `minimax`, `nvidia`, `qianfan`, `mistral`, `xiaomi`, `bedrock_access_key_id`, `bedrock_secret_access_key`, `bedrock_region`, `custom_llm_key`, `remote_token`

### In-Memory Cache

Keys are loaded once into a `Mutex<HashMap<String, String>>` static cache:
- `load_all()` — Called once at startup, reads single Keychain blob
- `get_key(provider)` — Reads from cache (no Keychain access)
- `set_key(provider, value)` — Updates cache + flushes entire blob back to Keychain

### Migration Paths

Three migration paths exist for upgrading from older versions:

1. **Per-key Keychain → Unified blob**: Old per-key items consolidated, then deleted
2. **identity.json plaintext → Keychain**: Legacy plaintext keys imported, then erased from JSON
3. **Naming migration**: `Clawdbot` → `OpenClaw`, `moltbot` → `openclaw` path references

### Identity File (`identity.json`)

Post-migration, this file contains ONLY:
- `device_id` (UUID)
- `auth_token` (32-char random)
- `private_key` / `public_key` (Ed25519 PEM)
- Boolean `*_granted` flags per provider
- `gateway_mode`, `remote_url`
- `profiles[]`, `custom_secrets[]` (values are `#[serde(skip)]`)
- Feature flags

---

## 10. Security Model

### Defense in Depth

| Layer | Mechanism |
|-------|-----------|
| **Network** | Gateway binds to `127.0.0.1` only (loopback); mDNS disabled |
| **Authentication** | Token-based WS handshake + Ed25519 signature verification |
| **Credential Storage** | macOS Keychain (encrypted at rest, single-prompt) |
| **Memory** | `zeroize` crate wipes API key strings on Drop |
| **Process Isolation** | User-space execution, no root/sudo required |
| **Tool Policies** | Allow/deny lists per tool group (`group:ui`, `group:fs`, etc.) |
| **Approval Gate** | Human-in-the-loop for sensitive tool executions |
| **Path Sanitization** | `PATH` restricted to standard system directories |
| **Key Separation** | Keys stored in Keychain; `auth-profiles.json` is a runtime artifact (chmod 600, deleted on stop) |

### Tool Security Policy

When **Node Host is enabled**:
```json
{ "allow": ["group:ui", "group:fs", "group:runtime", "group:messaging"] }
```

When **Node Host is disabled** (safe default):
```json
{
  "allow": ["group:fs", "group:runtime", "group:messaging"],
  "deny": ["group:ui", "group:system"]
}
```

### Sanitized PATH

The engine process receives a restricted PATH:
```
/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin
```

### Ed25519 Signing

The connect handshake uses PKCS#8-encoded Ed25519 keys:

```rust
let payload = format!("v2|{}|cli|cli|operator|{}|{}|{}|{}",
    device_id, scopes, signed_at, token, nonce);
let sig: Signature = signing_key.sign(payload.as_bytes());
let signature_str = base64::URL_SAFE_NO_PAD.encode(sig.to_bytes());
```

Scopes requested: `operator.read`, `operator.write`, `operator.approvals`, `operator.admin`

---

## 11. Engine Configuration Generation

### Generated Files

`OpenClawConfig::generate_config()` produces an `OpenClawEngineConfig` struct that is serialized to three files:

#### 1. `openclaw_engine.json` — Main Engine Config

```json
{
  "gateway": {
    "mode": "local",
    "bind": "loopback",
    "port": 18789,
    "auth": { "mode": "token", "token": "<auth_token>" }
  },
  "discovery": { "mdns": { "mode": "off" } },
  "agents": {
    "defaults": {
      "workspace": "/path/to/workspace",
      "model": {
        "primary": "anthropic/claude-sonnet-4-5",
        "fallbacks": ["openai/gpt-5-mini", "local/model"]
      },
      "models": {
        "anthropic/claude-sonnet-4-5": {},
        "openai/gpt-5-mini": {},
        "local/model": {}
      }
    },
    "list": [{ "id": "main", "name": "OpenClaw", "model": "anthropic/claude-sonnet-4-5" }]
  },
  "models": {
    "providers": {
      "local": {
        "baseUrl": "http://127.0.0.1:53755",
        "api": "openai-completions",
        "apiKey": "<llama_server_token>",
        "models": [{ "id": "model", "name": "Local Model", "contextWindow": 16384, "maxTokens": 4096 }]
      }
    }
  },
  "channels": {
    "slack": { "enabled": false, ... },
    "telegram": { "enabled": false, ... }
  },
  "tools": { "allow": [...], "deny": [...] },
  "meta": { "lastTouchedVersion": "2026.2.14", "lastTouchedAt": "..." }
}
```

#### 2. `auth-profiles.json` — API Credentials

```json
{
  "profiles": {
    "anthropic:default": { "type": "api_key", "provider": "anthropic", "key": "sk-...", "label": "Anthropic (OpenClaw)" },
    "openai:default": { ... },
    "local:default": { "type": "api_key", "provider": "local", "key": "dummy-key", "label": "Local LLM" },
    "amazon-bedrock:default": { "type": "aws", "auth": "aws-sdk", "accessKeyId": "...", ... }
  }
}
```

#### 3. `agent.json` — Agent Instructions

```json
{
  "instructions": "You are OpenClaw, a helpful assistant running directly on the user's computer. You value privacy and speed."
}
```

### Implicit Provider Architecture

Built-in providers (Anthropic, OpenAI, Gemini, Groq, OpenRouter, xAI, Mistral, etc.) are handled **natively** by the engine's **pi-ai catalog**. They need NO explicit `models.providers` entries — only API keys in `auth-profiles.json`. The engine auto-discovers models, base URLs, context windows, maxTokens, and pricing.

Only **Local** and **Amazon Bedrock** providers need explicit `models.providers` entries because they have custom base URLs or non-standard authentication.

### Model Selection Algorithm

```
IF local_inference_enabled → "local/model"
ELSE IF selected_cloud_brain is granted AND has enabled models
    → "{brain}/{first_enabled_model_or_selected_model}"
ELSE IF any other enabled+granted provider with enabled models
    → "{fallback_provider}/{first_model}"
ELSE → "local/model" (final fallback)
```

The engine also receives a `fallbacks` array for automatic model failover.

### Model Allowlist

The `agents.defaults.models` map acts as a hard allowlist:
- Only models listed here can be used by the agent
- Prevents unexpected API costs from model discovery
- Always includes `local/model`

---

## 12. Tauri Command Layer

### Command Categories

#### Gateway Lifecycle (gateway.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_get_status` | Full status snapshot (40+ fields) |
| `openclaw_start_gateway` | Start engine + connect WS client |
| `openclaw_stop_gateway` | Stop WS client + kill engine process |
| `openclaw_get_diagnostics` | Version, platform, connectivity info |
| `openclaw_sync_local_llm` | Sync llama-server config to engine |

#### Key Management (keys.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_save_anthropic_key` | Save API key to Keychain |
| `openclaw_save_slack_config` | Save Slack connector config |
| `openclaw_save_telegram_config` | Save Telegram connector config |
| `openclaw_save_gateway_settings` | Set mode/URL/token |
| `openclaw_toggle_node_host` | Toggle OS automation |
| `openclaw_toggle_local_inference` | Toggle local LLM |
| `openclaw_toggle_secret_access` | Grant/revoke key access |
| `openclaw_save_api_key` | Generic API key save |
| `select_openclaw_brain` | Select primary cloud provider |
| `openclaw_set_hf_token` | Save HuggingFace token |
| `openclaw_add_agent_profile` / `openclaw_remove_agent_profile` | Manage fleet profiles |

#### Session Management (sessions.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_get_sessions` | List all sessions |
| `openclaw_delete_session` | Delete (with abort + retry) |
| `openclaw_reset_session` | Clear history |
| `openclaw_get_history` | Fetch message history |
| `openclaw_send_message` | Send a chat message |
| `openclaw_subscribe_session` | No-op (auto-broadcast) |
| `openclaw_abort_chat` | Abort running agent turn |
| `openclaw_resolve_approval` | Approve/deny tool execution |
| `openclaw_clear_memory` | Factory reset (memory/identity/all) |
| `openclaw_get_memory` / `openclaw_save_memory` | Read/write MEMORY.md |
| `openclaw_list_workspace_files` / `openclaw_get_file` / `openclaw_write_file` | Workspace file access |

#### RPC Wrappers (rpc.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_cron_list` / `openclaw_cron_run` / `openclaw_cron_history` | Cron management |
| `openclaw_skills_list` / `openclaw_skills_toggle` / `openclaw_skills_status` / `openclaw_install_skill_deps` / `openclaw_install_skill_repo` | Skill management |
| `openclaw_config_schema` / `openclaw_config_get` / `openclaw_config_set` / `openclaw_config_patch` | Engine config RPC |
| `openclaw_system_presence` | Node/instance listing |
| `openclaw_logs_tail` | Log tailing |
| `openclaw_update_run` | Engine update trigger |
| `openclaw_web_login_whatsapp` / `openclaw_web_login_telegram` | Web login flows |
| `openclaw_save_cloud_config` | Bulk cloud provider/model config |
| `openclaw_spawn_session` | Create new session for agent |
| `openclaw_agents_list` | List available agents |
| `openclaw_canvas_push` / `openclaw_canvas_navigate` / `openclaw_canvas_dispatch_event` | Canvas UI operations |

#### Extra Commands (extra_commands.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_switch_to_profile` | Switch gateway to a fleet profile |
| `openclaw_test_connection` | Test WS connectivity to a URL |

#### Fleet (fleet.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_get_fleet_status` | Parallel status check of all agents |
| `openclaw_broadcast_command` | Send command to all active sessions |

#### Deploy (deploy.rs)
| Command | Purpose |
|---------|---------|
| `openclaw_deploy_remote` | Run Ansible deployment script |

### Gateway Start Sequence (Detailed)

```
openclaw_start_gateway()
    │
    ├── 1. Get or init OpenClawConfig
    ├── 2. Retry-wait for local LLM config (10 × 100ms)
    ├── 3. Detect model family from GGUF metadata
    ├── 4. generate_config() → OpenClawEngineConfig
    ├── 5. write_config() → disk files
    ├── 6. deep_migrate() → fix legacy paths (skipped if .migration_v1_complete marker exists)
    │
    ├── IF local mode:
    │   ├── 7a. Kill existing gateway process
    │   ├── 7b. Check port availability
    │   ├── 7c. start_openclaw_engine_process("gateway")
    │   ├── 7d. Health poll loop (/health endpoint, 20s timeout)
    │   │       └── Abort if engine exits (AtomicBool check)
    │   └── 7e. Log readiness
    │
    ├── IF remote mode:
    │   ├── 7a. Kill local gateway if running
    │   └── 7b. Start node host process if enabled
    │
    ├── 8. Create mpsc::channel(256) for UI events (non-blocking try_send)
    ├── 9. Create McpRequestHandler
    ├── 10. Create OpenClawWsClient with gateway_alive flag
    ├── 11. Store WsHandle in manager state
    ├── 12. Spawn client.run_forever() in background
    └── 13. Spawn event listener → Tauri emit("openclaw-event")
```

---

## 13. MCP / IPC Request Handler

### Overview

The `McpRequestHandler` in `ipc.rs` handles **reverse RPC** — requests that flow from the OpenClawEngine gateway **back to** the Rust host. This enables the engine to leverage host-side tools, skills, and MCP servers.

### Supported Methods

| Method | Implementation |
|--------|---------------|
| `mcp.list_tools` | Delegates to `search_tools("")` |
| `mcp.search_tools` | Uses `tool_discovery::search_all_tools()` to search MCP servers, skills, and host tools |
| `mcp.call_tool` | Creates a `ToolRouter` with MCP client, skill manager, and Rhai sandbox; dispatches tool call |
| `mcp.list_skills` | Lists skills via `SkillManager` |
| `mcp.run_skill` | Creates transient `RigManager` + sandbox, executes Rhai script |
| `mcp.save_skill` | Persists skill manifest + script via `SkillManager` |

### Tool Call Flow

```
Engine (GW) ──[mcp.call_tool { name, arguments }]──► WS Client
    │
    ▼
McpRequestHandler.call_tool()
    │
    ├── 1. Create transient RigManager
    ├── 2. Create sandbox via SandboxFactory
    ├── 3. Create ToolRouter { mcp_client, skill_manager, sandbox }
    ├── 4. router.call(tool_name, args)
    ├── 5. summarize_result() — auto-truncation middleware
    └── 6. Return result as WsFrame::Res to engine
```

### TauriEventReporter

The `TauriEventReporter` bridges `ToolEvent`s to the Tauri event system:

```rust
// Emits "tool_event" to frontend with:
ToolEventPayload {
    kind: "tool_activity" | "status" | "progress",
    message: String,
    tool_name: Option<String>,
    percentage: Option<f32>,
    status: Option<String>,  // "running" | "complete" | "failed"
}
```

---

## 14. Fleet Management

### Architecture

The fleet system (`fleet.rs`) enables monitoring and orchestration of multiple OpenClaw agent instances across the network.

### AgentStatusSummary

Each agent reports:
- `id`, `name`, `url` — Identity
- `online`, `latency_ms` — Connectivity
- `version` — Engine version (from `package.json`)
- `capabilities` — Derived from config (inference, web_search, cloud providers, etc.)
- `run_status` — `idle` | `processing` | `waiting_approval` | `error` | `offline`
- `model` — Active model string
- `current_task`, `progress`, `active_session_id` — Current work
- `parent_id`, `children_ids` — Hierarchy

### Status Check Process

1. Read agent profiles from config
2. **Parallel WS connection test** to each agent URL (3s timeout)
3. Query gateway for active sessions to augment status
4. If local gateway is running, prepend "Local Core" as the root node
5. Set parent/child relationships for hierarchy visualization

### Capabilities Derivation

```rust
fn get_capabilities(cfg: &OpenClawConfig) -> Vec<String> {
    let caps = ["inference", "chat"];
    if cfg.node_host_enabled → "ui_automation"
    if cfg.brave_granted → "web_search"
    if cfg.local_inference_enabled → "local_inference"
    if cfg.anthropic_granted → "cloud:anthropic"
    // ... per provider
    always: "filesystem", "tool_use"
}
```

### Fleet Broadcast

`openclaw_broadcast_command()` sends a message to ALL active sessions:

```
For each session key → chat_send("[FLEET BROADCAST] {command}")
```

---

## 15. Remote Deployment

### Deployment Options

| Method | Target | Complexity | Production-Ready |
|--------|--------|-----------|-----------------|
| **Ansible Script** | Ubuntu/Debian VPS | Automated | ✅ Yes |
| **Docker Compose** | Any Docker host | Manual setup | ⚠️ Testing only |
| **In-App Wizard** | Ubuntu/Debian VPS | GUI-driven | ✅ Yes |

### Ansible Deployment (`deploy-remote.sh`)

1. Auto-installs Ansible if missing (Homebrew on macOS, apt on Debian)
2. Clones `openclaw/openclaw-ansible` playbook
3. Runs playbook against target server:
   - System hardening (UFW, Fail2ban)
   - Tailscale VPN installation
   - Node.js + Docker setup
   - OpenClaw Engine as systemd/PM2 service
   - Auto-start on boot

```bash
ansible-playbook -i "192.168.1.50," playbook.yml \
    -e "target_host=192.168.1.50" \
    -e "tailscale_enabled=true" \
    -e "ansible_user=root"
```

### Docker Deployment

```dockerfile
FROM node:22-alpine
WORKDIR /app
COPY package.json ./
RUN npm install
COPY main.js ./
RUN mkdir -p /app/state
ENV OPENCLAW_HOME=/app/state
ENV PORT=18789
EXPOSE 18789
CMD ["node", "main.js"]
```

```yaml
# docker-compose.yml
services:
  openclaw-engine:
    build: .
    restart: always
    ports: ["18789:18789"]
    environment:
      - NODE_ENV=production
      - PORT=18789
      - OPENCLAW_HOME=/app/state
    volumes:
      - ./state:/app/state
```

### In-App Remote Deployment (`deploy.rs`)

The Rust backend provides a `openclaw_deploy_remote` Tauri command that:

1. Locates `deploy-remote.sh` (resource dir or dev fallback)
2. Spawns it asynchronously via `tokio::process::Command`
3. Streams stdout/stderr to frontend via `deploy-log` Tauri events
4. Reports completion via `deploy-status` event

### Remote Connection Architecture

```
Desktop App (macOS)                     Remote Server (Linux)
┌──────────────┐                       ┌──────────────────────┐
│  Scrappy UI   │                       │  OpenClaw Engine     │
│               │◄─── Tailscale VPN ───►│  (Port 18789)        │
│  WS Client    │     (WireGuard)       │  Gateway + Agent     │
│               │                       │  LLM Runtime         │
└──────────────┘                       └──────────────────────┘
    │                                       │
    │ ws://100.x.y.z:18789                  │ Slack/Telegram
    │ (encrypted tunnel)                    │ API connections
    └───────────────────────────────────────┘
```

### Connection Modes

| Mode | Gateway URL | Engine Location | Use Case |
|------|------------|-----------------|----------|
| **Local** | `ws://127.0.0.1:18789` | Bundled sidecar on same machine | Default, private |
| **Remote** | `ws://<tailscale-ip>:18789` | Remote server | 24/7 bots, GPU server |

---

## 16. Frontend Integration (API Layer)

### API Wrapper (`frontend/src/lib/openclaw.ts`)

557 lines providing typed wrappers for ALL Tauri commands:

```typescript
// Safety guard for HMR reloads
function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    if (typeof window === 'undefined' || !(window as any).__TAURI_INTERNALS__) {
        return Promise.reject(new Error(`Tauri runtime not available (calling ${cmd})`));
    }
    return invoke<T>(cmd, args);
}
```

### Key TypeScript Interfaces

```typescript
interface OpenClawStatus {
    gateway_running: boolean;
    ws_connected: boolean;
    port: number;
    gateway_mode: string;
    // 40+ fields for all provider states, feature flags, etc.
}

interface OpenClawSession {
    session_key: string;
    title: string | null;
    updated_at_ms: number | null;
    source: string | null;  // slack | telegram | webchat
}

interface AgentStatusSummary {
    id: string;
    name: string;
    online: boolean;
    capabilities: string[] | null;
    run_status: string | null;  // idle | processing | waiting_approval | error
    model: string | null;
    // ... 15+ fields
}
```

### Event Listening

The frontend listens to Tauri events:

| Event Channel | Payload | Consumer |
|--------------|---------|----------|
| `openclaw-event` | `UiEvent` (serialized) | OpenClawChatView, FleetCommandCenter, LiveAgentStatus |
| `tool_event` | `ToolEventPayload` | Tool activity indicators |
| `sidecar_event` | `{ type, service, progress, message }` | LiveAgentStatus progress bars |
| `deploy-log` | `string` (log line) | Deployment wizard |
| `deploy-status` | `string` (success/failed) | Deployment wizard |
| `openclaw-canvas-push` | `string` (HTML content) | Canvas window |
| `openclaw-canvas-navigate` | `string` (URL) | Canvas window |

### Stream Run Tracking (`use-openclaw-stream.ts`)

Defines types for tracking live agent execution:

```typescript
interface StreamRun {
    id: string;
    text: string;
    tools: Array<{
        tool: string;
        input?: any;
        output?: any;
        status: 'started' | 'running' | 'completed' | 'failed';
        timestamp: number;
    }>;
    approvals: StreamApproval[];
    status: 'running' | 'completed' | 'failed' | 'idle';
    error?: string;
    startedAt: number;
    completedAt?: number;
}

interface StreamApproval {
    id: string;
    tool: string;
    input: any;
    status: 'pending' | 'approved' | 'denied';
}
```

---

## 17. Frontend Event Processing & UI Components

This section covers how the frontend **consumes**, **processes**, and **displays** events from the OpenClaw engine — the complete event-to-pixel pipeline.

### 17.1  Event Consumption in OpenClawChatView

The main chat component (`OpenClawChatView.tsx`, 1051 lines) listens to the `openclaw-event` Tauri event channel and routes each event to the appropriate state update:

```
Tauri emits "openclaw-event" → listen() callback
    │
    ├── Filter: event.payload.session_key !== effectiveSessionKey → SKIP
    │
    ├── IF kind ∈ [AssistantInternal, AssistantSnapshot, AssistantDelta, AssistantFinal, ToolUpdate, RunStatus]
    │   └── updateMessagesFromEvent(uiEvent) → updates messages[] state
    │
    ├── IF kind === 'RunStatus'
    │   ├── status ∈ ['started', 'in_flight'] → Create/continue StreamRun, set isSending=true
    │   └── status ∈ ['ok', 'error', 'aborted'] → Complete StreamRun, clear after delay
    │       └── IF error → toast.error() + inject error message into chat
    │
    ├── IF kind === 'ToolUpdate' → Accumulate into activeRun.tools[]
    │   ├── Existing tool (by name + started status) → Update status/input/output
    │   └── New tool → Append to tools[]
    │
    ├── IF kind === 'AssistantDelta' → Append delta to activeRun.text
    ├── IF kind ∈ ['AssistantSnapshot', 'AssistantFinal'] → Replace activeRun.text
    │
    ├── IF kind === 'ApprovalRequested' → Append to activeRun.approvals[]
    └── IF kind === 'ApprovalResolved' → Update approval status (approved/denied)
```

### 17.2  Message State Machine

The `updateMessagesFromEvent()` function maps UiEvents to the message array:

| UiEvent Kind | Message Role | Behavior |
|-------------|-------------|----------|
| `AssistantInternal` | `system` | Prepends 🧠 emoji, creates/updates thinking message |
| `AssistantSnapshot` | `assistant` | Creates/replaces full text snapshot |
| `AssistantFinal` | `assistant` | Same as snapshot (final version) |
| `AssistantDelta` | `assistant` | Appends delta text to existing message, or creates placeholder |
| `ToolUpdate` | `system` | Creates/updates tool message with metadata: `{ type: 'tool', name, status, input, output, run_id }` |

### 17.3  LiveAgentStatus Component

`LiveAgentStatus.tsx` (291 lines) renders a real-time agent execution panel:

```
┌─────────────────────────────────────────────────────┐
│ 🔵 Agent Working                                     │
│ Run ID: abc123 • 3 Actions                     [▾]  │
├─────────────────────────────────────────────────────┤
│ ▌ Progress: Reading context... 45%                   │
│ ████████████░░░░░░░░░░░░░░░                         │
├─────────────────────────────────────────────────────┤
│ │ ⚡ WEB_SEARCH                          [completed] │
│ │   Query: "latest rust async patterns"              │
│ │ ⚡ WRITE_FILE                           [running]  │
│ │   /src/main.rs                                     │
│ │ ⚡ BASH_EXEC                            [started]  │
├─────────────────────────────────────────────────────┤
│ 🧠 Live Response                                     │
│ │ Based on the search results, I'll update the...█   │
├─────────────────────────────────────────────────────┤
│ ⚠️ SECURITY APPROVAL REQUIRED                        │
│ Tool: bash_exec                                      │
│ Input: { "command": "rm -rf /tmp/build" }            │
│ [✓ Approve]  [✗ Deny]                                │
├─────────────────────────────────────────────────────┤
│ ❌ Error Details                                      │
│ │ Rate limit exceeded for model claude-sonnet-4-5    │
└─────────────────────────────────────────────────────┘
```

**Key behaviors:**
- **Auto-collapse**: When run completes/fails, collapses after 2 seconds (unless `persistent` prop)
- **Sidecar progress**: Listens to `sidecar_event` for context-reading progress (e.g., "Reading Context... 45%")
- **Progress clears**: Automatically when text starts streaming
- **Tool-specific rendering** via `ToolDetail` sub-component:

| Tool Pattern | Rendering |
|-------------|-----------|
| `browser_*` | Globe icon + URL + optional screenshot preview with hover overlay |
| `display_canvas` / `canvas` | Pink icon + inline base64 image render |
| `apply_patch` / `write_file` | File icon + path + expandable code diff |
| `*search*` | Search icon + query display |
| All others | Monospace JSON input display, expandable on hover |

### 17.4  RichToolCard Component

`RichToolCard` (in `OpenClawChatView.tsx`, lines 36–203) renders tool actions in the message history:

**Features:**
- **Status indicators**: Spinning loader (started/in_flight), green check (completed), red X (failed)
- **Sub-agent detection**: `sessions_spawn` or `*subagent*` tools display as "Sub-Agent Task" with navigation button
- **Expandable I/O**: Collapsible input/output sections with JSON pretty-printing
- **Error highlighting**: Red background for tool outputs containing errors
- **Two variants**: `'live'` (during execution) and `'history'` (in message history)

### 17.5  ApprovalCard Component

`ApprovalCard.tsx` (80 lines) implements the Human-in-the-Loop (HITL) gate:

- Displays tool name and full input as formatted JSON
- Two-button UI: **Approve** (green, sends `resolveOpenClawApproval(id, true)`) and **Deny** (red, sends `false`)
- Loading state prevents double-submission
- Toast notifications for success/failure
- Animated entry via `framer-motion`

### 17.6  Message Grouping & Filtering

The chat view implements intelligent message rendering:

**Grouping Algorithm:**
1. Messages are classified as "tool" (system role + tool metadata) or "content" (human/assistant)
2. Consecutive tool messages are grouped into a `ToolHistoryGroup`
3. `ToolHistoryGroup` renders as a collapsed card ("Executed 5 tools") with expand/collapse
4. If any tool in the group failed, the card shows a red "Issues Found" badge

**Core View Filtering** (when `coreTab === 'chat'`):
- Hides all system messages, tool calls, thinking (🧠), heartbeats
- Hides `SYSTEM_BOOT_SEQUENCE`, `SYSTEM_CONTEXT_UPDATE`, `NO_REPL`
- Shows only `role === 'user'` and `role === 'assistant'` for a clean chat experience

**Core View Tabs:**
| Tab | Content |
|-----|---------|
| **Chat** | Clean human/agent conversation (filtered) |
| **Logs** | Full console view including all tool calls and system messages |
| **Memory** | Inline `MemoryEditor` component for MEMORY.md editing |

**System Message Rendering** (`SystemMessageContent`):
1. Rich tool cards (if `metadata.type === 'tool'`)
2. `ACTION: TOOL_NAME (STATUS)` pattern parsing
3. Standalone JSON auto-detection (tools/results/status)
4. 🧠 Thinking/reasoning display (blue bordered block)
5. Legacy `[Tool Call: name]` format parsing
6. Plain text fallback

### 17.7  Scroll Management

- **Auto-scroll pin**: Scrolls to bottom on new messages unless user has scrolled up (>15px from bottom)
- **Pin break**: User scroll-up sets `isUserScrolling.current = true`
- **Pin restore**: Scrolling back to bottom (within 15px) restores auto-scroll
- **Hard pin on send**: `isUserScrolling.current = false` on message send

---

## 18. Agent Boot Sequence

The `handleWakeUp()` function in `OpenClawChatView` constructs a comprehensive boot message:

### Boot Sequence Steps

```
handleWakeUp()
    │
    ├── 1. openclaw_sync_local_llm() — Non-destructive LLM config sync (no restart)
    │
    ├── 2. Read SOUL.md → Agent identity/personality
    │
    ├── 3. Read MEMORY.md → Long-term memory
    │
    ├── 4. IF first_run (soul empty/short) → Read BOOTSTRAP.md
    │
    └── 5. Construct boot message:
        SYSTEM_BOOT_SEQUENCE
        [CONTEXT_UPDATE]
        CURRENT_DATE: 2026-02-24
        REAL_WORLD_TIME: 15:30:00

        LOADING IDENTITY_MATRIX...
        <SOUL.md content>

        LOADING LONG_TERM_MEMORY...
        <MEMORY.md content>

        [INTERACTION_PROTOCOL]
        1. TURN-TAKING: Send ONE message, then STOP
        2. INTERNAL AUTONOMY: May manage state files autonomously
        3. EXTERNAL SCOPE & HITL: External tools require approval
        4. CONVERSATIONAL PACE: One piece at a time during setup
        5. OPERATOR PRIORITY: Execute direct requests immediately

        [FIRST_RUN_DETECTED] (if applicable)
        <BOOTSTRAP.md guide>
```

### Auto-Date Context Injection

A `useEffect` hook monitors message history and auto-injects date context if no `CURRENT_DATE:` or `SYSTEM_BOOT_SEQUENCE` appears in the last 20 messages. This is sent as a silent message (`deliver: false`) that appears in history but doesn't trigger an agent turn.

---

## 19. Frontend Component Architecture

### 19.1  Component Hierarchy

```
ChatLayout
└── ChatProvider (context: activeOpenClawPage, selectedSession, gatewayRunning)
    ├── OpenClawSidebarSlice → OpenClawSidebar
    │   ├── Session list (fetched from gateway)
    │   ├── Page navigation (11 pages)
    │   ├── Agent profile switching (fleet)
    │   └── Gateway status indicator
    │
    └── OpenClawView (page router)
        ├── OpenClawChatView    (page: 'chat')
        │   ├── LiveAgentStatus (active run panel)
        │   │   ├── ToolDetail (per-tool UI)
        │   │   └── ApprovalCard (HITL gate)
        │   ├── RichToolCard (history tool cards)
        │   ├── ToolHistoryGroup (collapsed tool groups)
        │   ├── SystemMessageContent (message renderer)
        │   └── MemoryEditor (inline memory editing)
        │
        ├── OpenClawDashboard   (page: 'dashboard')
        │   └── StatCards: gateway status, sessions, providers, model
        │
        ├── FleetCommandCenter  (page: 'fleet')
        │   ├── FleetGraph (ReactFlow visualization)
        │   │   └── AgentNode (per-node card with progress, status)
        │   ├── FleetTerminal (multi-agent log stream)
        │   └── Orchestration controls (spawn task, broadcast)
        │
        ├── OpenClawChannels    (page: 'channels')
        ├── OpenClawPresence    (page: 'presence')
        │   └── PresenceItem (instance/node details)
        ├── OpenClawAutomations (page: 'automations')
        ├── OpenClawSkills      (page: 'skills')
        ├── OpenClawSystemControl (page: 'system-control')
        ├── OpenClawBrain       (page: 'brain')
        └── OpenClawMemory      (page: 'memory')
```

### 19.2  Fleet Command Center (Multi-Agent UI)

The `FleetCommandCenter` (678 lines) provides a comprehensive multi-agent monitoring and orchestration interface:

#### Real-time Agent State

Each agent tracks an `AgentRealtimeState` updated from `openclaw-event`:

```typescript
interface AgentRealtimeState {
    runStatus: 'idle' | 'processing' | 'waiting_approval' | 'error';
    currentRunId: string | null;
    currentTool: string | null;
    lastActivity: number;       // timestamp
    toolsCompleted: number;
    toolsStarted: number;
}
```

#### Event → Agent State Mapping

| Event Kind | State Update |
|-----------|-------------|
| `RunStatus: started/in_flight` | `runStatus = 'processing'`, reset tool counters |
| `RunStatus: ok/aborted` | `runStatus = 'idle'`, clear currentTool |
| `RunStatus: error` | `runStatus = 'error'` |
| `ToolUpdate: started` | `runStatus = 'processing'`, set currentTool, increment toolsStarted |
| `ToolUpdate: ok/error` | Clear currentTool, increment toolsCompleted |
| `ApprovalRequested` | `runStatus = 'waiting_approval'`, set currentTool |
| `ApprovalResolved` | `runStatus = 'processing'` |
| `AssistantDelta/Snapshot` | `runStatus = 'processing'` |

#### Event → Log Line Mapping

| Event Kind | Log Format |
|-----------|-----------|
| `ToolUpdate: started` | `[TOOL] ▶ web_search` |
| `ToolUpdate: ok` | `[TOOL] ✓ web_search → {result preview}` |
| `ToolUpdate: error` | `[ERROR] ✗ web_search failed` |
| `AssistantFinal` | `[RESPONSE] First 80 chars...` |
| `AssistantSnapshot` | `[THINKING] First 60 chars...` |
| `RunStatus` | `[RUN] ▶/✓/✗/● status` |
| `ApprovalRequested` | `[APPROVAL] ⏳ Awaiting: bash_exec` |
| `ApprovalResolved` | `[APPROVAL] ✓ Approved / ✗ Denied` |
| `AssistantDelta` | _(skipped — too noisy)_ |
| `ToolUpdate: stream` | _(skipped — too noisy)_ |

#### Agent-to-Event Matching

Events are matched to agents by:
1. Session key prefix: `agent:{agentId}:`
2. Active session ID match
3. Fallback: `agent:main*` → first agent (Local Core)

#### Progress Calculation

Progress is derived from tool counts during a run:
```
progress = min(0.9, toolsCompleted / max(toolsStarted, 1))
```
Shows as an animated progress bar in both the sidebar inspector and the graph node.

#### Graph Visualization (FleetGraph + AgentNode)

- **ReactFlow** graph with custom `AgentNode` type
- Nodes display: label, status dot (color-coded), current operation, model, progress bar
- **Status dot colors**: Idle (emerald glow) / Processing (indigo glow) / Waiting Approval (amber pulse) / Error (red) / Offline (red, dimmed)
- **Edge animations**: Animated edges between processing agents, color-coded by child status
- **Parent/child hierarchy**: Built from `parent_id` → auto-layout with 250px horizontal offset per depth level

#### FleetTerminal

A live log stream panel:
- Per-agent log buffering (max 100 lines per agent)
- Timestamped entries (`HH:MM:SS [TYPE] message`)
- Filters to selected agent(s) or all
- Auto-scrolls to bottom

#### Orchestration

| Action | UI Element | Backend Call |
|--------|-----------|-------------|
| **Spawn Task** | Input + button per agent | `openclaw_spawn_session(agentId, task)` |
| **Broadcast** | Terminal input | `openclaw_broadcast_command(command)` — sends to ALL sessions |
| **Abort Task** | Stop button per active task | `openclaw_abort_chat(sessionId, runId)` |
| **Refresh** | Refresh button | `openclaw_get_fleet_status()` (also polled every 3s, paused when tab hidden) |

### 19.3  OpenClawSidebar

The sidebar (426 lines) serves as the navigation and session management panel:

**Sections:**
- **Agent Profiles**: Switch between Local Core and remote agent profiles (calls `openclaw_switch_to_profile`)
- **Page Navigation**: 11 pages: Chat, Dashboard, Fleet Command, Channels, Presence, Automations, Skills, System Control, Brain, Memory
- **Session List**: Auto-refreshed session list with:
  - Source indicators (Slack/Telegram/Web)
  - Relative timestamp ("2m ago", "1h ago")
  - Inline delete with two-click confirmation
  - "Core System" pinned session (`agent:main`)

### 19.4  Session Management UI

| Feature | Implementation |
|---------|---------------|
| **Session switching** | `onSelectSession(key)` → updates `selectedOpenClawSession` in ChatProvider |
| **New session** | Button that navigates to Core System view |
| **Delete session** | Two-click confirmation: first click → amber pulse for 3s, second click → delete |
| **Delete flow** | `abortChat()` → `deleteSession()` → on failure: `resetSession()` → retry `deleteSession()` |
| **Sub-agent navigation** | `onViewSession(key)` prop drills through to set active session |

---

## 20. Data Flow Diagrams

### Chat Message Flow (Send)

```
User types message
    │
    ├── Frontend: sendOpenClawMessage(sessionKey, text)
    ├── Tauri IPC: invoke("openclaw_send_message", ...)
    ├── Rust: openclaw_send_message() → ws_handle.chat_send()
    ├── WS Client: WsFrame::Req { method: "chat.send", ... }
    ├── Engine: Processes message, starts agent run
    │
    ├── Engine emits: event["chat"] { state: "delta", ... }
    ├── WS Client: normalizer::normalize_event() → UiEvent::AssistantSnapshot
    ├── UI Event Channel: ui_tx.send(event)
    ├── Event Listener: app.emit("openclaw-event", event)
    └── Frontend: Updates chat UI with streaming text
```

### Tool Execution Flow (Engine → Host)

```
Engine wants to call a tool
    │
    ├── Engine sends: WsFrame::Req { method: "mcp.call_tool", params: { name, arguments } }
    ├── WS Client: handle_incoming_frame() → Req with "mcp." prefix
    ├── McpRequestHandler.call_tool()
    │   ├── Creates transient RigManager + Sandbox
    │   ├── ToolRouter.call(tool_name, args)
    │   │   ├── Check MCP servers
    │   │   ├── Check skill manager
    │   │   ├── Check host tools (calculator, web_search, etc.)
    │   │   └── Execute via appropriate backend
    │   ├── summarize_result() — truncate to max_chars
    │   └── Return Value
    ├── WS Client: Send WsFrame::Res { ok: true, payload: result }
    └── Engine: Receives tool result, continues agent run
```

### Approval Flow

```
Engine requests sensitive tool
    │
    ├── Engine emits: event["exec.approval.requested"] { approvalId, tool, input }
    ├── Normalizer → UiEvent::ApprovalRequested
    ├── Frontend: Shows approval dialog with tool details
    │
    ├── User clicks Approve/Deny
    ├── Frontend: resolveOpenClawApproval(approvalId, approved)
    ├── Rust: ws_handle.approval_resolve(approvalId, approved)
    ├── Engine: Proceeds with or cancels tool execution
    └── Engine emits: event["exec.approval.resolved"] { approved }
```

---

## 21. File Inventory

### Backend (Rust)

| File | Lines | Size | Purpose |
|------|-------|------|---------|
| `openclaw/mod.rs` | 23 | 605B | Module root, re-exports |
| `openclaw/ws_client.rs` | 737 | 24.9KB | WebSocket actor + handle |
| `openclaw/frames.rs` | 156 | 4.3KB | Protocol frame types |
| `openclaw/normalizer.rs` | 839 | 27.0KB | Event normalization + token sanitization |
| `openclaw/ipc.rs` | 426 | 14.4KB | MCP request handler + event reporter |
| `openclaw/fleet.rs` | 397 | 13.5KB | Fleet status + broadcast |
| `openclaw/deploy.rs` | 108 | 3.8KB | Remote deployment command |
| `openclaw/extra_commands.rs` | 80 | 2.4KB | Profile switching, connection test |
| `openclaw/commands/mod.rs` | 350 | 12.7KB | OpenClawManager + process lifecycle |
| `openclaw/commands/gateway.rs` | 518 | 18.1KB | Start/stop/status/diagnostics |
| `openclaw/commands/keys.rs` | ~800 | 29.9KB | API key management |
| `openclaw/commands/sessions.rs` | 766 | 28.0KB | Session CRUD, history, memory |
| `openclaw/commands/rpc.rs` | 567 | 16.4KB | Gateway RPC wrappers |
| `openclaw/commands/types.rs` | 154 | 4.7KB | Typed response structs |
| `openclaw/config/mod.rs` | 15 | 432B | Config module root |
| `openclaw/config/types.rs` | 457 | 14.7KB | All config structs |
| `openclaw/config/identity.rs` | 639 | 27.1KB | Identity + key persistence |
| `openclaw/config/engine.rs` | 1013 | 42.6KB | Config generation + migration |
| `openclaw/config/keychain.rs` | 392 | 16.2KB | macOS Keychain integration |
| **Total** | **~7,437** | **~281KB** | |

### Engine (Node.js)

| File | Lines | Size | Purpose |
|------|-------|------|---------|
| `main.js` | 116 | 3.8KB | Wrapper/launcher |
| `package.json` | 19 | 325B | Dependencies |
| `Dockerfile` | 27 | 389B | Docker image |
| `docker-compose.yml` | 18 | 368B | Docker Compose |
| `deploy-remote.sh` | 108 | 3.1KB | Ansible deployment |

### Frontend (TypeScript/React)

| File | Lines | Size | Purpose |
|------|-------|------|---------|
| `lib/openclaw.ts` | 557 | 15.8KB | Tauri command wrappers |
| `hooks/use-openclaw-stream.ts` | 26 | 616B | Stream run types |
| `components/openclaw/OpenClawChatView.tsx` | 1051 | 59KB | Main chat view + event processing |
| `components/openclaw/LiveAgentStatus.tsx` | 291 | 15.7KB | Real-time run tracking panel |
| `components/openclaw/ApprovalCard.tsx` | 80 | 3.6KB | HITL approval gate UI |
| `components/openclaw/OpenClawSidebar.tsx` | 426 | 22.2KB | Navigation + session management |
| `components/openclaw/OpenClawDashboard.tsx` | 280 | 14.1KB | Status dashboard |
| `components/openclaw/OpenClawPresence.tsx` | 235 | 11.3KB | Node/instance presence |
| `components/openclaw/OpenClawBrain.tsx` | — | — | Brain/model configuration |
| `components/openclaw/OpenClawMemory.tsx` | — | — | Memory management |
| `components/openclaw/OpenClawChannels.tsx` | — | — | Slack/Telegram config |
| `components/openclaw/OpenClawAutomations.tsx` | — | — | Cron/automation management |
| `components/openclaw/OpenClawSkills.tsx` | — | — | Skill management |
| `components/openclaw/OpenClawSystemControl.tsx` | — | — | System control panel |
| `components/openclaw/fleet/FleetCommandCenter.tsx` | 678 | 37.1KB | Multi-agent fleet dashboard |
| `components/openclaw/fleet/FleetGraph.tsx` | 72 | 2.5KB | ReactFlow graph visualization |
| `components/openclaw/fleet/FleetTerminal.tsx` | 62 | 2.7KB | Multi-agent log stream |
| `components/openclaw/fleet/AgentNode.tsx` | 130 | 6.0KB | Custom ReactFlow node |
| `components/chat/views/OpenClawView.tsx` | 61 | 2.6KB | Page router |
| `components/chat/sidebars/OpenClawSidebarSlice.tsx` | — | — | Sidebar integration |
| **Total** | **~4,000+** | **~200KB+** | |

---

## 22. Known Issues & Technical Debt

### Architecture

1. **`auth-profiles.json` contains plaintext API keys** — This is a necessary runtime artifact for the OpenClaw engine (it reads keys from this file). The Keychain stores the master copy; this file is regenerated on each gateway start. **Mitigated**: permissions set to `0600` on write, and the file is deleted in `openclaw_stop_gateway()`.

2. **Backward compatibility env vars** — The engine config sets THREE prefixes for environment variables (`OPENCLAW_*`, `CLAWDBOT_*`, `MOLTBOT_*`) to support older engine versions. This should eventually be cleaned up.

3. **Deep migration complexity** — The `deep_migrate()` function handles path normalization from three different naming conventions (`Clawdbot`, `moltbot`, `openclaw`) across session files and JSON indices. **Mitigated**: a `.migration_v1_complete` marker file is written after the first successful migration; subsequent starts skip it entirely.

4. **Process kill mechanism** — `OpenClawEngineProcess::kill()` only sets the `AtomicBool` flag to false; it doesn't send SIGTERM to the actual process. The Tauri sidecar should handle process cleanup natively.

### Protocol

5. **Session subscription is a no-op** — `openclaw_subscribe_session` exists for API stability but does nothing. The gateway automatically broadcasts all events to connected operators.

6. **Hardcoded protocol version** — The connect handshake requests `minProtocol: 3, maxProtocol: 3`. No version negotiation fallback exists.

7. ~~**RPC timeout only on handshake**~~ — **Resolved**: all `rpc()` calls now have a 30-second timeout via `tokio::time::timeout`. Additionally, on disconnect, all pending RPC senders are drained with a clear "WebSocket disconnected" error.

### Security

8. **Remote mode token in Keychain** — The remote gateway token is stored in the Keychain (`remote_token`), but it's also written to `auth-profiles.json` when connecting. This is necessary but creates a second copy.

9. **PATH injection risk** — While PATH is sanitized to standard directories, `/opt/homebrew/bin` is included which could contain user-installed tools accessible to the agent.

### Frontend

10. **Type drift risk** — The TypeScript interfaces in `openclaw.ts` are manually kept in sync with Rust structs in `commands/types.rs`. While `specta` generates bindings, the frontend wrapper layer adds another manual synchronization point.

11. **Missing error types** — Most API functions return `Promise<void>` or `Promise<T>` without structured error types. Frontend error handling relies on catch-all string error messages.

### Pending Features

12. **WhatsApp integration** — `web.login.whatsapp` RPC exists but the full WhatsApp Web (QR code + pairing) integration is partially implemented.

13. **Canvas UI** — Canvas push/navigate/dispatch commands exist but the CanvasWindow rendering is still in development.

14. **mDNS discovery** — Hardcoded to `"off"`. The fleet system currently relies on manually configured profiles rather than automatic discovery.

15. **Engine version check** — `get_engine_version()` reads from `node_modules/openclaw/package.json` at compile time paths, which may not match the runtime package version.

### Resolved in 2026-02-24

The following issues were identified during a deep analysis and fixed:
- **RPC timeout** — 30s timeout on all `rpc()` calls; pending HashMap drained on disconnect
- **auth-profiles.json hardening** — `chmod 600` on write, deleted on gateway stop
- **deep_migrate optimization** — `.migration_v1_complete` marker skips redundant I/O
- **Duplicated session delete logic** — Frontend now delegates entirely to backend
- **Verbose WS logging** — Hot-path `info!()` calls downgraded to `debug!()`/`trace!()`
- **Fleet polling** — Paused when browser tab is hidden (`document.visibilityState`)
- **Session key alignment** — Core view uses `agent:main` (matches backend canonical key)
- **Event channel backpressure** — Buffer increased to 256, `try_send()` prevents blocking

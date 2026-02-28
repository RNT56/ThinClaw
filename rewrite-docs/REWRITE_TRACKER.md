# OpenClaw → ThinClaw (Rust) Rewrite Tracker

This document tracks all components needed to completely replace the OpenClaw Node.js sidecar and Swift companion apps with a unified Rust/Tauri application containing the IronClaw agent.

> **Last updated:** 2026-02-28 · IronClaw v0.12.0 + 4 commits

## 🏆 Milestone: Tauri Integration Complete

The Node.js sidecar is **eliminated**. IronClaw runs natively inside Scrappy as a library crate.
See `TAURI_INTEGRATION.md` for full 11-phase completion report.

| Metric | Value |
|---|---|
| Commands migrated | 66 (WS RPC → direct API) |
| Dead code removed | ~2,458 LOC |
| Disk space recovered | ~625 MB (node_modules) |
| Build warnings | 0 |
| API stubs remaining | 0 |

---

## 🧠 Core Agent & Infrastructure

The core orchestrator that brings it all together.

| Component                  | Description                                                                                               | Original Size | Rust Implementation                       | Status              |
| :------------------------- | :-------------------------------------------------------------------------------------------------------- | :------------ | :---------------------------------------- | :------------------ |
| **RIG Agent Base**         | The core AI execution, replacing the TypeScript agent for basic chat.                                     | ~1MB (TS)     | `agent/agent_loop.rs` — full agent loop   | ✅ Complete          |
| **Pi Agent Orchestrator**  | Advanced features: tool streaming, model fallback, auth rotation, context compaction.                     | ~4.4MB (TS)   | `orchestrator/`, `llm/failover.rs`        | ✅ Complete          |
| **Local Inference**        | Running models locally.                                                                                   | Varies        | `llm/smart_routing.rs` — MLX/vLLM/llama  | ✅ Complete          |
| **Tool Orchestration**     | Framework for tools (browser, bash, search, etc.).                                                        | ~800KB (TS)   | `tools/` — 40+ tools, registry, approval  | ✅ Complete          |
| **Memory / Context**       | Persistent memory, vector DB, group context (CLAUDE.md style).                                            | ~150KB (TS)   | `workspace/` — libSQL + FTS5 + vectors    | ✅ Complete          |
| **Unified Config**         | Central configuration for all channels, agent settings, and tools.                                        | ~200KB (TS)   | `config/` + `settings.rs` — TOML + DB     | ✅ Complete          |
| **Scheduling (Cron)**      | Full scheduler: cron expressions, one-shot reminders, catchup, delivery targets, session isolation.       | ~71 files (TS)| `agent/routine.rs` — `RoutineEngine`      | ✅ Complete          |
| **Model Discovery**        | Fetching active model lists from HF Hub (MLX/GGUF) and Cloud APIs.                                       | ~100KB (TS)   | `registry/` — provider discovery          | 🔄 Partial (Cloud only) |
| **Hooks / Events**         | Lifecycle event bus: bootstrap, message received/sent, webhook endpoints.                                 | ~43 files (TS)| `hooks/` — `HookRegistry` + lifecycle     | ✅ Complete          |
| **Sub-Agent Orchestration**| Multi-agent spawning: run/session modes, depth limits, result announcement, thread binding.               | ~25 files (TS)| `SessionManager` + `tokio::spawn`         | ✅ Complete          |
| **Security Auditor**       | Boot-time config scanner: dangerous flags, channel security, exec policy, filesystem perms.               | ~28 files (TS)| `safety/` — `SecurityGuard`, policy engine| ✅ Complete          |
| **Daemon Service**         | OS-level service installation: launchd (macOS), systemd (Linux), auto-restart, service audit.             | ~40 files (TS)| `service.rs` — launchd/systemd support    | ✅ Complete          |
| **TTS Pipeline**           | Text-to-Speech: OpenAI TTS, speed/voice control, cost estimation.                                     | ~66KB (TS)    | `tools/builtin/tts.rs` — OpenAI TTS       | ✅ Complete          |
| **Canvas / a2UI**          | Agent-generated interactive UIs: local HTTP server, bridge script injection, live reload, Tauri WebView.  | ~6 files (TS) | Not yet ported                            | ⬜ Not Started       |

## 📡 Channels (Messaging Integrations)

Rebuilding the primary communication channels over to native Rust. The goal is to start lean (basic send/receive) and add features later.

IronClaw already has the `Channel` trait and `ChannelManager`. Adding channels means implementing the trait for each platform.

| Channel      | Complexity | Original Size | Target Rust Crate(s)       | Features to Port                                                         | Status         |
| :----------- | :--------- | :------------ | :------------------------- | :----------------------------------------------------------------------- | :------------- |
| **Tauri**    | ✅ Done    | New           | `channels/channel.rs`      | Desktop app channel — `TauriChannel` in Scrappy                          | ✅ Complete     |
| **Web**      | ✅ Done    | New           | `channels/web/`            | WebSocket-based browser channel                                          | ✅ Complete     |
| **REPL**     | ✅ Done    | New           | `channels/repl.rs`         | Terminal stdin/stdout channel                                            | ✅ Complete     |
| **WASM**     | ✅ Done    | New           | `channels/wasm/`           | Browser WASM channel                                                     | ✅ Complete     |
| **Signal**   | ✅ Done    | ~3K lines     | `channels/signal.rs`       | 2,454 LOC — SSE + JSON-RPC, groups, pairing, allowlists                  | ✅ Complete     |
| **Nostr**    | ✅ Done    | ~3K lines     | `channels/nostr.rs`        | NIP-04 encrypted DMs, multi-relay, allowlist, deterministic threads       | ✅ Complete     |
| **iMessage** | 🟢 Low-Med | ~2K lines     | `imessage-rs` / `rusqlite` | Poll `chat.db`, send via `osascript`, group support (macOS only).        | ⬜ Not Started |
| **Telegram** | 🟡 Medium  | ~11K lines    | `teloxide`                 | Bot API, long polling, text/media, chunking (4096 limit), commands.      | ⬜ Not Started |
| **Slack**    | 🟡 Medium  | ~8K lines     | `slack-morphism`           | Socket Mode (no public IP needed), Block Kit, threads, mentions.         | ⬜ Not Started |
| **Discord**  | 🔴 High    | ~20K lines    | `serenity` + `poise`       | Guild monitoring, thread replies, chunking (2000 limit), slash commands. | ⬜ Not Started |

_Note: OpenClaw supports 23+ channels. These 6 are the prioritized starting point based on the previous plan._

## 📱 Device Node Capabilities (Companion App Replacement)

Features previously handled by the macOS/iOS Swift apps that need to be ported to Rust (likely via Tauri commands/plugins if UI is involved, or native Rust libraries).

| Capability                 | Complexity | Original File(s)         | Target Rust Approach                                 | Status         |
| :------------------------- | :--------- | :----------------------- | :--------------------------------------------------- | :------------- |
| **System Commands**        | 🔴 High    | `SystemCommands.swift`   | `std::process::Command`, approval UI in Tauri        | ⬜ Not Started |
| **Canvas / A2UI**          | 🔴 High    | `CanvasCommands.swift`   | Tauri WebView manipulation & JS evaluation           | ⬜ Not Started |
| **Voice Wake (Hey Molty)** | 🔴 High    | `VoiceWakeRuntime.swift` | `sherpa-rs` (Sherpa-ONNX) for keyword detection      | ⬜ Not Started |
| **Talk Mode (PTT)**        | 🟡 Medium  | `TalkCommands.swift`     | Standard audio capture traits (`cpal`), `whisper-rs` | ⬜ Not Started |
| **Screen Recording**       | 🟡 Medium  | `ScreenCommands.swift`   | `scrap` or platform-specific capture APIs            | ⬜ Not Started |
| **Camera Capture**         | 🟡 Medium  | `CameraCommands.swift`   | `nokhwa` for cross-platform camera access            | ⬜ Not Started |
| **Location (GPS)**         | 🟡 Medium  | `LocationCommands.swift` | CoreLocation FFI (macOS), standard libs              | ⬜ Not Started |
| **Device Status**          | 🟢 Low     | `DeviceCommands.swift`   | `tools/builtin/device_info.rs` — `sysinfo` crate      | ✅ Complete    |

_Note: Mobile-specific capabilities (Contacts, Calendar, Walk/Activity, Watch, SMS) are excluded for now assuming a desktop-first Rust application._

## 🧰 Native Tools (RIG Equivalents)

OpenClaw's TypeScript agent has a robust built-in toolkit. To replace the TS agent entirely, these tools must be rebuilt in Rust using the `rig-core` `Tool` trait.

### 🌐 Web & Browser

| Tool               | Description                                          | OpenClaw TS Source | Target Rust Approach                      | Status  |
| :----------------- | :--------------------------------------------------- | :----------------- | :---------------------------------------- | :------ |
| **`web-search`**   | Search the web for current information.              | `web-search.ts`    | `tools/web_search.rs` — multi-engine      | ✅ Done  |
| **`web-fetch`**    | Read raw text content from URLs.                     | `web-fetch.ts`     | `tools/web_fetch.rs` — HTML→text          | ✅ Done  |
| **`browser-tool`** | Full browser automation: CDP, accessibility tree snapshots, profiles. | 101 files  | `chromiumoxide` — see `BROWSER_TOOL_RS.md` | ⬜ Not Started |

### 📱 Device Control (Companion Bridge)

| Tool               | Description                                    | OpenClaw TS Source | Target Rust Approach                         | Status |
| :----------------- | :--------------------------------------------- | :----------------- | :------------------------------------------- | :----- |
| **`nodes.invoke`** | Execute system-level tasks (shell, files, UI). | `nodes-tool.ts`    | `std::process::Command` + your Tauri backend | ⬜ Not Started |
| **`canvas.a2ui`**  | Push interactive UI components to the screen.  | `canvas-tool.ts`   | Send Tauri IPC events to the frontend        | ⬜ Not Started |

### 🧠 Memory & Swarm

| Tool                   | Description                                   | OpenClaw TS Source         | Target Rust Approach                      | Status  |
| :--------------------- | :-------------------------------------------- | :------------------------- | :---------------------------------------- | :------ |
| **`memory.save`**      | Store long-term facts and user preferences.   | `memory-tool.ts`           | `workspace/` — libSQL + vector search     | ✅ Done  |
| **`sessions.spawn`**   | Spawn sub-agents for specific/parallel tasks. | `subagents-tool.ts`        | `SessionManager` + `tokio::spawn`         | ✅ Done  |
| **`sessions.history`** | Read chat history across channels.            | `sessions-history-tool.ts` | `db/ConversationStore` — paginated query  | ✅ Done  |
| **`cron.schedule`**    | Schedule recurring background agent tasks.    | `cron-tool.ts`             | `agent/routine.rs` — `RoutineEngine`      | ✅ Done  |

### 🎨 Media Generation

| Tool             | Description                                | OpenClaw TS Source | Target Rust Approach                      | Status |
| :--------------- | :----------------------------------------- | :----------------- | :---------------------------------------- | :----- |
| **`image-tool`** | Generate images (DALL-E, Midjourney, etc). | `image-tool.ts`    | `tools/imagine.rs` — DALL-E / OpenAI API  | ✅ Done |
| **`tts-tool`**   | Text-to-speech string generation.          | `tts-tool.ts`      | `tools/builtin/tts.rs` — OpenAI TTS API   | ✅ Done |

### 💬 Channel Moderation & Admin

| Tool                   | Description                                       | OpenClaw TS Source    | Target Rust Approach                               | Status |
| :--------------------- | :------------------------------------------------ | :-------------------- | :------------------------------------------------- | :----- |
| **`discord.actions`**  | Kick, ban, manage roles, adjust presence.         | `discord-actions.ts`  | Specific functions on your `serenity` client       | ⬜ Not Started |
| **`slack.actions`**    | Slack-specific moderation and channel management. | `slack-actions.ts`    | Specific functions on your `slack-morphism` client | ⬜ Not Started |
| **`telegram.actions`** | Telegram specific tools and API interactions.     | `telegram-actions.ts` | Specific functions on your `teloxide` client       | ⬜ Not Started |

## 📻 Client ↔ Orchestrator Protocol (Remote Mode Infrastructure)

The WebSocket protocol that connects the Tauri Thin Client to the headless Remote Orchestrator is a significant infrastructure piece. For full specification see `NETWORKING_RS.md` and `HARDWARE_BRIDGE_RS.md`.

| Component | Description | Implementation | Status |
|---|---|---|---|
| **WebSocket Server (axum)** | Persistent WS endpoint on headless Orchestrator | `axum` + `tokio-tungstenite` | ⏳ Pending |
| **WS Auth (session token)** | Query-param token validation on connect | Custom middleware in `axum` | ⏳ Pending |
| **Version Handshake** | Protocol version exchange on connect; mismatch = 4002 close + UI warning | Typed message dispatch | ⏳ Pending |
| **Message Envelope** | Shared JSON envelope (`id`, `type`, `payload`) for all WS messages | `serde` structs | ⏳ Pending |
| **Chat Streaming** | `message.delta` / `message.done` events for token streaming to UI | `tokio::mpsc` + WS send | ⏳ Pending |
| **Config RPC** | `config.set` messages to change model/settings from Tauri UI | Typed handler dispatch | ⏳ Pending |
| **Secret Transmission** | `secret.set` to relay API keys from Tauri UI to remote Keychain | Encrypted at Tailscale layer | ⏳ Pending |
| **Hardware Bridge RPC** | `tool.rpc.request` / `tool.rpc.response` for cam/mic/screen | See `HARDWARE_BRIDGE_RS.md` | ⏳ Pending |
| **Model Discovery RPC** | `model.list.request` / `model.list.response` | Fetch on demand | ⏳ Pending |
| **Auto-Update Check** | Orchestrator polls GitHub Releases every 24h, self-updates via `self_update` crate | Background `tokio` task | ⏳ Pending |
| **Tailscale Discovery** | Tauri app queries Tailscale local API to auto-find Orchestrator | `reqwest` to `localhost:41112` | ⏳ Pending |
| **QR Code Pairing** | Fallback pairing for non-Tailscale setups, cert-pinned TLS | `rcgen` + QR display | ⏳ Pending |

## 🖥️ CLI, TUI & Setup Wizard (Rust-Native)

Built from scratch in Rust — not ports of the Node.js versions, but functionally equivalent.

| Component | Description | Crate(s) | Doc | Status |
|---|---|---|---|---|
| **CLI** | Full management CLI: 25+ commands (config, models, agents, sessions, memory, cron, channels, hooks, plugins, security, daemon) | `clap` + `clap_complete` | `CLI_RS.md` | ✅ Complete |
| **TUI** | Interactive terminal chat: full-screen layout, streaming tokens, slash commands, overlay selectors, tool call display, local shell | `ratatui` + `crossterm` | `TUI_RS.md` | ⏳ Pending |
| **Setup Wizard** | 8-step onboarding: security ack, provider/key, model selection, identity, channels, networking, review. Both Tauri UI and terminal (`inquire`) variants | `inquire` + Tauri IPC | `SETUP_WIZARD_RS.md` | ⏳ Pending |

All three are feature-gated:
```toml
[features]
default = ["postgres", "libsql", "html-to-markdown"]
desktop = ["libsql", "html-to-markdown"]
cli = ["clap", "clap_complete", "comfy-table"]
tui = ["ratatui", "crossterm"]
```

## ⛔ Explicitly Out-of-Scope for v1

| Area | Files | Reason |
|---|---|---|
| **ACP (Agent Communication Protocol)** | `src/acp/` (17 files) | IDE integration protocol using `@agentclientprotocol/sdk`. Can be added post-v1 if third-party IDE integration is needed. |
| **Community Channel Extensions** | `extensions/` (750 files, 38 channels) | IRC, Matrix, MS Teams, Twitch, etc. — replaced by MCP plugin strategy. |
| **Native Mobile Apps** | `apps/` (642 files), `Swabble/` (34 files) | iOS/Android/macOS Swift apps — replaced by Tauri cross-platform app. |

## 🗺️ Migration Strategy

1. ~~**Hybrid Phase (Current):** Rust app handles the RIG Agent and local UI, but still relies on the Node.js OpenClaw sidecar for channels and legacy infrastructure.~~ → **COMPLETE: Node.js eliminated.**
2. **Channel Porting:** Build messaging channels using the existing `Channel` trait. Port channels one by one (recommended order: Signal → Nostr → Telegram → iMessage → Slack → Discord).
3. **Capabilities Porting:** Implement host-device access (System, Camera, Voice) directly in Rust, eliminating the need for the Swift companion apps.
4. **Standalone App:** Turn off ALL legacy infrastructure once the 6 core channels and necessary capabilities are running natively in Rust.

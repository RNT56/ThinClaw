# IronClaw ↔ OpenClaw Feature Parity Matrix

> **Last reconciled:** 2026-03-05 05:09 CET

This document tracks feature parity between IronClaw (Rust implementation) and OpenClaw (TypeScript reference implementation). Use this to coordinate work across developers.

**Key source cross-references:** [`src/service.rs`](src/service.rs) (launchd/systemd), [`src/tools/builtin/canvas.rs`](src/tools/builtin/canvas.rs) (A2UI canvas tool), [`src/channels/imessage.rs`](src/channels/imessage.rs) (iMessage channel), [`src/media/video.rs`](src/media/video.rs) (video pipeline), [`src/channels/signal.rs`](src/channels/signal.rs) (Signal), [`src/channels/discord.rs`](src/channels/discord.rs) (Discord), [`src/channels/nostr.rs`](src/channels/nostr.rs) (Nostr).

**Legend:**
- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned (in scope but not started)
- 🚫 Out of scope (intentionally skipped)
- ➖ N/A (not applicable to Rust implementation)

---

## 1. Architecture

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Hub-and-spoke architecture | ✅ | ✅ | Web gateway as central hub |
| WebSocket control plane | ✅ | ✅ | Gateway with WebSocket + SSE |
| Single-user system | ✅ | ✅ | |
| Multi-agent routing | ✅ | ✅ | `AgentRouter` with workspace isolation, priority-based routing, thread ownership |
| Session-based messaging | ✅ | ✅ | Per-sender sessions |
| Loopback-first networking | ✅ | ✅ | HTTP binds to 0.0.0.0 but can be configured |

### Owner: IronClaw Agent

---

## 2. Gateway System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Gateway control plane | ✅ | ✅ | Web gateway with 40+ API endpoints |
| HTTP endpoints for Control UI | ✅ | ✅ | Web dashboard with chat, memory, jobs, logs, extensions |
| Channel connection lifecycle | ✅ | ✅ | ChannelManager + WebSocket tracker |
| Session management/routing | ✅ | ✅ | SessionManager exists |
| Configuration hot-reload | ✅ | ✅ | `ConfigWatcher` with mtime polling, debounce, broadcast subscribers |
| Network modes (loopback/LAN/remote) | ✅ | ✅ | Full loopback/LAN/remote with security validation ([`src/config/network_modes.rs`](src/config/network_modes.rs)) |
| OpenAI-compatible HTTP API | ✅ | ✅ | /v1/chat/completions, per-request `model` override |
| Canvas hosting | ✅ | ✅ | `CanvasTool` (621 LOC) + `CanvasStore` + canvas gateway routes (`/canvas/:panel_id` HTML, `/canvas/api/panels` JSON); [`src/channels/canvas_gateway.rs`](src/channels/canvas_gateway.rs) |
| Gateway lock (PID-based) | ✅ | ✅ | `gateway start` writes PID to `~/.ironclaw/gateway.pid`; launchd/systemd keepalive via `ironclaw service` |
| launchd/systemd integration | ✅ | ✅ | Full `service.rs` (358 LOC, [`src/service.rs`](src/service.rs)) — install/start/stop/status/uninstall; macOS launchd plist + Linux systemd unit generation |
| Bonjour/mDNS discovery | ✅ | ✅ | `MdnsConfig` + `DiscoveryTracker` ([`src/config/mdns_discovery.rs`](src/config/mdns_discovery.rs)) |
| Tailscale integration | ✅ | ✅ | Full tunnel module (`tunnel/tailscale.rs`) with serve + funnel |
| Health check endpoints | ✅ | ✅ | /api/health + /api/gateway/status |
| `doctor` diagnostics | ✅ | ✅ | `cli/doctor.rs` — DB, binary, LLM, and Tailscale checks |
| Agent event broadcast | ✅ | ✅ | SSE broadcast manager + routine engine lifecycle events (Status, JobStarted, JobResult) |
| Channel health monitor | ✅ | ✅ | `ChannelHealthMonitor`: periodic checks, failure tracking, auto-restart with cooldown |
| Presence system | ✅ | ✅ | `PresenceTracker` with beacons, status, stale pruning ([`src/agent/presence.rs`](src/agent/presence.rs)) |
| Trusted-proxy auth mode | ✅ | ✅ | `TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS` for reverse-proxy deployments |
| APNs push pipeline | ✅ | ❌ | Wake disconnected iOS nodes via push |
| Oversized payload guard | ✅ | ✅ | HTTP webhook 64KB body limit + Content-Length check + chat history cap (`max_context_messages` default 200) |
| Pre-prompt context diagnostics | ✅ | ✅ | `tracing::debug` logs message count, est. chars, tool count before each LLM call |

### Owner: IronClaw Agent

---

## 3. Messaging Channels

| Channel | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (simple) | ✅ | ✅ | - | For testing |
| WASM channels | ❌ | ✅ | - | IronClaw innovation |
| WhatsApp | ✅ | ❌ | P1 | Baileys (Web), same-phone mode with echo detection |
| Telegram | ✅ | ✅ | - | WASM channel(MTProto), DM pairing, caption, /start, bot_username |
| Discord | ✅ | ✅ | - | Native Rust Gateway WS + REST (`channels/discord.rs`) + WASM webhook channel |
| Signal | ✅ | ✅ | - | signal-cli daemon, SSE listener, user/group allowlists, DM pairing |
| Slack | ✅ | ✅ | - | WASM tool |
| iMessage | ✅ | ✅ | P3 | `IMessageChannel` (445 LOC) + `IMessageConfig` startup wiring ([`src/channels/imessage_wiring.rs`](src/channels/imessage_wiring.rs)) |
| Linq | ✅ | ❌ | P3 | Real iMessage via API, no Mac required |
| Feishu/Lark | ✅ | ❌ | P3 | Bitable create app/field tools |
| LINE | ✅ | ❌ | P3 | |
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ❌ | P3 | E2EE support |
| Mattermost | ✅ | ❌ | P3 | Emoji reactions |
| Google Chat | ✅ | ❌ | P3 | |
| MS Teams | ✅ | ❌ | P3 | |
| Twitch | ✅ | ❌ | P3 | |
| Voice Call | ✅ | ❌ | P3 | Twilio/Telnyx, stale call reaper, pre-cached greeting |
| Nostr | ✅ | ✅ | - | NIP-04 encrypted DM channel (`channels/nostr.rs`) |

### Telegram-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forum topic creation | ✅ | ✅ | `message_thread_id` pass-through for forum groups; replies target correct topic |
| channel_post support | ✅ | ✅ | Channel posts received via `channel_post` update + `sender_chat` for identification |
| User message reactions | ✅ | ✅ | `TgMessageReaction` + `TgReactionType` parsing; emojis surfaced in `IncomingMessage` metadata |
| sendPoll | ✅ | ✅ | `send_poll()` helper: question, options, anonymous/multiple-answer flags; wired to `Channel::poll()` trait |
| Cron/heartbeat topic targeting | ✅ | ✅ | `HEARTBEAT_NOTIFY_TOPIC_ID` config + `message_thread_id` injection in broadcast metadata |

### Discord-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forwarded attachment downloads | ✅ | ✅ | Platform-aware download with size limits ([`src/channels/forward_download.rs`](src/channels/forward_download.rs)) |
| Faster reaction state machine | ✅ | ✅ | Debounced state machine with watchdog cleanup ([`src/channels/reaction_machine.rs`](src/channels/reaction_machine.rs)) |
| Thread parent binding inheritance | ✅ | ✅ | Chain-resolved inheritance with agent propagation ([`src/agent/thread_inheritance.rs`](src/agent/thread_inheritance.rs)) |

### Slack-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Streaming draft replies | ✅ | ✅ | End-to-end: StreamMode + DraftReplyState + `respond_with_tools_streaming` + agent loop integration |
| Configurable stream modes | ✅ | ✅ | `DISCORD_STREAM_MODE` + `TELEGRAM_STREAM_MODE` env vars; ChannelManager delegation |
| Thread ownership | ✅ | ✅ | `SessionManager::set_thread_owner` + `AgentRouter::claim_thread` (first-responder wins) |

### Channel Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| DM pairing codes | ✅ | ✅ | `ironclaw pairing list/approve`, host APIs |
| Allowlist/blocklist | ✅ | ✅ | allow_from + block_from + pairing store (blocklist takes precedence) |
| Self-message bypass | ✅ | ✅ | Multi-bot-ID detection + message filtering ([`src/channels/self_message.rs`](src/channels/self_message.rs)) |
| Mention-based activation | ✅ | ✅ | bot_username + respond_to_all_group_messages |
| Per-group tool policies | ✅ | ✅ | `ToolPolicyManager` with AllowAll/AllowList/DenyList + hierarchical evaluation ([`src/tools/policy.rs`](src/tools/policy.rs)) |
| Thread isolation | ✅ | ✅ | Separate sessions per thread |
| Per-channel media limits | ✅ | ✅ | `MediaLimits` with per-channel env var overrides + `filter_attachments()` ([`src/media/limits.rs`](src/media/limits.rs)) |
| Typing indicators | ✅ | ✅ | `Channel::send_typing()` trait method with platform-agnostic interface; TUI + Telegram + extensible to Discord/Signal |
| Per-channel ackReaction config | ✅ | ✅ | Per-channel emoji overrides via env vars ([`src/channels/ack_reaction.rs`](src/channels/ack_reaction.rs)) |
| Group session priming | ✅ | ✅ | Member roster injection with configurable limits ([`src/channels/group_priming.rs`](src/channels/group_priming.rs)) |
| Sender_id in trusted metadata | ✅ | ✅ | `TrustedMetadata` struct with sender_id, channel, is_group ([`src/channels/self_message.rs`](src/channels/self_message.rs)) |

### Owner: IronClaw Agent

---

## 4. CLI Commands

| Command | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| `run` (agent) | ✅ | ✅ | - | Default command |
| `tool install/list/remove` | ✅ | ✅ | - | WASM tools |
| `gateway start/stop` | ✅ | ✅ | P2 | `gateway.rs`: start (foreground/bg with PID), stop (SIGTERM), status (health+uptime) |
| `onboard` (wizard) | ✅ | ✅ | - | Interactive setup |
| `tui` | ✅ | ✅ | - | Ratatui TUI |
| `config` | ✅ | ✅ | - | Read/write config |
| `channels` | ✅ | ✅ | P2 | `channels.rs`: list (env+WASM detection), info (per-channel details) |
| `models` | ✅ | ✅ | - | `list`, `info`, `test` subcommands with Ollama auto-discovery ([`src/cli/models.rs`](src/cli/models.rs)) |
| `status` | ✅ | ✅ | - | System status (enriched session details) |
| `agents` | ✅ | ✅ | P3 | `list`, `add`, `remove`, `show`, `set-default` subcommands |
| `sessions` | ✅ | ✅ | P3 | `list`, `show`, `prune` subcommands with thread ownership display |
| `memory` | ✅ | ✅ | - | Memory search CLI |
| `skills` | ✅ | ✅ | - | Skills tools + web API endpoints (install, list, activate) |
| `pairing` | ✅ | ✅ | - | list/approve, account selector |
| `nodes` | ✅ | ✅ | P3 | Device management with CRUD + formatted display ([`src/cli/nodes.rs`](src/cli/nodes.rs)) |
| `plugins` | ✅ | ✅ | - | `PluginManifest` — persistent JSON manifest, install/remove/toggle |
| `hooks` | ✅ | ✅ | P2 | Lifecycle hooks |
| `cron` | ✅ | ✅ | - | `cron add/edit/remove/trigger/runs/lint` — edit supports `--model`, `--thinking-budget`, `--schedule`, `--prompt`, `--enabled`; lint validates expressions offline |
| `webhooks` | ✅ | ✅ | - | `POST /hooks/routine/{id}` with HMAC-SHA256 secret validation |
| `message send` | ✅ | ✅ | P2 | `message.rs`: send to gateway with auth, auto-detect URL |
| `browser` | ✅ | ✅ | P3 | Headless Chrome: open/screenshot/links/check ([`src/cli/browser.rs`](src/cli/browser.rs)) |
| `sandbox` | ✅ | ✅ | - | WASM sandbox |
| `doctor` | ✅ | ✅ | - | Diagnostics (DB, binaries, LLM credentials, Tailscale) |
| `logs` | ✅ | ✅ | P3 | tail/search/show/levels with time-range/level/target filtering ([`src/cli/logs.rs`](src/cli/logs.rs)) |
| `update` | ✅ | ✅ | P3 | check/install/rollback with stable/beta/nightly channels ([`src/cli/update.rs`](src/cli/update.rs)) |
| `completion` | ✅ | ✅ | - | Shell completion |
| `/subagents spawn` | ✅ | ✅ | P3 | Command parsing + subagent tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs)) |
| `/export-session` | ✅ | ✅ | P3 | `sessions export` with markdown/JSON output ([`src/cli/sessions.rs`](src/cli/sessions.rs)) |

### Owner: IronClaw Agent

---

## 5. Agent System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Pi agent runtime | ✅ | ➖ | IronClaw uses custom runtime |
| RPC-based execution | ✅ | ✅ | Orchestrator/worker pattern |
| Multi-provider failover | ✅ | ✅ | `FailoverProvider` tries providers sequentially on retryable errors |
| Per-sender sessions | ✅ | ✅ | |
| Global sessions | ✅ | ✅ | Cross-channel shared context with LRU eviction ([`src/agent/global_session.rs`](src/agent/global_session.rs)) |
| Session pruning | ✅ | ✅ | `sessions prune` CLI + auto-cleanup with configurable TTL |
| Context compaction | ✅ | ✅ | Auto summarization |
| Post-compaction read audit | ✅ | ✅ | `ReadAuditor` with scope-based rule scanning + token-budgeted appendix ([`src/context/read_audit.rs`](src/context/read_audit.rs)) |
| Post-compaction context injection | ✅ | ✅ | Priority-based fragment assembly with token budgets ([`src/context/post_compaction.rs`](src/context/post_compaction.rs)) |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill routing blocks | ✅ | ✅ | ActivationCriteria: keywords, patterns, tags, `use_when`/`dont_use_when` routing blocks |
| Skill path compaction | ✅ | ✅ | `~` prefix via `RuntimeBehavior::compact_path()` ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |
| Thinking modes (low/med/high) | ✅ | ✅ | `ThinkingConfig` enum (Disabled/Enabled with budget_tokens), configurable via `thinking_enabled` + `thinking_budget_tokens` settings |
| Per-model thinkingDefault override | ✅ | ✅ | `MODEL_THINKING_OVERRIDE` env var: exact+prefix model match with per-model budget |
| Block-level streaming | ✅ | ✅ | `StreamChunk::Text` + `StreamChunk::ReasoningDelta` via `complete_stream()` |
| Tool-level streaming | ✅ | ✅ | `StreamChunk::ToolCall` + `StreamChunk::ToolCallDelta` via `complete_stream_with_tools()` |
| Z.AI tool_stream | ✅ | ✅ | Full tool_stream protocol with delta accumulation ([`src/channels/tool_stream.rs`](src/channels/tool_stream.rs)) |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Elevated mode | ✅ | ✅ | Timeout-based activation with command allowlisting ([`src/safety/elevated.rs`](src/safety/elevated.rs)) |
| Subagent support | ✅ | ✅ | Task framework |
| `/subagents spawn` command | ✅ | ✅ | Command parsing + tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs)) |
| Auth profiles | ✅ | ✅ | Multi-key rotation with health tracking ([`src/safety/auth_profiles.rs`](src/safety/auth_profiles.rs)) |
| Generic API key rotation | ✅ | ✅ | Multi-strategy rotation with health tracking ([`src/safety/key_rotation.rs`](src/safety/key_rotation.rs)) |
| Stuck loop detection | ✅ | ✅ | Consecutive same-tool detection with warn at 3, force-text at 5 |
| llms.txt discovery | ✅ | ✅ | .well-known probing + markdown link parsing ([`src/llm/llms_txt.rs`](src/llm/llms_txt.rs)) |
| Multiple images per tool call | ✅ | ✅ | `ImageExtractor::format_multiple_for_llm()` — multi-image content blocks |
| URL allowlist (web_search/fetch) | ✅ | ✅ | `HTTP_URL_ALLOWLIST` env var — comma-separated domain globs |
| suppressToolErrors config | ✅ | ✅ | `RuntimeBehavior::format_tool_error()` with generic fallback ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |
| Intent-first tool display | ✅ | ✅ | Human-readable intent + exec summaries ([`src/tools/intent_display.rs`](src/tools/intent_display.rs)) |
| Transcript file size in status | ✅ | ✅ | `TranscriptStats` with message/token/tool/attachment counts ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |

### Owner: IronClaw Agent

---

## 6. Model & Provider Support

| Provider | OpenClaw | IronClaw | Priority | Notes |
|----------|----------|----------|----------|-------|
| Anthropic (Claude) | ✅ | ✅ | - | Direct API via RigAdapter (Sonnet 4, etc.) |
| OpenAI (GPT) | ✅ | ✅ | - | Direct API via RigAdapter (GPT-4o, etc.) |
| OpenAI-compatible | ❌ | ✅ | - | **Default backend** — works with OpenRouter, vLLM, LiteLLM, Together, Fireworks |
| OpenRouter | ✅ | ✅ | - | Via OpenAI-compatible provider |
| Ollama (local) | ✅ | ✅ | - | Via `rig::providers::ollama` (full support) |
| Tinfoil | ❌ | ✅ | - | Private inference provider (IronClaw-only) |
| AWS Bedrock | ✅ | ✅ | P3 | OpenAI-to-Bedrock adapter with Converse API translation ([`src/llm/bedrock.rs`](src/llm/bedrock.rs)) |
| Google Gemini | ✅ | ✅ | P3 | AI Studio adapter with system instruction support ([`src/llm/gemini.rs`](src/llm/gemini.rs)) |
| NVIDIA API | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Nvidia` — pre-configured OpenAI-compatible endpoint ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| Perplexity | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Perplexity` — `sonar-pro` default model ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| MiniMax | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::MiniMax` with `X-MiniMax-Version` header ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| GLM-5 | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Glm` — Zhipu `glm-4-plus` default ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| node-llama-cpp | ✅ | ➖ | - | N/A for Rust |
| llama.cpp (native) | ❌ | ✅ | P3 | `LlamaModel` trait + `LlamaConfig` + `LlamaCppStub` fallback, ready for `llama-cpp-2` FFI ([`src/llm/llama_cpp.rs`](src/llm/llama_cpp.rs)) |

### Model Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Auto-discovery | ✅ | ✅ | `ModelDiscovery` service ([`src/llm/discovery.rs`](src/llm/discovery.rs)): OpenAI/Anthropic/Ollama endpoint scanning |
| Failover chains | ✅ | ✅ | `FailoverProvider` with configurable `fallback_model` |
| Cooldown management | ✅ | ✅ | Lock-free per-provider cooldown in `FailoverProvider` |
| Per-session model override | ✅ | ✅ | Model selector in TUI |
| Model selection UI | ✅ | ✅ | TUI keyboard shortcut |
| Per-model thinkingDefault | ✅ | ✅ | `MODEL_THINKING_OVERRIDE` env var with exact+prefix matching |
| 1M context beta header | ✅ | ✅ | `ExtendedContextConfig` with configurable beta header ([`src/llm/extended_context.rs`](src/llm/extended_context.rs)) |

### Owner: IronClaw Agent

---

## 7. Media Handling

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Image processing (Sharp) | ✅ | ✅ | P2 | `ImageExtractor`: dimension detection (PNG/GIF/WebP/JPEG), base64 data-URI, detail level hints |
| Configurable image resize dims | ✅ | ✅ | P2 | `with_max_dimensions()` — sets OpenAI `detail` level (high/low) based on image dims |
| Multiple images per tool call | ✅ | ✅ | P2 | `format_multiple_for_llm()` — array of image content blocks |
| Audio transcription | ✅ | ✅ | P2 | `AudioExtractor`: Whisper HTTP endpoint, multipart upload |
| Video support | ✅ | ✅ | P3 | `VideoAnalyzer` ([`src/media/video.rs`](src/media/video.rs)) — ffprobe metadata, ffmpeg keyframe + audio extraction, graceful fallback |
| PDF parsing | ✅ | ✅ | P2 | `PdfExtractor`: BT/ET text blocks, readable-sequence fallback |
| MIME detection | ✅ | ✅ | P2 | `media/types.rs`: extension + magic bytes detection |
| Media caching | ✅ | ✅ | P3 | Per-channel cache policies with eviction strategies ([`src/media/media_cache_config.rs`](src/media/media_cache_config.rs)) |
| Vision model integration | ✅ | ✅ | P2 | `ImageExtractor::format_for_llm()` — base64 data-URI for multimodal LLMs |
| TTS (Edge TTS) | ✅ | ✅ | - | `TtsSynthesizer` with Edge TTS provider support |
| TTS (OpenAI) | ✅ | ✅ | - | `tools/builtin/tts.rs` — OpenAI TTS tool |
| Incremental TTS playback | ✅ | ✅ | P3 | `SentenceChunker` + `TtsChunk` streaming via SSE, progressive chunk synthesis ([`src/media/tts_streaming.rs`](src/media/tts_streaming.rs)) |
| Sticker-to-image | ✅ | ✅ | P3 | WebP/TGS/WebM detection + ffmpeg conversion ([`src/media/sticker.rs`](src/media/sticker.rs)) |
| Media pipeline integration | ❌ | ✅ | - | `MediaPipeline` auto-wired into `process_user_input()` via `IncomingMessage.attachments` |

### Owner: IronClaw Agent

---

## 8. Plugin & Extension System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Dynamic loading | ✅ | ✅ | WASM modules |
| Manifest validation | ✅ | ✅ | WASM metadata |
| HTTP path registration | ✅ | ✅ | Plugin route registry with conflict detection ([`src/extensions/plugin_routes.rs`](src/extensions/plugin_routes.rs)) |
| Workspace-relative install | ✅ | ✅ | ~/.ironclaw/tools/ |
| Channel plugins | ✅ | ✅ | WASM channels |
| Auth plugins | ✅ | ✅ | `AuthPlugin` trait + `AuthCredentials`/`AuthToken` ([`src/extensions/plugin_interfaces.rs`](src/extensions/plugin_interfaces.rs)) |
| Memory plugins | ✅ | ✅ | `MemoryPlugin` trait + `MemoryEntry` ([`src/extensions/plugin_interfaces.rs`](src/extensions/plugin_interfaces.rs)) |
| Tool plugins | ✅ | ✅ | WASM tools |
| Hook plugins | ✅ | ✅ | Declarative hooks from extension capabilities |
| Provider plugins | ✅ | ✅ | `ProviderPlugin` trait + capabilities ([`src/extensions/plugin_interfaces.rs`](src/extensions/plugin_interfaces.rs)) |
| Plugin CLI (`install`, `list`) | ✅ | ✅ | `tool` subcommand |
| ClawHub registry | ✅ | ✅ | `ClawHubConfig` + `CatalogCache` with TTL, search, merge ([`src/extensions/clawhub.rs`](src/extensions/clawhub.rs)) |
| `before_agent_start` hook | ✅ | ✅ | `HookPoint::BeforeAgentStart` — fires before agent main loop, can reject startup |
| `before_message_write` hook | ✅ | ✅ | `HookPoint::BeforeMessageWrite` — fires before channel write, can modify/suppress |
| `llm_input`/`llm_output` hooks | ✅ | ✅ | Before/after hook pipeline with priority ordering ([`src/llm/llm_hooks.rs`](src/llm/llm_hooks.rs)) |

### Owner: IronClaw Agent

---

## 9. Configuration System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Primary config file | ✅ `~/.openclaw/openclaw.json` | ✅ `.env` | Different formats |
| JSON5 support | ✅ | ✅ | Comment stripping + trailing comma removal ([`src/config/formats.rs`](src/config/formats.rs)) |
| YAML alternative | ✅ | ✅ | YAML-to-JSON + JSON-to-YAML serialization ([`src/config/formats.rs`](src/config/formats.rs)) |
| Environment variable interpolation | ✅ | ✅ | `${VAR}` |
| Config validation/schema | ✅ | ✅ | Type-safe Config struct |
| Hot-reload | ✅ | ✅ | `ConfigWatcher` polls mtime, broadcasts changes |
| Legacy migration | ✅ | ➖ | |
| State directory | ✅ `~/.openclaw-state/` | ✅ `~/.ironclaw/` | |
| Credentials directory | ✅ | ✅ | Session files |
| Full model compat fields in schema | ✅ | ✅ | `ModelCompat` with context window, feature flags, pricing, pi-ai support ([`src/config/model_compat.rs`](src/config/model_compat.rs)) |

### Owner: IronClaw Agent

---

## 10. Memory & Knowledge System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Vector memory | ✅ | ✅ | pgvector |
| Session-based memory | ✅ | ✅ | |
| Hybrid search (BM25 + vector) | ✅ | ✅ | RRF algorithm |
| Temporal decay (hybrid search) | ✅ | ✅ | `apply_temporal_decay()` — exponential half-life scoring, wired into `hybrid_search()` post-RRF |
| MMR re-ranking | ✅ | ✅ | `mmr_rerank()` — greedy diversity selection with cosine similarity, wired into `hybrid_search()` |
| LLM-based query expansion | ✅ | ✅ | `expand_query_keywords()` — stop word removal + morphological variants, wired pre-FTS |
| OpenAI embeddings | ✅ | ✅ | |
| Gemini embeddings | ✅ | ✅ | `EmbeddingConfig::gemini()` ([`src/llm/embeddings.rs`](src/llm/embeddings.rs)) |
| Local embeddings | ✅ | ✅ | `EmbeddingConfig::local()` + Ollama support ([`src/llm/embeddings.rs`](src/llm/embeddings.rs)) |
| SQLite-vec backend | ✅ | ✅ | `SqliteVecConfig` with vec0 virtual table SQL, distance metrics ([`src/workspace/sqlite_vec.rs`](src/workspace/sqlite_vec.rs)) |
| LanceDB backend | ✅ | ✅ | `LanceDbConfig` with Arrow schema, S3/local URI support ([`src/workspace/lancedb.rs`](src/workspace/lancedb.rs)) |
| QMD backend | ✅ | ✅ | `QmdConfig` with product quantization, codebook sizing ([`src/workspace/qmd.rs`](src/workspace/qmd.rs)) |
| Atomic reindexing | ✅ | ✅ | |
| Embeddings batching | ✅ | ✅ | `embed_batch` on EmbeddingProvider trait |
| Citation support | ✅ | ✅ | `Citation` struct with inline/footnote formatting, deduplication, relevance sorting ([`src/workspace/citations.rs`](src/workspace/citations.rs)) |
| Memory CLI commands | ✅ | ✅ | `memory search/read/write/tree/status` CLI subcommands |
| Flexible path structure | ✅ | ✅ | Filesystem-like API |
| Identity files (AGENTS.md, etc.) | ✅ | ✅ | |
| Daily logs | ✅ | ✅ | |
| Heartbeat checklist | ✅ | ✅ | HEARTBEAT.md |

### Owner: IronClaw Agent

---

## 11. Mobile Apps

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| iOS app (SwiftUI) | ✅ | 🚫 | - | Out of scope initially |
| Android app (Kotlin) | ✅ | 🚫 | - | Out of scope initially |
| Apple Watch companion | ✅ | 🚫 | - | Send/receive messages MVP |
| Gateway WebSocket client | ✅ | 🚫 | - | |
| Camera/photo access | ✅ | 🚫 | - | |
| Voice input | ✅ | 🚫 | - | |
| Push-to-talk | ✅ | 🚫 | - | |
| Location sharing | ✅ | 🚫 | - | |
| Node pairing | ✅ | 🚫 | - | |
| APNs push notifications | ✅ | 🚫 | - | Wake disconnected nodes before invoke |
| Share to OpenClaw (iOS) | ✅ | 🚫 | - | iOS share sheet integration |
| Background listening toggle | ✅ | 🚫 | - | iOS background audio |

### Owner: IronClaw Agent (if ever prioritized)

---

## 12. macOS App (Scrappy — Tauri/Rust)

> Scrappy is the macOS desktop app built with Tauri v2 (Rust + React).
> A native Swift macOS app may be added later.

| Feature | OpenClaw | Scrappy | Status | Notes |
|---------|----------|---------|--------|-------|
| SwiftUI native app | ✅ | ✅ | Covered | Tauri v2 (Rust + React) — intentional tech choice |
| Menu bar presence | ✅ | ✅ | Covered | `TrayIconBuilder` with template icon + click handler |
| Bundled gateway | ✅ | ✅ | Superior | In-process IronClaw agent, no separate process |
| Canvas hosting | ✅ | ✅ | Covered | `CanvasWindow.tsx` with bidirectional iframe comms |
| Exec approval dialogs | ✅ | ✅ | Superior | 3-tier ApprovalCard (Deny/Once/Session) with session caching |
| Instances tab | ✅ | ✅ | Covered | Fleet Command Center with visual graph + broadcast terminal |
| Push-to-talk hotkey | ✅ | ✅ | Done | Global `Cmd+Shift+V` → record → transcribe → insert |
| Agent events debug | ✅ | ✅ | Done | `OpenClawEventInspector.tsx` with live event stream + filters |
| Canvas placement | ✅ | ✅ | Done | Draggable + resizable + maximize floating panel |
| Auto-updates | ✅ | ✅ | Done | `tauri-plugin-updater` + `UpdateChecker.tsx` — auto-check, download, install, restart |
| Voice wake | ✅ | ✅ | Full | VAD-based voice activation + Sherpa-ONNX keyword spotting backend with auto-fallback |
| iMessage integration | ✅ | ✅ | - | chat.db polling + osascript sending, group chats, attachments, dedup, diagnostics |

### Detailed Coverage Evidence

| Feature | Scrappy Implementation | Files |
|---------|----------------------|-------|
| **Menu bar** | `TrayIconBuilder` with `tray-iconTemplate.png`. Left-click toggles main window. Template icon follows macOS dark/light mode. **Now with** animated active state (dot badge on Thinking/ToolStarted, auto-reset after 3s). | `lib.rs`, `ironclaw_channel.rs` |
| **Bundled gateway** | In-process IronClaw agent via Rust FFI. No Node.js, no WS bridge. Saves ~120MB bundle. Zero cold-start latency. | `ironclaw_bridge.rs` |
| **Canvas hosting** | `CanvasWindow.tsx`. Receives `CanvasUpdate` events + legacy `canvas-push`/`canvas-navigate`. Sandboxed iframe with `postMessage` shim for bidirectional comms. Spring-animated slide-in/out. **Now with** mouse drag + corner resize handles + maximize toggle. | `CanvasWindow.tsx` |
| **Exec approval** | `ApprovalCard.tsx` (142 LOC) with 3-tier model: Deny / Allow Once / Allow Session. `TauriToolBridge` (343 LOC) with session-permission cache and 5-min timeout. **Superior to OpenClaw's** binary approve/deny. | `ApprovalCard.tsx`, `tool_bridge.rs` |
| **Instances/Fleet** | `FleetCommandCenter.tsx` with real-time status polling (visibility-based throttling). `FleetGraph.tsx` visual network graph. `FleetTerminal.tsx` command broadcast. `fleet.rs` (294 LOC) with `AgentStatusSummary`, capability detection. | `fleet.rs`, `FleetCommandCenter.tsx` |
| **PTT** | Global shortcut (Cmd+Shift+V, configurable via `tauri-plugin-global-shortcut`). `useAudioRecorder` hook for mic capture. Backend emits `ptt_toggle` event → ChatProvider toggles recording → Whisper transcription → text inserted. | `lib.rs`, `ChatProvider.tsx`, `use-audio-recorder.ts` |
| **Agent events** | `LiveAgentStatus.tsx` renders tool execution, approval cards, canvas updates inline. `OpenClawEventInspector.tsx` provides dedicated raw event stream with filters, pause, and clear. | `LiveAgentStatus.tsx`, `OpenClawEventInspector.tsx` |
| **Auto-updates** | `tauri-plugin-updater` + `tauri-plugin-process` in `Cargo.toml`. Updater config in `tauri.conf.json` with signing key + GitHub Releases endpoint. `UpdateChecker.tsx` — floating notification card: auto-check 5s after launch, download progress bar, install + restart. Signing keypair at `~/.tauri/scrappy.key`. | `Cargo.toml`, `tauri.conf.json`, `lib.rs`, `UpdateChecker.tsx`, `App.tsx` |
| **Voice wake** | `VoiceWakeOverlay.tsx` + `use-voice-wake.ts` — VAD-based (RMS energy threshold). Floating mic button toggles always-on listening. Auto-starts recording when speech detected, auto-transcribes via Whisper, inserts into chat input. Custom window event bridges to ChatProvider. **Gap:** No real wake-word (e.g. Porcupine/Snowboy) — uses energy-based VAD instead. | `VoiceWakeOverlay.tsx`, `use-voice-wake.ts`, `ChatProvider.tsx` |

### Owner: Scrappy Agent

---

## 13. Web Interface

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Control UI Dashboard | ✅ | ✅ | - | Web gateway with chat, memory, jobs, logs, extensions |
| Channel status view | ✅ | ✅ | P2 | `ChannelStatusView` with per-channel state machine, table/JSON format ([`src/channels/status_view.rs`](src/channels/status_view.rs)) |
| Agent management | ✅ | ✅ | P3 | CLI: `agents list/add/remove/show/set-default`; `AgentRouter` dispatch pipeline |
| Model selection | ✅ | ✅ | - | TUI only |
| Config editing | ✅ | ✅ | P3 | `Settings.set()/.get()/.list()/.reset()` with typed path-based access ([`src/settings.rs`](src/settings.rs)) |
| Debug/logs viewer | ✅ | ✅ | - | Real-time log streaming with level/target filters |
| WebChat interface | ✅ | ✅ | - | Web gateway chat with SSE/WebSocket |
| Canvas system (A2UI) | ✅ | ✅ | P3 | `CanvasTool` + `CanvasStore` + canvas gateway routes for HTML/JSON rendering ([`src/channels/canvas_gateway.rs`](src/channels/canvas_gateway.rs)) |
| Control UI i18n | ✅ | ✅ | - | EN/ES/ZH/JA locales with key-based lookup + fallback |
| WebChat theme sync | ✅ | ✅ | P3 | `WebChatConfig` + `WebChatTheme` (Light/Dark/System), CSS variables, env var loading ([`src/config/webchat.rs`](src/config/webchat.rs)) |
| Partial output on abort | ✅ | ✅ | - | Extracts last 3 tool/assistant results on interrupt |

### Owner: IronClaw Agent

---

## 14. Automation

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Cron jobs | ✅ | ✅ | - | Routines with cron trigger |
| Cron stagger controls | ✅ | ✅ | P3 | `StaggerConfig` + `CronGate` ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs)) |
| Cron finished-run webhook | ✅ | ✅ | P3 | `FinishedRunPayload` + `notify_finished_run()` ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs)) |
| Timezone support | ✅ | ✅ | - | Via cron expressions |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers |
| Channel health monitor | ✅ | ✅ | `ChannelHealthMonitor` wired into background tasks |
| `beforeInbound` hook | ✅ | ✅ | P2 | |
| `beforeOutbound` hook | ✅ | ✅ | P2 | |
| `beforeToolCall` hook | ✅ | ✅ | P2 | |
| `before_agent_start` hook | ✅ | ✅ | P2 | Fires before agent loop, can reject startup |
| `before_message_write` hook | ✅ | ✅ | P2 | Fires before channel write, can modify/suppress |
| `onMessage` hook | ✅ | ✅ | - | Routines with event trigger |
| `onSessionStart` hook | ✅ | ✅ | P2 | |
| `onSessionEnd` hook | ✅ | ✅ | P2 | |
| `transcribeAudio` hook | ✅ | ✅ | P3 | `BeforeTranscribeAudio` hook point ([`src/hooks/hook.rs`](src/hooks/hook.rs)) |
| `transformResponse` hook | ✅ | ✅ | P2 | |
| `llm_input`/`llm_output` hooks | ✅ | ✅ | P3 | Before/after hook pipeline ([`src/llm/llm_hooks.rs`](src/llm/llm_hooks.rs)) |
| Bundled hooks | ✅ | ✅ | P2 | Audit + declarative rule/webhook hooks |
| Plugin hooks | ✅ | ✅ | P3 | Registered from WASM `capabilities.json` |
| Workspace hooks | ✅ | ✅ | P2 | `hooks/hooks.json` and `hooks/*.hook.json` |
| Outbound webhooks | ✅ | ✅ | P2 | Fire-and-forget lifecycle event delivery |
| Heartbeat system | ✅ | ✅ | - | Periodic execution |
| Gmail pub/sub | ✅ | ✅ | P3 | `GmailConfig` + `parse_pubsub_push()` + sender filtering ([`src/channels/gmail_wiring.rs`](src/channels/gmail_wiring.rs)) |

### Owner: IronClaw Agent

---

## 15. Security Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Gateway token auth | ✅ | ✅ | Bearer token auth on web gateway |
| Device pairing | ✅ | ✅ | `PairingStore` with challenge-response states + fingerprint verification ([`src/safety/device_pairing.rs`](src/safety/device_pairing.rs)) |
| Tailscale identity | ✅ | ✅ | `TailscaleIdentity` + `extract_identity()` + `is_trusted_peer()` for passwordless gateway auth ([`src/tailscale.rs`](src/tailscale.rs)) |
| Trusted-proxy auth | ✅ | ✅ | `TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS` for reverse-proxy deployments |
| OAuth flows | ✅ | ✅ | Full Auth Code + PKCE flow, auto-refresh, scope aggregation, built-in Google/GitHub/Notion creds |
| DM pairing verification | ✅ | ✅ | ironclaw pairing approve, host APIs |
| Allowlist/blocklist | ✅ | ✅ | allow_from + block_from + pairing store, CLI block/unblock commands |
| Per-group tool policies | ✅ | ✅ | `ToolPolicyManager` ([`src/tools/policy.rs`](src/tools/policy.rs)) |
| Exec approvals | ✅ | ✅ | TUI overlay |
| TLS 1.3 minimum | ✅ | ✅ | reqwest rustls |
| SSRF protection | ✅ | ✅ | WASM allowlist |
| SSRF IPv6 transition bypass block | ✅ | ✅ | IPv4-mapped IPv6 (::ffff:x.x.x.x) detection in `is_ipv4_mapped_v6_private()` |
| Cron webhook SSRF guard | ✅ | ✅ | N/A — webhooks are inbound-only; no outbound delivery to user URLs |
| Loopback-first | ✅ | ✅ | Gateway binds 127.0.0.1 by default, HTTP webhook binds 0.0.0.0 for inbound |
| Docker sandbox | ✅ | ✅ | Orchestrator/worker containers |
| Podman support | ✅ | ✅ | `PodmanConfig` with rootless mode, resource limits, `podman run` arg building ([`src/sandbox/podman.rs`](src/sandbox/podman.rs)) |
| WASM sandbox | ❌ | ✅ | IronClaw innovation |
| Sandbox env sanitization | ✅ | ✅ | Shell tool scrubs env vars + LD*/DYLD* injection blocks + safe bins allowlist |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ✅ | ([`src/safety/elevated.rs`](src/safety/elevated.rs)) |
| Safe bins allowlist | ✅ | ✅ | `IRONCLAW_SAFE_BINS_ONLY` + extensible `IRONCLAW_EXTRA_BINS` |
| LD*/DYLD* validation | ✅ | ✅ | Blocks `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, etc. |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) |
| Credential theft via env injection | ✅ | ✅ | Shell env scrubbing + command injection detection + LD*/DYLD* blocking + safe bins |
| Session file permissions (0o600) | ✅ | ✅ | Handled by OS keychain + filesystem perms |
| Skill download path restriction | ✅ | ✅ | ([`src/safety/skill_path.rs`](src/safety/skill_path.rs)) |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ✅ | SSRF prevention: blocks private IPs, cloud metadata, disallowed schemes ([`src/safety/media_url.rs`](src/safety/media_url.rs)) |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ✅ | `DangerousToolTracker` ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs)) |

### Owner: IronClaw Agent

---

## 16. Development & Build System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Primary language | TypeScript | Rust | Different ecosystems |
| Build tool | tsdown | cargo | |
| Type checking | TypeScript/tsgo | rustc | |
| Linting | Oxlint | clippy | |
| Formatting | Oxfmt | rustfmt | |
| Package manager | pnpm | cargo | |
| Test framework | Vitest | built-in | |
| Coverage | V8 | tarpaulin/llvm-cov | |
| CI/CD | GitHub Actions | GitHub Actions | |
| Pre-commit hooks | prek | - | Consider adding |
| Docker: Chromium + Xvfb | ✅ | ✅ | `DockerChromiumConfig` with Xvfb + Chrome launch args ([`src/sandbox/docker_chromium.rs`](src/sandbox/docker_chromium.rs)) |
| Docker: init scripts | ✅ | ✅ | `DockerInitConfig` with script discovery + interpreter detection ([`src/sandbox/docker_init.rs`](src/sandbox/docker_init.rs)) |
| Browser: extraArgs config | ✅ | ✅ | `BrowserArgsConfig` with custom Chrome flags, proxy, hardening ([`src/tools/browser_args.rs`](src/tools/browser_args.rs)) |

### Owner: IronClaw Agent

---

## 17. Scrappy Feature Surfacing Analysis

> **Last updated:** 2026-03-04 10:48 CET — reconciled after Sprint 12 completion
>
> IronClaw has shipped far more capabilities than Scrappy currently exposes through its UI.
> This section tracks which IronClaw features Scrappy surfaces, which need UI, and wiring gaps.

### 17.1 Features Scrappy Has UI For (Wired)

| IronClaw Feature | Scrappy UI Component | Status |
|-----------------|---------------------|--------|
| Skills system (§5, §8) | `OpenClawSkills.tsx` — list, toggle, install | ✅ Wired via `openclaw_skills_*` commands |
| Memory/Workspace (§10) | `OpenClawMemory.tsx`, `MemoryEditor.tsx` | ✅ Wired via `openclaw_get_memory`, `openclaw_save_memory` |
| Cron/Routines (§14) | `OpenClawAutomations.tsx` | ✅ Wired via `openclaw_cron_*` commands |
| Channels (§3) | `OpenClawChannels.tsx` | ✅ Wired via `openclaw_channels_list` command |
| Subagent support (§5) | `SubAgentPanel.tsx` | ✅ Wired via Enhancement 2C |
| Fleet/presence (§2, §12) | `FleetCommandCenter.tsx`, `FleetGraph.tsx` | ✅ Wired |
| Canvas (§2, §12) | `CanvasWindow.tsx` | ✅ Wired with drag/resize/maximize |
| Exec approvals (§5, §15) | `ApprovalCard.tsx` | ✅ Wired, 3-tier |
| Thinking mode (§5) | Toggle + budget slider in `OpenClawChatView.tsx` | ✅ Native IronClaw `ThinkingConfig` via `openclaw_set_thinking` command |
| Dashboard (§2) | `OpenClawDashboard.tsx` | ✅ Wired |
| System control (§2) | `OpenClawSystemControl.tsx` | ✅ Wired |
| Cloud brain config (§6) | `CloudBrainConfigModal.tsx` | ✅ Wired |
| Presence/beacons (§2) | `OpenClawPresence.tsx` | ✅ Wired |
| Hooks management (§8, §14) | `OpenClawHooks.tsx` — list hooks, details, hook points | ✅ Wired via `openclaw_hooks_list` command |
| Extensions/Plugins (§8) | `OpenClawPlugins.tsx` — list, activate, remove | ✅ Wired via `openclaw_extensions_*` commands |
| Memory search (§10) | Search bar + semantic toggle in `OpenClawMemory.tsx` | ✅ Wired via `openclaw_memory_search` command |
| Session export (§4) | Export button in `OpenClawChatView.tsx` header | ✅ Wired via `openclaw_export_session` command |
| Push-to-talk hotkey (§5) | Global `Cmd+Shift+V` → record → transcribe → input | ✅ Wired via `ptt_toggle` event + `transcribe_audio` command |
| Config editor (§9) | `OpenClawConfig.tsx` — list/edit/add settings | ✅ Wired via `openclaw_config_get` / `openclaw_config_set` |
| Event inspector (§2) | `OpenClawEventInspector.tsx` — live event stream, filter, pause | ✅ Wired via `openclaw-event` listener |
| Doctor diagnostics (§2) | `OpenClawDoctor.tsx` — system health checks with progress bar | ✅ Wired via `openclaw_diagnostics` command |
| Tool policies (§8) | `OpenClawToolPolicies.tsx` — grouped tool list with search | ✅ Wired via `openclaw_tools_list` command |
| Canvas drag/resize (§2) | `CanvasWindow.tsx` — draggable, resizable, maximize | ✅ Enhanced with mouse drag + corner resize handles |
| DM pairing (§3) | `OpenClawPairing.tsx` — channel tabs, approve codes, sender list | ✅ Wired via `openclaw_pairing_list` / `openclaw_pairing_approve` |
| Stuck loop indicator (§5) | `LiveAgentStatus.tsx` — amber warning banner for 3+ consecutive same-tool calls | ✅ Frontend-only detection + visual warning |
| URL allowlist (§5) | `OpenClawConfig.tsx` — dedicated input section for `HTTP_URL_ALLOWLIST` | ✅ Wired via `setSetting` API |
| Context compaction (§5) | `OpenClawConfig.tsx` — analyze button with token stats display | ✅ Wired via `openclaw_compact_session` command |
| Failover model (§6) | `OpenClawConfig.tsx` — fallback model input for `LLM_FALLBACK_MODEL` | ✅ Wired via `setSetting` API |
| Animated tray icon (§2) | `ironclaw_channel.rs` — active dot badge on Thinking/ToolStarted, auto-reset | ✅ Backend-driven via TrayState |
| Auto-updates | `UpdateChecker.tsx` — check/download/install/restart via tauri-plugin-updater | ✅ Wired via `@tauri-apps/plugin-updater` + `tauri-plugin-process` |
| Voice wake (VAD + KWS) | `VoiceWakeOverlay.tsx` + `use-voice-wake.ts` — VAD energy detection + Sherpa-ONNX keyword spotting → auto-record → transcribe | ✅ Full — energy + Sherpa-ONNX backends |
| Channel listing (§3) | `OpenClawChannels.tsx` — all 6 channels (Slack, Telegram, Discord, Signal, Webhook, Nostr) with type badges | ✅ Wired via `openclaw_channels_list` command |
| Stream mode config (§3) | Per-channel stream mode selector in `OpenClawChannels.tsx` (Full/Typing/Disabled) | ✅ Wired via `setSetting` API |
| Cron lint validator (§14) | Expression validator in `OpenClawAutomations.tsx` with next-5-fire-times display | ✅ Wired via `openclaw_cron_lint` command |

### 17.2 Features IronClaw Has, Scrappy Has NO UI For

> **Sprint 14: All 8 stub commands wired** via unified `tauri_commands` facade module.
> Scrappy UI exists for 10/12 features. Gmail and routing rule builder deferred to Sprint 14.

| IronClaw Feature | API / Module | Scrappy UI | IronClaw Wiring | Status |
|-----------------|-------------|------------|-----------------|--------|
| **Channel status view** | `ChannelStatusView` ([`src/channels/status_view.rs`](src/channels/status_view.rs)) | ✅ Cards with state badges, counters, SSE listener | ✅ `openclaw_channel_status_list` live | ✅ End-to-end |
| **LLM cost tracker** | `CostTracker` ([`src/llm/cost_tracker.rs`](src/llm/cost_tracker.rs)) | ✅ Full dashboard: daily/monthly/model/agent bars, CSV export, alerts | ✅ `tauri_commands::cost_summary()` + `cost_export_csv()` | ✅ End-to-end |
| **LLM routing policy** | `RoutingPolicy` ([`src/llm/routing_policy.rs`](src/llm/routing_policy.rs)) | 🔮 Sprint 14 — full rule builder | ✅ Toggle API exists (`is_enabled`/`set_enabled`) | 🔮 Deferred |
| **ClawHub registry** | `CatalogCache` ([`src/extensions/clawhub.rs`](src/extensions/clawhub.rs)) | ✅ Search + install UI tab in Plugins page | ✅ `tauri_commands::clawhub_search()` + `clawhub_prepare_install()` | ✅ End-to-end |
| **Extension health monitor** | `ExtensionHealthMonitor` ([`src/extensions/ext_health_monitor.rs`](src/extensions/ext_health_monitor.rs)) | ✅ State badges (Running/Connecting/Degraded/Error) via Channel Status panel | ✅ Via `openclaw_channel_status_list` | ✅ End-to-end |
| **Routine audit log** | `RoutineAuditLog` ([`src/agent/routine_audit.rs`](src/agent/routine_audit.rs)) | ✅ Tabular log with outcome badges, filter, routine selector | ✅ `tauri_commands::routine_audit_list()` | ✅ End-to-end |
| **Multi-format session export** | `SessionExporter` ([`src/cli/session_export.rs`](src/cli/session_export.rs)) | ✅ `exportSession(key, format)` with backward compat | ✅ `openclaw_export_session` live (md/json/txt/csv/html) | ✅ End-to-end |
| **Agent management store** | `AgentManagementStore` ([`src/agent/management_api.rs`](src/agent/management_api.rs)) | ✅ Multi-agent picker + API wrapper + extended `AgentProfile` type | ✅ `openclaw_agents_set_default` live (writes to config) | ✅ End-to-end |
| **Gmail channel** | `GmailConfig` ([`src/channels/gmail_wiring.rs`](src/channels/gmail_wiring.rs)) | 🔮 Sprint 14 — needs OAuth PKCE browser flow | ⚠️ Config exists, no PKCE wiring | 🔮 Deferred |
| **Plugin manifest validator** | `ManifestValidator` ([`src/extensions/manifest_validator.rs`](src/extensions/manifest_validator.rs)) | ✅ Per-extension validate button with inline error/warning display | ✅ `tauri_commands::manifest_validate()` | ✅ End-to-end |
| **Plugin lifecycle hooks** | `LifecycleHookRegistry` ([`src/extensions/lifecycle_hooks.rs`](src/extensions/lifecycle_hooks.rs)) | ✅ Timeline tab in Plugins page with color-coded events | ✅ `tauri_commands::plugin_lifecycle_list()` | ✅ End-to-end |
| **Response cache metrics** | `CachedResponseStore` ([`src/llm/response_cache_ext.rs`](src/llm/response_cache_ext.rs)) | ✅ Hits/misses/rate/size cards + efficiency bar | ✅ `tauri_commands::cache_stats()` | ✅ End-to-end |

### 17.5 IronClaw Stub Wiring — ✅ ALL WIRED

> **All 8 Tauri commands wired** via unified `tauri_commands` facade ([`src/tauri_commands.rs`](src/tauri_commands.rs)).
> Scrappy’s `rpc.rs` stubs just need one-line calls to `ironclaw::tauri_commands::*`.

| # | Command | Wired To | Status |
|---|---------|----------|--------|
| 1 | `openclaw_cost_summary` | `tauri_commands::cost_summary()` → `CostTracker::summary()` | ✅ |
| 2 | `openclaw_cost_export_csv` | `tauri_commands::cost_export_csv()` → `CostTracker::export_csv()` | ✅ |
| 3 | `openclaw_clawhub_search` | `tauri_commands::clawhub_search()` → `CatalogCache::search(query)` | ✅ |
| 4 | `openclaw_clawhub_install` | `tauri_commands::clawhub_prepare_install()` → cache lookup + path resolution | ✅ |
| 5 | `openclaw_routine_audit_list` | `tauri_commands::routine_audit_list()` → `RoutineAuditLog::query_by_routine()` | ✅ |
| 6 | `openclaw_cache_stats` | `tauri_commands::cache_stats()` → `CachedResponseStore::stats()` | ✅ |
| 7 | `openclaw_plugin_lifecycle_list` | `tauri_commands::plugin_lifecycle_list()` → `AuditLogHook::events_serialized()` | ✅ |
| 8 | `openclaw_manifest_validate` | `tauri_commands::manifest_validate()` → `ManifestValidator::validate()` | ✅ |

### 17.3 Thinking Mode — ✅ Completed (2026-03-02)

The thinking toggle has been migrated from the localStorage hack to native IronClaw `ThinkingConfig`.

**Implemented:**
- `openclaw_set_thinking` Tauri command sets `AGENT_THINKING_ENABLED` env var + persists to IronClaw config store
- Toggle in `OpenClawChatView.tsx` now calls backend command instead of localStorage
- Budget slider (1K–32K tokens) accessible via right-click on 🧠 button
- Removed `"[Think step-by-step]"` message prefix injection from `handleSend`

### 17.4 Integration Contract — Sprint 12 APIs

> **Agreed with Scrappy agent:** 2026-03-04 11:00 CET
>
> All Tauri commands use `openclaw_*` prefix. No REST gateway — everything through Tauri IPC.
> Scrappy uses `specta` for type generation via `bindings.ts`.

#### Tauri Command Specifications

| Command | Module | Params | Response Shape | Notes |
|---------|--------|--------|---------------|-------|
| `openclaw_cost_summary` | `cost_tracker.rs` | — | `{ total_cost_usd, daily: {date→cost}, monthly: {month→cost}, by_model: {model→cost}, by_agent: {agent→cost}, alert_threshold_usd, alert_triggered }` | Single rich endpoint; frontend picks what to display |
| `openclaw_cost_export_csv` | `cost_tracker.rs` | — | `String` (CSV text) | For download/clipboard |
| `openclaw_channel_status_list` | `status_view.rs` | — | `Vec<ChannelStatusEntry>` with state/uptime/counters | Poll on mount; SSE for live updates |
| `openclaw_agents_list` | `management_api.rs` | — | **Extend existing** `Vec<AgentProfile>` with `is_default: Option<bool>`, `status: Option<String>`, `session_count: Option<u32>`, `last_active_at: Option<String>` | ⚠️ Already exists in `rpc.rs:834` — add fields as `Option<>` for backward compat |
| `openclaw_agents_set_default` | `management_api.rs` | `agent_id: String` | `Result<(), String>` | Follow `State<'_, OpenClawManager> + State<'_, IronClawState>` pattern |
| `openclaw_clawhub_search` | `clawhub.rs` | `query: String, filters: Option<...>` | Catalog entries | **Proxied** through IronClaw — `CLAWHUB_API_KEY` stays server-side |
| `openclaw_clawhub_install` | `clawhub.rs` | `plugin_id: String` | `Result<InstallResult, String>` | Installs to `~/.ironclaw/tools/` |
| `openclaw_routine_audit_list` | `routine_audit.rs` | `routine_key: String, limit: Option<u32>, outcome: Option<String>` | `Vec<RoutineAuditEntry>` | Default limit=20; outcome filter: `"success"` \| `"failure"` \| null |
| `openclaw_cache_stats` | `response_cache_ext.rs` | — | `CacheStats { hits, misses, evictions, size, hit_rate }` | |
| `openclaw_export_session` | `session_export.rs` | `session_key: String, format: Option<String>` | `SessionExportResponse` | **Extend existing** — add `format` param (`"md"` \| `"json"` \| `"csv"` \| `"html"` \| `"txt"`, default `"md"`) |
| `openclaw_plugin_lifecycle_list` | `lifecycle_hooks.rs` | — | `Vec<(String, LifecycleEvent)>` | |
| `openclaw_manifest_validate` | `manifest_validator.rs` | `plugin_id: String` | `{ errors: Vec<String>, warnings: Vec<String> }` | |

#### SSE Event Pattern

Channel status changes should be emitted via the existing `AppHandle::emit("openclaw-event", ...)` pipeline:

```json
{
  "kind": "ChannelStatus",
  "channel": "telegram",
  "state": "Reconnecting",
  "timestamp": "2026-03-04T10:30:00Z"
}
```

Scrappy subscribes to `openclaw-event` for live updates + polls `openclaw_channel_status_list` on mount. Fallback poll interval: 10s.

#### Gmail OAuth Flow

Gmail uses Scrappy's existing `cloud_oauth_start` / `cloud_oauth_complete` PKCE flow:

1. Frontend calls `cloudOauthStart("gmail")` → gets `{ auth_url, code_verifier }`
2. Opens browser → user authenticates with Google
3. Google redirects to localhost callback
4. Frontend calls `cloudOauthComplete("gmail", code, codeVerifier)`
5. Backend stores tokens in Keychain via `KeychainSecretsAdapter`

**IronClaw action required:** Add `"gmail"` variant to `oauth_defaults.rs` with Google OAuth client credentials (client_id, redirect_uri, scopes: `gmail.readonly`, `gmail.send`, `pubsub`). Do NOT build a separate `/auth/gmail` gateway endpoint.

#### ClawHub Architecture Decision

**Proxy through IronClaw gateway** (not direct frontend → ClawHub):
- `CLAWHUB_API_KEY` stays server-side (security)
- `CatalogCache` TTL benefits all consumers (caching)
- Consistent with all-via-Tauri-IPC pattern

#### Known Issues to Fix

1. **`openclaw_cron_history` is a stub** — Returns `[]` always (`rpc.rs:203`). Frontend `handleViewHistory()` in `OpenClawAutomations.tsx` already calls it with `(key, limit)` args. **Action:** Wire to `RoutineAuditLog`. → §17.5 item 5
2. **`openclaw_agents_list` missing fields** — ✅ **Fixed in Sprint 14** — `AgentSummary` extended with `session_count`, `last_active_at`, `update_activity()`, `increment_sessions()`, `find_by_status()`.
3. **`openclaw_channels_list` reads env vars** — Currently uses `OpenClawManager` + env vars instead of IronClaw Agent API. **Action:** Expose `channels_status()` API on Agent for cleaner integration.

#### Test Pattern

Scrappy has `openclaw.test.ts` (209 lines, Vitest) — mocks `invoke`, asserts command name + payload. New commands should get corresponding test stubs in `openclaw.test.ts` when frontend wrappers are added.

### Owner: Scrappy Agent

---

## 18. Scrappy-Exclusive Features

> These features exist only in Scrappy (Tauri) and are **not documented in IronClaw**.
> They represent Scrappy's own value proposition beyond the IronClaw engine.

| Feature | Components | Status |
|---------|-----------|--------|
| **Image generation (Imagine)** | `ImagineView.tsx`, `ImagineGeneration.tsx`, `ImagineGallery.tsx`, `ImagineSidebar.tsx` | ✅ Scrappy-only |
| **Local inference engine mgmt** | Sidecar manager — llama.cpp, whisper-server, embedding-server, stable-diffusion | ✅ Scrappy-only |
| **Model library + HF discovery** | `ModelBrowser.tsx`, `HFDiscovery.tsx`, `model-library.ts` (600+ LOC model catalog) | ✅ Scrappy-only |
| **Chat (Rig-based)** | `ChatView.tsx`, `use-chat.ts` — independent chat pipeline, not IronClaw agent | ✅ Scrappy-only |
| **Cloud storage** | 7 providers (S3, GCS, Azure, B2, R2, MinIO, Storj), sync, client-side encryption | ✅ Scrappy-only |
| **Cloud model discovery** | `use-cloud-models.ts` — discover models from cloud providers | ✅ Scrappy-only |
| **Spotlight bar** | `SpotlightBar.tsx` + global shortcut (Cmd+Shift+K) — quick-access floating chat | ✅ Scrappy-only |
| **Onboarding wizard** | `OnboardingWizard.tsx` — first-run setup flow | ✅ Scrappy-only |
| **MCP integration** | `McpTab.tsx` — Model Context Protocol server management | ✅ Scrappy-only |
| **Remote deploy wizard** | `RemoteDeployWizard.tsx` — deploy agent to remote infrastructure | ✅ Scrappy-only |
| **Inference mode tabs** | `InferenceModeTab.tsx` — local / cloud / hybrid mode selection | ✅ Scrappy-only |
| **Persona system** | `PersonaTab.tsx`, `PersonalizationTab.tsx` — custom AI personas | ✅ Scrappy-only |
| **Web search with progress** | `web_search` tool with `WebSearchBubble` progress events | ✅ Scrappy-only |
| **RAG pipeline** | `rag.rs` — document ingestion, chunking, vector retrieval | ✅ Scrappy-only |
| **Secrets management** | `SecretsTab.tsx` — macOS Keychain integration for API keys | ✅ Scrappy-only |
| **Settings system** | 10+ settings tabs (Chat Provider, Engine, Storage, MCP, Gateway, etc.) | ✅ Scrappy-only |

### Owner: Scrappy Agent

---

## 19. IronClaw → Scrappy Integration Tracker

> **Last updated:** 2026-03-04 11:02 CET — enriched with Scrappy agent feedback

### 19.1 Shipped — Scrappy UI Needed or In Progress

| IronClaw Feature | Shipped | Scrappy Work Needed | Scrappy State |
|-----------------|---------|---------------------|---------------|
| **Multi-agent picker** | ✅ `AgentManagementStore` | Sidebar dropdown + full settings panel; extend `AgentProfile` with `is_default`/`status`/`session_count` `Option<>` fields | ⚠️ `openclaw_agents_list` exists but missing fields |
| **Session pruning** | ✅ CLI `sessions prune` | Pruning config (max sessions, age cutoff) in settings | Not started |
| **Channel status view** | ✅ `ChannelStatusView` | Full per-channel status panel; SSE push `kind: "ChannelStatus"` + poll mount | ⚠️ `OpenClawChannels.tsx` reads enabled/disabled only — needs state/uptime/counters |
| **Gmail channel** | ✅ `GmailConfig` + pub/sub | Gmail card in `OpenClawChannels.tsx`; OAuth via existing `cloud_oauth_start("gmail")` PKCE flow | Not started — add `"gmail"` to `oauth_defaults.rs` |
| **LLM cost tracker** | ✅ `CostTracker` | Cost dashboard via `openclaw_cost_summary` — summary card + per-model chart + alert badge | Not started |
| **LLM routing policy** | ✅ `RoutingPolicy` | Advanced settings, collapsed by default; "Smart Routing" toggle = Sprint 13, full rule builder = Sprint 14 | Not started |
| **ClawHub browser** | ✅ `CatalogCache` | Plugin discovery via `openclaw_clawhub_search/install` — proxied through IronClaw | Not started — `OpenClawPlugins.tsx` exists for local only |
| **Plugin lifecycle log** | ✅ `AuditLogHook` | Lifecycle event log tab via `openclaw_plugin_lifecycle_list` | Not started |
| **Extension health badges** | ✅ `ExtensionHealthMonitor` | Health badge on channel/plugin cards | Not started |
| **Routine run history** | ✅ `RoutineAuditLog` | Wire to **existing `openclaw_cron_history` stub** (`rpc.rs:203` currently returns `[]`) | ⚠️ Frontend `handleViewHistory()` already calls the stub — just needs backend wiring |
| **Session export formats** | ✅ `SessionExporter` (5 formats) | Add `format: Option<String>` param to existing `openclaw_export_session`; add format picker + save-to-file option | Not started |
| **Response cache stats** | ✅ `CachedResponseStore` | Cache stats indicator via `openclaw_cache_stats` | Not started |
| **Manifest validation feedback** | ✅ `ManifestValidator` | Error/warning badges via `openclaw_manifest_validate` | Not started |

### 19.2 Upcoming — IronClaw Not Yet Shipped

| IronClaw Gap | Priority | Scrappy Impact When Shipped |
|-------------|----------|-----------------------------|
| **Media pipeline** enhancements | P3 | Frontend rendering for images/PDFs/audio in chat bubbles |
| **iMessage channel** | P4 | iMessage settings panel, if macOS-only mode needed |
| **WhatsApp channel** | P4 | QR pairing flow + WhatsApp card in `OpenClawChannels.tsx` |
| **APNs push pipeline** | Deferred | iOS push wake — needs Apple Developer cert infra |

### 19.3 Shipped — Scrappy Already Wired ✅

| IronClaw Feature | Scrappy UI |
|-----------------|------------|
| BridgedTool auto-registration | Sensor tools auto-register; `AppBuilder::init_tools()` wired |
| Canvas system (A2UI) | `CanvasWindow.tsx` with drag/resize/maximize |
| Streaming draft replies | Per-channel stream mode selector in channel cards |
| Discord channel | `OpenClawChannels.tsx` — type badge, stream mode config |
| Cron lint | `OpenClawAutomations.tsx` — validator + next-5-fire-times |
| Auto model discovery | Model list auto-populates from endpoint scan |

### Owner: Both (IronClaw ships backend, Scrappy ships frontend)

---

## 20. Scrappy Priority Ordering

> **Last updated:** 2026-03-04 10:48 CET — updated after Sprint 12 with new backend APIs

### Tier 1 — Do Now (≤ 3 days total) — All Complete ✅

| # | Action | Why | Effort | Status |
|---|--------|-----|--------|--------|
| 1 | **Auto-updates** (`tauri-plugin-updater`) | Blocking production distribution | 1 day | ✅ Done |
| 2 | **Wire thinking toggle to IronClaw native** | Remove prompt-prefix hack, use real `ThinkingConfig` | 3-4 hours | ✅ Done |

### Tier 2 — Next Sprint — All Complete ✅

| # | Action | Why | Effort | Status |
|---|--------|-----|--------|--------|
| 3 | **PTT hotkey** | High UX value, all infrastructure exists | 2 days | ✅ Done |
| 4 | **Hooks management UI** | IronClaw has rich hooks system, zero Scrappy UI | 1-2 days | ✅ Done |
| 5 | **Plugin manager panel** | IronClaw has `PluginManifest`, zero Scrappy UI | 1 day | ✅ Done |
| 6 | **Memory search bar** | Backend has hybrid BM25+vector search, Memory panel has no search | 4-6 hours | ✅ Done |
| 7 | **Session export** | Download conversation transcript | 2-3 hours | ✅ Done |

### Tier 3 — Previous Backlog — All Complete ✅

| # | Action | Why | Effort | Status |
|---|--------|-----|--------|--------|
| 8 | **Config editor panel** | IronClaw config API exists | 1-2 days | ✅ Done |
| 9 | **Debug event inspector** | Raw event window for developers | 0.5 day | ✅ Done |
| 10 | **Canvas drag/resize** | UX polish for `CanvasWindow.tsx` | 1 day | ✅ Done |
| 11 | **Doctor diagnostics panel** | System health check UI | 4-6 hours | ✅ Done |
| 12 | **DM pairing management** | Pairing config in settings | 4-6 hours | ✅ Done |
| 13 | **Tool policies manager** | Allow/deny per tool | 4-6 hours | ✅ Done |
| 14 | **Voice wake** (VAD + Sherpa-ONNX) | Voice activation with recording overlay + keyword spotting | ✅ | ✅ Full |
| 15 | **Animated tray icon** | Activity states (loading, active, idle) | 0.5 day | ✅ Done |
| 16 | **iMessage integration** | chat.db polling + osascript sending, group chat, attachment awareness, dedup, diagnostics | macOS-only | ✅ Implemented |

### Tier 4 — Sprint 13 New Backend APIs — ✅ All IronClaw Wired

> **Scrappy Sprint 13 + IronClaw Sprint 14:** All 10 implementable items are end-to-end.
> `tauri_commands` facade ([`src/tauri_commands.rs`](src/tauri_commands.rs)) bridges all 8 previously-stub commands.
> Deferred: #22 Gmail (needs OAuth PKCE) and #25 Routing rule builder → Sprint 14.

| # | Action | Backend | Tauri Command | Scrappy UI | IronClaw Wiring | Status |
|---|--------|---------|---------------|------------|-----------------|--------|
| 17 | **Multi-agent picker in sidebar** | `AgentManagementStore` | `openclaw_agents_set_default` live | ✅ API wrapper + extended `AgentProfile` | ✅ Writes to config | ✅ End-to-end |
| 18 | **LLM cost dashboard** | `CostTracker` | `openclaw_cost_summary` / `_export_csv` | ✅ Full dashboard | ✅ `tauri_commands::cost_summary()` | ✅ End-to-end |
| 19 | **Channel status panel** | `ChannelStatusView` | `openclaw_channel_status_list` live | ✅ Cards with state badges, SSE | ✅ Reads config + env | ✅ End-to-end |
| 20 | **ClawHub plugin browser** | `CatalogCache` | `openclaw_clawhub_search` / `_install` | ✅ Search + install UI | ✅ `tauri_commands::clawhub_search()` | ✅ End-to-end |
| 21 | **Routine run history** | `RoutineAuditLog` | `openclaw_routine_audit_list` | ✅ Tabular log with filters | ✅ `tauri_commands::routine_audit_list()` | ✅ End-to-end |
| 22 | **Gmail channel card** | `GmailConfig` | Gmail card + PKCE flow | 🔮 **Deferred** — needs OAuth PKCE browser flow | ⚠️ Config only | 🔮 Sprint 14 |
| 23 | **Extension health badges** | `ExtensionHealthMonitor` | Via Channel Status panel | ✅ State badges | ✅ Via channel status | ✅ End-to-end |
| 24 | **Session export format picker** | `SessionExporter` | `openclaw_export_session` live | ✅ `exportSession(key, format)` | ✅ md/json/txt/csv/html | ✅ End-to-end |
| 25 | **LLM routing rule builder** | `RoutingPolicy` | (TBD) | 🔮 **Deferred** | ✅ Toggle API exists | 🔮 Sprint 14 |
| 26 | **Plugin lifecycle log tab** | `AuditLogHook` | `openclaw_plugin_lifecycle_list` | ✅ Timeline tab, color-coded | ✅ `tauri_commands::plugin_lifecycle_list()` | ✅ End-to-end |
| 27 | **Manifest validation feedback** | `ManifestValidator` | `openclaw_manifest_validate` | ✅ Validate button, inline errors | ✅ `tauri_commands::manifest_validate()` | ✅ End-to-end |
| 28 | **Response cache stats** | `CachedResponseStore` | `openclaw_cache_stats` | ✅ Hits/misses/rate/size cards | ✅ `tauri_commands::cache_stats()` | ✅ End-to-end |

**Tier 4 Score:** ✅ 10 end-to-end | 🔮 2 deferred (Gmail PKCE, routing builder)

### Tier 5 — Sprint 15 / Architecture

| # | Action | Notes |
|---|--------|-------|
| 25 | **Full LLM routing rule builder** | Advanced rule editor UI — Sprint 13 ships toggle only |
| 29 | **Tauri V2 Channel migration** | Replace GatewayChannel HTTP/SSE with native Tauri IPC for Scrappy (see §21) |
| — | **Session pruning UI** | Pruning config in settings (low priority) |

### Owner: Scrappy Agent + IronClaw

---

## §21 — Tauri V2 Channel Migration (Scrappy ↔ IronClaw Communication)

> **Status:** 🔮 Planned — Sprint 15
>
> **Problem:** Scrappy embeds IronClaw as a Rust library but communicates via a
> localhost HTTP server (`GatewayChannel` on port 3000). This is overkill for
> in-process communication and introduces security, port conflict, and performance concerns.
>
> **Solution:** Implement a `TauriChannel` that uses Tauri V2's native IPC
> (`invoke()` + `Channel<T>` streaming) instead of HTTP/SSE.

### 21.1 Communication Paradigm Comparison

| Aspect | **Current (HTTP/SSE)** | **Tauri Events** | **Tauri V2 Channels** ✅ |
|--------|------------------------|------------------|--------------------------|
| **Open port** | ⚠️ Yes — `127.0.0.1:3000` | ❌ None | ❌ None |
| **Port conflicts** | Possible | Impossible | Impossible |
| **Local attack surface** | Any local process can connect | Tauri-only | Tauri-only |
| **Overhead** | TCP + HTTP parsing + JSON | Tauri IPC (minimal) | Tauri IPC (minimal) |
| **Streaming** | ✅ SSE / WebSocket | ✅ `emit()` (broadcast) | ✅ Per-call `Channel<T>` |
| **Per-conversation isolation** | Via `thread_id` filtering | Manual filtering | ✅ Built-in (scoped channel) |
| **Works for external clients** | ✅ Browsers, curl | ❌ Tauri webview only | ❌ Tauri webview only |
| **Type safety** | JSON-over-HTTP | Serde payloads | ✅ Typed `Channel<ChatEvent>` |
| **Refactor effort** | Already done | ~2 days | ~3-4 days |
| **Recommended for Scrappy** | ❌ | ⚠️ Acceptable | ✅ **Best option** |

### 21.2 Current Architecture (to be replaced in Scrappy mode)

```text
┌──────── Scrappy.app ─────────┐
│  React Webview               │
│    │                         │
│    ├─ invoke("openclaw_*") ──┼──► Tauri rpc.rs → tauri_commands.rs (IronClaw)
│    │  (request-response)     │
│    │                         │
│    └─ fetch(localhost:3000) ──┼──► GatewayChannel (axum HTTP server)
│       EventSource(/events)   │    └─ SSE stream → agent loop
│       (streaming)            │
└──────────────────────────────┘
```

**Problems:**
1. Full axum HTTP server running inside same process (unnecessary TCP stack)
2. Open port `127.0.0.1:3000` — other local processes can connect
3. Two different communication patterns (invoke vs fetch)
4. Port conflicts with other services on 3000

### 21.3 Target Architecture (Tauri V2 Channels)

```text
┌──────── Scrappy.app ─────────┐
│  React Webview               │
│    │                         │
│    ├─ invoke("openclaw_*") ──┼──► Tauri rpc.rs → IronClaw (request-response)
│    │                         │
│    └─ invoke("openclaw_chat",┼──► TauriChannel (implements Channel trait)
│         { onEvent: channel })│    └─ channel.send(ChatEvent) → streaming
│       (streaming via Channel)│
└──────────────────────────────┘

No HTTP server. No open ports. No TCP. Pure IPC.
```

### 21.4 Implementation Tasks

| # | Task | Side | Effort | Depends On |
|---|------|------|--------|------------|
| **T1** | **Define `ChatEvent` enum** — Typed event variants: `StreamChunk`, `Thinking`, `ToolStarted`, `ToolCompleted`, `ToolResult`, `Status`, `ApprovalNeeded`, `Error`, `Done` | IronClaw | 0.5 day | — |
| **T2** | **Implement `TauriChannel`** — New struct implementing the `Channel` trait. Receives messages via an `mpsc::Sender` injected from the Tauri command, pushes responses through a Tauri `Channel<ChatEvent>` | IronClaw | 1-2 days | T1 |
| **T3** | **Create `openclaw_chat` Tauri command** — Accepts `message: String` + `on_event: Channel<ChatEvent>`. Injects the message into the `TauriChannel` and streams events back | IronClaw + Scrappy | 1 day | T2 |
| **T4** | **Conditional channel registration in `AppBuilder`** — When running inside Tauri: register `TauriChannel` instead of `GatewayChannel`. When standalone: keep `GatewayChannel` | IronClaw | 0.5 day | T2 |
| **T5** | **Migrate Scrappy React chat to `invoke()`** — Replace `fetch(localhost:3000)` / `EventSource` with `invoke("openclaw_chat", { onEvent })` pattern | Scrappy | 1-2 days | T3 |
| **T6** | **Migrate SSE listeners to Tauri events** — Channel status, tool progress, job events — use `emit()` / `listen()` instead of SSE | Both | 1 day | T4 |
| **T7** | **Remove GatewayChannel from Scrappy startup** — Feature-gate `GatewayChannel` creation behind `#[cfg(not(feature = "tauri"))]` or runtime detection | IronClaw | 0.5 day | T5, T6 |
| **T8** | **End-to-end testing** — Verify chat streaming, tool approvals, job events, session management all work through pure Tauri IPC | Both | 1 day | T7 |

**Total estimated effort:** 6-8 days (split across IronClaw + Scrappy)

### 21.5 Key Implementation Details

#### `TauriChannel` (IronClaw side — `src/channels/tauri_channel.rs`)

```rust
// Pseudocode — actual impl will use Tauri v2 Channel<T>
pub struct TauriChannel {
    /// Receives messages injected by the Tauri command handler.
    msg_rx: Arc<Mutex<Option<mpsc::Receiver<IncomingMessage>>>>,
    /// Pushes events to the frontend via Tauri Channel<ChatEvent>.
    event_sender: Arc<RwLock<Option<TauriChannelSender<ChatEvent>>>>,
}

#[async_trait]
impl Channel for TauriChannel {
    fn name(&self) -> &str { "tauri" }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Return stream from msg_rx — messages injected via invoke()
    }

    async fn respond(&self, msg: &IncomingMessage, response: OutgoingResponse) -> Result<(), ChannelError> {
        // Push ChatEvent::Response through the Tauri channel
        self.event_sender.read().await
            .as_ref()
            .map(|s| s.send(ChatEvent::Response { content: response.content }));
        Ok(())
    }

    async fn send_status(&self, status: StatusUpdate, _metadata: &serde_json::Value) -> Result<(), ChannelError> {
        // Map StatusUpdate → ChatEvent variant and send via Tauri channel
    }
}
```

#### `openclaw_chat` Tauri command (Scrappy side — `rpc.rs`)

```rust
#[tauri::command]
async fn openclaw_chat(
    message: String,
    thread_id: Option<String>,
    on_event: tauri::ipc::Channel<ChatEvent>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Inject message into TauriChannel
    state.tauri_channel.inject_message(IncomingMessage::new("tauri", "user", &message)).await;

    // Set the event sender for streaming responses
    state.tauri_channel.set_event_sender(on_event).await;

    // The agent loop processes asynchronously — events stream via on_event
    Ok(())
}
```

#### Frontend (Scrappy React — `ChatInput.tsx`)

```typescript
import { invoke, Channel } from "@tauri-apps/api/core";

async function sendMessage(text: string, threadId?: string) {
  const onEvent = new Channel<ChatEvent>();

  onEvent.onmessage = (event) => {
    switch (event.type) {
      case "stream_chunk":  appendToken(event.content); break;
      case "thinking":      showThinking(event.message); break;
      case "tool_started":  showToolCard(event.name); break;
      case "tool_completed": updateToolCard(event.name, event.success); break;
      case "response":      setFinalResponse(event.content); break;
      case "error":         showError(event.message); break;
    }
  };

  await invoke("openclaw_chat", { message: text, threadId, onEvent });
}
```

### 21.6 Migration Strategy

1. **Phase 1** (non-breaking): Implement `TauriChannel` alongside `GatewayChannel`. Both register. Frontend uses `TauriChannel` for chat, `GatewayChannel` remains available for browser/debug access.
2. **Phase 2**: Migrate all SSE listeners to Tauri events.
3. **Phase 3**: Feature-gate `GatewayChannel` in Tauri builds. It only starts in standalone mode.
4. **Standalone mode preserved**: `ironclaw run` (CLI binary) still uses `GatewayChannel` for browser access. Only Scrappy switches to `TauriChannel`.

## Implementation Priorities (IronClaw)

> **Last updated:** 2026-03-05 05:16 CET — Sprint 14 complete. Next: Tauri V2 Channel migration (§21) + Gmail PKCE.
>
> All open IronClaw work aggregated from project artifacts into a single prioritized list.
> For Scrappy-specific priorities, see §20 above. For TauriChannel architecture, see §21.
> Scrappy = macOS desktop app (Tauri/Rust). May later add a native Swift macOS app separately.

### P0 - Core (Complete ✅)
- ✅ TUI channel with approval overlays
- ✅ HTTP webhook channel
- ✅ DM pairing (ironclaw pairing list/approve, host APIs)
- ✅ WASM tool sandbox
- ✅ Workspace/memory with hybrid search + embeddings batching
- ✅ Prompt injection defense
- ✅ Heartbeat system
- ✅ Session management
- ✅ Context compaction
- ✅ Model selection
- ✅ Gateway control plane + WebSocket
- ✅ Web Control UI (chat, memory, jobs, logs, extensions, routines)
- ✅ WebChat channel (web gateway)
- ✅ Slack channel (WASM tool)
- ✅ Telegram channel (WASM tool, MTProto)
- ✅ Docker sandbox (orchestrator/worker)
- ✅ Cron job scheduling (routines)
- ✅ CLI subcommands (onboard, config, status, memory)
- ✅ Gateway token auth
- ✅ Skills system (prompt-based with trust gating, attenuation, activation criteria)
- ✅ Session file permissions (0o600)
- ✅ Memory CLI commands (search, read, write, tree, status)
- ✅ Shell env scrubbing + command injection detection
- ✅ Tinfoil private inference provider
- ✅ OpenAI-compatible / OpenRouter provider support
- ✅ Extended thinking (ThinkingConfig, reasoning extraction, SSE/WS/OpenAI-compat exposure)

### P1 - High Priority (Complete ✅)
- ✅ Telegram channel (WASM, DM pairing, caption, /start)
- ✅ Multi-provider failover (`FailoverProvider` with retryable error classification)
- ✅ Hooks system (core lifecycle hooks + bundled/plugin/workspace hooks + outbound webhooks + `llm_input`/`llm_output` hooks)

### P2 - Medium Priority (Mostly Complete)
- ✅ Media handling (images, PDFs, audio — `src/media/`, wired into agent loop)
- ✅ Configurable image resize dimensions (`with_max_dimensions()`, OpenAI `detail` level hints)
- ✅ Multiple images per tool call (`format_multiple_for_llm()`)
- ✅ Skills routing blocks (`use_when`/`dont_use_when` in ActivationCriteria)
- ✅ `before_agent_start` / `before_message_write` hooks
- ✅ Ollama/local model support (via rig::providers::ollama)
- ✅ Configuration hot-reload (`ConfigWatcher`)
- ✅ Channel health monitor with auto-restart (`ChannelHealthMonitor`)
- ✅ Partial output preservation on abort
- ✅ Webhook trigger endpoint (`POST /hooks/routine/{id}`)
- ✅ Real token-level streaming (`StreamChunk` + `complete_stream()`)
- ✅ Wire temporal decay / MMR / query expansion into `hybrid_search()` call sites
- ✅ CLI: `gateway start/stop/status`
- ✅ CLI: `channels list/info` (env detection + WASM plugin scan)
- ✅ CLI: `cron edit` — add model + thinking-budget fields
- ✅ CLI: `message send` to channels
- ✅ Fix all clippy warnings (15 → 0, verified zero-warning build)
- ✅ Agent event broadcast — SSE broadcast manager, routine engine emits `Status`/`JobStarted`/`JobResult` lifecycle events
- ✅ Trusted-proxy auth — `TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS` for reverse-proxy deployments
- ✅ Safe bins allowlist — `IRONCLAW_SAFE_BINS_ONLY` mode with extensible `IRONCLAW_EXTRA_BINS`
- ✅ LD*/DYLD* environment variable validation — blocks library injection attacks via `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES` etc.
- ✅ TAURI_INTEGRATION.md §9–§11 verified against code — comprehensive and accurate
- ✅ Allowlist/blocklist — allow_from + block_from + pairing store (blocklist precedence), CLI `pairing block/unblock/blocked`
- ✅ OAuth flows — full Auth Code + PKCE browser flow ([`src/cli/tool.rs`](src/cli/tool.rs): `auth_tool_oauth()`, 192 LOC), auto-refresh (`OAuthRefreshConfig`), scope aggregation across tools, built-in Google/GitHub/Notion credentials ([`src/cli/oauth_defaults.rs`](src/cli/oauth_defaults.rs), 571 LOC), token storage with expiry

### P3 - Lower Priority
**Channels**
- ✅ TTS/audio features (`TtsSynthesizer`)
- ✅ Video support (`VideoAnalyzer` — ffprobe metadata + ffmpeg keyframe/audio extraction; [`src/media/video.rs`](src/media/video.rs))
- ✅ Plugin registry (`PluginManifest`)
- ✅ Memory enhancements (temporal decay, MMR, query expansion)
- ✅ Control UI i18n (EN/ES/ZH/JA)
- ✅ Stuck loop detection
- ✅ Discord channel — native Rust Gateway WS + REST (`channels/discord.rs`) + WASM webhook (`channels-src/discord/`)
- ❌ Matrix channel — E2EE stub
- ✅ Telegram: forum topic creation + `message_thread_id` threading
- ✅ Telegram: `channel_post` support + `sender_chat` identification
- ✅ Streaming draft replies — End-to-end: `respond_with_tools_streaming` in Reasoning, agent loop integration in dispatcher, DraftReplyState send-then-edit, Discord + Telegram `send_draft`
- ✅ Per-channel stream mode config — `DISCORD_STREAM_MODE` + `TELEGRAM_STREAM_MODE` env vars; `ChannelManager::stream_mode()` + `send_draft()` delegation
- ✅ Telegram: user message reactions — `TgMessageReaction` parsing with emoji/custom emoji support, surfaced in `IncomingMessage` metadata
- ✅ Telegram: sendPoll — `send_poll()` helper with question, options, anonymous/multiple-answer flags; wired to `Channel::poll()` trait
- ✅ Telegram: set_message_reaction — `set_message_reaction()` helper; wired to `Channel::react()` trait
- ✅ Multi-agent routing — `AgentRouter` with workspace isolation, priority-based routing (thread ownership → @mention → keyword → channel binding → default); wired into dispatch pipeline in `agent_loop.rs`
- ✅ Thread ownership — `SessionManager::set_thread_owner` + `AgentRouter::claim_thread` (first-responder wins, transfer, release, pruning)
- ✅ Cron/heartbeat topic targeting — `HEARTBEAT_NOTIFY_TOPIC_ID` config + `message_thread_id` injection in broadcast metadata for Telegram forum topics

**CLI**
- ✅ CLI: `agents` multi-agent management — `list`, `add`, `remove`, `show`, `set-default` subcommands
- ✅ CLI: `sessions` listing — `list`, `show`, `prune` subcommands with thread ownership display
- ✅ CLI: `nodes` device management — CRUD + formatted display ([`src/cli/nodes.rs`](src/cli/nodes.rs))
- ✅ CLI: `/subagents spawn` from chat — command parsing + tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs))
- ✅ CLI: `logs` query/filter — tail, search, show, levels subcommands with time-range/level/target filtering ([`src/cli/logs.rs`](src/cli/logs.rs))
- ✅ CLI: `update` self-update — check/install/rollback with stable/beta/nightly channels + binary backup ([`src/cli/update.rs`](src/cli/update.rs))
- ✅ CLI: `browser` automation — headless Chrome open/screenshot/links/check with DOM extraction ([`src/cli/browser.rs`](src/cli/browser.rs))
- ✅ CLI: `sessions export` — markdown/JSON transcript export with role labels and timestamps ([`src/cli/sessions.rs`](src/cli/sessions.rs))
- ✅ CLI: `models` — list/info/test with built-in model knowledge + Ollama auto-discovery ([`src/cli/models.rs`](src/cli/models.rs))

**LLM & Inference**
- ✅ Gemini embeddings — `EmbeddingConfig::gemini()` ([`src/llm/embeddings.rs`](src/llm/embeddings.rs))
- ✅ Local embeddings (on-device) — `EmbeddingConfig::local()` + Ollama support ([`src/llm/embeddings.rs`](src/llm/embeddings.rs))
- ✅ AWS Bedrock provider — OpenAI-to-Bedrock Converse API adapter ([`src/llm/bedrock.rs`](src/llm/bedrock.rs))
- ✅ Google Gemini provider — AI Studio adapter with system instruction + generation config ([`src/llm/gemini.rs`](src/llm/gemini.rs))
- ✅ Anthropic 1M context beta header — `ExtendedContextConfig` ([`src/llm/extended_context.rs`](src/llm/extended_context.rs))
- ✅ Auto model discovery from endpoints ([`src/llm/discovery.rs`](src/llm/discovery.rs): OpenAI/Anthropic/Ollama endpoint scanning with auto-discover)
- ✅ `llama.cpp` native interface — `LlamaModel` trait, `LlamaConfig` (GPU layers, sampling, context length), `ModelLoadStatus`, `LlamaCppStub` fallback ([`src/llm/llama_cpp.rs`](src/llm/llama_cpp.rs))
- ✅ Provider presets — NVIDIA, Perplexity, MiniMax, GLM-5 pre-configured for OpenAI-compatible endpoint ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs))

**Memory & Search**
- ✅ Citation support in search results ([`src/workspace/search.rs`](src/workspace/search.rs): `Citation` struct with title/path/URL/page/line_range + `format_citations()` inline rendering)
- ✅ Post-compaction context injection — `ContextInjector` with priority-based fragment assembly ([`src/context/post_compaction.rs`](src/context/post_compaction.rs))
- ✅ Skill path compaction — `RuntimeBehavior::compact_path()` with `~` prefix ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs))
- ✅ Media caching layer — TTL-based SHA-256 keyed file cache with LRU eviction ([`src/media/cache.rs`](src/media/cache.rs))

**Hooks & Automation**
- ✅ `llm_input` / `llm_output` hooks ([`src/hooks/hook.rs`](src/hooks/hook.rs): `BeforeLlmInput` + `AfterLlmOutput` hook points with model/tokens/content inspection)
- ✅ `transcribeAudio` hook — `BeforeTranscribeAudio` hook point with audio size/MIME/duration metadata ([`src/hooks/hook.rs`](src/hooks/hook.rs))
- ✅ Cron stagger controls — `StaggerConfig` with random jitter + `CronGate` atomic concurrency limiter ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs))
- ✅ Cron finished-run webhook — `FinishedRunPayload` + `notify_finished_run()` POST to `CRON_FINISHED_WEBHOOK` ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs))

**Security**
- ✅ Trusted-proxy auth (`TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS`)
- ✅ Safe bins allowlist (`IRONCLAW_SAFE_BINS_ONLY` mode)
- ✅ LD*/DYLD* env validation (library injection blocking)
- ✅ Per-group tool policies ([`src/tools/policy.rs`](src/tools/policy.rs): AllowAll/AllowList/DenyList with group→channel→global evaluation, serializable config)
- ✅ Elevated execution mode — `ElevatedMode` with timeout + command allowlist ([`src/safety/elevated.rs`](src/safety/elevated.rs))
- ✅ Skill download path restriction — `SkillPathConfig` with path traversal prevention, symlink detection, name sanitization ([`src/safety/skill_path.rs`](src/safety/skill_path.rs))
- ✅ Dangerous tool re-enable warning — `DangerousToolTracker` with state history and warning generation ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs))

**Media**
- ✅ Video keyframe extraction — `VideoAnalyzer` ([`src/media/video.rs`](src/media/video.rs)): ffprobe metadata + ffmpeg keyframe/audio extraction with graceful fallback
- ✅ Media caching layer — `MediaCache` with SHA-256 keying, TTL expiry, LRU eviction ([`src/media/cache.rs`](src/media/cache.rs))
- ✅ Incremental TTS playback — `SentenceChunker` + `TtsChunk` SSE streaming with sentence-aligned chunking ([`src/media/tts_streaming.rs`](src/media/tts_streaming.rs))
- ✅ Telegram sticker-to-image conversion — WebP/TGS/WebM magic-byte detection + ffmpeg pipeline ([`src/media/sticker.rs`](src/media/sticker.rs))

**UI & Control**
- ✅ Canvas system (A2UI) — `CanvasTool` + `CanvasStore` + canvas gateway routes ([`src/channels/canvas_gateway.rs`](src/channels/canvas_gateway.rs))
- ✅ WebChat theme sync — `WebChatConfig` + `WebChatTheme` (Light/Dark/System), CSS class/variable generation ([`src/config/webchat.rs`](src/config/webchat.rs))
- ✅ Agent management API — `AgentManagementStore` with CRUD, session_count, last_active_at, find_by_status ([`src/agent/management_api.rs`](src/agent/management_api.rs))
- ✅ Config editing API — `Settings.set()/.get()/.list()/.reset()` with typed path-based access ([`src/settings.rs`](src/settings.rs))

**Plugin System**
- ✅ ClawHub registry — `ClawHubConfig` + `CatalogCache` ([`src/extensions/clawhub.rs`](src/extensions/clawhub.rs))
- ✅ HTTP path registration for plugins — `PluginRouter` ([`src/extensions/plugin_routes.rs`](src/extensions/plugin_routes.rs))
- ✅ Auth / Memory / Provider plugin types — trait interfaces ([`src/extensions/plugin_interfaces.rs`](src/extensions/plugin_interfaces.rs))

**Housekeeping**
- ✅ `Default` derives for TtsProvider, TtsOutputFormat (clippy-driven)
- ✅ JSON5 / YAML config support — ([`src/config/formats.rs`](src/config/formats.rs))

### P4 - Postponed
- ❌ Slack channel (native implementation — currently WASM tool)
- ❌ WhatsApp channel (Baileys Web, echo detection)
- ✅ iMessage channel — `IMessageChannel` (530 LOC, [`src/channels/imessage.rs`](src/channels/imessage.rs)) with chat.db polling + osascript sending, group chats, attachments, dedup, diagnostics, 23 tests
- ❌ Other messaging platforms (LINE, Feishu/Lark, Google Chat, MS Teams, Twitch)

### P5 - Scrappy (Tauri) Integration
> Scrappy is the macOS desktop app, built with Tauri v2 (Rust + React).
> These tasks require action in the Scrappy codebase, not IronClaw.
> For full Scrappy priorities see §20. For TauriChannel migration see §21.

**Wiring (updated 2026-03-02 — confirmed live by Scrappy Agent)**
- ✅ `ToolBridge` wiring — both lines uncommented, live
- ✅ `StatusUpdate::Thinking(text)` → `UiEvent::AssistantInternal` (surfaces reasoning with 🧠)
- ✅ `refresh_secrets()` hot-reload — uncommented, uses `(secrets_store, "local_user")` signature
- ✅ `BridgedTool` → `Tool` trait adapter — fully implemented ([`src/hardware_bridge.rs`](src/hardware_bridge.rs): 610 LOC, 7 tests) + auto-registration in `AppBuilder::init_tools()` ([`src/app.rs`](src/app.rs))
- ✅ Tauri commands facade — 8 commands wired via [`src/tauri_commands.rs`](src/tauri_commands.rs) (12 tests)

**macOS App Features (from Scrappy feature parity report, 2026-03-02)**
- ✅ Auto-updates — `tauri-plugin-updater` + `UpdateChecker.tsx`, signing keys, GitHub endpoint (was P1)
- ✅ Thinking toggle — native IronClaw `ThinkingConfig` with budget slider (was P1)
- ✅ Memory search — hybrid BM25+vector search surfaced in Memory panel (was P2)
- ✅ Session export — markdown transcript export to clipboard (was P2)
- ✅ Hooks management UI — `OpenClawHooks.tsx` panel with lifecycle hook details (was P2)
- ✅ Plugin manager UI — `OpenClawPlugins.tsx` panel with activate/remove actions (was P2)
- ✅ Push-to-talk — global `Cmd+Shift+V` hotkey → record → transcribe → insert into input (was P2)
- ✅ Voice wake — VAD-based energy detection + Sherpa-ONNX keyword spotting + recording overlay + auto-transcribe ([`src/voice_wake.rs`](src/voice_wake.rs))
- ✅ Canvas drag/resize — draggable + resizable + maximize floating panel (was P3)
- ✅ Debug event inspector — `OpenClawEventInspector.tsx` with live stream, filters, pause (was P3)
- ✅ Config editor — `OpenClawConfig.tsx` with inline edit, add, export + URL allowlist + compaction + failover (was P3)
- ✅ Doctor diagnostics — `OpenClawDoctor.tsx` with health bar and per-component checks (was P3)
- ✅ Tool policies — `OpenClawToolPolicies.tsx` with grouped tool listing and search (was P3)
- ✅ Animated tray icon — TrayState with active dot badge on Thinking/ToolStarted, 3s debounced reset (was P3)
- ✅ iMessage integration — `IMessageChannel` (chat.db polling + osascript, group chats, attachments, dedup, diagnostics)

### P6 - Tauri V2 Channel Migration 🔮 (Sprint 15)

> **Goal:** Replace the GatewayChannel HTTP/SSE localhost server with native Tauri V2
> `invoke()` + `Channel<ChatEvent>` streaming for all Scrappy ↔ IronClaw communication.
> See §21 for full architecture comparison, diagrams, and pseudocode.

| # | Task | Owner | Effort | Status |
|---|------|-------|--------|--------|
| **T1** | Define `ChatEvent` enum — typed variants: `StreamChunk`, `Thinking`, `ToolStarted`, `ToolCompleted`, `ToolResult`, `Status`, `ApprovalNeeded`, `Error`, `Done` | IronClaw | 0.5 day | 🔮 Planned |
| **T2** | Implement `TauriChannel` (`src/channels/tauri_channel.rs`) — new `Channel` trait impl using `mpsc::Sender` + `Channel<ChatEvent>` | IronClaw | 1-2 days | 🔮 Planned |
| **T3** | Create `openclaw_chat` Tauri command — accepts message + `on_event: Channel<ChatEvent>`, injects into `TauriChannel` | Both | 1 day | 🔮 Planned |
| **T4** | Conditional channel registration — `AppBuilder` uses `TauriChannel` in Tauri builds, `GatewayChannel` in standalone | IronClaw | 0.5 day | 🔮 Planned |
| **T5** | Migrate Scrappy React chat — replace `fetch(localhost:3000)` / `EventSource` with `invoke("openclaw_chat", { onEvent })` | Scrappy | 1-2 days | 🔮 Planned |
| **T6** | Migrate SSE listeners to Tauri events — channel status, tool progress, job events via `emit()`/`listen()` | Both | 1 day | 🔮 Planned |
| **T7** | Feature-gate GatewayChannel — `#[cfg(not(feature = "tauri"))]` or runtime detection; only starts in standalone mode | IronClaw | 0.5 day | 🔮 Planned |
| **T8** | End-to-end testing — chat streaming, tool approvals, job events, session management through pure Tauri IPC | Both | 1 day | 🔮 Planned |

**Total: 6-8 days** (split across IronClaw + Scrappy)

**Migration phases:**
1. **Phase 1** — Implement `TauriChannel` alongside `GatewayChannel` (both active, non-breaking)
2. **Phase 2** — Migrate all SSE listeners to Tauri events
3. **Phase 3** — Feature-gate `GatewayChannel` in Tauri builds (standalone mode preserved)

### Deferred (No Urgency)
- ✅ Sherpa-ONNX keyword spotting ([`src/voice_wake.rs`](src/voice_wake.rs): `detection_loop_sherpa()` — 3-thread pipeline with auto-fallback)
- 🔮 Skill cross-crate deps (no skills use Rust deps yet)
- 🔮 WS RPC for remote Hardware Bridge (desktop uses internal trait)
- 🔮 whisper-rs local inference (WhisperHttp covers desktop)
- 🔮 macOS dictation backend (scaffold in `talk_mode.rs`)
- 🔮 GatewayChannel deprecation in Tauri mode (post-P6 completion)


---

## How to Contribute

1. **Claim a section**: Edit this file and add your name/handle to the "Owner" field
2. **Create a tracking issue**: Link to GitHub issue for the feature area
3. **Update status**: Change ❌ to 🚧 when starting, ✅ when complete
4. **Add notes**: Document any design decisions or deviations

### Coordination

- Each major section should have one owner to avoid conflicts
- Owners can delegate sub-features to others
- Update this file as part of your PR

---

## Deviations from OpenClaw

IronClaw intentionally differs from OpenClaw in these ways:

1. **Rust vs TypeScript**: Native performance, memory safety, single binary distribution
2. **WASM sandbox vs Docker**: Lighter weight, faster startup, capability-based security
3. **PostgreSQL + libSQL vs SQLite**: Dual-backend (production PG + embedded libSQL for zero-dep local mode)
4. **Provider agnostic**: No vendor lock-in — works with any OpenAI-compatible endpoint (default), Anthropic, OpenAI, Ollama, Tinfoil, or OpenRouter
5. **No mobile/desktop apps**: Focus on server-side and CLI; Scrappy (Tauri/Rust) provides the macOS desktop experience. A native Swift macOS app may be added later.
6. **WASM channels**: Novel extension mechanism not in OpenClaw
7. **Tinfoil private inference**: IronClaw-only provider for private/encrypted inference
8. **GitHub WASM tool**: Native GitHub integration as WASM tool
9. **Prompt-based skills**: Different approach than OpenClaw capability bundles (trust gating, attenuation)
10. **OS Keychain secrets**: API keys stored in macOS Keychain / Linux Secret Service rather than encrypted file

These are intentional architectural choices, not gaps to be filled.

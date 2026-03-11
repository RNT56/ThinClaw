# IronClaw ↔ OpenClaw Feature Parity Matrix

> **Last reconciled:** 2026-03-07 18:15 CET

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
| Gmail | ✅ | ✅ | - | `GmailChannel` (700+ LOC) — Pub/Sub pull + Gmail API read/reply + sender allowlist ([`src/channels/gmail.rs`](src/channels/gmail.rs)) |
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
| Subagent system | ✅ | ✅ | Full `SubagentExecutor` ([`src/agent/subagent_executor.rs`](src/agent/subagent_executor.rs)): in-process agentic loops with isolated context, filtered tools, configurable timeouts, cancellation via watch channels |
| `spawn_subagent` tool | ✅ | ✅ | Declarative tool → dispatcher interception → `SubagentExecutor::spawn()`. Sync (wait=true) and async (wait=false) modes ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `list_subagents` tool | ✅ | ✅ | Query active/recent sub-agents with ID, status, task, timing info ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `cancel_subagent` tool | ✅ | ✅ | Cancel running sub-agents by UUID; watch channel + task abort ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| Sub-agent lifecycle | ✅ | ✅ | Concurrency limits (default 5), per-agent timeout, status tracking (Running/Completed/Failed/TimedOut/Cancelled), user progress notifications via StatusUpdate::AgentMessage |
| `/subagents spawn` command | ✅ | ✅ | Command parsing + tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs)) |
| Workspace sandbox modes | ❌ | ✅ | `WORKSPACE_MODE` config: `unrestricted` (default), `sandboxed` (confined to `WORKSPACE_ROOT`), `project` (working dir set, files unrestricted). Drives tool registration + dynamic system prompt ([`src/config/agent.rs`](src/config/agent.rs)) |
| Dynamic system prompt | ❌ | ✅ | `Reasoning::build_workspace_capabilities_section()` generates mode-specific Desktop Capabilities based on active workspace config and available tools ([`src/llm/reasoning.rs`](src/llm/reasoning.rs)) |
| Screen capture tool | ❌ | ✅ | `ScreenCaptureTool` — macOS screencapture/Linux gnome-screenshot/scrot; registered when `ALLOW_LOCAL_TOOLS=true` + `SCREEN_CAPTURE_ENABLED=true` (user opt-in via Scrappy toggle). Dynamic system prompt auto-detects and injects guidance ([`src/tools/builtin/screen_capture.rs`](src/tools/builtin/screen_capture.rs)) |
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
| Workspace sandbox modes | ❌ | ✅ | Done | 3 modes: unrestricted (Cursor-style), sandboxed (confined dir), project (working dir). User selects in Settings. Sets `WORKSPACE_MODE` + `WORKSPACE_ROOT` env vars |
| OS governance toggles | ❌ | ✅ | Done | Live permission checks: `ACCESSIBILITY_GRANTED` (AXIsProcessTrusted), `SCREEN_RECORDING_GRANTED` (CGPreflightScreenCaptureAccess). Screen capture gated on user toggle (`SCREEN_CAPTURE_ENABLED`) |
| Screen capture (desktop) | ❌ | ✅ | Done | `ScreenCaptureTool` registered when `ALLOW_LOCAL_TOOLS=true` + `SCREEN_CAPTURE_ENABLED=true`. Agent prompt auto-detects and provides usage guidance |

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
| Workspace sandbox modes | ❌ | ✅ | `WORKSPACE_MODE` (unrestricted/sandboxed/project) + `WORKSPACE_ROOT` — file tools + shell confined to workspace when sandboxed ([`src/config/agent.rs`](src/config/agent.rs), [`src/tools/registry.rs`](src/tools/registry.rs)) |
| Shell sandbox (3-layer) | ❌ | ✅ | When `base_dir` set: (1) Safe bins allowlist auto-enforced, (2) workdir validation (must be under base_dir), (3) command path scanning (`detect_path_escape` + `..` traversal detection). 11 dedicated tests ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs)) |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ✅ | ([`src/safety/elevated.rs`](src/safety/elevated.rs)) |
| Safe bins allowlist | ✅ | ✅ | `IRONCLAW_SAFE_BINS_ONLY` + extensible `IRONCLAW_EXTRA_BINS` + desktop bins (open, xdg-open, pbcopy, pbpaste, tee, xargs, chmod) |
| LD*/DYLD* validation | ✅ | ✅ | Blocks `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, etc. |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) + `..` traversal in shell commands |
| Credential theft via env injection | ✅ | ✅ | Shell env scrubbing + command injection detection + LD*/DYLD* blocking + safe bins |
| Session file permissions (0o600) | ✅ | ✅ | Handled by OS keychain + filesystem perms |
| Skill download path restriction | ✅ | ✅ | ([`src/safety/skill_path.rs`](src/safety/skill_path.rs)) |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ✅ | SSRF prevention: blocks private IPs, cloud metadata, disallowed schemes ([`src/safety/media_url.rs`](src/safety/media_url.rs)) |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ✅ | `DangerousToolTracker` ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs)) |
| OS governance env vars | ❌ | ✅ | Scrappy live-checks macOS permissions and passes to IronClaw: `ACCESSIBILITY_GRANTED` (AXIsProcessTrusted), `SCREEN_RECORDING_GRANTED` (CGPreflightScreenCaptureAccess), `SCREEN_CAPTURE_ENABLED` (user toggle) |

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

> **Last updated:** 2026-03-07 14:23 CET — reconciled with workspace sandbox modes, shell sandboxing, screen capture, OS governance
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
| Workspace sandbox modes (§5, §15) | Settings panel — unrestricted/sandboxed/project | ✅ Sets `WORKSPACE_MODE` + `WORKSPACE_ROOT` env vars |
| OS governance (§15) | Settings panel — screen capture toggle, permission checks | ✅ `ACCESSIBILITY_GRANTED`, `SCREEN_RECORDING_GRANTED`, `SCREEN_CAPTURE_ENABLED` |
| Screen capture (§5) | Gated on `SCREEN_CAPTURE_ENABLED` toggle | ✅ Wired via OS governance toggle |
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

> **Sprint 15: All features fully wired** via unified `tauri_commands` facade module.
> Scrappy UI exists for 12/12 features. Gmail + routing wired end-to-end in Sprint 15.

| IronClaw Feature | API / Module | Scrappy UI | IronClaw Wiring | Status |
|-----------------|-------------|------------|-----------------|--------|
| **Channel status view** | `ChannelStatusView` ([`src/channels/status_view.rs`](src/channels/status_view.rs)) | ✅ Cards with state badges, counters, SSE listener | ✅ `openclaw_channel_status_list` live | ✅ End-to-end |
| **LLM cost tracker** | `CostTracker` ([`src/llm/cost_tracker.rs`](src/llm/cost_tracker.rs)) | ✅ Full dashboard: daily/monthly/model/agent bars, CSV export, alerts | ✅ `tauri_commands::cost_summary()` + `cost_export_csv()` | ✅ End-to-end |
| **LLM routing policy** | `RoutingPolicy` ([`src/llm/routing_policy.rs`](src/llm/routing_policy.rs)) | ✅ Full rule builder — `OpenClawRouting.tsx` (585 LOC) | ✅ 6 CRUD commands via `tauri_commands` | ✅ End-to-end |
| **ClawHub registry** | `CatalogCache` ([`src/extensions/clawhub.rs`](src/extensions/clawhub.rs)) | ✅ Search + install UI tab in Plugins page | ✅ `tauri_commands::clawhub_search()` + `clawhub_prepare_install()` | ✅ End-to-end |
| **Extension health monitor** | `ExtensionHealthMonitor` ([`src/extensions/ext_health_monitor.rs`](src/extensions/ext_health_monitor.rs)) | ✅ State badges (Running/Connecting/Degraded/Error) via Channel Status panel | ✅ Via `openclaw_channel_status_list` | ✅ End-to-end |
| **Routine audit log** | `Database::list_routine_runs()` ([`src/db/mod.rs`](src/db/mod.rs)) | ✅ Tabular log with outcome badges, filter, routine selector | ✅ `tauri_commands::routine_audit_list()` queries DB | ✅ End-to-end |
| **Multi-format session export** | `SessionExporter` ([`src/cli/session_export.rs`](src/cli/session_export.rs)) | ✅ `exportSession(key, format)` with backward compat | ✅ `openclaw_export_session` live (md/json/txt/csv/html) | ✅ End-to-end |
| **Agent management store** | `AgentManagementStore` ([`src/agent/management_api.rs`](src/agent/management_api.rs)) | ✅ Multi-agent picker + API wrapper + extended `AgentProfile` type | ✅ `openclaw_agents_set_default` live (writes to config) | ✅ End-to-end |
| **Gmail channel** | `GmailChannel` ([`src/channels/gmail.rs`](src/channels/gmail.rs), 700+ LOC) | ✅ Gmail card with real status + automated PKCE via `startGmailOAuth()` | ✅ `openclaw_gmail_status` + `openclaw_gmail_oauth_start` | ✅ End-to-end |
| **Plugin manifest validator** | `ManifestValidator` ([`src/extensions/manifest_validator.rs`](src/extensions/manifest_validator.rs)) | ✅ Per-extension validate button with inline error/warning display | ✅ `tauri_commands::manifest_validate()` | ✅ End-to-end |
| **Plugin lifecycle hooks** | `LifecycleHookRegistry` ([`src/extensions/lifecycle_hooks.rs`](src/extensions/lifecycle_hooks.rs)) | ✅ Timeline tab in Plugins page with color-coded events | ✅ `tauri_commands::plugin_lifecycle_list()` | ✅ End-to-end |
| **Response cache metrics** | `CachedResponseStore` ([`src/llm/response_cache_ext.rs`](src/llm/response_cache_ext.rs)) | ✅ Hits/misses/rate/size cards + efficiency bar | ✅ `tauri_commands::cache_stats()` | ✅ Shared instance via `AppComponents.response_cache` → `AgentDeps` → `Reasoning::with_response_cache()` |

### 17.5 IronClaw Stub Wiring — ✅ ALL WIRED (18 commands)

> **All 18 Tauri commands wired** via unified `tauri_commands` facade ([`src/tauri_commands.rs`](src/tauri_commands.rs)).
> 8 Sprint 14 + 7 Sprint 15 (routing CRUD + Gmail status + Gmail OAuth PKCE) + 3 Canvas.

| # | Command | Wired To | Sprint | Status |
|---|---------|----------|--------|--------|
| 1 | `openclaw_cost_summary` | `tauri_commands::cost_summary()` → `CostTracker::summary()` | 14 | ✅ |
| 2 | `openclaw_cost_export_csv` | `tauri_commands::cost_export_csv()` → `CostTracker::export_csv()` | 14 | ✅ |
| 3 | `openclaw_clawhub_search` | `tauri_commands::clawhub_search()` → `CatalogCache::search(query)` | 14 | ✅ |
| 4 | `openclaw_clawhub_install` | `tauri_commands::clawhub_prepare_install()` → cache lookup + path resolution | 14 | ✅ |
| 5 | `openclaw_routine_audit_list` | `tauri_commands::routine_audit_list()` → `Database::get_routine_by_name()` + `list_routine_runs()` | 14 | ✅ |
| 6 | `openclaw_cache_stats` | `tauri_commands::cache_stats()` → `CachedResponseStore::stats()` | 14 | ✅ |
| 7 | `openclaw_plugin_lifecycle_list` | `tauri_commands::plugin_lifecycle_list()` → `AuditLogHook::events_serialized()` | 14 | ✅ |
| 8 | `openclaw_manifest_validate` | `tauri_commands::manifest_validate()` → `ManifestValidator::validate()` | 14 | ✅ |
| 9 | `openclaw_routing_rules_list` | `tauri_commands::routing_rules_list()` → `RoutingPolicy::rules()` | 15 | ✅ |
| 10 | `openclaw_routing_rules_add` | `tauri_commands::routing_rules_add()` → validation + insert | 15 | ✅ |
| 11 | `openclaw_routing_rules_remove` | `tauri_commands::routing_rules_remove()` → bounds-checked removal | 15 | ✅ |
| 12 | `openclaw_routing_rules_reorder` | `tauri_commands::routing_rules_reorder()` → priority reorder | 15 | ✅ |
| 13 | `openclaw_routing_status` | `tauri_commands::routing_status()` → enabled + rule count + default | 15 | ✅ |
| 14 | `openclaw_gmail_status` | `tauri_commands::gmail_status()` → `GmailConfig` summary | 15 | ✅ |
| 15 | `openclaw_gmail_oauth_start` | `tauri_commands::gmail_oauth_start()` → full PKCE flow | 15 | ✅ |
| 16 | `openclaw_canvas_panels_list` | `tauri_commands::canvas_panels_list()` → `CanvasStore::list()` | 15+ | ✅ |
| 17 | `openclaw_canvas_panel_get` | `tauri_commands::canvas_panel_get()` → `CanvasStore::get()` | 15+ | ✅ |
| 18 | `openclaw_canvas_panel_dismiss` | `tauri_commands::canvas_panel_dismiss()` → `CanvasStore::dismiss()` | 15+ | ✅ |

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
| `openclaw_routine_audit_list` | `routine.rs` + `db/mod.rs` | `routine_name: String, user_id: String, limit: Option<i64>` | `Vec<RoutineRun>` (from DB) | Default limit=20; queries DB via `get_routine_by_name()` + `list_routine_runs()` |
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

#### Gmail OAuth Flow — ✅ Fully Automated PKCE

Gmail uses a single automated IronClaw command for the full PKCE flow:

1. Frontend calls `startGmailOAuth()` → invokes `openclaw_gmail_oauth_start`
2. IronClaw opens browser → user authenticates with Google
3. Google redirects to localhost callback
4. IronClaw exchanges code for tokens, persists refresh token in settings store
5. Returns `GmailOAuthResult` with success/error + token info

**✅ Completed:** `oauth_defaults.rs` has `GmailOAuthConfig` with Google OAuth client credentials. Scrappy wired `handleGmailConnect()` in `OpenClawChannels.tsx` to call `startGmailOAuth()` directly — removed old 3-step manual flow.

#### ClawHub Architecture Decision

**Proxy through IronClaw gateway** (not direct frontend → ClawHub):
- `CLAWHUB_API_KEY` stays server-side (security)
- `CatalogCache` TTL benefits all consumers (caching)
- Consistent with all-via-Tauri-IPC pattern

#### Known Issues to Fix

1. ~~**`openclaw_cron_history` is a stub**~~ — ✅ **Fixed** — `routine_audit_list()` now queries `Database::list_routine_runs()` directly (migrated from orphaned in-memory `RoutineAuditLog`)
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

> **Last updated:** 2026-03-07 14:23 CET — Sprint 15 fully complete. Workspace sandbox modes + OS governance + screen capture added. Tauri commands: 18 total.

### 19.1 Shipped — Scrappy UI Needed or In Progress

| IronClaw Feature | Shipped | Scrappy Work Needed | Scrappy State |
|-----------------|---------|---------------------|---------------|
| **Multi-agent picker** | ✅ `AgentManagementStore` | Sidebar dropdown + full settings panel | ✅ End-to-end (Scrappy agent confirmed) |
| **Session pruning** | ✅ CLI `sessions prune` | Pruning config (max sessions, age cutoff) in settings | ✅ End-to-end (Scrappy agent confirmed) |
| **Channel status view** | ✅ `ChannelStatusView` | Full per-channel status panel | ✅ End-to-end (Scrappy agent confirmed) |
| **Gmail channel** | ✅ `GmailChannel` (700+ LOC) + `GmailChannelConfig` | Gmail card + automated PKCE via `startGmailOAuth()` | ✅ G3 complete — fully automated PKCE flow |
| **LLM cost tracker** | ✅ `CostTracker` | Cost dashboard via `openclaw_cost_summary` | ✅ End-to-end (Scrappy agent confirmed) |
| **LLM routing policy** | ✅ `RoutingPolicy` + CRUD API | Full rule builder UI | ✅ R2 complete — `OpenClawRouting.tsx` rewritten (585 LOC) |
| **ClawHub browser** | ✅ `CatalogCache` | Plugin discovery via `openclaw_clawhub_search/install` | ✅ End-to-end (Scrappy agent confirmed) |
| **Plugin lifecycle log** | ✅ `AuditLogHook` | Lifecycle event log tab | ✅ End-to-end (Scrappy agent confirmed) |
| **Extension health badges** | ✅ `ExtensionHealthMonitor` | Health badge on channel/plugin cards | ✅ End-to-end (Scrappy agent confirmed) |
| **Routine run history** | ✅ `Database::list_routine_runs()` | ✅ Queries DB via `tauri_commands::routine_audit_list()` | ✅ End-to-end (Scrappy agent confirmed) |
| **Session export formats** | ✅ `SessionExporter` (5 formats) | Format picker + save-to-file | ✅ End-to-end (Scrappy agent confirmed) |
| **Response cache stats** | ✅ `CachedResponseStore` | Cache stats indicator | ✅ End-to-end (Scrappy agent confirmed) |
| **Manifest validation feedback** | ✅ `ManifestValidator` | Error/warning badges | ✅ End-to-end (Scrappy agent confirmed) |

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
| Canvas system (A2UI) | `CanvasWindow.tsx` multi-panel manager (A2UI + legacy), `CanvasPanelRenderer.tsx` (11 component types), `CanvasProvider.tsx` (context), `CanvasToolbar.tsx` (badge + popover), action dispatch via `openclaw_canvas_dispatch_event` |
| Streaming draft replies | Per-channel stream mode selector in channel cards |
| Discord channel | `OpenClawChannels.tsx` — type badge, stream mode config |
| Cron lint | `OpenClawAutomations.tsx` — validator + next-5-fire-times |
| Auto model discovery | Model list auto-populates from endpoint scan |

### Owner: Both (IronClaw ships backend, Scrappy ships frontend)

---

## 20. Scrappy Priority Ordering

> **Last updated:** 2026-03-05 08:52 CET — updated after Sprint 15 + Scrappy agent verification

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

> **Sprint 15 complete.** All 12 Tier 4 items are end-to-end.
> `tauri_commands` facade ([`src/tauri_commands.rs`](src/tauri_commands.rs)) exposes 18 commands (8 Sprint 14 + 7 Sprint 15 + 3 Canvas).
> Gmail PKCE: Scrappy wired `startGmailOAuth()` to `openclaw_gmail_oauth_start` — fully automated browser flow.
> Scrappy backend also fixed upstream: `canvas_store: None` in `AgentDeps` + `CanvasAction` match arm → emits `CanvasUpdate` events.

| # | Action | Backend | Tauri Command | Scrappy UI | IronClaw Wiring | Status |
|---|--------|---------|---------------|------------|-----------------|--------|
| 17 | **Multi-agent picker in sidebar** | `AgentManagementStore` | `openclaw_agents_set_default` live | ✅ API wrapper + extended `AgentProfile` | ✅ Writes to config | ✅ End-to-end |
| 18 | **LLM cost dashboard** | `CostTracker` | `openclaw_cost_summary` / `_export_csv` | ✅ Full dashboard | ✅ `tauri_commands::cost_summary()` | ✅ End-to-end |
| 19 | **Channel status panel** | `ChannelStatusView` | `openclaw_channel_status_list` live | ✅ Cards with state badges, SSE | ✅ Reads config + env | ✅ End-to-end |
| 20 | **ClawHub plugin browser** | `CatalogCache` | `openclaw_clawhub_search` / `_install` | ✅ Search + install UI | ✅ `tauri_commands::clawhub_search()` | ✅ End-to-end |
| 21 | **Routine run history** | `Database::list_routine_runs()` | `openclaw_routine_audit_list` | ✅ Tabular log with filters | ✅ `tauri_commands::routine_audit_list()` queries DB | ✅ End-to-end |
| 22 | **Gmail channel card** | `GmailChannel` (700+ LOC) | `openclaw_gmail_status` + `openclaw_gmail_oauth_start` | ✅ Gmail card + `startGmailOAuth()` (automated PKCE) | ✅ Full channel + PKCE endpoint | ✅ End-to-end |
| 23 | **Extension health badges** | `ExtensionHealthMonitor` | Via Channel Status panel | ✅ State badges | ✅ Via channel status | ✅ End-to-end |
| 24 | **Session export format picker** | `SessionExporter` | `openclaw_export_session` live | ✅ `exportSession(key, format)` | ✅ md/json/txt/csv/html | ✅ End-to-end |
| 25 | **LLM routing rule builder** | `RoutingPolicy` | `openclaw_routing_rules_*` (6 commands) | ✅ R2 complete — `OpenClawRouting.tsx` (585 LOC) | ✅ CRUD API | ✅ End-to-end |
| 26 | **Plugin lifecycle log tab** | `AuditLogHook` | `openclaw_plugin_lifecycle_list` | ✅ Timeline tab, color-coded | ✅ `tauri_commands::plugin_lifecycle_list()` | ✅ End-to-end |
| 27 | **Manifest validation feedback** | `ManifestValidator` | `openclaw_manifest_validate` | ✅ Validate button, inline errors | ✅ `tauri_commands::manifest_validate()` | ✅ End-to-end |
| 28 | **Response cache stats** | `CachedResponseStore` | `openclaw_cache_stats` | ✅ Hits/misses/rate/size cards | ✅ `tauri_commands::cache_stats()` | ✅ End-to-end |

**Tier 4 Score:** ✅ **12/12 end-to-end**

### Tier 5 — Recent Additions (2026-03-07)

| # | Action | Notes | Status |
|---|--------|-------|--------|
| ~~—~~ | ~~**Canvas / A2UI end-to-end wiring**~~ | ✅ **Done** — §22, all 9 tasks complete (5 IronClaw + 4 Scrappy) | ✅ |
| 29 | **Workspace sandbox modes** | 3 modes (unrestricted/sandboxed/project) in Settings. IronClaw: `WORKSPACE_MODE` + `WORKSPACE_ROOT`, dynamic system prompt, tool sandboxing. Scrappy: UI toggle → env vars | ✅ End-to-end |
| 30 | **Shell sandbox (3-layer)** | Safe bins + workdir validation + path escape + `..` traversal. Desktop bins added (open, pbcopy, tee, xargs, chmod). 11 new tests | ✅ IronClaw |
| 31 | **Screen capture tool** | `ScreenCaptureTool` gated on `SCREEN_CAPTURE_ENABLED` + `ALLOW_LOCAL_TOOLS`. Dynamic prompt auto-detects. Scrappy: OS governance toggle | ✅ End-to-end |
| 32 | **OS governance env vars** | Scrappy live-checks: `ACCESSIBILITY_GRANTED` (AXIsProcessTrusted), `SCREEN_RECORDING_GRANTED` (CGPreflightScreenCaptureAccess) | ✅ Scrappy |
| 33 | **Dynamic system prompt** | `Reasoning::build_workspace_capabilities_section()` — mode-specific guidance + auto-detect screen_capture | ✅ IronClaw |
| — | **Session pruning UI** | Pruning config in settings (low priority) | 🔮 |
| — | **Per-conversation channel scoping** | Optional optimization: replace `app.emit()` broadcast with Tauri V2 `Channel<T>` per-invoke | 🔮 |

### Owner: Scrappy Agent + IronClaw

---

## §21 — Scrappy ↔ IronClaw Communication Architecture

> **Status:** ✅ **Already implemented** — Verified by Scrappy Agent audit, 2026-03-05
>
> **Correction:** This section originally described a migration plan from HTTP/SSE
> to Tauri IPC. After the Scrappy agent audited the actual codebase, it was
> confirmed that the "target architecture" was **already the current architecture**.
> Scrappy has never used `GatewayChannel` for chat. This section is now updated
> to document the actual working architecture.

### 21.1 Actual Architecture (verified ✅)

```text
┌──────────────── Scrappy.app (single macOS process) ─────────────────────┐
│                                                                          │
│  ┌─── React Webview ───────┐         ┌─── Tauri Rust Backend ─────────┐ │
│  │                          │         │                                │ │
│  │  invoke("openclaw_*")   │─ IPC ──►│  rpc.rs                        │ │
│  │  (66+ commands via IPC)  │         │  ├─ ironclaw::api::chat::      │ │
│  │                          │         │  │   send_message()             │ │
│  │  listen("openclaw-event")│◄─ emit ─│  ├─ ironclaw::api::sessions::* │ │
│  │  (8 listener components) │         │  ├─ ironclaw::api::config::*   │ │
│  │                          │         │  └─ ironclaw::tauri_commands::* │ │
│  │                          │         │                                │ │
│  │                          │         │  ironclaw_channel.rs (272 LOC) │ │
│  │                          │         │  └─ implements Channel trait    │ │
│  │                          │         │     └─ StatusUpdate → UiEvent  │ │
│  │                          │         │        → app.emit()            │ │
│  └──────────────────────────┘         └────────────────────────────────┘ │
│                                                                          │
│  No HTTP server. No open ports. No TCP. Pure in-process Rust FFI + IPC.  │
└──────────────────────────────────────────────────────────────────────────┘
```

### 21.2 How It Works

| Layer | Mechanism | Details |
|-------|-----------|---------|
| **Chat (user → agent)** | `invoke("openclaw_send_message")` | → `rpc.rs` → `ironclaw::api::chat::send_message()` — direct in-process call |
| **Chat (agent → user)** | `app.emit("openclaw-event", UiEvent)` | `StatusUpdate` mapped to `UiEvent` variants; 8 React components listen |
| **Management commands** | `invoke("openclaw_*")` (66+ commands) | `ironclaw::tauri_commands::*` facade functions — all via Tauri IPC |
| **Tool approvals** | `invoke("openclaw_resolve_approval")` | → `ironclaw::api::chat::resolve_approval()` |
| **Session management** | `invoke("openclaw_*_session")` | → `ironclaw::api::sessions::*` |
| **Hardware access** | `ToolBridge` trait (in-process) | Camera, microphone, screen via `BridgedTool` |

The `GatewayChannel` (axum HTTP server) is **only** used in standalone mode
(`ironclaw run` from CLI) for browser access. It is **never started** when
running inside Scrappy.

### 21.3 What Was Already Built (Scrappy Side)

- `ironclaw_channel.rs` (272 LOC) — Implements IronClaw's `Channel` trait for Tauri
- `rpc.rs` — 66+ Tauri commands, all using `invoke()` IPC
- `openclaw-event` — Tauri event bus, 8 React listener components
- `status_to_ui_event()` — Converts `StatusUpdate` → `UiEvent` for emit
- Zero `EventSource`, zero `fetch(localhost:*)`, zero HTTP for chat

### 21.4 Remaining Optimization (P3, Optional)

> **Per-conversation channel scoping via Tauri V2 `Channel<T>`**
>
> Currently, all events broadcast to all 8 listener components via
> `app.emit("openclaw-event", ...)`, and each filters by `session_key`.
> A Tauri V2 `Channel<ChatEvent>` per-`invoke()` call would scope events
> per-conversation automatically, eliminating broadcast fan-out.
>
> **Effort:** ~2 days | **Priority:** Low (P3) — current broadcast works fine
> **Benefit:** Cleaner event isolation, slightly less filtering overhead

## Implementation Priorities (IronClaw)

> **Last updated:** 2026-03-05 08:52 CET — Sprint 15: IronClaw ✅ complete (G1+G2+R1+PKCE). Scrappy R2 ✅ complete. G3 PKCE wiring remaining.
>
> All open IronClaw work aggregated from project artifacts into a single prioritized list.
> For Scrappy-specific priorities, see §20 above. For Scrappy ↔ IronClaw architecture, see §21.
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
- ✅ Safe bins allowlist (`IRONCLAW_SAFE_BINS_ONLY` mode + desktop-essential bins: `open`, `xdg-open`, `pbcopy`, `pbpaste`, `tee`, `xargs`, `chmod`, `realpath`, `basename`, `dirname`)
- ✅ LD*/DYLD* env validation (library injection blocking)
- ✅ Per-group tool policies ([`src/tools/policy.rs`](src/tools/policy.rs): AllowAll/AllowList/DenyList with group→channel→global evaluation, serializable config)
- ✅ Elevated execution mode — `ElevatedMode` with timeout + command allowlist ([`src/safety/elevated.rs`](src/safety/elevated.rs))
- ✅ Skill download path restriction — `SkillPathConfig` with path traversal prevention, symlink detection, name sanitization ([`src/safety/skill_path.rs`](src/safety/skill_path.rs))
- ✅ Dangerous tool re-enable warning — `DangerousToolTracker` with state history and warning generation ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs))
- ✅ Workspace sandbox modes — `WORKSPACE_MODE` (unrestricted/sandboxed/project) with `WORKSPACE_ROOT`; drives `register_dev_tools_with_config()` + dynamic system prompt ([`src/config/agent.rs`](src/config/agent.rs))
- ✅ Shell 3-layer sandbox — When `base_dir` set: (1) `check_safe_bins_forced()`, (2) workdir parameter validation, (3) `detect_path_escape()` with `..` traversal detection. 11 tests ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs))
- ✅ Screen capture opt-in — `ScreenCaptureTool` gated on `SCREEN_CAPTURE_ENABLED` env var (user toggle in Scrappy UI) + `ALLOW_LOCAL_TOOLS` ([`src/app.rs`](src/app.rs))
- ✅ OS governance env vars — Scrappy passes `ACCESSIBILITY_GRANTED` + `SCREEN_RECORDING_GRANTED` from live macOS permission checks

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
- ✅ Tauri commands facade — 18 commands wired via [`src/tauri_commands.rs`](src/tauri_commands.rs) (27 tests) — 8 Sprint 14 + 7 Sprint 15 (routing CRUD + Gmail status + Gmail OAuth PKCE) + 3 Canvas (panels CRUD)

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

### P6 - Tauri IPC Architecture ✅ (Already Complete)

> **Verified 2026-03-05** by Scrappy Agent codebase audit.
> Scrappy already uses pure in-process Tauri IPC (no HTTP/SSE/GatewayChannel).
> See §21 for actual architecture documentation.

- ✅ `ironclaw_channel.rs` (272 LOC) — Implements `Channel` trait in Scrappy
- ✅ `rpc.rs` — 66+ Tauri commands, all via `invoke()` IPC
- ✅ `openclaw-event` — Tauri event bus, 8 React listener components
- ✅ `ironclaw::api::chat::send_message()` — Direct in-process chat API
- ✅ `ironclaw::api::chat::resolve_approval()` — Tool approval via IPC
- ✅ `StatusUpdate` → `UiEvent` → `app.emit()` pipeline
- ✅ Zero HTTP, zero open ports, zero EventSource, zero fetch(localhost)

### P7 - Sprint 15 (Gmail + Routing) — ✅ Fully Complete

| # | Task | Owner | Effort | Status |
|---|------|-------|--------|--------|
| **G1** | Gmail PKCE OAuth defaults — `oauth_defaults.rs` | IronClaw | 0.5 day | ✅ Complete (`GmailOAuthConfig`) |
| **G2** | Gmail channel — `GmailChannel` (700+ LOC) Pub/Sub + Gmail API | IronClaw | 1 day | ✅ Complete (`360d7d6`) |
| **G3** | Gmail channel card UI + PKCE wiring | Both | 0.5 day | ✅ Complete — Scrappy wired `startGmailOAuth()` to `openclaw_gmail_oauth_start` (automated PKCE), Gmail card loads real status |
| **R1** | Routing rule CRUD API — 6 commands in `tauri_commands.rs` | IronClaw | 0.5 day | ✅ Complete (`b5a5605`) |
| **R2** | Routing rule builder UI — `OpenClawRouting.tsx` (585 LOC) | Scrappy | 1-1.5 days | ✅ Complete (Scrappy agent, 2026-03-05) |
| **PKCE** | Gmail OAuth PKCE Tauri command — `openclaw_gmail_oauth_start` | IronClaw | 0.5 day | ✅ Complete |

**IronClaw: ✅ Complete** (G1, G2, R1, PKCE) | **Scrappy: ✅ Complete** (G3, R2) — **Sprint 15 fully done on both sides**

### Deferred (No Urgency)
- ✅ Sherpa-ONNX keyword spotting ([`src/voice_wake.rs`](src/voice_wake.rs): `detection_loop_sherpa()` — 3-thread pipeline with auto-fallback)
- 🔮 Skill cross-crate deps (no skills use Rust deps yet)
- 🔮 WS RPC for remote Hardware Bridge (desktop uses internal trait)
- 🔮 whisper-rs local inference (WhisperHttp covers desktop)
- 🔮 macOS dictation backend (scaffold in `talk_mode.rs`)
- 🔮 Per-conversation channel scoping — replace `app.emit()` broadcast with Tauri V2 `Channel<T>` per-invoke (~2 days, P3)


---

## §22 — Canvas / A2UI End-to-End Wiring — ✅ Fully Complete

> **Last updated:** 2026-03-05 11:29 CET — **All 9 tasks complete** (5 IronClaw + 4 Scrappy). Both cargo check and tsc --noEmit clean.
>
> Full details in [canvas_implementation_plan.md](file:///Users/mt/.gemini/antigravity/brain/a7a7a4d6-90d9-4646-bcf3-a581e62959ab/canvas_implementation_plan.md).

### 22.1 IronClaw Tasks (✅ Complete)

| # | Task | Files Changed | Status |
|---|------|--------------|--------|
| **IC-C1** | `StatusUpdate::CanvasAction` variant | `channel.rs`, `web/mod.rs`, `repl.rs`, `wasm/wrapper.rs` | ✅ Done |
| **IC-C2** | Agent loop canvas interception → emit + CanvasStore | `dispatcher.rs`, `agent_loop.rs` | ✅ Done |
| **IC-C3** | Canvas routes mounted in `main.rs` | `main.rs` | ✅ Done |
| **IC-C4** | Action callback queue (HTTP → agent) | `canvas_gateway.rs` (`QueuedAction`, `push_action`, `drain_actions`) | ✅ Done |
| **IC-C5** | Tauri commands for panel CRUD | `tauri_commands.rs` (`canvas_panels_list`, `canvas_panel_get`, `canvas_panel_dismiss`) | ✅ Done |

### 22.2 Scrappy Tasks (✅ Complete)

| # | Task | Implementation | Status |
|---|------|----------------|--------|
| **SC-C1** | Canvas action event wiring | `ironclaw_types.rs`: `CanvasAction` → `UiEvent::CanvasUpdate`; `ironclaw_bridge.rs`: `canvas_store: None`; `CanvasProvider.tsx`: event listener + dispatch | ✅ Done |
| **SC-C2** | Native panel rendering (11 components) | `CanvasPanelRenderer.tsx`: all 11 `UiComponent` types; `CanvasWindow.tsx`: multi-panel manager (A2UI + legacy), draggable/resizable, position-aware (right/bottom/center/floating), modal overlay | ✅ Done |
| **SC-C3** | Action callback dispatch | `canvasDispatchAction()` → `openclaw_canvas_dispatch_event` Tauri command → agent session message injection; button clicks + form submits | ✅ Done |
| **SC-C4** | Panel management UI | `CanvasToolbar.tsx`: floating badge pill (bottom-right), popover with panel list, focus/dismiss actions, color coding (cyan/amber/purple), dismiss all | ✅ Done |

### Owner: ✅ Both complete (IronClaw + Scrappy Agent)

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

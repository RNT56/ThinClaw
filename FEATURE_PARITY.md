# ThinClaw Parity And ThinClaw-First Feature Matrix

> **Last reconciled:** 2026-04-18 (docs + desktop autonomy reconciliation)

This document tracks both feature parity against OpenClaw (TypeScript reference implementation) and ThinClaw-first capabilities that now extend well beyond parity. Use it both as a compatibility map and as a ledger of the newer Rust-native features we are actively adding.

Scrappy-oriented sections are maintained from ThinClaw-side contracts, local Tauri wiring, and the integration surfaces present in this repo. They are useful for planning and contract verification, but external Scrappy UI implementation details should still be revalidated in the Scrappy repo before release sign-off.

**Key source cross-references:** [`src/service.rs`](src/service.rs) (launchd/systemd), [`src/tools/builtin/canvas.rs`](src/tools/builtin/canvas.rs) (A2UI canvas tool), [`src/channels/imessage.rs`](src/channels/imessage.rs) (iMessage channel), [`src/media/video.rs`](src/media/video.rs) (video pipeline), [`src/channels/signal.rs`](src/channels/signal.rs) (Signal), [`src/channels/discord.rs`](src/channels/discord.rs) (Discord), [`src/channels/nostr.rs`](src/channels/nostr.rs) (Nostr).

**Legend:**
- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned (in scope but not started)
- 🚫 Out of scope (intentionally skipped)
- ➖ N/A (not applicable to Rust implementation)

---

## 0. ThinClaw-First Snapshot

These are the higher-signal capabilities that now go beyond simple OpenClaw catch-up and should continue to grow as first-class ThinClaw features.

| Capability | Status | Notes |
|---------|--------|-------|
| Channel-owned formatting hints | ✅ | Native channels own formatting guidance through `Channel::formatting_hints()`. WASM channels can declare `formatting_hints` in `*.capabilities.json`, and prompt assembly consumes the resolved hint through `ChannelManager::formatting_hints_for()` instead of hard-coded channel-name switches. |
| Watched OAuth credential sync | ✅ | Claude Code, Codex, and custom JSON auth files can seed provider credentials and hot-reload updated tokens into the live runtime without restart. |
| External pre-exec shell scanner | ✅ | First-party `thinclaw-shell-scan` adds a pre-approval structural scanner with configured/PATH/bundled/cached resolution, verified cache install, and fail-open/fail-closed policy control. |
| Filesystem checkpoints + `/rollback` | ✅ | Shadow-git checkpoints create reversible file mutation history with list, diff, and restore support. |
| Remote skill federation | ✅ | ThinClaw now supports GitHub taps plus `/.well-known/skills` registries, quarantine scanning, provenance lock files, and risky-install approval gates. |
| Accessibility-tree browser automation | ✅ | Managed `agent-browser` integration and cloud browser routing move ThinClaw from screenshot-only inspection toward interaction-oriented browsing. |
| Session-level `/personality` overlays (`/vibe` alias) | ✅ | Session-scoped personality overlays add temporary tone shifts without mutating durable identity files. |
| CLI skin system | ✅ | Shared TOML-backed local skins now cover boot, REPL, full-screen TUI, onboarding TUI, setup prompts, and human-readable CLI subcommands with prompt symbols, ASCII art, taglines, and tool emoji labels. |
| Reckless desktop autonomy | ✅ | Privileged host-level desktop autonomy adds native app adapters, generic UI automation, evidence capture, seeded desktop routines, managed shadow-canary code autorollout, and rollback for promoted builds. |
| Trajectory archive + training export | ✅ | Structured turn archives and `trajectory export` provide SFT/DPO-friendly offline training datasets. |
| Anthropic prompt caching | ✅ | Provider-scoped message metadata now carries Anthropic-compatible cache hints where supported. |

---

## 1. Architecture

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Hub-and-spoke architecture | ✅ | ✅ | Web gateway as central hub |
| WebSocket control plane | ✅ | ✅ | Gateway with WebSocket + SSE |
| Single-user system | ✅ | ✅ | |
| Multi-agent routing | ✅ | ✅ | `AgentRouter` with workspace isolation, priority-based routing, thread ownership, and per-agent tool/skill allowlists |
| Session-based messaging | ✅ | ✅ | Principal-scoped direct sessions sync across channels/devices; group sessions remain isolated |
| Loopback-first networking | ✅ | ✅ | HTTP binds to 0.0.0.0 but can be configured |

### Owner: ThinClaw Agent

---

## 2. Gateway System

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Gateway control plane | ✅ | ✅ | Web gateway with 40+ API endpoints |
| HTTP endpoints for Control UI | ✅ | ✅ | Web dashboard with chat, memory, jobs, logs, extensions |
| Channel connection lifecycle | ✅ | ✅ | ChannelManager + WebSocket tracker |
| Session management/routing | ✅ | ✅ | SessionManager with principal-scoped direct session cutover + cross-channel thread alias reuse |
| Household multi-actor identity | ❌ | ✅ | Actor registry + `ResolvedIdentity` + conversation-scope session keys are landed, with `thinclaw identity ...` management, request-scoped gateway identity resolution, actor-aware protected routes, actor-partitioned SSE/WS fanout, and cross-channel direct-thread continuity that stays bound to the owning actor |
| Configuration hot-reload | ✅ | ✅ | `ConfigWatcher` with mtime polling, debounce, broadcast subscribers |
| Network modes (loopback/LAN/remote) | ✅ | ✅ | Full loopback/LAN/remote with security validation ([`src/config/network_modes.rs`](src/config/network_modes.rs)) |
| OpenAI-compatible HTTP API | ✅ | ✅ | /v1/chat/completions, per-request `model` override |
| Canvas hosting | ✅ | ✅ | `CanvasTool` (620 LOC) + `CanvasStore` + canvas gateway routes (`/canvas/:panel_id` HTML, `/canvas/api/panels` JSON); [`src/channels/canvas_gateway.rs`](src/channels/canvas_gateway.rs) |
| Gateway lock (PID-based) | ✅ | ✅ | `gateway start` writes PID to `~/.thinclaw/gateway.pid`; launchd/systemd keepalive via `thinclaw service` |
| launchd/systemd integration | ✅ | ✅ | Full `service.rs` (401 LOC, [`src/service.rs`](src/service.rs)) — install/start/stop/status/uninstall; macOS launchd plist + Linux systemd unit generation |
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

### Owner: ThinClaw Agent

---

## 3. Messaging Channels

| Channel | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (local shell) | ✅ | ✅ | - | Full local shell surface with slash commands, skins, `/rollback`, and `/personality` (`/vibe` alias); no longer just a testing stub |
| WASM channels | ❌ | ✅ | - | ThinClaw innovation |
| WhatsApp | ✅ | ✅ | - | WASM channel via Cloud API webhook — text, media (image/audio/video/document/sticker), reply threading, DM pairing, markdown→WhatsApp formatting, message chunking |
| Telegram | ✅ | ✅ | - | WASM channel, DM pairing, caption, /start, bot_username, forum threading, sendMessage+editMessageText streaming (host-side, HTML formatted) |
| Discord | ✅ | ✅ | - | Native Rust Gateway WS + REST ([`src/channels/discord.rs`](src/channels/discord.rs)) + WASM interactions channel (slash commands) |
| Signal | ✅ | ✅ | - | signal-cli daemon, SSE listener, user/group allowlists, DM pairing |
| Slack | ✅ | ✅ | - | WASM channel (Events API webhook). Native dead code (`slack.rs`) removed. |
| iMessage | ✅ | ✅ | P3 | `IMessageChannel` + `IMessageConfig` native runtime ([`src/channels/imessage.rs`](src/channels/imessage.rs)) |
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
| Apple Mail | ❌ | ✅ | P3 | `AppleMailChannel` — Envelope Index polling, sender allowlist, unread-only, mark-as-read. Wizard onboarding + WebUI settings ([`src/channels/apple_mail.rs`](src/channels/apple_mail.rs)) |
| Nostr | ✅ | ✅ | - | NIP-04 encrypted DM channel (`channels/nostr.rs`), broadcast() with pubkey validation, empty allowlist = accept all |

### Telegram-Specific Features (since Feb 2025)

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Forum topic creation | ✅ | ✅ | `message_thread_id` pass-through for forum groups; replies target correct topic. WASM channel extracts + propagates thread ID in metadata |
| channel_post support | ✅ | ✅ | Channel posts received via `channel_post` update + `sender_chat` for identification |
| User message reactions | ✅ | ✅ | `TgMessageReaction` + `TgReactionType` parsing; emojis surfaced in `IncomingMessage` metadata |
| sendPoll | ✅ | ✅ | `send_poll()` helper: question, options, anonymous/multiple-answer flags; wired to `Channel::poll()` trait |
| Cron/heartbeat topic targeting | ✅ | ✅ | `HEARTBEAT_NOTIFY_TOPIC_ID` config + `message_thread_id` injection in broadcast metadata |
| sendMessage+editMessageText streaming | ✅ | ✅ | Host-side streaming via `sendMessage` (first chunk) + `editMessageText` (subsequent). Unified Markdown→HTML converter exposed via WIT `markdown-to-telegram-html` — WASM guest delegates to host, eliminating duplicate code. Persistent draft across tool-call iterations. Overflow detection (>3800 chars): deletes partial message and falls back to `on_respond()` message splitting. Enabled via `/api/settings/telegram_stream_mode` with hot-reload or `TELEGRAM_STREAM_MODE=edit` env var. |
| Temporary subagent session routing | ❌ | ✅ | Telegram WASM channel now routes subagent lifecycle updates into temporary forum topics (`createForumTopic`/`closeForumTopic`) with deterministic fallback to reply-chain, then compact notices (`compact_off`) when topic/reply delivery is unavailable |

### Discord-Specific Features (since Feb 2025)

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Forwarded attachment downloads | ✅ | ✅ | Platform-aware download with size limits ([`src/channels/forward_download.rs`](src/channels/forward_download.rs)) |
| Faster reaction state machine | ✅ | ✅ | Debounced state machine with watchdog cleanup ([`src/channels/reaction_machine.rs`](src/channels/reaction_machine.rs)) |
| Thread parent binding inheritance | ✅ | ✅ | Chain-resolved inheritance with agent propagation ([`src/agent/thread_inheritance.rs`](src/agent/thread_inheritance.rs)) |

### Slack-Specific Features (since Feb 2025)

Slack remains a supported WASM Events API channel with webhook ingestion, thread replies, DM/mention handling, and attachment parsing. However, the live draft-streaming/runtime stream-mode controls currently wired in ThinClaw are for **Telegram** and **Discord**, not Slack (`src/channels/wasm/wrapper.rs`, `src/channels/web/handlers/settings.rs`).

### Channel Features

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| DM pairing codes | ✅ | ✅ | `thinclaw pairing list/approve`, host APIs |
| Allowlist/blocklist | ✅ | ✅ | allow_from + block_from + pairing store (blocklist takes precedence) |
| Self-message bypass | ✅ | ✅ | Multi-bot-ID detection + message filtering ([`src/channels/self_message.rs`](src/channels/self_message.rs)) |
| Mention-based activation | ✅ | ✅ | bot_username + respond_to_all_group_messages |
| Per-group tool policies | ✅ | ✅ | `ToolPolicyManager` with AllowAll/AllowList/DenyList + hierarchical evaluation ([`src/tools/policy.rs`](src/tools/policy.rs)) |
| Thread isolation | ✅ | ✅ | Separate sessions per thread |
| Platform formatting hints | ✅ | ✅ | Active-channel formatting guidance is injected into prompts across native and WASM channels. WASM channels resolve hints with two-tier precedence: explicit channel/package hint first, built-in platform fallback second, then uniform retrieval through `ChannelManager::formatting_hints_for()` |
| Linked DM continuity across channels | ❌ | 🚧 | DM scope metadata, actor-linked recall, and handoff storage added; explicit group recall and full UX restrictions still need completion |
| Per-channel media limits | ✅ | ✅ | `MediaLimits` with per-channel env var overrides + `filter_attachments()` ([`src/media/limits.rs`](src/media/limits.rs)) |
| Typing indicators | ✅ | ✅ | `Channel::send_typing()` trait method with platform-agnostic interface; TUI + Telegram + extensible to Discord/Signal |
| Per-channel ackReaction config | ✅ | ✅ | Per-channel emoji overrides via env vars ([`src/channels/ack_reaction.rs`](src/channels/ack_reaction.rs)) |
| Group session priming | ✅ | ✅ | Member roster injection with configurable limits ([`src/channels/group_priming.rs`](src/channels/group_priming.rs)) |
| Sender_id in trusted metadata | ✅ | ✅ | `TrustedMetadata` struct with sender_id, channel, is_group ([`src/channels/self_message.rs`](src/channels/self_message.rs)) |

### Owner: ThinClaw Agent

---

## 4. CLI Commands

| Command | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| `run` (agent) | ✅ | ✅ | - | Default command |
| `tool install/list/remove` | ✅ | ✅ | - | WASM tools |
| `gateway start/stop` | ✅ | ✅ | P2 | `gateway.rs`: start (foreground/bg with PID), stop (SIGTERM), status (health+uptime) |
| `onboard` (wizard) | ✅ | ✅ | - | Interactive setup with five onboarding lanes, including `Custom / Advanced`, plus Humanist Cockpit copy/readiness framing and shared CliSkin presentation across CLI and TUI |
| `/skin` | ❌ | ✅ | - | Runtime local-client skin switching with built-in TOML skins (`cockpit`, `midnight`, `solar`, `athena`, `delphi`, `olympus`), ASCII art, and user overrides |
| `/personality` | ❌ | ✅ | - | Session-scoped personality overlay command (`/personality`, `/personality <name>`, `/personality reset`) with `/vibe` retained as a compatibility alias |
| `tui` | ✅ | ✅ | - | Ratatui TUI |
| `config` | ✅ | ✅ | - | Read/write config |
| `channels` | ✅ | ✅ | P2 | `channels.rs`: list (env+WASM detection), info (per-channel details) |
| `models` | ✅ | ✅ | - | `list`, `info`, `test`, and `verify` subcommands with live provider discovery/probing ([`src/cli/models.rs`](src/cli/models.rs)) |
| `status` | ✅ | ✅ | - | System status (enriched session details) |
| `agents` | ✅ | ✅ | P3 | `list`, `add`, `remove`, `show`, `set-default` subcommands |
| `sessions` | ✅ | ✅ | P3 | `list`, `show`, `prune`, and `export` subcommands with thread ownership display and transcript export ([`src/cli/sessions.rs`](src/cli/sessions.rs)) |
| `memory` | ✅ | ✅ | - | `search`, `read`, `write`, `tree`, and `status` subcommands for workspace memory operations ([`src/cli/memory.rs`](src/cli/memory.rs)) |
| `trajectory` | ❌ | ✅ | P3 | `trajectory export` supports raw archive export plus training-ready `sft` and `dpo` datasets derived from labeled turn trajectories; `trajectory stats` summarizes the local archive ([`src/cli/trajectory.rs`](src/cli/trajectory.rs), [`src/agent/learning.rs`](src/agent/learning.rs)) |
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
| `reset` | ❌ | ✅ | - | Destructive full-state reset: clears ThinClaw DB tables, removes `~/.thinclaw`, and deletes ThinClaw-managed keychain entries so onboarding can restart cleanly |
| `logs` | ✅ | ✅ | P3 | tail/search/show/levels with time-range/level/target filtering ([`src/cli/logs.rs`](src/cli/logs.rs)) |
| `update` | ✅ | ✅ | P3 | `check`, `install`, `rollback`, and `info` with stable/beta/nightly channels ([`src/cli/update.rs`](src/cli/update.rs)) |
| `completion` | ✅ | ✅ | - | Shell completion |
| `/subagents spawn` | ✅ | ✅ | P3 | Command parsing + subagent tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs)) |
| `/export-session` | ✅ | ✅ | P3 | `sessions export` with markdown/JSON output ([`src/cli/sessions.rs`](src/cli/sessions.rs)) |

### Owner: ThinClaw Agent

---

## 5. Agent System

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Pi agent runtime | ✅ | ➖ | ThinClaw uses custom runtime |
| RPC-based execution | ✅ | ✅ | Orchestrator/worker pattern |
| Multi-provider failover | ✅ | ✅ | `FailoverProvider` tries providers sequentially on retryable errors, applies per-provider cooldowns, leases individual credential entries (not just provider slots) with fill-first / round-robin / least-used / random selection strategies so multi-key backends can spread parallel traffic across keys, and participates in live OAuth credential refresh via watched auth-file sync + runtime reload ([`src/llm/failover.rs`](src/llm/failover.rs), [`src/llm/credential_sync.rs`](src/llm/credential_sync.rs), [`src/app.rs`](src/app.rs)) |
| Per-sender sessions | ✅ | ✅ | Direct sessions are canonicalized by principal scope (cross-channel/device continuity); group scopes remain isolated |
| Global sessions | ✅ | ✅ | Cross-channel shared context with LRU eviction ([`src/agent/global_session.rs`](src/agent/global_session.rs)) |
| Session pruning | ✅ | ✅ | `sessions prune` CLI + auto-cleanup with configurable TTL |
| Context compaction | ✅ | ✅ | Auto summarization with user-visible pressure warnings at 85%/95% before compaction |
| Closed-loop learning orchestrator | ❌ | ✅ | Event→evaluation→candidate loop with dedupe/cooldown, safe-mode thresholds, Tier A auto-apply, Tier C approval-gated code proposals ([`src/agent/learning.rs`](src/agent/learning.rs), [`src/agent/thread_ops.rs`](src/agent/thread_ops.rs)) |
| Learning tool suite | ❌ | ✅ | `session_search` (FTS + optional cheap-model transcript summaries with fallback), `prompt_manage`, `skill_manage`, `learning_status`, `learning_history`, `learning_feedback`, `learning_proposal_review` ([`src/tools/builtin/memory.rs`](src/tools/builtin/memory.rs), [`src/agent/session_search.rs`](src/agent/session_search.rs), [`src/tools/builtin/learning_tools.rs`](src/tools/builtin/learning_tools.rs)) |
| Learning API + audit UI | ❌ | ✅ | `/api/learning/*` endpoints + dedicated Web Learning tab with proposals, feedback, rollbacks, and provider health ([`src/api/learning.rs`](src/api/learning.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/index.html`](src/channels/web/static/index.html)) |
| Optional research automation / experiments | ❌ | ✅ | Advanced opt-in `experiments.*` settings, `/api/experiments/*` gateway routes, CLI `thinclaw experiments ...`, routine action integration, Web Research tab, queued-campaign draining, autonomous planner/mutator/reviewer iteration, telemetry-derived opportunities, persisted target linking, normalized LLM-cost + runner-cost attribution, provider/budget detail in the Research WebUI, GPU Cloud setup cards, lease-scoped remote runner mode, local benchmark execution, and controller-managed RunPod/Vast/Lambda plus SSH/Slurm/Kubernetes launches are shipped, including a first-class Lambda launch form that builds `backend_config.launch_payload` server-side for turnkey controller launches ([`src/api/experiments.rs`](src/api/experiments.rs), [`src/experiments/mod.rs`](src/experiments/mod.rs), [`src/experiments/adapters.rs`](src/experiments/adapters.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/index.html`](src/channels/web/static/index.html), [`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Optional external memory providers | ❌ | ✅ | Honcho + Zep adapters, local-first canonical memory, non-fatal provider fallback ([`src/agent/learning.rs`](src/agent/learning.rs)) |
| Post-compaction read audit | ✅ | ✅ | `ReadAuditor` with scope-based rule scanning + token-budgeted appendix ([`src/context/read_audit.rs`](src/context/read_audit.rs)) |
| Post-compaction context injection | ✅ | ✅ | Priority-based fragment assembly with token budgets ([`src/context/post_compaction.rs`](src/context/post_compaction.rs)) |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Persona seed templates | ❌ | ✅ | P4 | Built-in SOUL.md seed presets plus setup-wizard selection for fresh workspaces |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill routing blocks | ✅ | ✅ | ActivationCriteria: keywords, patterns, tags, `use_when`/`dont_use_when` routing blocks |
| Skill path compaction | ✅ | ✅ | `~` prefix via `RuntimeBehavior::compact_path()` ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |
| Thinking modes (low/med/high) | ✅ | ✅ | `ThinkingConfig` enum (Disabled/Enabled with budget_tokens), configurable via `thinking_enabled` + `thinking_budget_tokens` settings |
| Per-model thinkingDefault override | ✅ | ✅ | `MODEL_THINKING_OVERRIDE` env var: exact+prefix model match with per-model budget |
| Model-specific prompt guidance | ❌ | ✅ | Model-family prompt guardrails (GPT/Gemini) injected into conversation system prompts, controlled by `agent.model_guidance_enabled` |
| Block-level streaming | ✅ | ✅ | `StreamChunk::Text` + `StreamChunk::ReasoningDelta` via `complete_stream()` |
| Tool-level streaming | ✅ | ✅ | `StreamChunk::ToolCall` + `StreamChunk::ToolCallDelta` via `complete_stream_with_tools()` |
| Z.AI tool_stream | ✅ | ✅ | Full tool_stream protocol with delta accumulation ([`src/channels/tool_stream.rs`](src/channels/tool_stream.rs)) |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Autonomous approval mode | ❌ | ✅ | `auto_approve_tools` with NEVER_AUTO_APPROVE_PATTERNS safety preserved; manual “allow session” approvals are channel-scoped by default for safer cross-channel direct-session sharing ([`src/agent/dispatcher.rs`](src/agent/dispatcher.rs), [`src/agent/session.rs`](src/agent/session.rs), [`src/setup/wizard.rs`](src/setup/wizard.rs)) |
| Smart shell approval triage | ❌ | ✅ | Soft-flag shell commands now pass through `SmartApprover` on the approval path; APPROVE skips the prompt, DENY fails closed, ESCALATE falls back to human approval, while `BLOCKED_COMMANDS` stay unconditional ([`src/safety/smart_approve.rs`](src/safety/smart_approve.rs), [`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs)) |
| Self-update & restart | ❌ | ✅ | `thinclaw update install` + `/restart` command for orderly shutdown; service-managed installs auto-relaunch and foreground `thinclaw` runs now self-relaunch too ([`src/cli/update.rs`](src/cli/update.rs), [`src/agent/submission.rs`](src/agent/submission.rs), [`src/main.rs`](src/main.rs)) |
| Elevated mode | ✅ | ✅ | Timeout-based activation with command allowlisting ([`src/safety/elevated.rs`](src/safety/elevated.rs)) |
| Subagent system | ✅ | ✅ | Full `SubagentExecutor` ([`src/agent/subagent_executor.rs`](src/agent/subagent_executor.rs)): in-process agentic loops with isolated context, filtered tools/skills, configurable timeouts, cancellation via watch channels, and durable async-subagent resume metadata |
| `spawn_subagent` tool | ✅ | ✅ | Declarative tool → dispatcher interception → `SubagentExecutor::spawn()`. Sync (wait=true) and async (wait=false) modes ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `list_subagents` tool | ✅ | ✅ | Query active/recent sub-agents with ID, status, task, timing info ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `cancel_subagent` tool | ✅ | ✅ | Cancel running sub-agents by UUID; watch channel + task abort ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| Sub-agent lifecycle | ✅ | ✅ | Concurrency limits (default 5), per-agent timeout, periodic `StatusUpdate::SubagentProgress` heartbeat updates during long-running runs, status tracking (Running/Completed/Failed/TimedOut/Cancelled), user progress notifications via StatusUpdate::AgentMessage, and worker-side inactivity keepalives so long-running delegated work does not trip the parent wall-clock timeout |
| Trajectory archive + training export | ❌ | ✅ | Per-turn trajectory logging now stores structured assessment metadata, hydrates explicit learning feedback when available, and exports `json`, `jsonl`, `sft`, and `dpo` datasets for offline training workflows ([`src/agent/learning.rs`](src/agent/learning.rs), [`src/cli/trajectory.rs`](src/cli/trajectory.rs)) |
| Subagent transparency controls | ❌ | ✅ | End-to-end transparency controls shipped: typed Web subagent SSE events + temporal Web subsessions + `agent.subagent_transparency_level` (`balanced`, `detailed`) filtering + Telegram session mode routing |
| `/subagents spawn` command | ✅ | ✅ | Command parsing + tracking ([`src/cli/subagent_spawn.rs`](src/cli/subagent_spawn.rs)) |
| Persistent multi-agent orchestration | ❌ | ✅ | `AgentRegistry` with DB-backed CRUD, 5 LLM tools (`create_agent`/`list_agents`/`update_agent`/`remove_agent`/`message_agent`), workspace seeding, validation, dual-backend persistence ([`src/agent/agent_registry.rs`](src/agent/agent_registry.rs), [`src/tools/builtin/agent_management.rs`](src/tools/builtin/agent_management.rs)) |
| A2A communication | ❌ | ✅ | `message_agent` tool resolves target agent, builds scoped context (system prompt + memory), returns structured A2A payload; dispatcher intercepts and routes through `SubagentExecutor` for actual LLM execution ([`src/tools/builtin/agent_management.rs`](src/tools/builtin/agent_management.rs), [`src/agent/dispatcher.rs`](src/agent/dispatcher.rs)) |
| Workspace sandbox modes | ❌ | ✅ | `WORKSPACE_MODE` config: `unrestricted` (default), `sandboxed` (confined to `WORKSPACE_ROOT`), `project` (working dir set, files unrestricted). Drives tool registration + dynamic system prompt ([`src/config/agent.rs`](src/config/agent.rs)) |
| Dynamic system prompt | ❌ | ✅ | `Reasoning::build_workspace_capabilities_section()` generates mode-specific Desktop Capabilities based on active workspace config and available tools ([`src/llm/reasoning.rs`](src/llm/reasoning.rs)) |
| Screen capture tool | ❌ | ✅ | `ScreenCaptureTool` — macOS screencapture/Linux gnome-screenshot/scrot; registered when `ALLOW_LOCAL_TOOLS=true` + `SCREEN_CAPTURE_ENABLED=true` (user opt-in via Scrappy toggle). Dynamic system prompt auto-detects and injects guidance ([`src/tools/builtin/screen_capture.rs`](src/tools/builtin/screen_capture.rs)) |
| Auth profiles | ✅ | ✅ | Multi-key rotation with health tracking, plus watched OAuth auth-file sources (Claude Code, Codex, and custom JSON token files) that prime injected provider credentials and hot-reload the live runtime on token changes without restart ([`src/safety/auth_profiles.rs`](src/safety/auth_profiles.rs), [`src/llm/credential_sync.rs`](src/llm/credential_sync.rs), [`src/app.rs`](src/app.rs), [`src/config/mod.rs`](src/config/mod.rs)) |
| Generic API key rotation | ✅ | ✅ | Multi-strategy rotation with health tracking ([`src/safety/key_rotation.rs`](src/safety/key_rotation.rs)) |
| Stuck loop detection | ✅ | ✅ | Consecutive same-tool detection with warn at 3, force-text at 5 |
| llms.txt discovery | ✅ | ✅ | .well-known probing + markdown link parsing ([`src/llm/llms_txt.rs`](src/llm/llms_txt.rs)) |
| Multiple images per tool call | ✅ | ✅ | `ImageExtractor::format_multiple_for_llm()` — multi-image content blocks |
| URL allowlist (web_search/fetch) | ✅ | ✅ | `HTTP_URL_ALLOWLIST` env var — comma-separated domain globs |
| suppressToolErrors config | ✅ | ✅ | `RuntimeBehavior::format_tool_error()` with generic fallback ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |
| Intent-first tool display | ✅ | ✅ | Human-readable intent + exec summaries ([`src/tools/intent_display.rs`](src/tools/intent_display.rs)) |
| Transcript file size in status | ✅ | ✅ | `TranscriptStats` with message/token/tool/attachment counts ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |

### Owner: ThinClaw Agent

---

## 6. Model & Provider Support

| Provider | OpenClaw | ThinClaw | Priority | Notes |
|----------|----------|----------|----------|-------|
| Anthropic (Claude) | ✅ | ✅ | - | Direct API via RigAdapter (Sonnet 4, etc.) |
| OpenAI (GPT) | ✅ | ✅ | - | Direct API via RigAdapter (GPT-4o, etc.) |
| OpenAI-compatible | ❌ | ✅ | - | **Default backend** — works with OpenRouter, vLLM, LiteLLM, Together, Fireworks |
| OpenRouter | ✅ | ✅ | - | Via OpenAI-compatible provider |
| Ollama (local) | ✅ | ✅ | - | Via `rig::providers::ollama` (full support) |
| Tinfoil | ❌ | ✅ | - | Private inference provider (ThinClaw-only) |
| AWS Bedrock | ✅ | ✅ | P3 | Native Bedrock Mantle OpenAI-compatible endpoint is now the primary path, with legacy proxy fallback still supported ([`src/llm/provider_factory.rs`](src/llm/provider_factory.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs)) |
| Google Gemini | ✅ | ✅ | P3 | AI Studio adapter with system instruction support ([`src/llm/gemini.rs`](src/llm/gemini.rs)) |
| NVIDIA API | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Nvidia` — pre-configured OpenAI-compatible endpoint ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| Perplexity | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Perplexity` — `sonar-pro` default model ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| MiniMax | ✅ | ✅ | P3 | Provider preset + catalog updated to the current `api.minimax.io/v1` OpenAI-compatible endpoint and M2-family defaults ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs), [`src/config/provider_catalog.rs`](src/config/provider_catalog.rs)) |
| GLM (Zhipu) | ✅ | ✅ | P3 | Provider preset via `ProviderPreset::Glm` — current default is `glm-4-plus`; parser accepts `glm5` / `glm-5` aliases but the shipped preset is not named `GLM-5` in code ([`src/llm/provider_presets.rs`](src/llm/provider_presets.rs)) |
| node-llama-cpp | ✅ | ➖ | - | N/A for Rust |
| llama.cpp (server-mode integration) | ❌ | ✅ | P3 | ThinClaw exposes llama.cpp through a local `llama-server` / OpenAI-compatible endpoint today, while `src/llm/llama_cpp.rs` provides the abstraction layer and future native-hook scaffold rather than a fully wired FFI backend ([`src/llm/provider_factory.rs`](src/llm/provider_factory.rs), [`src/llm/llama_cpp.rs`](src/llm/llama_cpp.rs)) |

ThinClaw's current provider catalog also includes **Groq, Mistral, xAI, Together, Venice, Moonshot, DeepSeek, Cerebras, and Cohere** via OpenAI-compatible endpoints, in addition to the rows above ([`src/config/provider_catalog.rs`](src/config/provider_catalog.rs)).

### Model Features

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Auto-discovery | ✅ | ✅ | `ModelDiscovery` service covers OpenAI-compatible, Anthropic, Ollama, Cohere, and provider-specific verification paths used by both CLI and WebUI flows ([`src/llm/discovery.rs`](src/llm/discovery.rs), [`src/cli/models.rs`](src/cli/models.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs)) |
| Failover chains | ✅ | ✅ | `FailoverProvider` with configurable `fallback_model` |
| Cooldown management | ✅ | ✅ | Lock-free per-provider cooldown in `FailoverProvider` |
| Per-session model override | ✅ | ✅ | Model selector in TUI |
| Model selection UI | ✅ | ✅ | TUI keyboard shortcut |
| Per-model thinkingDefault | ✅ | ✅ | `MODEL_THINKING_OVERRIDE` env var with exact+prefix matching |
| Prompt caching metadata seam | ✅ | ✅ | `ChatMessage` carries provider-scoped metadata and reasoning attaches Anthropic-compatible `cache_control: {type: "ephemeral"}` hints only when `supports_prompt_caching()` is true; unsupported providers stay metadata-clean while `with_prompt_caching()` remains compatibility fallback in the adapter path |
| 1M context beta header | ✅ | ✅ | `ExtendedContextConfig` with configurable beta header ([`src/llm/extended_context.rs`](src/llm/extended_context.rs)) |
| Smart routing WebUI config | ❌ | ✅ | Providers tab now includes a dedicated Routing editor for enable/disable, mode selection, provider roster, per-provider model slot persistence, drag-and-drop primary/cheap pool reordering, fallback chain, policy rules, route simulation, an opt-in “Separate tool planning from final answer” toggle for two-phase cheap synthesis on main-agent tool turns, where the cheap phase only runs after an explicit `NO_TOOLS_NEEDED` planner signal and any primary-model final answer is returned directly, plus a separate toggle to keep primary planning thinking enabled by default ([`src/channels/web/static/app.js`](src/channels/web/static/app.js), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/agent/dispatcher.rs`](src/agent/dispatcher.rs)) |
| Wizard cheap model API key | ❌ | ✅ | `step_smart_routing` detects cross-provider cheap model, prompts for the missing API key without clobbering the primary backend, and persists canonical provider settings ([`src/setup/wizard/llm.rs`](src/setup/wizard/llm.rs)) |
| Claude Code runtime model config | ❌ | ✅ | WebUI Settings: change model/max-turns without restart, hot-reloaded into `ContainerJobManager` ([`src/orchestrator/job_manager.rs`](src/orchestrator/job_manager.rs)) |
| Provider Vault (WebUI key mgmt) | ❌ | ✅ | Providers tab: list providers, add/remove encrypted credentials for catalog providers plus custom OpenAI-compatible endpoints and native Bedrock API keys, auto-enable them for routing, and hot-reload the live LLM runtime ([`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Agent-initiated model switching | ❌ | ✅ | `llm_select` + `llm_list_models` tools: agent can switch LLM mid-conversation via conversation-scoped runtime overrides; runtime probe blocks dead model switches, dispatcher auto-resets failed overrides to the previous working model, and model discovery exposes current primary/cheap config [`src/tools/builtin/llm_tools.rs`](src/tools/builtin/llm_tools.rs) |
| Wizard fallback providers step | ❌ | ✅ | `step_fallback_providers` adds secondary providers and models into canonical provider routing settings without mutating the chosen primary backend ([`src/setup/wizard/llm.rs`](src/setup/wizard/llm.rs)) |
| RoutingPolicy in runtime | ❌ | ✅ | Policy rules are resolved by the live LLM runtime manager and applied to request-time routing, explicit fallback chains, cost-aware target selection, OpenAI-compatible calls, and hot-reloaded provider settings ([`src/llm/runtime_manager.rs`](src/llm/runtime_manager.rs), [`src/llm/routing_policy.rs`](src/llm/routing_policy.rs)) |
| Routing Engine V2 (unified planner) | ❌ | ✅ | Unified `RoutePlanner` drives all routing decisions: structured scoring (quality/cost/latency/health), capability gating, budget-aware cost pressure, session affinity, and context window gating. Supports 4 modes: PrimaryOnly, CheapSplit (preserved), AdvisorExecutor (new), Policy. Live cutover complete — planner replaces legacy `SmartRoutingProvider` branching. `consult_advisor` tool intercepted by dispatcher for advisor-executor pattern. Normalized telemetry keys (`role|slug|model`), circuit-breaker + latency-weighted health probes ([`src/llm/route_planner.rs`](src/llm/route_planner.rs), [`src/tools/builtin/advisor.rs`](src/tools/builtin/advisor.rs), [`src/llm/runtime_manager.rs`](src/llm/runtime_manager.rs), [`src/agent/dispatcher.rs`](src/agent/dispatcher.rs)) |

### Owner: ThinClaw Agent

---

## 7. Media Handling

| Feature | OpenClaw | ThinClaw | Priority | Notes |
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
| Multimodal media routing | ❌ | ✅ | - | Images/audio/video → rig-core `UserContent::Image/Audio/Video` (provider-agnostic); PDFs → text extraction. Attachment size limits: 20MB/file, 50MB/msg |
| Telegram media download | ❌ | ✅ | - | Layer 6: photo/voice/audio/document/video/video_note/sticker download via Bot API `getFile`. All media types populate `MediaAttachment` and route to multimodal LLM |
| Discord media download | ❌ | ✅ | - | Native gateway `MESSAGE_CREATE` with `attachments[]` CDN download; size-limited to 20MB/file |
| Signal media download | ❌ | ✅ | - | Typed `SignalAttachment` from signal-cli SSE, reads binary from local attachment store; size-limited |
| iMessage media download | ❌ | ✅ | - | Queries `attachment` + `message_attachment_join` tables from chat.db, reads files from disk |
| WhatsApp media download | ❌ | ✅ | - | 2-step Cloud API download (media URL → binary), supports image/audio/video/document/sticker |
| Slack media download | ❌ | ✅ | - | `SlackFile.url_private_download` with Bearer token auth, size-limited to 20MB |

### Owner: ThinClaw Agent

---

## 8. Plugin & Extension System

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Dynamic loading | ✅ | ✅ | WASM modules |
| Manifest validation | ✅ | ✅ | WASM metadata |
| HTTP path registration | ✅ | ❌ | No public plugin-specific HTTP route registry is exposed in the current ThinClaw runtime; packaged channels use host-owned webhook routing instead |
| Home-directory install roots | ✅ | ✅ | Default install locations are `~/.thinclaw/tools/` and `~/.thinclaw/channels/`, not workspace-relative |
| Channel plugins | ✅ | ✅ | WASM channels |
| Auth plugins | ✅ | ❌ | No public auth-plugin trait surface is exposed in the current ThinClaw runtime |
| Memory plugins | ✅ | ❌ | No public memory-plugin trait surface is exposed in the current ThinClaw runtime |
| Tool plugins | ✅ | ✅ | WASM tools |
| Hook plugins | ✅ | ✅ | Declarative hooks from extension capabilities |
| Provider plugins | ✅ | ❌ | No public provider-plugin trait surface is exposed in the current ThinClaw runtime |
| Plugin CLI (`install`, `list`) | ✅ | ✅ | `registry list/install/install-defaults` subcommands ([`src/cli/registry.rs`](src/cli/registry.rs)) |
| Plugin CLI (`search`) | ✅ | ✅ | `registry search <query>` — full-text search across name, description, keywords |
| Plugin CLI (`remove`) | ✅ | ✅ | `registry remove <name>` — deletes `.wasm` + `.capabilities.json` from channels/tools dir |
| ClawHub registry | ✅ | ✅ | `ClawHubConfig` + `CatalogCache` with TTL, search, merge ([`src/extensions/clawhub.rs`](src/extensions/clawhub.rs)) |
| Channel hot-reload | ❌ | ✅ | `ChannelWatcher` polls `~/.thinclaw/channels/` for `.wasm` changes; auto-loads/reloads/removes channels ([`src/channels/wasm/channel_watcher.rs`](src/channels/wasm/channel_watcher.rs)) |
| REST API (extensions) | ❌ | ✅ | Full management surface: `GET /api/extensions`, `POST install`, `POST activate`, `POST {name}/remove`, `GET/POST setup` |
| MCP server integration | ✅ | ✅ | Full MCP client with OAuth 2.1, stdio + HTTP transport, session management ([`src/tools/mcp/`](src/tools/mcp/)) |
| `before_agent_start` hook | ✅ | ✅ | `HookPoint::BeforeAgentStart` — fires before agent main loop, can reject startup |
| `before_message_write` hook | ✅ | ✅ | `HookPoint::BeforeMessageWrite` — fires before channel write, can modify/suppress |
| `llm_input`/`llm_output` hooks | ✅ | ✅ | Before/after hook pipeline with priority ordering ([`src/llm/llm_hooks.rs`](src/llm/llm_hooks.rs)) |

### Owner: ThinClaw Agent

---

## 9. Configuration System

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Primary config file | ✅ `~/.openclaw/openclaw.json` | ✅ `~/.thinclaw/settings.json` | ThinClaw persists primary settings in `settings.json`; `config.toml` is an overlay and `.env` remains bootstrap/env input |
| JSON5 support | ✅ | ✅ | Comment stripping + trailing comma removal ([`src/config/formats.rs`](src/config/formats.rs)) |
| YAML alternative | ✅ | ✅ | YAML-to-JSON + JSON-to-YAML serialization ([`src/config/formats.rs`](src/config/formats.rs)) |
| Environment variable interpolation | ✅ | ✅ | `${VAR}` |
| Config validation/schema | ✅ | ✅ | Type-safe Config struct |
| Hot-reload | ✅ | ✅ | `ConfigWatcher` polls mtime, broadcasts changes |
| Legacy migration | ✅ | ✅ | Legacy bootstrap/settings formats are still migrated forward during startup and settings load |
| State directory | ✅ `~/.openclaw-state/` | ✅ `~/.thinclaw/` | |
| Credentials directory | ✅ | ✅ | Session files |
| Full model compat fields in schema | ✅ | ✅ | `ModelCompat` with context window, feature flags, pricing, pi-ai support ([`src/config/model_compat.rs`](src/config/model_compat.rs)) |

### Owner: ThinClaw Agent

---

## 10. Memory & Knowledge System

| Feature | OpenClaw | ThinClaw | Notes |
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
| Proactive BOOT.md execution | ❌ | ✅ | Executes BOOT.md on every startup; routes response to preferred notification channel ([`src/agent/agent_loop.rs`](src/agent/agent_loop.rs)) |
| Proactive BOOTSTRAP.md execution | ❌ | ✅ | Executes BOOTSTRAP.md on first run only; auto-deletes after completion; routes response to preferred channel ([`src/agent/agent_loop.rs`](src/agent/agent_loop.rs)) |
| Daily logs | ✅ | ✅ | |
| Heartbeat checklist | ✅ | ✅ | HEARTBEAT.md |

### Owner: ThinClaw Agent

---

## 11. Mobile Apps

| Feature | OpenClaw | ThinClaw | Priority | Notes |
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

### Owner: ThinClaw Agent (if ever prioritized)

---

## 12. macOS App (Scrappy — Tauri/Rust)

> Scrappy is the macOS desktop app built with Tauri v2 (Rust + React).
> A native Swift macOS app may be added later.

| Feature | OpenClaw | Scrappy | Status | Notes |
|---------|----------|---------|--------|-------|
| SwiftUI native app | ✅ | ✅ | Covered | Tauri v2 (Rust + React) — intentional tech choice |
| Menu bar presence | ✅ | ✅ | Covered | `TrayIconBuilder` with template icon + click handler |
| Bundled gateway | ✅ | ✅ | Superior | In-process ThinClaw agent, no separate process |
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
| **Bundled gateway** | In-process ThinClaw agent via Rust FFI. No Node.js, no WS bridge. Saves ~120MB bundle. Zero cold-start latency. | `ironclaw_bridge.rs` |
| **Canvas hosting** | `CanvasWindow.tsx`. Receives `CanvasUpdate` events + legacy `canvas-push`/`canvas-navigate`. Sandboxed iframe with `postMessage` shim for bidirectional comms. Spring-animated slide-in/out. **Now with** mouse drag + corner resize handles + maximize toggle. | `CanvasWindow.tsx` |
| **Exec approval** | `ApprovalCard.tsx` (142 LOC) with 3-tier model: Deny / Allow Once / Allow Session. `TauriToolBridge` (343 LOC) with session-permission cache and 5-min timeout. **Superior to OpenClaw's** binary approve/deny. | `ApprovalCard.tsx`, `tool_bridge.rs` |
| **Instances/Fleet** | `FleetCommandCenter.tsx` with real-time status polling (visibility-based throttling). `FleetGraph.tsx` visual network graph. `FleetTerminal.tsx` command broadcast. `fleet.rs` (294 LOC) with `AgentStatusSummary`, capability detection. | `fleet.rs`, `FleetCommandCenter.tsx` |
| **PTT** | Global shortcut (Cmd+Shift+V, configurable via `tauri-plugin-global-shortcut`). `useAudioRecorder` hook for mic capture. Backend emits `ptt_toggle` event → ChatProvider toggles recording → Whisper transcription → text inserted. | `lib.rs`, `ChatProvider.tsx`, `use-audio-recorder.ts` |
| **Agent events** | `LiveAgentStatus.tsx` renders tool execution, approval cards, canvas updates inline. `OpenClawEventInspector.tsx` provides dedicated raw event stream with filters, pause, and clear. | `LiveAgentStatus.tsx`, `OpenClawEventInspector.tsx` |
| **Auto-updates** | `tauri-plugin-updater` + `tauri-plugin-process` in `Cargo.toml`. Updater config in `tauri.conf.json` with signing key + GitHub Releases endpoint. `UpdateChecker.tsx` — floating notification card: auto-check 5s after launch, download progress bar, install + restart. Signing keypair at `~/.tauri/scrappy.key`. | `Cargo.toml`, `tauri.conf.json`, `lib.rs`, `UpdateChecker.tsx`, `App.tsx` |
| **Voice wake** | `VoiceWakeOverlay.tsx` + `use-voice-wake.ts` — hybrid wake stack with RMS-energy VAD plus Sherpa-ONNX keyword spotting when the wake-word backend and models are available, with automatic fallback to energy mode otherwise. Floating mic button toggles always-on listening, auto-starts recording on detection, auto-transcribes via Whisper, and inserts text into chat input. | `VoiceWakeOverlay.tsx`, `use-voice-wake.ts`, `ChatProvider.tsx`, `voice_wake.rs` |

### Owner: Scrappy Agent

---

## 13. Web Interface

| Feature | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Control UI Dashboard | ✅ | ✅ | - | Web gateway with chat, memory, jobs, logs, extensions, costs |
| Cost Dashboard (WebUI) | ❌ | ✅ | - | Dedicated Costs tab: time‑range selector (Today/7d/30d/All), daily spend bar chart, budget progress, total tokens, active models, actions/hr summary cards + per‑model horizontal bar chart (input/output split) + model breakdown table with requests, cost shares, and totals. Backed by `/api/costs/summary` (CostTracker historical data) + `/api/gateway/status` (CostGuard real-time). SSE `cost_alert` toast notifications. Secure CSV export via `/api/costs/export` (Authorization header). Cost reset via `POST /api/costs/reset` (clears CostTracker + CostGuard). Auto‑refreshes every 30s. Persistent across restarts (60s background flush + shutdown flush). Provider-sourced pricing forwarded to CostGuard budget enforcement. Entry compaction at 50K cap preserves daily/model aggregates. UTC-consistent date handling. ([`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Channel status view | ✅ | ✅ | P2 | `ChannelStatusView` with per-channel state machine, table/JSON format ([`src/channels/status_view.rs`](src/channels/status_view.rs)) |
| Agent management | ✅ | ✅ | P3 | CLI: `agents list/add/remove/show/set-default`; `AgentRouter` dispatch pipeline |
| Model selection | ✅ | ✅ | - | TUI only |
| Config editing | ✅ | ✅ | P3 | `Settings.set()/.get()/.list()/.reset()` with typed path-based access ([`src/settings.rs`](src/settings.rs)). Web gateway Settings tab with grouped sections (Notifications, Heartbeat, Agent, Channels [Telegram/Signal/Discord/Slack/Nostr/iMessage/Gmail/Gateway], Safety, Features), toggle switches, import/export. Includes `agent.main_tool_profile`, `agent.worker_tool_profile`, `agent.subagent_tool_profile`, `agent.subagent_transparency_level`, and `channels.telegram_subagent_session_mode` plumbing |
| Debug/logs viewer | ✅ | ✅ | - | Real-time log streaming with level/target filters |
| WebChat interface | ✅ | ✅ | - | Web gateway chat with SSE/WebSocket |
| Temporal subagent subsessions | ❌ | ✅ | WebUI now renders live child subsessions under active threads, with temporal transcript inspection and collapse/reopen after completion; state remains ephemeral in browser session memory (not DB-persisted) |
| Canvas system (A2UI) | ✅ | ✅ | P3 | `CanvasTool` + `CanvasStore` + canvas gateway routes for HTML/JSON rendering ([`src/channels/canvas_gateway.rs`](src/channels/canvas_gateway.rs)) |
| Control UI i18n | ✅ | ✅ | - | EN/ES/ZH/JA locales with key-based lookup + fallback |
| WebChat theme sync | ✅ | ✅ | P3 | `WebChatConfig` + `WebChatTheme` (Light/Dark/System), CSS variables, env var loading ([`src/config/webchat.rs`](src/config/webchat.rs)) |
| Partial output on abort | ✅ | ✅ | - | Extracts last 3 tool/assistant results on interrupt |

### Owner: ThinClaw Agent

---

## 14. Automation

| Feature | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Cron jobs | ✅ | ✅ | - | Routines with cron trigger |
| Cron stagger controls | ✅ | ✅ | P3 | `StaggerConfig` + `CronGate` ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs)) |
| Cron finished-run webhook | ✅ | ✅ | P3 | `FinishedRunPayload` + `notify_finished_run()` ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs)) |
| Timezone support | ✅ | ✅ | - | Via cron expressions |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers |
| Actor-private routines/jobs | ❌ | ✅ | `actor_id` persistence, actor-scoped cron/tool lookups, actor-bound routine/job ownership checks, request-scoped gateway enforcement, in-channel job listing, pairing-linked delivery, and actor-private profile/routine writes are all wired end to end |
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
| Heartbeat system | ✅ | ✅ | - | Periodic execution with self-critique feedback loop: post-completion evaluator persists critique to `heartbeat.last_critique`, next heartbeat reads and avoids repeating mistakes. Configurable `max_iterations` (WebUI Settings → Heartbeat, default 10). Stuck heartbeats write targeted self-critique and notify the user via preferred channel. |
| Notification routing | ✅ | ✅ | P1 | `NotificationSettings` (preferred_channel + recipient) in `Settings`. Per-channel broadcast validation guards. Wizard step 16 collects preferences. Heartbeat/routine notifications route to user-chosen channel. WebUI Settings tab exposes notification preferences. |
| Wizard notification preferences | ❌ | ✅ | P1 | `step_notification_preferences` in wizard: auto-selects single channel, prompts for multi-channel, collects recipient (phone/chat ID/email). |
| Gmail pub/sub | ✅ | ✅ | P3 | `GmailConfig` + `parse_pubsub_push()` + sender filtering ([`src/channels/gmail_wiring.rs`](src/channels/gmail_wiring.rs)) |

### Owner: ThinClaw Agent

---

## 15. Security Features

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Gateway token auth | ✅ | ✅ | Bearer token auth on web gateway |
| Device pairing | ✅ | ✅ | `PairingStore` with challenge-response states + fingerprint verification ([`src/safety/device_pairing.rs`](src/safety/device_pairing.rs)) |
| Tailscale identity | ✅ | ✅ | `TailscaleIdentity` + `extract_identity()` + `is_trusted_peer()` for passwordless gateway auth ([`src/tailscale.rs`](src/tailscale.rs)) |
| Trusted-proxy auth | ✅ | ✅ | `TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS` for reverse-proxy deployments |
| OAuth flows | ✅ | ✅ | Full Auth Code + PKCE flow, auto-refresh, scope aggregation, built-in Google/GitHub/Notion creds |
| DM pairing verification | ✅ | ✅ | thinclaw pairing approve, host APIs |
| Allowlist/blocklist | ✅ | ✅ | allow_from + block_from + pairing store, CLI block/unblock commands |
| Per-group tool policies | ✅ | ✅ | `ToolPolicyManager` ([`src/tools/policy.rs`](src/tools/policy.rs)) |
| Exec approvals | ✅ | ✅ | TUI overlay |
| TLS 1.3 minimum | ✅ | ✅ | reqwest rustls |
| SSRF protection | ✅ | ✅ | WASM allowlist |
| SSRF IPv6 transition bypass block | ✅ | ✅ | IPv4-mapped IPv6 (::ffff:x.x.x.x) detection in `is_ipv4_mapped_v6_private()` |
| Cron webhook SSRF guard | ✅ | ✅ | N/A — webhooks are inbound-only; no outbound delivery to user URLs |
| Loopback-first | ✅ | ✅ | Gateway binds 127.0.0.1 by default, HTTP webhook binds 0.0.0.0 for inbound |
| Docker sandbox | ✅ | ✅ | Orchestrator/worker containers with HEALTHCHECK, startup orphan cleanup, wizard auto-build, Claude Code keychain auth ([`src/orchestrator/job_manager.rs`](src/orchestrator/job_manager.rs), [`Dockerfile.worker`](Dockerfile.worker)) |
| Podman support | ✅ | ✅ | `PodmanConfig` with rootless mode, resource limits, `podman run` arg building ([`src/sandbox/podman.rs`](src/sandbox/podman.rs)) |
| WASM sandbox | ❌ | ✅ | ThinClaw innovation |
| Sandbox env sanitization | ✅ | ✅ | Shell tool scrubs env vars + LD*/DYLD* injection blocks + safe bins allowlist |
| Workspace sandbox modes | ❌ | ✅ | `WORKSPACE_MODE` (unrestricted/sandboxed/project) + `WORKSPACE_ROOT` — file tools + shell confined to workspace when sandboxed ([`src/config/agent.rs`](src/config/agent.rs), [`src/tools/registry.rs`](src/tools/registry.rs)) |
| Shell sandbox (3-layer) | ❌ | ✅ | When `base_dir` set: (1) Safe bins allowlist auto-enforced, (2) workdir validation (must be under base_dir), (3) command path scanning (`detect_path_escape` + `..` traversal detection); plus a first-party pre-exec external shell scanner (`thinclaw-shell-scan`) resolved from configured path / `PATH` / bundled asset / verified cache, with SHA-256-checked cached install, 24h failure cooldown, and fail-open/fail-closed enforcement before smart approval ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs), [`src/tools/builtin/shell_security.rs`](src/tools/builtin/shell_security.rs), [`src/bin/thinclaw-shell-scan.rs`](src/bin/thinclaw-shell-scan.rs)) |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ✅ | ([`src/safety/elevated.rs`](src/safety/elevated.rs)) |
| Safe bins allowlist | ✅ | ✅ | `THINCLAW_SAFE_BINS_ONLY` + extensible `THINCLAW_EXTRA_BINS` + desktop bins (open, xdg-open, pbcopy, pbpaste, tee, xargs, chmod) |
| LD*/DYLD* validation | ✅ | ✅ | Blocks `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, etc. |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) + `..` traversal in shell commands |
| Credential theft via env injection | ✅ | ✅ | Shell env scrubbing + command injection detection + LD*/DYLD* blocking + safe bins |
| Session file permissions (0o600) | ✅ | ✅ | Handled by OS keychain + filesystem perms |
| Skill download path restriction | ✅ | ✅ | ([`src/safety/skill_path.rs`](src/safety/skill_path.rs)) |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ✅ | SSRF prevention: blocks private IPs, cloud metadata, disallowed schemes ([`src/safety/media_url.rs`](src/safety/media_url.rs)) |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization, and context-file scanning for AGENTS.md/SOUL.md/USER.md with invisible Unicode stripping ([`src/safety/sanitizer.rs`](src/safety/sanitizer.rs), [`src/workspace/workspace_core.rs`](src/workspace/workspace_core.rs)) |
| Prompt-surface PII redaction | ✅ | ✅ | System prompt assembly now hashes actor/conversation identifiers for non-Discord channels and replaces raw actor-private path labels with prompt-safe aliases ([`src/safety/pii_redactor.rs`](src/safety/pii_redactor.rs), [`src/workspace/workspace_core.rs`](src/workspace/workspace_core.rs), [`src/agent/dispatcher.rs`](src/agent/dispatcher.rs)) |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ✅ | `DangerousToolTracker` ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs)) |
| OS governance env vars | ❌ | ✅ | Scrappy live-checks macOS permissions and passes to ThinClaw: `ACCESSIBILITY_GRANTED` (AXIsProcessTrusted), `SCREEN_RECORDING_GRANTED` (CGPreflightScreenCaptureAccess), `SCREEN_CAPTURE_ENABLED` (user toggle) |

### Owner: ThinClaw Agent

---

## 16. Development & Build System

| Feature | OpenClaw | ThinClaw | Notes |
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
| Docker: Chromium + Xvfb | ✅ | ✅ | `DockerChromiumConfig` with Xvfb + Chrome launch args, full container lifecycle (start/stop/health-check), wired into `BrowserTool` as automatic fallback when no local Chrome binary found. Forced via `BROWSER_DOCKER=always` env var. ([`src/sandbox/docker_chromium.rs`](src/sandbox/docker_chromium.rs), [`src/tools/builtin/browser.rs`](src/tools/builtin/browser.rs)) |
| Docker: init scripts | ✅ | ✅ | `DockerInitConfig` with script discovery + interpreter detection ([`src/sandbox/docker_init.rs`](src/sandbox/docker_init.rs)) |
| Browser: extraArgs config | ✅ | ✅ | `BrowserArgsConfig` with custom Chrome flags, proxy, hardening ([`src/tools/browser_args.rs`](src/tools/browser_args.rs)) |
| Browser: managed cloud CDP providers | ✅ | ✅ | `BrowserTool` now supports Browserbase session provisioning/release and Browser Use remote CDP URLs via `cloud_browser_provider`, while preserving local Chrome and Docker fallback. ([`src/tools/builtin/browser.rs`](src/tools/builtin/browser.rs), [`src/tools/builtin/browser_cloud.rs`](src/tools/builtin/browser_cloud.rs), [`src/config/agent.rs`](src/config/agent.rs)) |

### Owner: ThinClaw Agent

---

## 17. Scrappy Feature Surfacing Analysis

> **Last updated:** 2026-04-14 — parity-audited against current ThinClaw contracts and Tauri wiring
>
> ThinClaw now exposes a broad Scrappy-facing contract surface in this repo.
> This section tracks ThinClaw-side wiring, expected Scrappy feature surfacing, and the integration seams that should be revalidated in the external Scrappy repo when UI behavior changes.

### 17.1 Features Scrappy Has UI For (Wired)

| ThinClaw Feature | Scrappy UI Component | Status |
|-----------------|---------------------|--------|
| Skills system (§5, §8) | `OpenClawSkills.tsx` — list, toggle, install | ✅ Wired via `openclaw_skills_*` commands |
| Memory/Workspace (§10) | `OpenClawMemory.tsx`, `MemoryEditor.tsx` | ✅ Wired via `openclaw_get_memory`, `openclaw_save_memory` |
| Cron/Routines (§14) | `OpenClawAutomations.tsx` | ✅ Wired via `openclaw_cron_*` commands |
| Channels (§3) | `OpenClawChannels.tsx` | ✅ Wired via ThinClaw-side channel status/list surfaces, primarily `openclaw_channel_status_list` |
| Subagent support (§5) | `SubAgentPanel.tsx` | ✅ Wired via Enhancement 2C |
| Workspace sandbox modes (§5, §15) | Settings panel — unrestricted/sandboxed/project | ✅ Sets `WORKSPACE_MODE` + `WORKSPACE_ROOT` env vars |
| OS governance (§15) | Settings panel — screen capture toggle, permission checks | ✅ `ACCESSIBILITY_GRANTED`, `SCREEN_RECORDING_GRANTED`, `SCREEN_CAPTURE_ENABLED` |
| Screen capture (§5) | Gated on `SCREEN_CAPTURE_ENABLED` toggle | ✅ Wired via OS governance toggle |
| Fleet/presence (§2, §12) | `FleetCommandCenter.tsx`, `FleetGraph.tsx` | ✅ Wired |
| Canvas (§2, §12) | `CanvasWindow.tsx` | ✅ Wired with drag/resize/maximize |
| Exec approvals (§5, §15) | `ApprovalCard.tsx` | ✅ Wired, 3-tier |
| Thinking mode (§5) | Toggle + budget slider in `OpenClawChatView.tsx` | ✅ Native ThinClaw `ThinkingConfig` via `openclaw_set_thinking` command |
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
| Channel listing (§3) | `OpenClawChannels.tsx` — channel inventory and status cards | ✅ Wired via `openclaw_channel_status_list` |
| Stream mode config (§3) | Per-channel stream mode selector in `OpenClawChannels.tsx` (Full/Typing/Disabled) | ✅ Wired via `setSetting` API |
| Cron lint validator (§14) | Expression validator in `OpenClawAutomations.tsx` with next-5-fire-times display | ✅ Wired via `openclaw_cron_lint` command |

### 17.2 Additional ThinClaw Features Scrappy Surfacing Tracks

> ThinClaw-side wiring for the features below is present in this repo.
> Treat the UI status here as an integration snapshot, not a substitute for checking the external Scrappy repo before release.

| ThinClaw Feature | API / Module | Scrappy UI | ThinClaw Wiring | Status |
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

### 17.3 Tauri Command Wiring Snapshot

> 22+ Tauri commands are wired via the unified `tauri_commands` facade ([`src/tauri_commands.rs`](src/tauri_commands.rs)).
> All Tauri commands use the `openclaw_*` prefix. Pure Tauri IPC — no REST gateway.
> Scrappy uses `specta` for type generation via `bindings.ts`.
> Channel-facing UI currently relies on `openclaw_channel_status_list`; broader inventory/status parity should still be revalidated whenever the external Scrappy repo changes.

### Owner: Scrappy Agent

---

## 18. Scrappy-Exclusive Features

> These features exist only in Scrappy (Tauri) and are **not documented in ThinClaw**.
> They represent Scrappy's own value proposition beyond the ThinClaw engine.

| Feature | Components | Status |
|---------|-----------|--------|
| **Image generation (Imagine)** | `ImagineView.tsx`, `ImagineGeneration.tsx`, `ImagineGallery.tsx`, `ImagineSidebar.tsx` | ✅ Scrappy-only |
| **Local inference engine mgmt** | Sidecar manager — llama.cpp, whisper-server, embedding-server, stable-diffusion | ✅ Scrappy-only |
| **Model library + HF discovery** | `ModelBrowser.tsx`, `HFDiscovery.tsx`, `model-library.ts` (600+ LOC model catalog) | ✅ Scrappy-only |
| **Chat (Rig-based)** | `ChatView.tsx`, `use-chat.ts` — independent chat pipeline, not ThinClaw agent | ✅ Scrappy-only |
| **Cloud storage** | 7 providers (S3, GCS, Azure, B2, R2, MinIO, Storj), sync, client-side encryption | ✅ Scrappy-only |
| **Cloud model discovery** | `use-cloud-models.ts` — discover models from cloud providers | ✅ Scrappy-only |
| **Spotlight bar** | `SpotlightBar.tsx` + global shortcut (Cmd+Shift+K) — quick-access floating chat | ✅ Scrappy-only |
| **Onboarding wizard** | `OnboardingWizard.tsx` — first-run setup flow | ✅ Scrappy-only |
| **MCP integration** | `McpTab.tsx` — Model Context Protocol server management | ✅ Scrappy-only |
| **Remote deploy wizard** | `RemoteDeployWizard.tsx` — deploy agent to remote infrastructure | ✅ Scrappy-only |
| **Inference mode tabs** | `InferenceModeTab.tsx` — local / cloud / hybrid mode selection | ✅ Scrappy-only |
| **Persona system** | `PersonaTab.tsx`, `PersonalizationTab.tsx` — custom AI personas | ✅ Scrappy-only |
| **Web search with progress** | `web_search` tool with `WebSearchBubble` progress events | ✅ Scrappy-only | `brave-search` WASM tool available in `tools-src/brave-search/` — `web_search` + `news_search` actions, API key via CLI (`thinclaw tool auth brave-search`) or WebUI inline auth prompt; hot-reloads after key entry |
| **RAG pipeline** | `rag.rs` — document ingestion, chunking, vector retrieval | ✅ Scrappy-only |
| **Secrets management** | `SecretsTab.tsx` — macOS Keychain integration for API keys | ✅ Scrappy-only |
| **Settings system** | 10+ settings tabs (Chat Provider, Engine, Storage, MCP, Gateway, etc.) | ✅ Scrappy-only |

### Owner: Scrappy Agent

---

## 19. ThinClaw → Scrappy Integration Tracker

> **Last updated:** 2026-03-25 — all sprint items complete (28 features end-to-end). 22+ Tauri commands wired.

### 19.1 Active Integration Gaps

| ThinClaw Gap | Priority | Scrappy Impact When Shipped |
|-------------|----------|----------------------------|
| **Multimodal media pipeline** | ✅ Done | Telegram/channel → binary download → rig-core multimodal. Frontend rendering for images/PDFs/audio in chat bubbles |
| **WhatsApp channel** | ✅ Done | Cloud API webhook — text, media, DM pairing, reply threading, formatting |
| **APNs push pipeline** | Deferred | iOS push wake — needs Apple Developer cert infra |

### 19.2 Future Considerations

| Item | Notes | Status |
|------|-------|--------|
| **Session pruning UI** | Pruning config in settings (low priority) | 🔮 |
| **Per-conversation channel scoping** | Optional optimization: replace `app.emit()` broadcast with Tauri V2 `Channel<T>` per-invoke | 🔮 |

### Owner: Both (ThinClaw ships backend, Scrappy ships frontend)

---

## §21 — Scrappy ↔ ThinClaw Communication Architecture

> **Status:** ✅ **Already implemented** — Verified by Scrappy Agent audit, 2026-03-05

### 21.1 Actual Architecture (verified ✅)

```text
┌──────────────── Scrappy.app (single macOS process) ─────────────────────┐
│                                                                          │
│  ┌─── React Webview ───────┐         ┌─── Tauri Rust Backend ─────────┐ │
│  │                          │         │                                │ │
│  │  invoke("openclaw_*")   │─ IPC ──►│  rpc.rs                        │ │
│  │  (140+ commands via IPC)  │         │  ├─ ironclaw::api::chat::      │ │
│  │                          │         │  │   send_message()             │ │
│  │  listen("openclaw-event")│◄─ emit ─│  ├─ ironclaw::api::sessions::* │ │
│  │  (8 listener components) │         │  ├─ ironclaw::api::config::*   │ │
│  │                          │         │  └─ ironclaw::tauri_commands::* │ │
│  │                          │         │                                │ │
│  │                          │         │  ironclaw_channel.rs (329 LOC) │ │
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
| **Management commands** | `invoke("openclaw_*")` (140+ commands) | `ironclaw::tauri_commands::*` facade functions — all via Tauri IPC |
| **Tool approvals** | `invoke("openclaw_resolve_approval")` | → `ironclaw::api::chat::resolve_approval()` |
| **Session management** | `invoke("openclaw_*_session")` | → `ironclaw::api::sessions::*` |
| **Hardware access** | `ToolBridge` trait (in-process) | Camera, microphone, screen via `BridgedTool` |

The `GatewayChannel` (axum HTTP server) is **only** used in standalone mode
(`thinclaw run` from CLI) for browser access. It is **never started** when
running inside Scrappy.

### 21.3 What Was Already Built (Scrappy Side)

- `ironclaw_channel.rs` (329 LOC) — Implements ThinClaw's `Channel` trait for Tauri
- `rpc.rs` — 140+ Tauri commands, all using `invoke()` IPC
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

## Implementation Priorities (ThinClaw)

> **Last updated:** 2026-04-14 — All P0–P3 complete. P5–P7 complete. See §21 for Scrappy architecture.
>
> All open ThinClaw work aggregated from project artifacts into a single prioritized list.
> Scrappy = macOS desktop app (Tauri/Rust). May later add a native Swift macOS app separately.

### P0 - Core (Complete ✅)
- ✅ TUI channel with approval overlays
- ✅ HTTP webhook channel
- ✅ DM pairing (thinclaw pairing list/approve, host APIs)
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
- ✅ Remote skill federation + quarantine pipeline (`GitHubSkillSource`, `WellKnownSkillSource`, `QuarantineManager`, provenance lock files, risky-install approval gate)
- ✅ Accessibility-tree browser backend + managed cloud browser routing (`agent-browser`, Browserbase, Browser Use)
- ✅ Anthropic prompt caching (`with_prompt_caching()` compatibility path + provider-scoped message metadata seam with Anthropic `cache_control` hints on supported providers)
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
- ✅ Safe bins allowlist — `THINCLAW_SAFE_BINS_ONLY` mode with extensible `THINCLAW_EXTRA_BINS`
- ✅ LD*/DYLD* environment variable validation — blocks library injection attacks via `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES` etc.
- ✅ TAURI_INTEGRATION.md §9–§11 verified against code — comprehensive and accurate
- ✅ Allowlist/blocklist — allow_from + block_from + pairing store (blocklist precedence), CLI `pairing block/unblock/blocked`
- ✅ OAuth flows — full Auth Code + PKCE browser flow ([`src/cli/tool.rs`](src/cli/tool.rs): `auth_tool_oauth()`, 192 LOC), auto-refresh (`OAuthRefreshConfig`), scope aggregation across tools, built-in Google/GitHub/Notion credentials ([`src/cli/oauth_defaults.rs`](src/cli/oauth_defaults.rs), 571 LOC), token storage with expiry
- ✅ Watched OAuth credential sync — Claude Code, Codex, and custom JSON auth files can prime injected provider credentials, merge updated tokens into the live runtime overlay, and trigger runtime reloads without restart ([`src/llm/credential_sync.rs`](src/llm/credential_sync.rs), [`src/app.rs`](src/app.rs), [`src/config/mod.rs`](src/config/mod.rs))
- ✅ External pre-exec shell scanner — `thinclaw-shell-scan` runs before smart approval, supports configured/PATH/bundled/cached resolution, SHA-256-verified cache installs, and fail-open / fail-closed modes ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs), [`src/tools/builtin/shell_security.rs`](src/tools/builtin/shell_security.rs), [`src/bin/thinclaw-shell-scan.rs`](src/bin/thinclaw-shell-scan.rs))

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
- ✅ Streaming draft replies — End-to-end: `respond_with_tools_streaming` in Reasoning, agent loop integration in dispatcher, DraftReplyState send-then-edit with HTML formatting, persistent draft across tool-call iterations, sequential mpsc chunk processing (FIFO guaranteed), overflow detection with fallback to `on_respond()`, `delete_message` cleanup, Discord + Telegram `send_draft`
- ✅ Per-channel stream mode config — Hot-reloadable WebUI `/api/settings` integration (`telegram_stream_mode`) or env vars; `ChannelManager::set_channel_stream_mode()` runtime delegation
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
- ✅ CLI: filesystem checkpoints + `/rollback` — shadow git snapshots, per-turn dedup, restore/diff/list support ([`src/agent/checkpoint.rs`](src/agent/checkpoint.rs), [`src/agent/commands.rs`](src/agent/commands.rs), [`src/tools/builtin/file.rs`](src/tools/builtin/file.rs))
- ✅ CLI: `browser` automation — headless Chrome open/screenshot/links/check with DOM extraction ([`src/cli/browser.rs`](src/cli/browser.rs))
- ✅ CLI: `sessions export` — markdown/JSON transcript export with role labels and timestamps ([`src/cli/sessions.rs`](src/cli/sessions.rs))
- ✅ CLI: `models` — list/info/test plus `models verify` for live remote provider discovery + chat probes ([`src/cli/models.rs`](src/cli/models.rs))

**LLM & Inference**
- ✅ Gemini embeddings — `EmbeddingConfig::gemini()` ([`src/llm/embeddings.rs`](src/llm/embeddings.rs))
- ✅ Local embeddings (on-device) — `EmbeddingConfig::local()` + Ollama support ([`src/llm/embeddings.rs`](src/llm/embeddings.rs))
- ✅ AWS Bedrock provider — native Mantle OpenAI-compatible path with legacy proxy fallback ([`src/llm/provider_factory.rs`](src/llm/provider_factory.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs))
- ✅ Google Gemini provider — AI Studio adapter with system instruction + generation config ([`src/llm/gemini.rs`](src/llm/gemini.rs))
- ✅ Anthropic 1M context beta header — `ExtendedContextConfig` ([`src/llm/extended_context.rs`](src/llm/extended_context.rs))
- ✅ Auto model discovery from endpoints ([`src/llm/discovery.rs`](src/llm/discovery.rs): OpenAI/Anthropic/Ollama plus provider-specific Cohere discovery and native Bedrock-compatible endpoint scanning)
- ✅ `llama.cpp` server-mode integration scaffold — `LlamaModel` trait, `LlamaConfig` (GPU layers, sampling, context length), `ModelLoadStatus`, `LlamaCppStub`, and provider-factory wiring through `llama-server` / OpenAI-compatible endpoints ([`src/llm/llama_cpp.rs`](src/llm/llama_cpp.rs), [`src/llm/provider_factory.rs`](src/llm/provider_factory.rs))
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
- ✅ Safe bins allowlist (`THINCLAW_SAFE_BINS_ONLY` mode + desktop-essential bins: `open`, `xdg-open`, `pbcopy`, `pbpaste`, `tee`, `xargs`, `chmod`, `realpath`, `basename`, `dirname`)
- ✅ LD*/DYLD* env validation (library injection blocking)
- ✅ Per-group tool policies ([`src/tools/policy.rs`](src/tools/policy.rs): AllowAll/AllowList/DenyList with group→channel→global evaluation, serializable config)
- ✅ Elevated execution mode — `ElevatedMode` with timeout + command allowlist ([`src/safety/elevated.rs`](src/safety/elevated.rs))
- ✅ Skill download path restriction — `SkillPathConfig` with path traversal prevention, symlink detection, name sanitization ([`src/safety/skill_path.rs`](src/safety/skill_path.rs))
- ✅ Dangerous tool re-enable warning — `DangerousToolTracker` with state history and warning generation ([`src/safety/dangerous_tools.rs`](src/safety/dangerous_tools.rs))
- ✅ Workspace sandbox modes — `WORKSPACE_MODE` (unrestricted/sandboxed/project) with `WORKSPACE_ROOT`; drives `register_dev_tools_with_config()` + dynamic system prompt ([`src/config/agent.rs`](src/config/agent.rs))
- ✅ Shell 3-layer sandbox — When `base_dir` set: (1) `check_safe_bins_forced()`, (2) workdir parameter validation, (3) `detect_path_escape()` with `..` traversal detection. 11 tests ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs))
- ✅ External pre-exec shell scanner — first-party `thinclaw-shell-scan` with configured/PATH/bundled/cached resolution, SHA-256-verified cache install, 24h failure cooldown, and fail-open / fail-closed enforcement before smart approval ([`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs), [`src/tools/builtin/shell_security.rs`](src/tools/builtin/shell_security.rs), [`src/bin/thinclaw-shell-scan.rs`](src/bin/thinclaw-shell-scan.rs))
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
- ❌ Public plugin HTTP-route registry — not exposed in the current ThinClaw runtime
- ❌ Public auth / memory / provider plugin trait surface — not exposed in the current ThinClaw runtime

**Housekeeping**
- ✅ `Default` derives for TtsProvider, TtsOutputFormat (clippy-driven)
- ✅ JSON5 / YAML config support — ([`src/config/formats.rs`](src/config/formats.rs))

### P4 - Postponed
- ❌ Slack channel (native implementation — currently WASM tool)
- ✅ WhatsApp channel — WASM Cloud API channel (1449 LOC, [`channels-src/whatsapp/src/lib.rs`](channels-src/whatsapp/src/lib.rs)) with text/media/document/sticker, DM pairing, reply threading, markdown formatting, 3 tests
- ✅ iMessage channel — `IMessageChannel` (720 LOC, [`src/channels/imessage.rs`](src/channels/imessage.rs)) with chat.db polling + osascript sending, group chats, attachments, dedup, diagnostics, 23 tests
- ❌ Other messaging platforms (LINE, Feishu/Lark, Google Chat, MS Teams, Twitch)

### P5 - Scrappy (Tauri) Integration — ✅ ThinClaw-Side Contract Complete
> ThinClaw-side Tauri commands, macOS-facing hooks, and integration surfaces are present in this repo.
> External Scrappy UI behavior should still be revalidated against the Scrappy repo. See §17 for feature surfacing details and §21 for IPC architecture.

### P6 - Tauri IPC Architecture — ✅ ThinClaw-Side Complete
> Pure in-process Tauri IPC (no HTTP/SSE/GatewayChannel) is implemented on the ThinClaw side. See §21 for architecture docs.

### P7 - Sprint 15 (Gmail + Routing) — ✅ ThinClaw-Side Complete
> Gmail PKCE OAuth and routing CRUD are complete on the ThinClaw side. Scrappy-side release readiness should be confirmed in the external repo.

---

## Engineering Infrastructure (IronClaw Parity)

> Added in v0.14 sprint to close engineering gaps vs IronClaw.

| Feature | IronClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Dependency auditing (`cargo-deny`) | ✅ | ✅ | `deny.toml` — CVE, license, ban, source checks |
| Code coverage (Codecov) | ✅ | ✅ | `codecov.yml` — 60% project / 70% patch targets |
| Complexity guardrails (`clippy.toml`) | ✅ | ✅ | cognitive-complexity 15, too-many-lines 100 |
| Feature flag restructuring (light default) | ✅ | ✅ | `default = ["light"]`; `docs/BUILD_PROFILES.md` |
| Document extraction (pdf-extract) | ✅ | ✅ | `document-extraction` feature flag; PDF/DOCX/PPTX/XLSX |
| Document extraction middleware | ✅ | ✅ | `DocumentExtractor` wired into `MediaPipeline`; auto-routes `Document` type attachments |
| `extract_document` agent tool | ✅ | ✅ | Built-in tool: URL fetch or base64 → text extraction (PDF/DOCX/PPTX/XLSX/text) |
| Timezone handling (chrono-tz) | ✅ | ✅ | `src/timezone.rs` — priority chain, system detection |
| PG TLS (tokio-postgres-rustls) | ✅ | ✅ | Auto-negotiate prefer semantics; wired into `Store::new()` pool creation |
| HMAC webhook signing | ✅ | ✅ | `src/hooks/webhook_signing.rs` — sign + verify |
| ed25519 manifest signing | ✅ | ✅ | `src/extensions/signing.rs` — verify + hex helpers |
| Musl static builds | ✅ | ✅ | 7 dist targets including musl (aarch64 + x86_64) |
| Fuzz testing | ✅ | ✅ | 4 targets (sanitizer, leak, validator, credential), CI-integrated |
| Benchmarks (criterion) | ✅ | ✅ | 2 bench targets (`safety_sanitize`, `safety_pipeline`) |
| Snapshot testing (insta) | ✅ | ✅ | 8 snapshot tests: Settings, Validation, MediaType, Timezone, Errors, Tool schema |
| Tenant isolation (compile-time) | ✅ | ❌ | IronClaw: `TenantScope` — deferred (ThinClaw is single-user) |
| Psychographic profiling | ✅ | ✅ | `PsychographicProfile` (9-dimension analysis) + `profile_evolution` (weekly cron prompt) + tiered system prompt injection (confidence-gated: skip <0.3, basics 0.3–0.6, full >0.6). [`src/profile.rs`](src/profile.rs), [`src/profile_evolution.rs`](src/profile_evolution.rs), [`src/workspace/workspace_core.rs`](src/workspace/workspace_core.rs) |
| SIGHUP hot-reload | ✅ | ✅ | Unix SIGHUP handler: refresh secrets overlay, reload `Config` from DB, two-phase TCP listener swap for zero-downtime webhook rebind. [`src/main.rs`](src/main.rs), [`src/channels/webhook_server.rs`](src/channels/webhook_server.rs) |
| WASM channel persistence | ✅ | ✅ | Active WASM channels serialized to DB `settings` on activation; auto-reactivated on restart. [`src/extensions/manager.rs`](src/extensions/manager.rs), [`src/main.rs`](src/main.rs) |
| TOOLS.md workspace seeding | ✅ | ✅ | `paths::TOOLS` constant; tool descriptions seeded to workspace during bootstrap. [`src/workspace/document.rs`](src/workspace/document.rs) |

### Owner: ThinClaw Agent

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

ThinClaw intentionally differs from OpenClaw in these ways:

1. **Rust vs TypeScript**: Native performance, memory safety, single binary distribution
2. **WASM sandbox vs Docker**: Lighter weight, faster startup, capability-based security
3. **PostgreSQL + libSQL vs SQLite**: Dual-backend (production PG + embedded libSQL for zero-dep local mode)
4. **Provider agnostic**: No vendor lock-in — works with any OpenAI-compatible endpoint (default), Anthropic, OpenAI, Ollama, Tinfoil, or OpenRouter
5. **No mobile/desktop apps**: Focus on server-side and CLI; Scrappy (Tauri/Rust) provides the macOS desktop experience. A native Swift macOS app may be added later.
6. **WASM channels**: Novel extension mechanism not in OpenClaw
7. **Tinfoil private inference**: ThinClaw-only provider for private/encrypted inference
8. **GitHub WASM tool**: Native GitHub integration as WASM tool
9. **Prompt-based skills**: Different approach than OpenClaw capability bundles (trust gating, attenuation)
10. **OS Keychain secrets**: API keys stored in macOS Keychain / Linux Secret Service rather than encrypted file

These are intentional architectural choices, not gaps to be filled.

---

## 20. Shipped Built-in Tools (75 max; some conditional or feature-gated)

> **Updated:** 2026-04-14

### 20.1 File & Code Operations (9 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `read_file` | [`file.rs`](src/tools/builtin/file.rs) | Read file contents with line ranges |
| `write_file` | [`file.rs`](src/tools/builtin/file.rs) | Write/create files with safety checks |
| `apply_patch` | [`file.rs`](src/tools/builtin/file.rs) | Apply unified diffs to files |
| `list_dir` | [`file.rs`](src/tools/builtin/file.rs) | List directory contents recursively |
| `grep` | [`file.rs`](src/tools/builtin/file.rs) | Regex/literal content search across files |
| `search_files` | [`search_files.rs`](src/tools/builtin/search_files.rs) | Filename pattern search with fuzzy suggestions |
| `execute_code` | [`execute_code.rs`](src/tools/builtin/execute_code.rs) | Sandboxed code execution (Python/JS/TS/Bash); scrubbed env, timeouts, output capture |
| `shell` | [`shell.rs`](src/tools/builtin/shell.rs) | Shell command execution with env scrubbing, timeout enforcement |
| `json` | [`json.rs`](src/tools/builtin/json.rs) | JSON parsing, formatting, and query |

### 20.2 Browser & Web (2 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `browser` | [`browser.rs`](src/tools/builtin/browser.rs) | Full browser automation across local Chrome, Docker Chromium, `agent-browser`, and managed cloud CDP providers (Browserbase / Browser Use): navigate, snapshot, click, type, press_key, scroll, back, forward, screenshot, evaluate JS, get_text, get_images, console, tabs, switch_tab, close |
| `http` | [`http.rs`](src/tools/builtin/http.rs) | HTTP client with SSRF protection, credential injection, URL allowlists |

### 20.3 Memory & Knowledge (7 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `memory_read` | [`memory.rs`](src/tools/builtin/memory.rs) | Read memory documents |
| `memory_write` | [`memory.rs`](src/tools/builtin/memory.rs) | Write/update memory documents |
| `memory_search` | [`memory.rs`](src/tools/builtin/memory.rs) | Semantic search across memory |
| `memory_delete` | [`memory.rs`](src/tools/builtin/memory.rs) | Delete memory documents |
| `memory_tree` | [`memory.rs`](src/tools/builtin/memory.rs) | Browse memory namespace hierarchy |
| `session_search` | [`memory.rs`](src/tools/builtin/memory.rs) | Search past conversation sessions |
| `extract_document` | [`extract_document.rs`](src/tools/builtin/extract_document.rs) | Extract text/tables from PDFs and documents (feature-gated) |

### 20.4 Agent & Sub-agent Management (8 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `create_agent` | [`agent_management.rs`](src/tools/builtin/agent_management.rs) | Create persistent agents with config |
| `list_agents` | [`agent_management.rs`](src/tools/builtin/agent_management.rs) | List registered agents |
| `update_agent` | [`agent_management.rs`](src/tools/builtin/agent_management.rs) | Update agent configuration |
| `remove_agent` | [`agent_management.rs`](src/tools/builtin/agent_management.rs) | Remove a persistent agent |
| `message_agent` | [`agent_management.rs`](src/tools/builtin/agent_management.rs) | A2A communication via dispatcher |
| `spawn_subagent` | [`subagent.rs`](src/tools/builtin/subagent.rs) | Spawn sync/async sub-agent loops |
| `list_subagents` | [`subagent.rs`](src/tools/builtin/subagent.rs) | Query active/recent sub-agents |
| `cancel_subagent` | [`subagent.rs`](src/tools/builtin/subagent.rs) | Cancel running sub-agent by UUID |

### 20.5 Jobs & Routines (11 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `create_job` | [`job.rs`](src/tools/builtin/job.rs) | Create deferred jobs |
| `cancel_job` | [`job.rs`](src/tools/builtin/job.rs) | Cancel pending/running jobs |
| `list_jobs` | [`job.rs`](src/tools/builtin/job.rs) | List jobs with filters |
| `job_status` | [`job.rs`](src/tools/builtin/job.rs) | Get job status and results |
| `job_events` | [`job.rs`](src/tools/builtin/job.rs) | Get job event stream |
| `job_prompt` | [`job.rs`](src/tools/builtin/job.rs) | Send interactive prompts to jobs |
| `routine_create` | [`routine.rs`](src/tools/builtin/routine.rs) | Create cron-scheduled routines |
| `routine_list` | [`routine.rs`](src/tools/builtin/routine.rs) | List active routines |
| `routine_delete` | [`routine.rs`](src/tools/builtin/routine.rs) | Delete a routine |
| `routine_update` | [`routine.rs`](src/tools/builtin/routine.rs) | Update routine config |
| `routine_history` | [`routine.rs`](src/tools/builtin/routine.rs) | View routine execution history |

### 20.6 Skills & Extensions (12 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `skill_install` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | Install prompt-based skills from direct content, ClawHub, configured GitHub taps, or configured `/.well-known/skills` registries with quarantine scanning, provenance lock files, and risky-install approval gates |
| `skill_list` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | List installed skills |
| `skill_read` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | Read skill source |
| `skill_remove` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | Remove a skill |
| `skill_reload` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | Hot-reload skills from disk |
| `skill_search` | [`skill_tools.rs`](src/tools/builtin/skill_tools.rs) | Search local skills, ClawHub, configured GitHub taps, and configured `/.well-known/skills` registries |
| `tool_install` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | Install WASM tool extensions |
| `tool_list` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | List installed WASM tools |
| `tool_search` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | Search tool registry |
| `tool_remove` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | Remove a WASM tool |
| `tool_activate` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | Activate/deactivate tools |
| `tool_auth` | [`extension_tools.rs`](src/tools/builtin/extension_tools.rs) | Manage tool credentials |

### 20.7 Learning & Self-Improvement (6 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `learning_status` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | View learning system status |
| `learning_history` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | Browse learning history |
| `learning_feedback` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | Submit learning feedback |
| `learning_proposal_review` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | Review/approve learning proposals |
| `prompt_manage` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | Manage system prompt overrides |
| `skill_manage` | [`learning_tools.rs`](src/tools/builtin/learning_tools.rs) | Manage skill lifecycle via learning |

### 20.8 Messaging & Communication (2 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `send_message` | [`send_message.rs`](src/tools/builtin/send_message.rs) | Unified cross-platform messaging (Discord/Slack/Telegram/etc.) via gateway callback |
| `apple_mail` | [`apple_mail.rs`](src/tools/builtin/apple_mail.rs) | Read/send Apple Mail via AppleScript |

### 20.9 LLM & Reasoning (5 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `llm_select` | [`llm_tools.rs`](src/tools/builtin/llm_tools.rs) | Switch active LLM model at runtime |
| `llm_list_models` | [`llm_tools.rs`](src/tools/builtin/llm_tools.rs) | List available models across providers |
| `mixture_of_agents` | [`moa.rs`](src/tools/builtin/moa.rs) | Multi-LLM parallel dispatch + synthesis aggregation |
| `consult_advisor` | [`advisor.rs`](src/tools/builtin/advisor.rs) | Advisor-executor consultation tool, registered conditionally when routing mode uses the advisor lane |
| `agent_think` | [`agent_control.rs`](src/tools/builtin/agent_control.rs) | Explicit reasoning scratchpad (implicit capability tool) |

### 20.10 Hardware & Environment (3 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `device_info` | [`device_info.rs`](src/tools/builtin/device_info.rs) | System hardware/OS info |
| `screen_capture` | [`screen_capture.rs`](src/tools/builtin/screen_capture.rs) | macOS/Linux screenshot capture |
| `tts` | [`tts.rs`](src/tools/builtin/tts.rs) | Text-to-speech synthesis |

Source-present but not currently registered as built-in runtime tools: `slack_actions`, `discord_actions`, `telegram_actions`, `location`, and `camera_capture` ([`src/tools/builtin/mod.rs`](src/tools/builtin/mod.rs), [`src/tools/registry.rs`](src/tools/registry.rs), [`src/app.rs`](src/app.rs), [`src/main.rs`](src/main.rs)).

### 20.11 UI & Interaction (4 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `canvas` | [`canvas.rs`](src/tools/builtin/canvas.rs) | A2UI canvas panels with interactive components |
| `clarify` | [`clarify.rs`](src/tools/builtin/clarify.rs) | Structured questions with multiple-choice/free-form options |
| `vision_analyze` | [`vision.rs`](src/tools/builtin/vision.rs) | Proactive image/video analysis via multimodal LLM |
| `emit_user_message` | [`agent_control.rs`](src/tools/builtin/agent_control.rs) | Emit messages to user (implicit capability tool) |

### 20.12 Process & Task Management (3 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `process` | [`process.rs`](src/tools/builtin/process.rs) | Background process lifecycle (start/poll/wait/kill/write) with shared registry + auto-reaper |
| `todo` | [`todo.rs`](src/tools/builtin/todo.rs) | In-session task planner with merge/replace modes; survives context compaction |
| `time` | [`time.rs`](src/tools/builtin/time.rs) | Current time, timezone conversion, date arithmetic |

### 20.13 Smart Home & IoT (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `homeassistant` | [`homeassistant.rs`](src/tools/builtin/homeassistant.rs) | Home Assistant REST API: entity listing, state queries, service calls; gated on `HASS_URL` + `HASS_TOKEN` |

### 20.14 Builder (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `build_software` | [`builder/core.rs`](src/tools/builder/core.rs) | LLM-driven iterative build loop for WASM tools, CLI apps, and scripts |

### 20.15 Utility (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `echo` | [`echo.rs`](src/tools/builtin/echo.rs) | Echo input back (testing/debugging) |

### 20.16 Supporting Infrastructure

| Component | Source | Description |
|-----------|--------|-------------|
| Composable toolsets | [`toolset.rs`](src/tools/toolset.rs) | Named tool collections (web/dev/memory/safe/full/communication/automation/agents) with recursive includes + cycle detection |
| OSV malware scanner | [`osv_check.rs`](src/safety/osv_check.rs) | Package scanning via Google OSV API for MCP servers; MAL-* advisory detection, 1h cache, fail-open |
| Intent display | [`intent_display.rs`](src/tools/intent_display.rs) | Human-readable intent hints and argument extraction for all tool calls |
| Tool registry | [`registry.rs`](src/tools/registry.rs) | Protected-name enforcement, conditional registration, rate limiting |

### Owner: ThinClaw Agent

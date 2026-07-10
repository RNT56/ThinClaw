# ThinClaw Parity And ThinClaw-First Feature Matrix

> **Last reconciled:** 2026-07-10 (documentation audit: reconciled the matrix against the Wave-4 dead-code erasures. Corrected the removed Tailscale passwordless-identity auth, `DangerousToolTracker`, `self_message`/`TrustedMetadata`, and `/subagents` slash-command rows to reflect that those capabilities no longer exist, renamed the `nodes` CLI row to `devices`, and repointed decomposed source anchors to their new module paths). Prior: 2026-07-05 (audit-driven hardening: content-aware/CJK token estimation with tool-call accounting, cross-turn tool-call/result continuity in reconstructed context, provider context-overflow error classification into `LlmError::ContextLengthExceeded`, sub-agent cost-guard enforcement + per-principal concurrency cap, background MCP health monitor populating `McpRuntimeHealth` with auto-reconnect, Gmail unattended OAuth-refresh loop, zero-config `web_search` built-in tool, removal of superseded dead action tools). Prior: 2026-07-02 agent-loop hardening (dispatcher transient retry, credential-aware failover, compaction summary re-injection, model-derived context window, routine failure backoff/auto-disable, two-phase subagent cancellation; runtime-hardening batch: per-conversation dispatch concurrency, renewable routine run-leases, durable `/undo`/`/clear` watermark, persisted `/personality` overlay, token-based context caps, declarative command registry, structured worker completion, durable subagent ledger, verifier-based eval rewards, hydrated trajectory export, per-hook failure auto-disable, severity-driven prompt sanitization, pooled/cached learning-provider orchestrator)

This document tracks both feature parity against OpenClaw (TypeScript reference implementation) and ThinClaw-first capabilities that now extend well beyond parity. Use it both as a compatibility map and as a ledger of the newer Rust-native features we are actively adding.

Desktop-oriented sections are maintained from ThinClaw Desktop contracts, local Tauri wiring, and the integration surfaces present in this repo. Detailed desktop runtime boundaries live under `apps/desktop/documentation/`.

> **Desktop overhaul roadmap:** the end-to-end Desktop overhaul/upgrade/refinement plan (parity-first) is tracked in [`apps/desktop/documentation/OVERHAUL_PLAN.md`](apps/desktop/documentation/OVERHAUL_PLAN.md), with a tracker-ready epic/issue backlog in [`apps/desktop/documentation/OVERHAUL_BACKLOG.md`](apps/desktop/documentation/OVERHAUL_BACKLOG.md).

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
| Full status-type coverage for WASM channels | ✅ | The `wit/channel.wit` `status-type` enum covers every host `StatusUpdate` variant (lifecycle, sub-agent spawn/progress/complete, credential prompt, usage, plan, canvas, agent message, error, context compaction, advisor consultation, self-repair), so packaged channels classify these events directly instead of them collapsing to the generic `status` variant. The channel WIT package is versioned (`near:agent@x.y.z`, mirrored by `CHANNEL_WIT_VERSION`) for additive host/artifact negotiation. |
| Watched OAuth credential sync | ✅ | Claude Code, Codex, and custom JSON auth files can seed provider credentials and hot-reload updated tokens into the live runtime without restart when providers opt into external OAuth sources. Multi-key pools use process-local credential leases with health snapshots for capacity diagnostics. |
| External pre-exec shell scanner | ✅ | First-party `thinclaw-shell-scan` adds a pre-approval structural scanner with configured/PATH/bundled/cached resolution, bundled/cache manifest hash/signature verification, optional `external_scanner_require_verified`, and fail-open/fail-closed policy control. |
| Filesystem checkpoints + `/rollback` | ✅ | Shadow-git checkpoints create reversible file mutation history with list, diff, and restore support. |
| Remote skill federation | ✅ | ThinClaw now supports GitHub taps plus `/.well-known/skills` registries, v2 quarantine scanning across package files, scanner-version/content-hash provenance locks, finding summaries, package-layout rejection, and risky-install approval gates. |
| Accessibility-tree browser automation | ✅ | Managed `agent-browser` integration and cloud browser routing move ThinClaw from screenshot-only inspection toward interaction-oriented browsing. |
| Session-level `/personality` overlays (`/vibe` alias) | ✅ | Personality overlays add temporary tone shifts without mutating durable identity files; the overlay now persists across restart via a dedicated conversation-metadata key restored on hydration. |
| CLI skin system | ✅ | Shared TOML-backed local skins now cover boot, REPL, full-screen TUI, onboarding TUI, setup prompts, and human-readable CLI subcommands with prompt symbols, ASCII art, taglines, and tool emoji labels. Extended with `border_style` (plain/rounded/double/thick), `header_alignment`, `status_gradient`, `spinner_style` (kawaii/braille/dots/arrows), and custom `spinner_frames`. All new fields are optional with backward-compatible defaults. |
| Reckless desktop autonomy | ✅ | Privileged host-level desktop autonomy adds native app adapters, generic UI automation, evidence capture, seeded desktop routines, managed shadow-canary code autorollout, and rollback for promoted builds. |
| Trajectory archive + training export | ✅ | Structured turn archives and `trajectory export` provide validated SFT/DPO offline datasets with score filtering, max-record limits, skipped-record accounting, and optional manifest hashes; exported records are now hydrated with explicit learning feedback/evaluations from the database when reachable. |
| Anthropic prompt caching | ✅ | Provider-scoped message metadata now carries Anthropic-compatible cache hints where supported. |
| Zero-config web search | ✅ | Built-in `web_search` tool ([`crates/thinclaw-tools/src/builtin/web_search.rs`](crates/thinclaw-tools/src/builtin/web_search.rs)) queries DuckDuckGo's keyless endpoint through the SSRF-guarded fetch path and is registered by default — no API key or install. The Brave Search WASM extension remains available for operators wanting a ranked API. |
| Cross-turn tool-result continuity | ✅ | `Thread::messages()` reconstructs prior turns' tool calls + results (paired ids, bounded bodies) so a later turn can see what an earlier turn's tools returned, not just the final prose. Undo/redo checkpoints round-trip the richer stream. |
| Content-aware token estimation | ✅ | Context/compaction token estimates use a content-aware heuristic (`max(chars/4, words×1.3)` with per-character CJK/kana/Hangul counting) and now include tool-call JSON and tool-result framing, instead of pure whitespace-word counting. |
| Provider context-overflow recovery | ✅ | The rig adapter classifies real provider "prompt too long" rejections (OpenAI/Anthropic/Gemini/Ollama phrasings) into `LlmError::ContextLengthExceeded`, so the dispatcher's compact-and-retry recovery path fires on genuine overflows instead of misfiling them as transient failures. |
| Sub-agent budget + fairness controls | ✅ | Delegated sub-agents are gated by the same `CostGuard` daily-budget/hourly-rate limits as the main loop, and an optional per-principal concurrency cap (`SUBAGENT_MAX_PER_PRINCIPAL`) prevents one principal from starving others on a shared gateway. |
| MCP health monitor + auto-reconnect | ✅ | A background monitor probes active MCP servers, persists `McpRuntimeHealth` (surfaced on the extensions list), and rebuilds crashed stdio servers through the normal activation path. Previously `McpRuntimeHealth` was defined but never written or read. |
| Gmail unattended OAuth refresh | ✅ | The Gmail channel proactively refreshes its access token before expiry (and reactively on auth failure) when `GMAIL_REFRESH_TOKEN`/`GMAIL_CLIENT_ID`/`GMAIL_CLIENT_SECRET` are set, so long-running deployments no longer silently stop after ~1 hour. |
| Prometheus metrics backend | ✅ | `OBSERVABILITY_BACKEND=prometheus` records the `Observer` event/metric stream into a Prometheus registry exposed at `GET /metrics` (counters/histograms/gauges for LLM calls, tokens, cost, tool calls, latencies, errors, channel traffic). OTLP push export is deferred (tonic version conflict with libSQL). See [`docs/OBSERVABILITY.md`](docs/OBSERVABILITY.md). |
| Real `/api/health` readiness | ✅ | `GET /api/health` returns `200` only when the database is reachable, an LLM provider is configured, **and** the gateway's inbound message channel is wired to the runtime; otherwise `503`. Pure, unit-tested decision. |
| Typed Rust client SDK | ✅ | `thinclaw-client` ([`crates/thinclaw-client`](crates/thinclaw-client)) wraps the gateway HTTP+SSE surface: `send_message`/`send_and_wait`, streamed `events()` (tool-call visibility), `history`/`list_threads`/`new_thread`, `resolve_approval`, `abort`, plus an OpenAI-compat fast path. `SseEvent` degrades unknown server events to `Unknown{..}` so additive server changes never break older clients (contract-tested). Pre-1.0. See [`docs/CLIENT_SDK.md`](docs/CLIENT_SDK.md). |
| Unified conversation+file rewind (`/rewind`) | ✅ | `/rewind <n>` restores both the conversation (undo manager, to the start of turn `n`) and the working files (turn-tagged shadow-git checkpoint) in one command; `/rewind list` is a non-destructive dry run of available rewind points. Filesystem checkpoints are now tagged with their conversation turn (`[thinclaw][tN]`) so the two systems align. Previously `/undo` (conversation) and `/rollback` (files) were separate with no turn correlation. |
| Plan mode (`/plan`) | ✅ | `/plan on` puts a thread into plan mode: the agent is prompted to investigate read-only and propose a numbered plan, and every state-changing (non-read) tool it calls is escalated to require operator approval before running — even tools that would normally auto-run. `/plan off` resumes. State persists across restart via the thread runtime snapshot. Enforced at the interactive tool-approval gate (`src/agent/dispatcher/tool_execution.rs`). |
| Argument-scoped tool permissions | ✅ | Tool policy now goes beyond tool-name granularity: `tool_policies.arg_policies` (`thinclaw_settings::arg_policy`) matches a call's *final arguments* (glob on a param, e.g. `shell` command `npm run *`; URL-host allow/deny for `http`) and yields `allow` / `deny` / `require_approval`. Round-trips through the same persisted/TOML/env layers as name-based policy; enforced on post-hook args at `prepare_tool_call` and the interactive path. See `src/tools/README.md`. |

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
| Canvas hosting | ✅ | ✅ | `CanvasTool` + `CanvasStore` ([`crates/thinclaw-tools/src/builtin/canvas.rs`](crates/thinclaw-tools/src/builtin/canvas.rs)) + canvas gateway routes (`/canvas/:panel_id` HTML, `/canvas/api/panels` JSON, [`crates/thinclaw-channels/src/canvas_gateway.rs`](crates/thinclaw-channels/src/canvas_gateway.rs)) |
| Gateway lock (PID-based) | ✅ | ✅ | `gateway start` writes PID to `~/.thinclaw/gateway.pid`; launchd/systemd keepalive via `thinclaw service` |
| launchd/systemd integration | ✅ | ✅ | `service.rs` ([`src/service.rs`](src/service.rs)) — install/start/stop/status/uninstall; macOS launchd plist + Linux systemd unit generation |
| Bonjour/mDNS discovery | ✅ | ✅ | Settings-gated (`discovery.enabled` / `MDNS_ENABLED`, default-off) `_thinclaw._tcp` advertiser behind the `mdns` feature (milestone B3); locator-only TXT (`version`, `api`, `name`, `fp` = base64url(sha256(instance-id))), loopback skipped. Config types `MdnsConfig`/`DiscoveryTracker` in [`crates/thinclaw-config/src/mdns_discovery.rs`](crates/thinclaw-config/src/mdns_discovery.rs); responder in [`src/channels/web/discovery.rs`](src/channels/web/discovery.rs), spawned from `GatewayChannel::start()` |
| Tailscale integration | ✅ | ✅ | Full tunnel module (`tunnel/tailscale.rs`) with serve + funnel |
| Health check endpoints | ✅ | ✅ | `/api/health` is a real readiness probe (DB ping with 2s timeout + ≥1 LLM provider configured → `503` otherwise, so load balancers route away from not-ready instances) plus `/api/gateway/status` |
| `doctor` diagnostics | ✅ | ✅ | `cli/doctor.rs` — DB, binary, LLM, and Tailscale checks |
| Agent event broadcast | ✅ | ✅ | SSE broadcast manager + routine engine lifecycle events (Status, JobStarted, JobResult) |
| Channel health monitor | ✅ | ✅ | `ChannelHealthMonitor`: periodic checks, failure tracking, auto-restart with cooldown |
| Presence system | ✅ | ❌ | Dedicated `PresenceTracker` module removed as stale/unwired; channel/runtime liveness is tracked through `ChannelHealthMonitor` + channel status surfaces |
| Trusted-proxy auth mode | ✅ | ✅ | `TRUSTED_PROXY_HEADER` + `TRUSTED_PROXY_IPS` for reverse-proxy deployments |
| APNs push pipeline | ✅ | ✅ | Native lifecycle channel with registration endpoints, signed APNs client path, persisted device registry, and mock-backed delivery coverage; real Apple delivery smoke remains credential-gated |
| Oversized payload guard | ✅ | ✅ | HTTP webhook 64KB body limit + Content-Length check + chat history cap (`max_context_messages` default 200) |
| Pre-prompt context diagnostics | ✅ | ✅ | `tracing::debug` logs message count, est. chars, tool count before each LLM call |

### Owner: ThinClaw Agent

---

## 3. Messaging Channels

| Channel | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI with `ratatui-textarea` multi-line input (Alt+Enter/Shift+Enter newline, Ctrl+Enter force-submit, dynamic 3–8 line height), tick-based `KawaiiSpinner` animation in thinking/streaming states, dynamic skin-driven `BorderType` on all panels, gradient status bar, and braille startup spinner |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (local shell) | ✅ | ✅ | - | Full local shell surface with slash commands, skins, `/rollback`, and `/personality` (`/vibe` alias); multi-line input via rustyline `Validator` (backslash `\` continuation + triple-backtick fencing); no longer just a testing stub |
| WASM channels | ❌ | ✅ | - | ThinClaw innovation |
| WhatsApp | ✅ | ✅ | - | WASM channel via Cloud API webhook — GET verify-token + POST HMAC validation, text/media/location/contacts/interactive/reaction inbound handling, outbound text + media replies, DM pairing, markdown→WhatsApp formatting, Unicode-safe chunking |
| Telegram | ✅ | ✅ | - | WASM channel, DM pairing, caption, /start, bot_username, forum threading, sendMessage+editMessageText streaming (host-side, HTML formatted) |
| Discord | ✅ | ✅ | - | Native Rust Gateway WS + REST ([`src/channels/discord.rs`](src/channels/discord.rs)) + WASM interactions channel (slash commands) |
| Signal | ✅ | ✅ | - | signal-cli daemon, SSE listener, user/group allowlists, DM pairing |
| Slack | ✅ | ✅ | - | WASM channel (Events API webhook). Native dead code (`slack.rs`) removed. |
| iMessage | ✅ | ✅ | P3 | `IMessageChannel` + `IMessageConfig` native runtime ([`src/channels/imessage.rs`](src/channels/imessage.rs)) |
| Linq | ✅ | ❌ | P3 | Real iMessage via API, no Mac required |
| Feishu/Lark | ✅ | ✅ | P3 | WASM package with challenge, event callback, tenant-token, and bot-reply path; live tenant smoke remains env-gated |
| LINE | ✅ | ✅ | P3 | WASM package with batched events, HMAC validation, reply-token routing, and Messaging API response path |
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ✅ | P3 | Native lifecycle channel plus packaged channel shape; E2EE and live homeserver smoke remain follow-up checks |
| Mattermost | ✅ | ✅ | P3 | WASM package with webhook request/response path and mock-backed delivery coverage |
| Google Chat | ✅ | ✅ | P3 | WASM package with app event parsing and response path |
| MS Teams | ✅ | ✅ | P3 | WASM package with Bot Framework activity parsing and reply routing |
| Twitch | ✅ | ✅ | P3 | WASM package with EventSub challenge/HMAC path and chat response routing |
| Voice Call | ✅ | ✅ | P3 | Native lifecycle channel with webhook transcript ingress and response client; live media/transcription smoke remains provider-gated |
| Gmail | ✅ | ✅ | - | `GmailChannel` (700+ LOC) — Pub/Sub pull + Gmail API read/reply + sender allowlist ([`src/channels/gmail.rs`](src/channels/gmail.rs)) |
| Apple Mail | ❌ | ✅ | P3 | `AppleMailChannel` — Envelope Index polling, sender allowlist, unread-only, mark-as-read. Wizard onboarding + WebUI settings ([`src/channels/apple_mail.rs`](src/channels/apple_mail.rs)) |
| Nostr | ✅ | ✅ | ✅ | Owner-only encrypted DM control plus `nostr_actions` social tool, dual-stack NIP-04/Gift Wrap DM support, DM-only `send_message(platform="nostr")` |

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
| Self-message bypass | ✅ | ✅ | Inline per-adapter filtering (e.g. Discord drops messages whose author id equals the bot id, [`crates/thinclaw-channels/src/discord.rs`](crates/thinclaw-channels/src/discord.rs)); the dedicated `self_message` module was removed as dead code. |
| Mention-based activation | ✅ | ✅ | bot_username + respond_to_all_group_messages |
| Per-group tool policies | ✅ | ✅ | `ToolPolicyManager` with AllowAll/AllowList/DenyList + hierarchical evaluation ([`src/tools/policy.rs`](src/tools/policy.rs)) |
| Thread isolation | ✅ | ✅ | Separate sessions per thread |
| Platform formatting hints | ✅ | ✅ | Active-channel formatting guidance is injected into prompts across native and WASM channels. WASM channels resolve hints with two-tier precedence: explicit channel/package hint first, built-in platform fallback second, then uniform retrieval through `ChannelManager::formatting_hints_for()` |
| Linked DM continuity across channels | ❌ | 🚧 | DM scope metadata, actor-linked recall, and handoff storage added; explicit group recall and full UX restrictions still need completion |
| Per-channel media limits | ✅ | ✅ | `MediaLimits` with per-channel env var overrides + `filter_attachments()` ([`src/media/limits.rs`](src/media/limits.rs)) |
| Typing indicators | ✅ | ✅ | `Channel::send_typing()` trait method with platform-agnostic interface; TUI + Telegram + extensible to Discord/Signal |
| Per-channel ackReaction config | ✅ | ✅ | Per-channel emoji overrides via env vars ([`src/channels/ack_reaction.rs`](src/channels/ack_reaction.rs)) |
| Group session priming | ✅ | ✅ | Member roster injection with configurable limits ([`src/channels/group_priming.rs`](src/channels/group_priming.rs)) |
| Sender_id in trusted metadata | ✅ | ❌ | The dedicated `TrustedMetadata` struct and `self_message` module were removed as dead code. |

### Owner: ThinClaw Agent

---

## 4. CLI Commands

| Command | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| `run` (agent) | ✅ | ✅ | - | Default command |
| `tool install/list/remove` | ✅ | ✅ | - | WASM tools |
| `gateway start/stop` | ✅ | ✅ | P2 | `gateway.rs`: start (foreground/bg with PID), stop (SIGTERM), status (health+uptime) |
| `onboard` (wizard) | ✅ | ✅ | - | Interactive setup now exposes a Quick Setup vs Advanced Setup split in the onboarding TUI, keeps shared Humanist Cockpit readiness framing across CLI/TUI, and can continue directly into the matching local runtime |
| `/skin` | ❌ | ✅ | - | Runtime local-client skin switching with built-in TOML skins (`cockpit`, `midnight`, `solar`, `athena`, `delphi`, `olympus`), ASCII art, and user overrides |
| `/personality` | ❌ | ✅ | - | Personality overlay command (`/personality`, `/personality <name>`, `/personality reset`) with `/vibe` retained as a compatibility alias. The overlay now persists across restart via a dedicated `personality_overlay` conversation-metadata key restored on hydration, mirroring `/model`'s hydration-restore pattern (though `/model` stores its override as a field on the shared `ThreadRuntimeState` envelope, while `/personality` deliberately uses its own sibling metadata key) ([`src/agent/commands.rs`](src/agent/commands.rs), [`src/agent/thread_ops/hydration.rs`](src/agent/thread_ops/hydration.rs)) |
| `/undo` / `/clear` | ❌ | ✅ | - | Durable, restart-safe undo: an active-watermark (`active_message_row_count`) in the thread runtime envelope truncates rehydrated history on read while leaving the underlying DB rows intact for audit; the undo stack (newest 5 persisted checkpoints) persists to and restores from conversation metadata across restart. `last_active_at` is now touched on every turn transition rather than only at session start/end ([`src/agent/thread_ops/`](src/agent/thread_ops/), [`crates/thinclaw-agent/src/undo.rs`](crates/thinclaw-agent/src/undo.rs), [`crates/thinclaw-agent/src/session.rs`](crates/thinclaw-agent/src/session.rs)) |
| `tui` | ✅ | ✅ | - | Explicit full-screen runtime entrypoint (`thinclaw tui`) backed by the Ratatui local runtime |
| `config` | ✅ | ✅ | - | Read/write config |
| `channels` | ✅ | ✅ | P2 | `channels.rs`: list (env+WASM detection), info (per-channel details) |
| `models` | ✅ | ✅ | - | `list`, `info`, `test`, and `verify` subcommands with live provider discovery/probing ([`src/cli/models.rs`](src/cli/models.rs)) |
| `status` | ✅ | ✅ | - | System status (enriched session details) |
| `agents` | ✅ | ✅ | P3 | `list`, `add`, `remove`, `show`, `set-default` subcommands |
| `sessions` | ✅ | ✅ | P3 | `list`, `show`, `prune`, and `export` subcommands with thread ownership display and transcript export ([`src/cli/sessions.rs`](src/cli/sessions.rs)) |
| `memory` | ✅ | ✅ | - | `search`, `read`, `write`, `tree`, and `status` subcommands for workspace memory operations ([`src/cli/memory.rs`](src/cli/memory.rs)) |
| `trajectory` | ❌ | ✅ | P3 | `trajectory export` supports raw archive export plus training-ready `sft` and `dpo` datasets derived from labeled turn trajectories; `trajectory stats` summarizes the local archive ([`src/cli/trajectory.rs`](src/cli/trajectory.rs), [`src/agent/learning/mod.rs`](src/agent/learning/mod.rs)) |
| `backup` | ❌ | ✅ | P2 | Whole-agent export/restore as a single portable **encrypted bundle** (scrypt + XChaCha20-Poly1305). `backup export` bundles the ThinClaw home file tree (config, `SOUL.md`, skills, channels) — excluding `logs/`, `.env`, pid/capture dirs, and the live DB file — plus a database payload (WAL-checkpointed libSQL snapshot, or `pg_dump --format=custom` for Postgres); `backup inspect` shows the manifest; `backup import --yes` restores config + workspace, with `--restore-database` overwriting the local libSQL DB (or printing the `pg_restore` command for Postgres). Passphrase via `THINCLAW_BACKUP_PASSPHRASE`. Secrets are deliberately excluded (keychain-owned). Root-independent bundle/crypto core in [`crates/thinclaw-portability`](crates/thinclaw-portability) (15 tests: envelope round-trip/wrong-pass/tamper, path-traversal-safe extract, checksum verify); CLI in [`src/cli/backup.rs`](src/cli/backup.rs); see `docs/CLI_REFERENCE.md` → Backup & Restore |
| `skills` | ✅ | ✅ | - | Skills tools + web API endpoints (install, list, activate) |
| `pairing` | ✅ | ✅ | - | list/approve, account selector |
| `devices` | ✅ | ✅ | P3 | Device management with CRUD + formatted display (formerly `nodes`) ([`src/cli/devices.rs`](src/cli/devices.rs)) |
| `plugins` | ✅ | ✅ | - | `PluginManifest` — persistent JSON manifest, install/remove/toggle |
| `hooks` | ✅ | ✅ | P2 | Lifecycle hooks |
| `cron` | ✅ | ✅ | - | `cron add/edit/remove/trigger/runs/lint` — edit supports `--model`, `--thinking-budget`, `--schedule`, `--prompt`, `--enabled`; lint validates expressions offline |
| `webhooks` | ✅ | ✅ | - | `POST /hooks/routine/{id}` with HMAC-SHA256 secret validation; the validated, size-capped body is forwarded into the triggered routine's prompt via `RoutineRun.trigger_detail` as a delimited untrusted-data block |
| `message send` | ✅ | ✅ | P2 | `message.rs`: send to gateway with auth, auto-detect URL |
| `browser` | ✅ | ✅ | P3 | Headless Chrome: open/screenshot/links/check ([`src/cli/browser.rs`](src/cli/browser.rs)) |
| `sandbox` | ✅ | ✅ | - | WASM sandbox |
| `doctor` | ✅ | ✅ | - | Diagnostics (DB, binaries, LLM credentials, Tailscale) |
| `reset` | ❌ | ✅ | - | Destructive full-state reset: clears ThinClaw DB tables, removes `~/.thinclaw`, and deletes ThinClaw-managed keychain entries so onboarding can restart cleanly |
| `logs` | ✅ | ✅ | P3 | tail/search/show/levels with time-range/level/target filtering ([`src/cli/logs.rs`](src/cli/logs.rs)) |
| `update` | ✅ | ✅ | P3 | `check`, `install`, `rollback`, and `info` with stable/beta/nightly channels ([`src/cli/update.rs`](src/cli/update.rs)) |
| `completion` | ✅ | ✅ | - | Shell completion |
| `/subagents spawn` | ✅ | ❌ | P3 | No such CLI/slash command (not in `COMMAND_REGISTRY`); subagent spawning is exposed only via the `spawn_subagent` tool ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)). |
| `/export-session` | ✅ | ✅ | P3 | `sessions export` with markdown/JSON output ([`src/cli/sessions.rs`](src/cli/sessions.rs)) |

### Owner: ThinClaw Agent

---

## 5. Agent System

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Pi agent runtime | ✅ | ➖ | ThinClaw uses custom runtime |
| RPC-based execution | ✅ | ✅ | Orchestrator/worker pattern |
| Worker completion + tool-result event schema | ✅ | ✅ | Canonical completion payload (`status`, `session_id`, `success`, `message`) and structured tool-result projection (`output_text`, `output_json`) are now preserved end-to-end for orchestrator/SSE consumers. A `complete_job` tool is intercepted by the dispatcher the same way `emit_user_message` is, giving the worker a structured `summary` + `success` completion path; the prior free-text completion heuristic remains as a fallback when the model doesn't call the tool ([`src/agent/worker.rs`](src/agent/worker.rs), [`crates/thinclaw-agent/src/worker_runtime.rs`](crates/thinclaw-agent/src/worker_runtime.rs)) |
| Multi-provider failover | ✅ | ✅ | `FailoverProvider` tries providers sequentially on retryable errors, advances past credential-scoped failures (`AuthFailed`/`SessionExpired`) to fallback entries carrying their own credentials, applies per-provider cooldowns, leases individual credential entries (not just provider slots) with fill-first / round-robin / least-used / random selection strategies so multi-key backends can spread parallel traffic across keys, exposes process-local credential pool health snapshots, and participates in live OAuth credential refresh via watched auth-file sync + runtime reload when the provider opts in ([`src/llm/failover.rs`](src/llm/failover.rs), [`src/llm/credential_sync.rs`](src/llm/credential_sync.rs), [`src/app.rs`](src/app.rs)) |
| Per-sender sessions | ✅ | ✅ | Direct sessions are canonicalized by principal scope (cross-channel/device continuity); group scopes remain isolated |
| Global sessions | ✅ | ✅ | Cross-channel shared context with LRU eviction ([`src/agent/global_session.rs`](src/agent/global_session.rs)) |
| Session pruning | ✅ | ✅ | `sessions prune` CLI + auto-cleanup with configurable TTL |
| Context compaction | ✅ | ✅ | Auto summarization with user-visible pressure warnings at 85%/95% before compaction. The generated summary is re-injected into the post-compaction context fragment (and persisted with it) so the model keeps the gist of dropped turns, and the compaction trigger derives its context window from the active model's catalog entry instead of a fixed 100k default ([`src/agent/thread_ops/input.rs`](src/agent/thread_ops/input.rs), [`src/agent/thread_ops/compaction_context.rs`](src/agent/thread_ops/compaction_context.rs)) |
| Closed-loop learning orchestrator | ❌ | ✅ | Event→evaluation→candidate loop with dedupe/cooldown, safe-mode thresholds, Tier A auto-apply, Tier C approval-gated code proposals ([`src/agent/learning/mod.rs`](src/agent/learning/mod.rs), [`src/agent/thread_ops/`](src/agent/thread_ops/)) |
| Learning tool suite | ❌ | ✅ | `session_search` (FTS + optional cheap-model transcript summaries with fallback), `prompt_manage`, `skill_manage`, `learning_status`, `learning_history`, `learning_feedback`, `learning_proposal_review` ([`src/tools/builtin/memory.rs`](src/tools/builtin/memory.rs), [`src/agent/session_search.rs`](src/agent/session_search.rs), [`src/tools/builtin/learning_tools.rs`](src/tools/builtin/learning_tools.rs)) |
| Learning API + audit UI | ❌ | ✅ | `/api/learning/*` endpoints + dedicated Web Learning tab with proposals, feedback, rollbacks, and provider health ([`src/api/learning.rs`](src/api/learning.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/index.html`](src/channels/web/static/index.html)) |
| Optional research automation / experiments | ❌ | ✅ | Advanced opt-in `experiments.*` settings, `/api/experiments/*` gateway routes, CLI `thinclaw experiments ...`, routine action integration, Web Research tab, queued-campaign draining, autonomous planner/mutator/reviewer iteration, telemetry-derived opportunities, persisted target linking, normalized LLM-cost + runner-cost attribution, provider/budget detail in the Research WebUI, GPU Cloud setup cards, lease-scoped remote runner mode, local benchmark execution, and controller-managed RunPod/Vast/Lambda plus SSH/Slurm/Kubernetes launches are shipped, including a first-class Lambda launch form that builds `backend_config.launch_payload` server-side for turnkey controller launches; campaign, trial, and artifact reads are owner-scoped at the storage boundary; `default_artifact_retention_days` is enforced by a daily reaper, remote-runner artifacts are uploaded to durable host storage (`fetchable: true`) so they survive pod teardown, and the RunPod credit≈USD cost approximation is surfaced on the campaign `cost_summary` ([`src/api/experiments/`](src/api/experiments/), [`src/experiments/mod.rs`](src/experiments/mod.rs), [`src/experiments/artifact_store.rs`](src/experiments/artifact_store.rs), [`src/experiments/adapters.rs`](src/experiments/adapters.rs), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/index.html`](src/channels/web/static/index.html), [`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Optional external memory providers | ❌ | ✅ | Honcho + Zep adapters, local-first canonical memory, non-fatal provider fallback. Providers now share a `LearningOrchestrator`-owned manager with a 60-second TTL cache for provider readiness/health (replacing a live health probe on every operation) and a single pooled `reqwest::Client` (replacing a fresh client built per request) ([`src/agent/learning/providers/mod.rs`](src/agent/learning/providers/mod.rs), [`src/agent/learning/orchestrator/`](src/agent/learning/orchestrator/)) |
| Post-compaction read audit | ✅ | ✅ | `ReadAuditor` with scope-based rule scanning + token-budgeted appendix ([`src/context/read_audit.rs`](src/context/read_audit.rs)) |
| Post-compaction context injection | ✅ | ✅ | Priority-based fragment assembly with token budgets ([`src/context/post_compaction.rs`](src/context/post_compaction.rs)) |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Persona seed templates | ❌ | ✅ | P4 | Built-in SOUL.md seed presets plus setup-wizard selection for fresh workspaces |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill routing blocks | ✅ | ✅ | ActivationCriteria: keywords, patterns, tags, `use_when`/`dont_use_when` routing blocks |
| Skill path compaction | ✅ | ✅ | `~` prefix via `RuntimeBehavior::compact_path()` ([`src/agent/runtime_behavior.rs`](src/agent/runtime_behavior.rs)) |
| Thinking modes (low/med/high) | ✅ | ✅ | `ThinkingConfig` enum (Disabled/Enabled with budget_tokens), configurable via `thinking_enabled` + `thinking_budget_tokens` settings |
| Per-model thinkingDefault override | ✅ | ✅ | `MODEL_THINKING_OVERRIDE` env var: exact+prefix model match with per-model budget; prefix matches resolve deterministically (longest-prefix-wins, lexicographic tie-break) so overlapping prefixes no longer depend on `HashMap` iteration order ([`crates/thinclaw-config/src/agent.rs`](crates/thinclaw-config/src/agent.rs)) |
| Adaptive per-turn thinking budget | ❌ | ✅ (opt-in) | `ADAPTIVE_THINKING` env var (`agent.adaptive_thinking_enabled`, default off): when enabled, scales the resolved thinking budget per turn using the existing smart-routing complexity heuristic on the last user message (`Simple` disables thinking, `Moderate` halves the budget floor 1024, `Complex` unchanged); static per-model behavior is unchanged when the flag is off ([`crates/thinclaw-config/src/agent.rs`](crates/thinclaw-config/src/agent.rs), [`src/agent/dispatcher/llm_turn.rs`](src/agent/dispatcher/llm_turn.rs)) |
| Model-specific prompt guidance | ❌ | ✅ | Model-family prompt guardrails (GPT/Gemini) injected into conversation system prompts, controlled by `agent.model_guidance_enabled` |
| Block-level streaming | ✅ | ✅ | `StreamChunk::Text` + `StreamChunk::ReasoningDelta` via `complete_stream()` |
| Tool-level streaming | ✅ | ✅ | `StreamChunk::ToolCall` + `StreamChunk::ToolCallDelta` via `complete_stream_with_tools()` |
| Z.AI tool_stream | ✅ | ❌ | Standalone `tool_stream` helper module was removed as stale/unwired; canonical tool streaming remains via `StreamChunk::ToolCall` / `ToolCallDelta` in provider and OpenAI-compat paths |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Autonomous approval mode | ❌ | ✅ | `auto_approve_tools` with NEVER_AUTO_APPROVE_PATTERNS safety preserved; manual “allow session” approvals are channel-scoped by default for safer cross-channel direct-session sharing ([`src/agent/dispatcher/tool_execution.rs`](src/agent/dispatcher/tool_execution.rs), [`src/agent/session.rs`](src/agent/session.rs), [`src/setup/wizard/mod.rs`](src/setup/wizard/mod.rs)) |
| Smart shell approval triage | ❌ | ✅ | Soft-flag shell commands now pass through `SmartApprover` on the approval path; APPROVE skips the prompt, DENY fails closed, ESCALATE falls back to human approval, while `BLOCKED_COMMANDS` stay unconditional ([`src/tools/builtin/smart_approve.rs`](src/tools/builtin/smart_approve.rs), [`src/tools/builtin/shell.rs`](src/tools/builtin/shell.rs)) |
| Self-update & restart | ❌ | ✅ | `thinclaw update install` + `/restart` command for orderly shutdown; service-managed installs auto-relaunch and foreground `thinclaw` runs now self-relaunch too ([`src/cli/update.rs`](src/cli/update.rs), [`src/agent/submission.rs`](src/agent/submission.rs), [`src/main.rs`](src/main.rs)) |
| Elevated mode | ✅ | ✅ | Timeout-based activation with command allowlisting ([`crates/thinclaw-safety/src/elevated.rs`](crates/thinclaw-safety/src/elevated.rs)) |
| Subagent system | ✅ | ✅ | Full `SubagentExecutor` ([`src/agent/subagent_executor/mod.rs`](src/agent/subagent_executor/mod.rs)): in-process agentic loops with isolated context, filtered tools/skills, configurable timeouts, cancellation via watch channels, and durable async-subagent resume metadata |
| `spawn_subagent` tool | ✅ | ✅ | Declarative tool → dispatcher interception → `SubagentExecutor::spawn()`. Sync (wait=true) and async (wait=false) modes ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `list_subagents` tool | ✅ | ✅ | Query active/recent sub-agents with ID, status, task, timing info ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| `cancel_subagent` tool | ✅ | ✅ | Cancel running sub-agents by UUID; watch channel + task abort ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)) |
| Sub-agent lifecycle | ✅ | ✅ | Concurrency limits (default 5), per-agent timeout, periodic `StatusUpdate::SubagentProgress` heartbeat updates during long-running runs, status tracking (Running/Completed/Failed/TimedOut/Cancelled), user progress notifications via StatusUpdate::AgentMessage, and worker-side inactivity keepalives so long-running delegated work does not trip the parent wall-clock timeout. Cancellation is two-phase: the cooperative cancel watch (raced against LLM/tool awaits) lets the loop exit through normal finalization (completion event, learning event, routine-run completion), with a hard abort only after a grace period; finished handles are evicted from the active map after a retention window |
| Durable subagent ledger | ❌ | ✅ | A `subagent_runs` table (migration `V28__subagent_runs.sql`, mirrored for both Postgres and libsql/SQLite backends) records each in-process sub-agent run, previously tracked only in an in-memory map that silently dropped in-flight work on process restart. Rows are written at spawn and updated at completion; a startup reconciliation pass (wired in `src/main.rs`) fails any row still `running` after a crash, along with its linked routine run when one exists ([`src/agent/subagent_executor/mod.rs`](src/agent/subagent_executor/mod.rs), [`migrations/V28__subagent_runs.sql`](migrations/V28__subagent_runs.sql)) |
| Trajectory archive + training export | ❌ | ✅ | Per-turn trajectory logging stores structured assessment metadata and exports `json`, `jsonl`, `sft`, and `dpo` datasets with validation, score filtering, max-record limits, skipped-record counts, and optional manifests for offline training workflows. `trajectory export`/`trajectory stats` now best-effort connect to the database so exported records are hydrated with explicit learning feedback and evaluations via `hydrate_trajectory_record`, falling back to status-derived labeling when the DB is unreachable — closing a gap where the CLI path previously never invoked that hydration. Subagent learning confidence is now derived from observable signals (iteration count, duration) via `subagent_learning_confidence()`, replacing the prior hardcoded `0.82`/`0.38` success/failure constants ([`src/agent/learning/trajectory.rs`](src/agent/learning/trajectory.rs), [`src/cli/trajectory.rs`](src/cli/trajectory.rs), [`crates/thinclaw-agent/src/subagent.rs`](crates/thinclaw-agent/src/subagent.rs)) |
| Verifier-based eval harness | ❌ | ✅ | Benchmark environments execute and score the agent's own action (e.g. running its shell command, checking its own answer) rather than comparing against a canned response, with a graded heuristic fallback (0.0 empty/error, 0.3 trivial, 0.6 substantive) capped at the `SFT_QUALITY_GATE_SCORE` (0.6) SFT gate when no verifier applies. Episodes run concurrently via `buffer_unordered` with a concurrency of 4 ([`crates/thinclaw-agent/src/env.rs`](crates/thinclaw-agent/src/env.rs)) |
| Live-agent benchmark trials | ❌ | ✅ | AgentEnv benchmark configs accept an opt-in `live_agent` flag: actions are produced per case by the registered subagent runtime (a real LLM-backed agent) instead of scripted reference actions, with per-case failures scored 0.0; the default remains deterministic/offline ([`src/api/experiments/execution.rs`](src/api/experiments/execution.rs), [`crates/thinclaw-agent/src/env.rs`](crates/thinclaw-agent/src/env.rs)) |
| Subagent transparency controls | ❌ | ✅ | End-to-end transparency controls shipped: typed Web subagent SSE events + temporal Web subsessions + `agent.subagent_transparency_level` (`balanced`, `detailed`) filtering + Telegram session mode routing |
| `/subagents spawn` command | ✅ | ❌ | No such slash command (not in `COMMAND_REGISTRY`); subagent spawning is exposed only via the `spawn_subagent` tool ([`src/tools/builtin/subagent.rs`](src/tools/builtin/subagent.rs)). |
| Persistent multi-agent orchestration | ❌ | ✅ | `AgentRegistry` with DB-backed CRUD, 5 LLM tools (`create_agent`/`list_agents`/`update_agent`/`remove_agent`/`message_agent`), workspace seeding, validation, dual-backend persistence ([`src/agent/agent_registry.rs`](src/agent/agent_registry.rs), [`src/tools/builtin/agent_management.rs`](src/tools/builtin/agent_management.rs)) |
| A2A communication | ❌ | ✅ | `message_agent` tool resolves target agent, builds scoped context (system prompt + memory), returns structured A2A payload; dispatcher intercepts and routes through `SubagentExecutor` for actual LLM execution ([`src/tools/builtin/agent_management.rs`](src/tools/builtin/agent_management.rs), [`src/agent/dispatcher/tool_execution.rs`](src/agent/dispatcher/tool_execution.rs)) |
| Workspace sandbox modes | ❌ | ✅ | `WORKSPACE_MODE` config: `unrestricted` (default), `sandboxed` (confined to `WORKSPACE_ROOT`), `project` (working dir set, files unrestricted). Drives tool registration + dynamic system prompt ([`src/config/agent.rs`](src/config/agent.rs)) |
| Dynamic system prompt | ❌ | ✅ | `Reasoning::build_workspace_capabilities_section()` generates mode-specific Desktop Capabilities based on active workspace config and available tools ([`src/llm/reasoning.rs`](src/llm/reasoning.rs)) |
| Screen capture tool | ❌ | ✅ | `ScreenCaptureTool` — macOS screencapture/Linux gnome-screenshot/scrot; registered when `ALLOW_LOCAL_TOOLS=true` + `SCREEN_CAPTURE_ENABLED=true` (user opt-in via desktop toggle). Dynamic system prompt auto-detects and injects guidance ([`src/tools/builtin/screen_capture.rs`](src/tools/builtin/screen_capture.rs)) |
| Auth profiles | ✅ | ✅ | Multi-key rotation with health tracking, plus watched OAuth auth-file sources (Claude Code, Codex, and custom JSON token files) that prime injected provider credentials and hot-reload the live runtime on token changes without restart ([`crates/thinclaw-safety/src/auth_profiles.rs`](crates/thinclaw-safety/src/auth_profiles.rs), [`src/llm/credential_sync.rs`](src/llm/credential_sync.rs), [`src/app.rs`](src/app.rs), [`src/config/mod.rs`](src/config/mod.rs)) |
| Generic API key rotation | ✅ | ✅ | Multi-strategy rotation with health tracking ([`crates/thinclaw-safety/src/key_rotation.rs`](crates/thinclaw-safety/src/key_rotation.rs)) |
| Stuck loop detection | ✅ | ✅ | Consecutive same-tool detection with warn at 3, force-text at 5 |
| Dispatcher-level LLM retry policy | ❌ | ✅ | Turn failures are classified (cancelled / context-length / transient / fatal): transient provider errors get a bounded backoff retry before the turn is abandoned, context-length errors compact and retry once across both streaming and non-streaming paths, cancellation propagates untouched (it no longer clears an active `llm_select` model override), and streamed attempts only retry when no partial output reached the user ([`src/agent/dispatcher/llm_turn.rs`](src/agent/dispatcher/llm_turn.rs), [`crates/thinclaw-agent/src/dispatcher_policy.rs`](crates/thinclaw-agent/src/dispatcher_policy.rs)) |
| Per-conversation dispatch concurrency | ❌ | ✅ | `dispatch_incoming_message` keys an ordered per-conversation worker on `conversation_scope_id`, so independent conversations run concurrently while messages within one conversation stay strictly ordered; turn processing is bounded globally by an 8-permit `tokio::Semaphore` (`MAIN_LOOP_MAX_CONCURRENT_TURNS`), not a per-conversation limit. Single-slash `/interrupt`, `/quit`, and `/restart` are classified before enqueue and dispatched directly, bypassing the worker queue so they still reach a busy conversation ([`src/agent/agent_loop/mod.rs`](src/agent/agent_loop/mod.rs), [`crates/thinclaw-agent/src/submission.rs`](crates/thinclaw-agent/src/submission.rs)) |
| Token-based dispatcher context caps | ❌ | ✅ | The hard context-trim cap and pre-compaction memory-flush trigger now fire on estimated tokens against the active model's context window (90% hard cap via `CONTEXT_HARD_CAP_RATIO`, 80% memory-flush via `MEMORY_FLUSH_RATIO`, trimming back down to a 70% target), with message count (`max_context_messages`) retained only as a secondary bound for pathologically message-heavy conversations that could undercount on tokens alone ([`crates/thinclaw-agent/src/dispatcher_policy.rs`](crates/thinclaw-agent/src/dispatcher_policy.rs), [`src/agent/dispatcher/loop.rs`](src/agent/dispatcher/loop.rs)) |
| Declarative command registry | ❌ | ✅ | A single `COMMAND_REGISTRY` table in [`crates/thinclaw-agent/src/command_registry.rs`](crates/thinclaw-agent/src/command_registry.rs) is the source of truth for help text, the TUI's forwarded-command and autocomplete lists, and submission parsing (consumed by `command_catalog.rs` and `submission.rs`). Fixes surface drift where `/debug` was help-listed but missing from TUI autocomplete, and `/skin` was autocompletable but missing from the TUI forwarded-commands list; a regression test now keeps the three derived surfaces consistent with the registry |
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
| Smart routing WebUI config | ❌ | ✅ | Providers tab now includes a dedicated Routing editor for enable/disable, mode selection, provider roster, per-provider model slot persistence, drag-and-drop primary/cheap pool reordering, fallback chain, policy rules, route simulation, an opt-in “Separate tool planning from final answer” toggle for two-phase cheap synthesis on main-agent tool turns, where the cheap phase only runs after an explicit `NO_TOOLS_NEEDED` planner signal and any primary-model final answer is returned directly, plus a separate toggle to keep primary planning thinking enabled by default ([`src/channels/web/static/app.js`](src/channels/web/static/app.js), [`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/agent/dispatcher/llm_turn.rs`](src/agent/dispatcher/llm_turn.rs)) |
| Wizard cheap model API key | ❌ | ✅ | `step_smart_routing` detects cross-provider cheap model, prompts for the missing API key without clobbering the primary backend, and persists canonical provider settings ([`src/setup/wizard/llm/steps.rs`](src/setup/wizard/llm/steps.rs)) |
| Claude Code runtime model config | ❌ | ✅ | WebUI Settings: change model/max-turns without restart, hot-reloaded into `ContainerJobManager` ([`src/orchestrator/job_manager.rs`](src/orchestrator/job_manager.rs)) |
| Provider Vault (WebUI key mgmt) | ❌ | ✅ | Providers tab: list providers, add/remove encrypted credentials for catalog providers plus custom OpenAI-compatible endpoints and native Bedrock API keys, auto-enable them for routing, and hot-reload the live LLM runtime ([`src/channels/web/server.rs`](src/channels/web/server.rs), [`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Agent-initiated model switching | ❌ | ✅ | `llm_select` + `llm_list_models` tools: agent can switch LLM mid-conversation via conversation-scoped runtime overrides; runtime probe blocks dead model switches, dispatcher auto-resets failed overrides to the previous working model, and model discovery exposes current primary/cheap config [`src/tools/builtin/llm_tools.rs`](src/tools/builtin/llm_tools.rs) |
| Wizard fallback providers step | ❌ | ✅ | `step_fallback_providers` adds secondary providers and models into canonical provider routing settings without mutating the chosen primary backend ([`src/setup/wizard/llm/steps.rs`](src/setup/wizard/llm/steps.rs)) |
| RoutingPolicy in runtime | ❌ | ✅ | Policy rules are resolved by the live LLM runtime manager and applied to request-time routing, explicit fallback chains, cost-aware target selection, OpenAI-compatible calls, and hot-reloaded provider settings ([`src/llm/runtime_manager/mod.rs`](src/llm/runtime_manager/mod.rs), [`src/llm/routing_policy.rs`](src/llm/routing_policy.rs)) |
| Routing Engine V2 (unified planner) | ❌ | ✅ | Unified `RoutePlanner` drives all routing decisions: structured scoring (quality/cost/latency/health), capability gating, budget-aware cost pressure, session affinity, and context window gating. Supports 4 modes: PrimaryOnly, CheapSplit (preserved), AdvisorExecutor (new), Policy. Live cutover complete — planner replaces legacy `SmartRoutingProvider` branching. `consult_advisor` tool intercepted by dispatcher for advisor-executor pattern. Normalized telemetry keys (`role\|slug\|model`), circuit-breaker + latency-weighted health probes ([`src/llm/route_planner.rs`](src/llm/route_planner.rs), [`src/tools/builtin/advisor.rs`](src/tools/builtin/advisor.rs), [`src/llm/runtime_manager/mod.rs`](src/llm/runtime_manager/mod.rs), [`src/agent/dispatcher/llm_turn.rs`](src/agent/dispatcher/llm_turn.rs)) |

### Owner: ThinClaw Agent

---

## 7. Media Handling

| Feature | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Image processing (Sharp) | ✅ | ✅ | P2 | `ImageExtractor`: dimension detection (PNG/GIF/WebP/JPEG), base64 data-URI, detail level hints |
| Configurable image resize dims | ✅ | ✅ | P2 | `with_max_dimensions()` — sets OpenAI `detail` level (high/low) based on image dims |
| Multiple images per tool call | ✅ | ✅ | P2 | `format_multiple_for_llm()` — array of image content blocks |
| Audio transcription | ✅ | ✅ | P2 | `AudioExtractor`: Whisper HTTP endpoint, multipart upload |
| Video support | ✅ | ✅ | P3 | `VideoAnalyzer` ([`src/media/video.rs`](src/media/video.rs)) — ffprobe metadata, ffmpeg keyframe + audio extraction, graceful fallback; canonical transcript field is `audio_transcript_path` with one-cycle deprecated alias `audio_transcript` |
| PDF parsing | ✅ | ✅ | P2 | `PdfExtractor`: BT/ET text blocks, readable-sequence fallback |
| MIME detection | ✅ | ✅ | P2 | `media/types.rs`: extension + magic bytes detection |
| Media caching | ✅ | ✅ | P3 | SHA-keyed media cache with TTL + LRU eviction (`CacheConfig`/`MediaCache`) ([`src/media/cache.rs`](src/media/cache.rs)) |
| Vision model integration | ✅ | ✅ | P2 | `ImageExtractor::format_for_llm()` — base64 data-URI for multimodal LLMs |
| TTS (Edge TTS) | ✅ | ✅ | - | `TtsSynthesizer` with Edge TTS provider support |
| TTS (OpenAI) | ✅ | ✅ | - | `tools/builtin/tts.rs` — OpenAI TTS tool |
| Incremental TTS playback | ✅ | ✅ | P3 | `SentenceChunker` + `TtsChunk` streaming via SSE, progressive chunk synthesis ([`src/media/tts_streaming.rs`](src/media/tts_streaming.rs)) |
| ComfyUI prompt-to-image generation | ❌ | ✅ | - | Built-in `image_generate` tool backed by ComfyUI REST/WebSocket workflow execution, renderable generated-media artifacts, output sanitization, and bundled starter workflows ([`src/tools/builtin/comfyui.rs`](src/tools/builtin/comfyui.rs), [`crates/thinclaw-media/src/comfyui.rs`](crates/thinclaw-media/src/comfyui.rs), [`docs/COMFYUI_MEDIA_GENERATION.md`](docs/COMFYUI_MEDIA_GENERATION.md)) |
| ComfyUI workflow operations | ❌ | ✅ | - | `comfy_health`, `comfy_check_deps`, `comfy_run_workflow`, and approval-gated `comfy_manage`; lifecycle actions are explicit because ComfyUI is an operator-trusted sidecar, not a WASM sandbox |
| Creative ComfyUI skill | ❌ | ✅ | - | Bundled `creative-comfyui` skill selects `image_generate` for normal image requests and reserves lifecycle changes for explicit setup/management requests ([`skills/creative-comfyui/SKILL.md`](skills/creative-comfyui/SKILL.md)) |
| Sticker-to-image | ✅ | ✅ | P3 | WebP/TGS/WebM detection + ffmpeg conversion ([`src/media/sticker.rs`](src/media/sticker.rs)) |
| Media pipeline integration | ❌ | ✅ | - | `MediaPipeline` auto-wired into `process_user_input()` via `IncomingMessage.attachments` |
| Multimodal media routing | ❌ | ✅ | - | Images/audio/video → rig-core `UserContent::Image/Audio/Video` (provider-agnostic); PDFs → text extraction. Attachment size limits: 20MB/file, 50MB/msg |
| Telegram media download | ❌ | ✅ | - | Layer 6: photo/voice/audio/document/video/video_note/sticker download via Bot API `getFile`. All media types populate `MediaAttachment` and route to multimodal LLM |
| Discord media download | ❌ | ✅ | - | Native gateway `MESSAGE_CREATE` with `attachments[]` CDN download; size-limited to 20MB/file |
| Signal media download | ❌ | ✅ | - | Typed `SignalAttachment` from signal-cli SSE, reads binary from local attachment store; size-limited |
| iMessage media download | ❌ | ✅ | - | Queries `attachment` + `message_attachment_join` tables from chat.db, reads files from disk |
| WhatsApp media download | ❌ | ✅ | - | 2-step Cloud API download (media URL → binary), uses configured Graph API version, supports image/audio/video/document/sticker |
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
| Native (dynamic-library) plugins | ✅ | ✅ | Wired but **default-off**, ed25519-signature + SHA-256 + operator-allowlist gated and `catch_unwind`-isolated before `dlopen`. Runs **in-process with full host privilege (NOT sandboxed)**; operator-only — deliberately not exposed via the web gateway ([`src/extensions/native_activation.rs`](src/extensions/native_activation.rs); see `docs/EXTENSION_SYSTEM.md`, `src/NETWORK_SECURITY.md`) |
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
| Vector memory | ✅ | ✅ | pgvector (Postgres) and `libsql_vector_idx` (libSQL). **Caveat:** migration `V9` dropped the Postgres HNSW index to allow flexible embedding dimensions, so pgvector search is currently an exact per-user sequential scan; libSQL ANN indexing applies only to 1536-dim embeddings (other dimensions fall back to a brute-force cosine scan). Accurate for personal-scale corpora; see `docs/MEMORY_AND_GROWTH.md`. |
| Session-based memory | ✅ | ✅ | |
| Hybrid search (BM25 + vector) | ✅ | ✅ | RRF algorithm |
| Temporal decay (hybrid search) | ✅ | ✅ | `apply_temporal_decay()` — exponential half-life scoring, wired into `hybrid_search()` post-RRF |
| MMR re-ranking | ✅ | ✅ | `mmr_rerank()` — greedy diversity selection with cosine similarity, wired into `hybrid_search()` |
| LLM-based query expansion | ✅ | ✅ | `expand_query_keywords()` — stop word removal + morphological variants, wired pre-FTS |
| OpenAI embeddings | ✅ | ✅ | |
| Gemini embeddings | ✅ | ✅ | `EmbeddingConfig::gemini()` ([`src/llm/embeddings.rs`](src/llm/embeddings.rs)) |
| Local embeddings | ✅ | ✅ | `EmbeddingConfig::local()` + Ollama support ([`src/llm/embeddings.rs`](src/llm/embeddings.rs)) |
| Bedrock embeddings | ❌ | ✅ | `BedrockEmbeddings` — Titan Text Embeddings V2 via AWS SDK `invoke_model()`, feature-gated behind `--features bedrock` ([`src/workspace/embeddings.rs`](src/workspace/embeddings.rs)) |
| SQLite-vec backend | ✅ | 🚧 | **Config/SQL scaffolding only — not wired to a live store.** `SqliteVecConfig` builds vec0 virtual-table SQL strings and distance metrics but no `sqlite-vec` client executes them and nothing implements `WorkspaceStore` against it ([`crates/thinclaw-workspace/src/sqlite_vec.rs`](crates/thinclaw-workspace/src/sqlite_vec.rs)). Wiring this is the highest-value follow-up: it would restore true local ANN for edge/libSQL deployments at arbitrary embedding dimensions. |
| LanceDB backend | ✅ | 🔮 | **Not implemented.** `LanceDbConfig`/schema builders exist as unwired structs with no `lancedb` dependency or store binding ([`crates/thinclaw-workspace/src/lancedb.rs`](crates/thinclaw-workspace/src/lancedb.rs)). Planned/optional; low priority for a personal-scale agent. |
| QMD backend | ✅ | 🔮 | **Not implemented.** `QmdConfig` quantization math exists as unwired scaffolding with no store binding ([`crates/thinclaw-workspace/src/qmd.rs`](crates/thinclaw-workspace/src/qmd.rs)). Planned/optional. |
| Atomic reindexing | ✅ | ✅ | |
| Embeddings batching | ✅ | ✅ | `embed_batch` on EmbeddingProvider trait |
| Citation support | ✅ | ✅ | `Citation` struct with inline/footnote formatting, deduplication, relevance sorting ([`src/workspace/citations.rs`](src/workspace/citations.rs)) |
| Memory CLI commands | ✅ | ✅ | `memory search/read/write/tree/status` CLI subcommands |
| Flexible path structure | ✅ | ✅ | Filesystem-like API |
| Identity files (AGENTS.md, etc.) | ✅ | ✅ | |
| Proactive BOOT.md execution | ❌ | ✅ | Executes BOOT.md on every startup; routes response to preferred notification channel ([`src/agent/agent_loop/mod.rs`](src/agent/agent_loop/mod.rs)) |
| Proactive BOOTSTRAP.md execution | ❌ | ✅ | Executes BOOTSTRAP.md on first run only; auto-deletes after completion; routes response to preferred channel ([`src/agent/agent_loop/mod.rs`](src/agent/agent_loop/mod.rs)) |
| Daily logs | ✅ | ✅ | |
| Heartbeat checklist | ✅ | ✅ | HEARTBEAT.md |

### Owner: ThinClaw Agent

---

## 11. Mobile Apps

> The native iOS surface has **landed all planned milestones in code** (R0→M5,
> B1→B3): the whole app + widget + NSE embeds build for the iOS 26 simulator and
> the watch app + complication build for the watchOS 26 simulator (both CI hard
> gates — `build-app` and `watch-build`), gateway backends are Rust-tested, and
> Swift logic is
> `swift test`-covered. A `🚧` in the ThinClaw column below means "implemented in
> code but not yet verified against a real device, a live gateway, or Apple's
> push/TestFlight infrastructure" — the honest remaining work is bring-up, not
> new milestones (see `docs/MOBILE_APP.md` → Remaining work). Roadmap, milestones,
> and the backend contract live in `docs/MOBILE_APP.md`; the security model lives
> in `docs/MOBILE_SECURITY.md`.

| Feature | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| iOS app (SwiftUI) | ✅ | 🚧 | P1 | M1 client landed: all 7 SPM packages pass `swift test` on macOS (SSE transport + `GatewaySession`/reconcile, pairing-payload parse, Secure-Enclave keypair, SPKI pinning + D-X2 policy, Keychain, GRDB persistence, generated `ThinClawAPI`). Onboarding (state machine + VisionKit QR scanner + manual path) and chat/sessions (`ChatStore`/`SessionsStore` over the live pinned session + cache) are wired; the whole app target **builds for the iOS 26 simulator** (the hard `build-app` CI gate, green on `main`), onboarding store has 32 simulator tests. M2 approvals surface landed: risk-tiered `ApprovalCard`s over a UI-free `ApprovalsStore` (cold-load `GET /api/chat/approvals` + live `approval_needed` fan-out, `POST /api/chat/approval` decisions), a Face-ID gate on high-risk **approve** only (D-K3, injected `BiometricGating`), and inline chat cards for `auth_required` (opens the OAuth URL, never captures the token) and `credential_prompt` (handle-on-desktop per D-T4) — all exercised by `swift test` on macOS. M5 jobs glance landed (read-only, `jobs:read`): a UI-free `JobsStore` (in `ThinClawCore`, macOS-tested) lists jobs + summary and loads a job's detail over the generated client, and tails a job's event log by **polling** `GET /api/jobs/{id}/events` (a JSON snapshot, not SSE — there is no per-job stream on the gateway), folding new rows by their monotonic id cursor with backoff and stopping on a terminal phase; `FeatureJobs` renders the list (pull-to-refresh, summary chips, empty state, an explicit "view only — can't cancel/restart from this device" footer) and a detail view (header, state transitions, live activity tail). M5 Settings + device management landed (in code): a UI-free `SettingsStore` (in `ThinClawCore`, macOS-tested) shows this device (`GET /api/devices/me`), lists companions with per-companion Revoke (`DELETE /api/devices/me/companions/{id}` — de-authorize the watch from the phone), and Unpairs (`AppDependencies.unpair()`); **no self-rename/rotate** (those routes are admin-only and reject a device token — deliberately omitted). Per-category notification preview preferences (`NotificationPreferences`; always/when-unlocked/never + approvals-only "app only") persist to shared App Group defaults that the NSE reads before rewriting; the connection row derives from the live `GatewaySession` state + Keychain identity (never `/api/gateway/status`), the gateway URL/pin reveal is Face-ID-gated (D-K3), and "Enhanced protection" drives `GRDBTranscriptStore.applyFileProtection(enhanced:)` + the shared app-switcher-overlay defaults key. 10 settings + 8 notification `swift test` cases on macOS; the SwiftUI screen compiles in the `build-app` hard gate. M5 **TestFlight archive pipeline landed** (fastlane-free): a tag-triggered (`ios-v*`) `archive` job in `.github/workflows/ios.yml` + `apps/ios/scripts/archive.sh` + `apps/ios/Config/ExportOptions.plist` run `xcodebuild archive` → `-exportArchive` (method `app-store`) → `xcrun altool --upload-app`, authenticated by an App Store Connect API key (`-authenticationKeyPath`/`-allowProvisioningUpdates`; no `match`, no committed profiles); it is **credential-gated** (no-op-with-message when the `APPLE_DEVELOPMENT_TEAM`/`APP_STORE_CONNECT_KEY_ID`/`APP_STORE_CONNECT_ISSUER_ID`/`APP_STORE_CONNECT_KEY_P8` secrets are absent, so tag pushes never fail CI). YAML validated with actionlint; the real archive is unrun (repo has no Apple team). M5 **accessibility + app-switcher redaction landed**: pure `PrivacyRedactionPolicy` + `TimelineAccessibility` VoiceOver labels (macOS-tested), `App/Sources/PrivacyOverlay` covers the window on background/inactive `scenePhase` off the shared enhanced-protection defaults key, and `FeatureChat`/`ThinClawDesign` honor VoiceOver + Reduce Motion. **All planned milestones (M1–M5) are now landed in code.** Not yet (bring-up, not milestones and not compilation — every target compiles under the `build-app`/`watch-build` CI hard gates): chat/sessions async orchestration UI tests, simulator/real-device E2E, and real Apple push/NSE/TestFlight delivery. Milestones M1–M5 in `docs/MOBILE_APP.md` |
| Android app (Kotlin) | ✅ | 🚫 | - | Out of scope |
| Apple Watch companion | ✅ | 🚧 | P2 | M4 **backend landed**: the watch is modeled as a *companion* device — a child `DeviceRecord` (`parent_device_id`, serde-default for legacy rows) minted by a paired parent at `POST /api/devices/me/companions` (`devices:self` scope) with a narrowed `chat`+`approvals` grant (no `jobs:read`, no `devices:self`), listed via `GET /api/devices/me/companions` and revoked via `DELETE /api/devices/me/companions/{id}`; revoking any device **cascades** to its companions (`DeviceStore::revoke_cascade` — one locked write, push regs cleared, live streams torn down). Watch approvals are enforced **low-risk-only server-side** in `POST /api/chat/approval` (a watchOS-companion principal is refused a high/unknown-risk approve with a generic 403, using the D-K3 gateway-side risk tier; deny always allowed; phone tokens unaffected). `companion.created`/`companion.revoked` audit events; OpenAPI regenerated. Rust unit + `device_pairing_integration` coverage. Watch **client UI landed (M4)**: `Watch/Sources` renders a glanceable status (mirrored `AgentStatusSnapshot` phase + pending count + relay/direct/queued route badge), an approvals list that offers Approve/Deny only for **low-risk** entries (high/unknown → "Approve on iPhone" hand-off; deny always allowed) with success/failure `WKInterfaceDevice` haptics, and a dictated quick-ask with a sent/queued/failed receipt — all behind a `WatchGatewayProxy` seam (relay-first, watch's own token). `WatchWidgets/Sources` renders a real WidgetKit status complication (circular/corner/inline) from the watch App Group mirror, resilient to a missing snapshot. Watch UI compiles for watchOS 26 in the `watch-build` hard gate. Watch **relay + companion provisioning wired live (M4 + follow-up)**: `ThinClawWatchBridge` + the `App/Sources/WatchProvisioning` hook carry the bridge half, and the watch surface is now driven live rather than by the read-only mirror. On the watch, `WatchSessionDelegate` activates the `WCSession`, stores the phone-provisioned companion credential in the **watch's own** keychain, mirrors received snapshots into the watch App Group, and reloads the complication on a fresh mirror; `RouterGatewayProxy` (replacing the read-only `MirroredSnapshotProxy`, kept only as a non-WatchConnectivity fallback) drives a `WatchGatewayRouter` over live `WCSession.sendMessage` relay / pinned-URLSession direct / `transferUserInfo` queue transports. The phone-side `WatchRelayHost` (`WCSessionDelegate`) mints the watch a companion (`POST /api/devices/me/companions`, pinned parent client) on a missing/stale-credential report and pushes it (token + gateway URLs + SPKI pin + instance id) via `updateApplicationContext`, answers relayed RPCs by forwarding the **watch's own token opaquely** (the phone never substitutes its own — unit-tested, failing closed to `reprovisionRequired` on a missing token or 401/403), and pushes snapshot mirrors on significant changes (`AppDependencies` → `WatchProvisioning.mirror`); the companion is `DELETE`d on unpair. Route selection is relay→direct→queue with per-route timeout fall-through inside the <5s approval budget. 39 macOS `swift test` cases cover the pure seams; the whole-target watchOS compile is now a CI hard gate (`watch-build`), and a full phone↔watch round-trip still needs physically paired hardware (WatchConnectivity does not function end-to-end in the simulator). See `docs/MOBILE_SECURITY.md` D-K4 and `docs/MOBILE_APP.md` |
| Home-screen widgets + App Group snapshot pipeline | ✅ | 🚧 | P2 | Snapshot pipeline landed (M3): `SnapshotPublisher` (in `ThinClawCore`, macOS-testable, UIKit-free) projects live agent state into `AgentStatusSnapshot` / `PendingApprovalsSnapshot` (fail-closed `RiskTier` so the widget gates inline approve) / `JobsSnapshot`, debouncing bursts and dropping no-op writes, with a `SnapshotPrivacyPolicy` that truncates or (in "app only" mode) drops human-authored text so App Group snapshots stay content-free (D-N/data-at-rest). Three triggers feed one fetch→write→reload: foreground (live approvals mirroring + one kick), silent push (`BackgroundRefresh.handleSilentPush` fetches gateway status + `GET /api/chat/approvals` + jobs list over the pinned client, writes via `SnapshotStoreSink`, reloads timelines), and `BGAppRefresh` (`com.thinclaw.ios.refresh`, registered at launch, re-armed on background). The four widgets are implemented (`AgentStatusWidget`, `PendingApprovalsWidget`, `QuickAskWidget`, and the `AgentRunLiveActivity` Dynamic Island): each reads the App Group snapshots through `WidgetSnapshotAccess` and degrades to placeholders on read failure, and `PendingApprovalsWidget` offers inline Approve/Deny only on low-risk rows (`ApproveToolIntent` refuses high/unknown-risk — a lock screen can never approve a high-risk action, D-K3). The agent-run Live Activity manager (`ThinClawLiveActivity`: pure `RunTracker`/`RunInputClassifier` + `@MainActor` `LiveActivityManager` over an `ActivityController` protocol) updates locally on progress, forwards per-activity + push-to-start tokens over the pinned client, and `DELETE`s on run end. Pure mapping/debounce/privacy + publisher→store integration + `RunTracker`/`LiveActivityManager` logic pass `swift test`; the WidgetKit/ActivityKit + BGTaskScheduler/UIKit compile lands in the `build-app` hard gate |
| Gateway WebSocket client | ✅ | 🚧 | P2 | Mobile client is SSE-primary; the `ThinClawTransport` streaming layer (`GatewaySession`/`GatewayStream`, reconnect + watchdog, per-thread event routing) is implemented and tested over SSE. WS remains available on the gateway but the mobile client has no dedicated WS transport yet |
| Camera/photo access | ✅ | 🚫 | - | Post-M5 candidate |
| Voice input | ✅ | 🚫 | - | Watch dictation arrives with M4; broader voice input later |
| Push-to-talk | ✅ | 🚫 | - | |
| Location sharing | ✅ | 🚫 | - | |
| Node pairing | ✅ | ✅ | P1 | Backend device identity (milestone B1) landed: QR pairing issues per-device, revocable, scoped `tcd_` tokens over a pinned-TLS or tailnet gateway listener; see `docs/MOBILE_SECURITY.md` and `docs/MOBILE_APP.md` |
| APNs push notifications | ✅ | 🚧 | P2 | First-party device push landed (B2): content-free policy (`push_policy`) + notifier (`first_party_push.rs`) reusing `ApnsPusher`, device-linked registration (`PUT/DELETE /api/devices/me/push`, `/live-activity/{id}`, `/live-activity-start-token`), live-stream suppression, throttled Live Activity, and prune-on-410. Live Activity run routing wired (M3): registrations carry the `thread_id`/`job_id` they mirror, and the notifier auto-tracks a run from that association — run-progress events emit throttled Live Activity updates to the per-activity token, `response` emits the end, and a run beginning on a device with a push-to-start token but no active activity emits a one-shot push-to-start; a Live Activity 410 prunes only that activity (or start token), never the alert registration. Client push handling landed (M2): `apps/ios` `AppDelegate` registers for remote notifications while paired and `PUT`s the APNs token over the pinned client (`DELETE` on unpair); `PushCoordinator` registers the risk-split categories (inline Approve/Deny for low-risk only, Open-only deep-link for high-risk), routes content-free pushes to `thinclaw://` deep links, and hands silent pushes to `BackgroundRefresh` (which fetches fresh snapshots then reloads widget timelines — see the snapshot-pipeline row); a new `ThinClawNotificationService` app-extension rewrites approval title/body from `GET /api/chat/approvals` over the shared pinned connection. Off without APNs config; mock-tested only — the whole app + NSE extension compile in the `build-app` hard gate, but real Apple/TestFlight delivery is still credential-gated |
| Share to OpenClaw (iOS) | ✅ | 🚫 | - | iOS share sheet integration; post-M5 candidate |
| Background listening toggle | ✅ | 🚫 | - | iOS background audio |

### Owner: ThinClaw Agent

---

## 12. ThinClaw Desktop

> ⚠️ **Experimental — not yet production-ready.** ThinClaw Desktop is under heavy
> active development; a number of features/flows were originally built against an
> earlier agent framework and are still being migrated to the current ThinClaw
> runtime. Treat desktop-facing parity rows as in-progress.

ThinClaw Desktop is the maintained macOS app in this repository. It uses Tauri v2 with a Rust backend and React frontend under `apps/desktop/`.

The desktop app has two intentional runtime surfaces:

- **Direct AI Workbench**: local app workflows such as project creation, model/provider setup, app shell, local state, onboarding, and desktop-specific settings.
- **ThinClaw Agent Cockpit**: the embedded ThinClaw runtime and agent-facing bridge for sessions, tools, approvals, channels, and runtime status.

Current contracts live in:

- [`apps/desktop/documentation/runtime-boundaries.md`](apps/desktop/documentation/runtime-boundaries.md)
- [`apps/desktop/documentation/bridge-contract.md`](apps/desktop/documentation/bridge-contract.md)
- [`apps/desktop/documentation/runtime-parity-checklist.md`](apps/desktop/documentation/runtime-parity-checklist.md)

Historical Scrappy/OpenClaw component inventories were removed from this parity ledger because they duplicated and drifted from the desktop documentation.

### Parity batch — agent-internals & channel config (2026-06-29)

First parity-closure batch (merged or in-flight; see `OVERHAUL_BACKLOG.md` for per-item state):

| Capability | Status | Surface |
|---|---|---|
| Dual-mode bridge contract: `RouteMode`, typed `BridgeError`, `gated()`, `ROUTE_TABLE` linter | in-flight — `ROUTE_TABLE` now classifies **all** registered commands with a total-coverage guard (`all_registered_commands_are_classified`); per-command `BridgeError` error-type migration still pending | `thinclaw/bridge.rs` |
| Undo / redo | in-flight | `thinclaw_undo`/`_redo` commands + cockpit toolbar buttons |
| Session search | merged | `thinclaw_session_search` + Session Search panel |
| Checkpoints / rollback | merged | `list`/`diff`/`restore` commands + Rollback panel |
| Trajectory viewer | merged | `stats`/`records` commands + Trajectory panel |
| Agent eval | in-flight | `thinclaw_experiments_list_envs`/`run_eval` |
| Lifecycle events (context compaction, advisor, self-repair) | in-flight | new `StatusUpdate` variants → `UiEvent::AgentLifecycleEvent` |
| Channel runtime config | in-flight | `Channel::config_schema()` + DTOs + read/submit commands + Channel Config panel |
| Honest sidecar status | in-flight | `image_configured`/`tts_configured` (CLI tools — availability, not process state) |
| Tool-policy enforcement | in-flight | `disabled_tools` deny-list enforced in dispatcher preflight |

### Owner: ThinClaw Desktop

---

## 13. Web Interface

| Feature | OpenClaw | ThinClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Control UI Dashboard | ✅ | ✅ | - | Web gateway with chat, memory, jobs, logs, extensions, costs |
| Cost Dashboard (WebUI) | ❌ | ✅ | - | Dedicated Costs tab: time‑range selector (Today/7d/30d/All), daily spend bar chart, budget progress, total tokens, active models, actions/hr summary cards + per‑model horizontal bar chart (input/output split) + model breakdown table with requests, cost shares, and totals. Backed by `/api/costs/summary` (CostTracker historical data) + `/api/gateway/status` (CostGuard real-time). SSE `cost_alert` toast notifications. Secure CSV export via `/api/costs/export` (Authorization header). Cost reset via `POST /api/costs/reset` (clears CostTracker + CostGuard). Auto‑refreshes every 30s. Persistent across restarts (60s background flush + shutdown flush). Provider-sourced pricing forwarded to CostGuard budget enforcement. Entry compaction at 50K cap preserves daily/model aggregates. UTC-consistent date handling. ([`src/channels/web/static/app.js`](src/channels/web/static/app.js)) |
| Channel status view | ✅ | ✅ | P2 | `ChannelStatusView` with per-channel state machine, table/JSON format ([`src/channels/status_view.rs`](src/channels/status_view.rs)) |
| Agent management | ✅ | ✅ | P3 | CLI: `agents list/add/remove/show/set-default`; `AgentRouter` dispatch pipeline |
| Model selection | ✅ | ✅ | - | TUI only |
| Config editing | ✅ | ✅ | P3 | `Settings.set()/.get()/.list()/.reset()` with typed path-based access ([`crates/thinclaw-settings/src/settings_impl.rs`](crates/thinclaw-settings/src/settings_impl.rs)). Web gateway Settings tab with grouped sections (Notifications, Heartbeat, Agent, Channels [Telegram/Signal/Discord/Slack/Nostr/iMessage/Gmail/Gateway], Safety, Features), toggle switches, import/export. Includes `agent.main_tool_profile`, `agent.worker_tool_profile`, `agent.subagent_tool_profile`, `agent.subagent_transparency_level`, and `channels.telegram_subagent_session_mode` plumbing |
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
| Routine failure backoff + auto-disable | ❌ | ✅ | - | Consecutive run failures push `next_fire_at` out on an escalating backoff schedule (from the 3rd failure) and auto-disable the routine after 10 in a row with a warn-level operator log; success resets the counter ([`crates/thinclaw-agent/src/routine_engine.rs`](crates/thinclaw-agent/src/routine_engine.rs), [`src/agent/routine_engine.rs`](src/agent/routine_engine.rs)) |
| Routine run-lease model | ❌ | ✅ | - | A renewable `lease_expires_at` column replaces the old fixed 10-minute zombie-reap TTL, which falsely failed legitimately long-running jobs; workers and subagents periodically renew the lease while executing, and the reaper only fails runs whose lease has actually expired. Legacy rows with a NULL lease fall back to a conservative 3600s TTL instead of the old 10-minute cutoff. Main-session heartbeat runs injected into the live session now correctly complete as `Ok` instead of being falsely reaped as zombies (migration [`V27__routine_run_lease.sql`](migrations/V27__routine_run_lease.sql), [`crates/thinclaw-db`](crates/thinclaw-db), [`src/agent/routine_engine.rs`](src/agent/routine_engine.rs)) |
| Cron stagger controls | ✅ | ✅ | P3 | `StaggerConfig` + `CronGate`; jitter delay is now actually applied before a staggered run executes rather than only computed ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs), [`src/agent/routine_engine.rs`](src/agent/routine_engine.rs)) |
| Cron finished-run webhook | ✅ | ✅ | P3 | `FinishedRunPayload` + `notify_finished_run()` is now actually called when a routine run finishes, rather than defined but unreferenced ([`src/agent/cron_stagger.rs`](src/agent/cron_stagger.rs), [`src/agent/routine_engine.rs`](src/agent/routine_engine.rs)) |
| Timezone support | ✅ | ✅ | - | Via cron expressions |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers; unknown persisted mode values now surface explicitly as `unknown` with raw mode preserved for observability |
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
| Hook reliability controls | ❌ | ✅ | P2 | Per-hook consecutive-failure tracking auto-disables a FailOpen hook after 5 failures in a row (`MAX_CONSECUTIVE_HOOK_FAILURES`), with manual re-enable; FailClosed hooks are exempt from auto-disable so a failing guardrail keeps blocking instead of silently becoming a pass-through; hook contexts now deliver real invocation metadata instead of an always-empty default; hook payloads are validated against their declared `HookPoint` shape before dispatch. An additive, typed `HookPatch` mechanism is consumed end to end: `HookRegistry` requests a typed patch from each hook after a successful `execute` and the dispatcher's `BeforeLlmInput` site honors `LlmInput` user/system-message overrides via `run_returning_event` (`AgentStart` patches are accepted structurally but not yet honored) |
| Outbound webhooks | ✅ | ✅ | P2 | Fire-and-forget lifecycle event delivery |
| Heartbeat system | ✅ | ✅ | - | Periodic execution with self-critique feedback loop: post-completion evaluator persists critique to `heartbeat.last_critique`, next heartbeat reads and avoids repeating mistakes. Configurable `max_iterations` (WebUI Settings → Heartbeat, default 10). Stuck heartbeats write targeted self-critique and notify the user via preferred channel. Output routing honors `target` (`none` runs silently, `chat` delivers to the default surface, a channel name overrides delivery) and `include_reasoning` (retains the reasoning chain in the emitted summary). `HeartbeatRunner` now actually honors `enabled=false` and short-circuits without touching the workspace or LLM ([`crates/thinclaw-agent/src/heartbeat.rs`](crates/thinclaw-agent/src/heartbeat.rs)). The stub `WorkspacePromptAssemblyPort` (zero production callers) was deleted; `src/agent/workspace_prompt_assembly.rs` is now a doc-only, uncompiled module. |
| Notification routing | ✅ | ✅ | P1 | `NotificationSettings` (preferred_channel + recipient) in `Settings`. Per-channel broadcast validation guards. Wizard step 16 collects preferences. Heartbeat/routine notifications route to user-chosen channel. WebUI Settings tab exposes notification preferences. |
| Wizard notification preferences | ❌ | ✅ | P1 | `step_notification_preferences` in wizard: auto-selects single channel, prompts for multi-channel, collects recipient (phone/chat ID/email). |
| Gmail pub/sub | ✅ | ✅ | P3 | `GmailConfig` + `parse_pubsub_push()` + sender filtering ([`src/channels/gmail_wiring.rs`](src/channels/gmail_wiring.rs)) |
| Repo project supervisor | ❌ | ✅ | P2 | GitHub App backed repository project automation; default off (`repo_projects.enabled`). Live pipeline: open/track a PR from the write target, poll+classify CI check runs / workflow jobs (redacted log triage + PR comments), bounded CI-repair re-dispatch into the sandbox, gather review+branch-freshness evidence, evaluate the guarded merge gate against real evidence, and squash-merge+delete branch only in `maintainer_auto_merge`. Cross-surface GitHub connector (discover+enroll repos via agent tools/CLI/gateway/desktop card/WebUI), restart recovery, durable webhook-delivery storage + project-run records, and live SSE. Shared domain types in [`crates/thinclaw-repo-projects`](crates/thinclaw-repo-projects), root runtime in [`src/repo_projects`](src/repo_projects), CLI in [`src/cli/repo_projects.rs`](src/cli/repo_projects.rs). See [`docs/REPO_PROJECT_SUPERVISOR.md`](docs/REPO_PROJECT_SUPERVISOR.md). |

### Owner: ThinClaw Agent

---

## 15. Security Features

| Feature | OpenClaw | ThinClaw | Notes |
|---------|----------|----------|-------|
| Gateway token auth | ✅ | ✅ | Bearer token auth on web gateway |
| Gateway RBAC principals (opt-in) | ❌ | ✅ | Extra gateway principals layered on the primary admin token, each with a token + role (`read_only`/`operator`/`admin`); requests are classified into a coarse capability (`ReadState`/`Chat`/`ManageConfig`) by method+path and gated against the caller's role in `auth_middleware`. Admin surfaces (`/api/settings`, `/api/providers`, `/api/extensions`, `/api/mcp`, `/api/hooks`, …) are admin-only. The classifier is **fail-closed**: only an explicit operator-writable allowlist (`/api/chat`, `/api/sessions`, `/api/memory`, `/api/jobs`) is Operator-mutable — every other state-changing route (e.g. `/api/gateway/restart`, `/api/autonomy/*`, `/api/experiments/runners`, pairing approval) is admin-only by default. Configured via `channels.gateway_principals` or the `GATEWAY_PRINCIPALS` env (JSON). Default-off: no principals ⇒ single-admin-token behavior, unchanged. Token→role→capability path (incl. `?token=` fallback) covered by end-to-end middleware tests ([`crates/thinclaw-gateway/src/web/rbac.rs`](crates/thinclaw-gateway/src/web/rbac.rs), [`crates/thinclaw-gateway/src/web/auth.rs`](crates/thinclaw-gateway/src/web/auth.rs), [`crates/thinclaw-settings/src/gateway_rbac.rs`](crates/thinclaw-settings/src/gateway_rbac.rs); see `src/NETWORK_SECURITY.md` → Authentication → RBAC principals) |
| Device pairing | ✅ | ✅ | `PairingStore` with challenge-response states + fingerprint verification ([`crates/thinclaw-safety/src/device_pairing.rs`](crates/thinclaw-safety/src/device_pairing.rs)) |
| Tailscale identity (passwordless auth) | ✅ | ❌ | No identity-based passwordless gateway auth exists. The former `TailscaleIdentity`/`extract_identity()`/`is_trusted_peer()` path was deleted as dead code; Tailscale survives only as outbound tunnel/serve exposure of the gateway (see the "Tailscale integration" row above, [`src/tunnel/tailscale.rs`](src/tunnel/tailscale.rs)). |
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
| Host-direct execution network isolation | ❌ | ✅ | Host `run_shell`/`run_script` with networking disabled are wrapped in a **hard** egress block: macOS seatbelt `(deny network*)`, Linux `bwrap --unshare-net`; `host_local_network_isolation()` reports the guarantee level ([`crates/thinclaw-tools/src/execution.rs`](crates/thinclaw-tools/src/execution.rs)) |
| Host-direct filesystem confinement (opt-in) | ❌ | ✅ | `THINCLAW_HOST_FS_SANDBOX=1` confines host-direct **writes** to the command workspace (plus temp/device paths) while keeping reads/exec broad: macOS seatbelt `(deny file-write*)` + workspace `subpath` allow, Linux `bwrap --ro-bind / /` + writable workspace `--bind` + `--tmpfs /tmp`. Default-off (zero behavior change when unset); kernel enforcement proven by a behavioral `sandbox-exec` test on macOS ([`crates/thinclaw-tools/src/execution.rs`](crates/thinclaw-tools/src/execution.rs); see `src/NETWORK_SECURITY.md` → Host-Direct Command Execution) |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ✅ | ([`crates/thinclaw-safety/src/elevated.rs`](crates/thinclaw-safety/src/elevated.rs)) |
| Safe bins allowlist | ✅ | ✅ | `THINCLAW_SAFE_BINS_ONLY` + extensible `THINCLAW_EXTRA_BINS` + desktop bins (open, xdg-open, pbcopy, pbpaste, tee, xargs, chmod) |
| LD*/DYLD* validation | ✅ | ✅ | Blocks `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, etc. |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) + `..` traversal in shell commands |
| Credential theft via env injection | ✅ | ✅ | Shell env scrubbing + command injection detection + LD*/DYLD* blocking + safe bins |
| Session file permissions (0o600) | ✅ | ✅ | Handled by OS keychain + filesystem perms |
| Skill download path restriction | ✅ | ✅ | ([`crates/thinclaw-safety/src/skill_path.rs`](crates/thinclaw-safety/src/skill_path.rs)) |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ✅ | SSRF prevention: blocks private IPs, cloud metadata, disallowed schemes ([`crates/thinclaw-safety/src/media_url.rs`](crates/thinclaw-safety/src/media_url.rs)) |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization, and context-file scanning for AGENTS.md/SOUL.md/USER.md with invisible Unicode stripping. Detected-threat severity now drives policy: `Critical` content is dropped from the prompt with a replacement notice, `High` logs a warning but keeps the content, and lower severities only log; threat regexes are compiled once behind `LazyLock` instead of per-check ([`crates/thinclaw-safety/src/sanitizer.rs`](crates/thinclaw-safety/src/sanitizer.rs), [`crates/thinclaw-agent/src/prompt_sanitation.rs`](crates/thinclaw-agent/src/prompt_sanitation.rs), [`src/workspace/workspace_core.rs`](src/workspace/workspace_core.rs)) |
| Prompt-surface PII redaction | ✅ | ✅ | System prompt assembly now hashes actor/conversation identifiers for non-Discord channels and replaces raw actor-private path labels with prompt-safe aliases ([`crates/thinclaw-safety/src/pii_redactor.rs`](crates/thinclaw-safety/src/pii_redactor.rs), [`src/workspace/workspace_core.rs`](src/workspace/workspace_core.rs), [`src/agent/dispatcher/prompt_context.rs`](src/agent/dispatcher/prompt_context.rs)) |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ❌ | Not implemented; the never-wired `DangerousToolTracker` was removed as orphaned dead code. |
| OS governance env vars | ❌ | ✅ | ThinClaw Desktop live-checks macOS permissions and passes to ThinClaw: `ACCESSIBILITY_GRANTED` (AXIsProcessTrusted), `SCREEN_RECORDING_GRANTED` (CGPreflightScreenCaptureAccess), `SCREEN_CAPTURE_ENABLED` (user toggle) |

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

## 17. ThinClaw Desktop Integration

ThinClaw Desktop integration is documented in `apps/desktop/documentation/` instead of this parity ledger. Keep runtime boundaries, bridge contracts, packaging checks, and manual validation evidence there so desktop-facing decisions stay close to the app code.

This file keeps broad ThinClaw parity and shipped-runtime capability tracking only.

### Owner: ThinClaw Desktop

---

## 20. Shipped Built-in Tools

> Counts are intentionally omitted; the live tool registry is authoritative. See
> [`src/tools/README.md`](src/tools/README.md) and `crates/thinclaw-tools-core`.
> Some tools are conditional or feature-gated.

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
| `skill_install` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | Install prompt-based skills from direct content, ClawHub, configured GitHub taps, or configured `/.well-known/skills` registries with quarantine scanning, provenance lock files, and risky-install approval gates |
| `skill_list` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | List installed skills |
| `skill_read` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | Read skill source |
| `skill_remove` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | Remove a skill |
| `skill_reload` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | Hot-reload skills from disk |
| `skill_search` | [`skill_tools.rs`](src/tools/builtin/skill_tools/mod.rs) | Search local skills, ClawHub, configured GitHub taps, and configured `/.well-known/skills` registries |
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

### 20.10 Media Generation (5 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `image_generate` | [`comfyui.rs`](src/tools/builtin/comfyui.rs) | Prompt-to-image generation through configured ComfyUI workflow; returns JSON plus renderable image artifacts |
| `comfy_health` | [`comfyui.rs`](src/tools/builtin/comfyui.rs) | Read-only ComfyUI server/object-info health check |
| `comfy_check_deps` | [`comfyui.rs`](src/tools/builtin/comfyui.rs) | Read-only workflow dependency report for models and custom nodes |
| `comfy_run_workflow` | [`comfyui.rs`](src/tools/builtin/comfyui.rs) | Run bundled or approved API-format ComfyUI workflows |
| `comfy_manage` | [`comfyui.rs`](src/tools/builtin/comfyui.rs) | Explicit setup/lifecycle/model/node actions; registered only when lifecycle management is enabled |

### 20.11 Hardware & Environment (3 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `device_info` | [`device_info.rs`](src/tools/builtin/device_info.rs) | System hardware/OS info |
| `screen_capture` | [`screen_capture.rs`](src/tools/builtin/screen_capture.rs) | macOS/Linux screenshot capture |
| `tts` | [`tts.rs`](src/tools/builtin/tts.rs) | Text-to-speech synthesis |

Source-present but not currently registered as built-in runtime tools: `location` and `camera_capture` ([`src/tools/builtin/mod.rs`](src/tools/builtin/mod.rs), [`src/tools/registry.rs`](src/tools/registry.rs), [`src/app.rs`](src/app.rs), [`src/main.rs`](src/main.rs)).

Removed dead code: the never-registered `slack_actions` / `telegram_actions` built-in Rust tools were deleted — Slack and Telegram actions ship as packaged **WASM tools** (`tools-src/slack`, `tools-src/telegram`), which is the intended production path. `discord_actions` was also removed; Discord is currently supported as an **inbound channel adapter** only, with no action/moderation tool (a first-party Discord action tool is a possible future addition, gated behind bot-token config).

### 20.12 UI & Interaction (4 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `canvas` | [`canvas.rs`](src/tools/builtin/canvas.rs) | A2UI canvas panels with interactive components |
| `clarify` | [`clarify.rs`](src/tools/builtin/clarify.rs) | Structured questions with multiple-choice/free-form options |
| `vision_analyze` | [`vision.rs`](src/tools/builtin/vision.rs) | Proactive image/video analysis via multimodal LLM |
| `emit_user_message` | [`agent_control.rs`](src/tools/builtin/agent_control.rs) | Emit messages to user (implicit capability tool) |

### 20.13 Process & Task Management (3 tools)

| Tool | Source | Description |
|------|--------|-------------|
| `process` | [`process.rs`](src/tools/builtin/process.rs) | Background process lifecycle (start/poll/wait/kill/write) with shared registry + auto-reaper |
| `todo` | [`todo.rs`](src/tools/builtin/todo.rs) | In-session task planner with merge/replace modes; survives context compaction |
| `time` | [`time.rs`](src/tools/builtin/time.rs) | Current time, timezone conversion, date arithmetic |

### 20.14 Smart Home & IoT (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `homeassistant` | [`homeassistant.rs`](src/tools/builtin/homeassistant.rs) | Home Assistant REST API: entity listing, state queries, service calls; gated on `HASS_URL` + `HASS_TOKEN` |

### 20.15 Builder (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `build_software` | [`builder/core.rs`](src/tools/builder/core.rs) | LLM-driven iterative build loop for WASM tools, CLI apps, and scripts |

### 20.16 Utility (1 tool)

| Tool | Source | Description |
|------|--------|-------------|
| `echo` | [`echo.rs`](src/tools/builtin/echo.rs) | Echo input back (testing/debugging) |

### 20.17 Supporting Infrastructure

| Component | Source | Description |
|-----------|--------|-------------|
| Tool profile controls | Runtime settings + policy surfaces | Canonical tool-surface shaping now uses runtime tool profiles and policy evaluation; stale standalone `ToolsetRegistry` module was removed |
| OSV malware scanner | [`osv_check.rs`](crates/thinclaw-safety/src/osv_check.rs) | Package scanning via Google OSV API for MCP servers; MAL-* advisory detection, 1h cache, fail-open |
| Intent display | [`intent_display.rs`](src/tools/intent_display.rs) | Human-readable intent hints and argument extraction for all tool calls |
| Tool registry | [`registry.rs`](src/tools/registry.rs) | Protected-name enforcement, conditional registration, rate limiting |

### Owner: ThinClaw Agent

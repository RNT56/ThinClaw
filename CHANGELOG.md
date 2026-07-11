# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ThinClaw is a fork of [IronClaw](https://github.com/nearai/ironclaw) by NEAR AI.
> Releases prior to v0.13.0 were published under the IronClaw name and are not listed here.
> See the [IronClaw releases](https://github.com/nearai/ironclaw/releases) for earlier history.

## [Unreleased]

### Added

- Native Swift iOS + watchOS surface (`apps/ios`): device pairing (QR + scoped tokens, TLS listener), streaming chat, push-pull approvals, LAN discovery, APNs push, home-screen widgets with Live Activity run tracking, and a watchOS companion with companion tokens/relay and wrist approvals.
- Typed Rust client SDK (`crates/thinclaw-client`) wrapping the gateway HTTP+SSE surface (send/stream/history/threads/approvals) with an OpenAI-compatible fast path and tokens redacted from `Debug`.
- Prometheus observability backend with `GET /metrics`.
- Real `GET /api/health` readiness probe: 200 only when the database is reachable, an LLM provider is configured, and an inbound channel is wired; otherwise 503.
- Plan mode (`/plan`): propose actions and approve before running.
- Unified `/rewind` command: restores both conversation (to turn N) and files via turn-tagged shadow-git checkpoints, with a non-destructive list/dry-run.
- Argument-scoped tool permission rules.
- Zero-config `web_search` built-in tool (keyless DuckDuckGo, SSRF-guarded, size-capped).
- MCP health monitor that populates `McpRuntimeHealth` and auto-reconnects crashed stdio servers with per-server exponential backoff.
- Loop-hardening observability: 8 new `thinclaw_loop_*` metric series covering loop and phase starts, stops, iterations, retries, and timing.
- `ExtensionKind::NativePlugin`: signature-gated native plugin loading, default-off.
- Discord Ed25519 webhook signature verification (host-side).
- Repo-project supervisor: GitHub App-backed automation with a live pipeline and connector, plus an autoplanner behind the `REPO_PROJECTS_AUTOPLAN` env gate (default-off).
- Opt-in gateway RBAC with role-scoped capability gating.
- Opt-in host-direct filesystem confinement (seatbelt/bwrap).
- First-party push: `ApnsPusher`, device push registration, and a content-free notifier.
- Whole-agent encrypted backup: `thinclaw backup` export/import.
- Desktop cockpit panels for session search, trajectory archive, rollback/checkpoints, and channel config, plus an undo/redo chat toolbar and advisor/self-repair/context-compaction lifecycle events.
- Rolling daily file log sink.

### Changed

- Landed a 13-workstream audit-driven remediation across security, DB correctness, WASM channels, self-repair, desktop, experiments, LLM routing, docs, and test/CI infrastructure.
- Hardened native and WASM channel lifecycles with owned shutdown/drain paths, bounded reconnect behavior, APNs HTTP/2 support, Gmail token refresh, and channel-manager ownership of hot-reload stream forwarders.
- Decomposed 10 historical god-files into focused directory modules, and added a CI god-file size guard (`MAX_LINES=2000`).
- Consolidated history/store onto `thinclaw-db`.
- Completed the `thinclaw-media` crate migration, slimming `src/media` to a facade.
- Marked `StatusUpdate` `#[non_exhaustive]` with fallback match arms.
- `ROUTE_TABLE` now classifies all 346 desktop bridge commands, enforced by a total-coverage CI test.
- Migrated gated desktop commands to a typed `BridgeError`.
- Hardened the agentic loop end to end: retry policy, compaction summary reuse, and failure backoff.
- Gated sub-agents by the shared `CostGuard` with an optional per-principal concurrency cap (`SUBAGENT_MAX_PER_PRINCIPAL`).
- Migrated the desktop frontend to Tailwind CSS v4, consolidated cargo/npm dependency bumps, and added a `[workspace.dependencies]` table for shared deps.
- Marked ThinClaw Desktop as experimental.
- Refreshed channel, APNs/browser-push, ComfyUI media-generation, setup, CLI, tool/skill, parity, and dependency documentation to match the current native lifecycle, built-in sidecar, and WASM channel architecture, while keeping real-provider smoke requirements clearly marked as credential-gated validation.
- Removed stale audit-planning closure documents from the current docs tree.

### Fixed

- Closed the empty `gateway_auth_token` auth bypass at both layers by filtering empty/whitespace tokens.
- OAuth state is now generated and constant-time-validated end to end (CSRF).
- Added DNS-rebinding protection via host pinning and fixed the trusted-proxy CIDR check.
- Sanitized libSQL FTS5 `MATCH` injection.
- Confined sandbox proxy credentials behind a store-backed resolver.
- Enforced WASM table/instance resource limits.
- Fixed the multibyte-UTF-8 `split_message` panic in the telegram, slack, and discord channels.
- Added routine run-lease zombie reaping.
- Fixed streaming `finish_reason` for tool calls.
- Guarded the `image_gen` progress divide-by-zero.
- Replaced hot-path `expect`/parse panics with typed errors and moved `OnceLock` hot-path regexes/selectors off the hot path.
- Removed blocking syscalls from async functions.
- Added Gmail unattended OAuth token refresh (proactive + on-401) so long-running deployments don't silently stop.
- Restored cross-turn tool-result continuity: `Thread::messages()` reconstructs prior turns' tool calls and results.
- Invalidated the learning ready-provider cache on bulk settings import.
- Resolved an RBAC route-coverage gap and a pre-auth KDF/zip DoS.

### Removed

- Erased roughly 7K lines of verified-dead code, including 14 `src/safety` orphans, 3 unwired CLI modules, the `self_message` anti-loop module, the `qr_pairing` scaffold, the tailscale identity module, the standalone heartbeat runner, the `SmartRoutingProvider` decorator, and the `InferenceRouter` chat modality.
- Removed the orphaned `DangerousToolTracker`.
- Removed dead desktop frontend code, dead web-search probe commands, and the dead WhatsApp QR login.

### Security

- Constant-time OAuth-state comparison via `subtle::ConstantTimeEq`.
- `subtle`-based constant-time comparisons in `thinclaw-tools` for pairing secrets, webhook secrets, and device tokens.
- Added desktop backend advisory scanning in CI (`cargo deny check advisories`) with a desktop-scoped `deny.toml`.
- Zero cargo-deny advisory ignores (`deny.toml` `[advisories] ignore = []`).
- Patched RUSTSEC-2026-0187 (pdf-extract/lopdf), RUSTSEC-2026-0188 (wasmtime-wasi), and RUSTSEC-2026-0190 (anyhow) at the source.
- Hardened the injection regex, audited in-memory secret access, and warn on token-in-URL.

## [0.14.0](https://github.com/RNT56/ThinClaw/releases/tag/v0.14.0) - 2026-05-14

### Added

- ComfyUI-backed media generation with native `image_generate`, `comfy_health`, `comfy_check_deps`, `comfy_run_workflow`, and approval-gated `comfy_manage` tools.
- ComfyUI REST/WebSocket workflow execution, API-format workflow validation, output sanitization, dependency scanning, and bundled starter workflows.
- Trusted `creative-comfyui` skill, ComfyUI configuration/settings, CLI commands, and documentation for local/cloud setup and generation.
- Renderable generated-media artifacts in web gateway tool results.

## [0.13.7](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.7) - 2026-04-24

### Added

- ACP v1 compatibility work for editor clients, including typed wire messages, JSON-RPC transcript coverage, prompt/session lifecycle handling, permission round-tripping, client filesystem/terminal bridges, MCP stdio descriptor wiring, and stdout cleanliness checks.
- AgentEnv and research campaign plumbing with benchmark adapters, Research WebUI/API surfaces, and trajectory metadata for token/logprob capture when providers support it.
- Extension manifest foundations for tool, channel, memory, context, and native-plugin contributions, with native plugin loading gated behind explicit unsafe configuration and signature metadata.
- WASM tool host-mediated invocation support through declared aliases with policy, approval, timeout, recursion-depth, and audit controls.
- Provider-native streaming capability metadata and streaming paths across the LLM stack, with simulated streaming retained only as an explicit fallback.
- Release and deployment improvements for Linux, Docker Compose, gateway access, readiness probes, WASM extension bundles, and build-profile documentation.

### Changed

- ACP capability advertisement now tracks implemented and tested behavior instead of exposing placeholder features.
- Release workflow validation now fails when WASM extension manifests are missing sources, capabilities, or bundle outputs instead of publishing incomplete artifacts.
- Setup wizard tests and docs now agree on the documented 12-step quick setup flow.
- User-tools documentation now prefers the canonical `~/.thinclaw/user-tools/` path while preserving the legacy underscore path as an alias.

### Fixed

- Docker availability detection no longer hangs when Docker Desktop or compatible runtimes leave `docker version`/`docker info` blocked; CLI probes now have killable per-command timeouts.
- Dispatcher streaming tests now model native streaming support explicitly, matching the production gate that avoids fake progressive streaming for non-native providers.
- ACP local validation, default build checks, feature-profile checks, and the broad library suite are green with the Docker hang fixed.
- Trusted-proxy web gateway identities no longer accept compatibility user/actor override parameters intended only for bearer-token development paths.
- Cargo-deny advisory coverage is clean after updating `rustls-webpki`, and default/full clippy gates are clean under `-D warnings`.
- Web gateway CORS now uses the actual bound listener port for ephemeral port binds.

## [0.13.6](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.6) - 2026-04-14

### Added

#### Routing Engine V2
- Unified `RoutePlanner` with three strategies: `Solo`, `Failover`, and new `AdvisorExecutor` mode (executor lane runs the turn, advisor lane consults and auto-escalates on risky or complex turns)
- Dispatcher interception layer: all outbound LLM calls flow through the routing policy
- Live cutover from legacy routing — no migration required
- Routing telemetry: per-call latency, token counts, and strategy-hit histograms
- Health signal tracking for automatic failover provider selection

#### Self-Improving Learning Runtime
- Closed-loop learning system: the agent reviews its own conversations, extracts patterns, and refines future behaviour
- Conversation recall store for pattern matching across sessions
- Learning conversation analysis pipeline with configurable feedback thresholds

#### Agent Subsystems
- **Checkpoint system**: durable mid-turn state snapshots for crash recovery and long-running tasks
- **Personality overlay engine**: adaptive tone/style selection based on conversation context and channel type
- **Session search**: full-text search across all agent sessions with relevance ranking
- **Context monitor**: proactive context-window utilisation tracking with compaction triggers

#### Server Decomposition (Monolith Breakup)
- Extracted 13 dedicated handler modules from the 7,900-line `server.rs` monolith: `chat`, `experiments`, `extensions`, `gateway`, `jobs`, `learning`, `logs`, `memory`, `pairing`, `projects`, `providers`, `routines`, `settings`
- New shared modules: `identity_helpers`, `rate_limiter`, `static_files`
- Each handler module is independently testable with clear API boundaries

#### New Built-in Tools
- `browser_a11y`: accessibility-tree-based page interaction for headless browser automation
- `browser_cloud`: cloud browser service integration (Browserbase, Steel, etc.)
- `execute_code`: sandboxed code execution in multiple languages
- `search_files`: recursive file search with glob patterns and content matching
- `clarify`: structured clarification requests to disambiguate user intent
- `send_message`: cross-channel message delivery from within tool pipelines
- `skill_tools`: runtime skill management (install, reload, inspect)
- `learning_tools`: manual learning annotation and pattern review
- `moa` (Mixture of Agents): fan-out queries across multiple models and merge results
- `advisor`: strategic consultation tool for the AdvisorExecutor routing strategy
- Enhanced `browser` tool with screenshot capture, element interaction, and cookie management
- Enhanced `shell` tool with working directory tracking and environment variable passthrough

#### Safety and Security
- **PII redactor**: regex + NER-based PII detection and masking for logs and stored conversations
- **Smart approve**: risk-scoring engine for tool calls with configurable thresholds
- **OSV vulnerability checker**: proactive dependency scanning against the OSV database
- Enhanced `sanitizer` with HTML/XSS stripping and prompt-injection pattern detection

#### Skill System Expansion
- `github_source`: install skills directly from GitHub repositories (public and private)
- `remote_source`: fetch skills from arbitrary HTTPS URLs with SHA256 verification
- `well_known_source`: curated skill catalog discovery via `.well-known/thinclaw-skills.json`
- `quarantine`: sandboxed skill staging with integrity checks before promotion to active use
- Skill file watcher for automatic hot-reload on disk changes

#### Personality Pack Library
- 6 production-ready personality packs: `default`, `creative_partner`, `mentor`, `minimal`, `professional`, `research_assistant`
- Psychographic identity profiles with tonal overlays and channel-aware formatting hints

#### TUI Skin System
- TOML-based terminal skin engine with runtime switching
- 6 bundled skins: `athena`, `cockpit`, `delphi`, `midnight`, `olympus`, `solar`
- Per-skin colour palettes, glyph sets, and layout tuning

#### Database and Migrations
- 8 new migrations (V10–V17): identity registry, actor-scoped history, job context, agent capability isolation, learning tables, experiments platform, research tables, experiment cost breakdown
- Schema divergence detection test suite with allowlist-based governance
- Database contract tests enforcing libSQL/Postgres feature parity
- Enhanced Postgres backend with full learning, experiment, and identity store support

#### CI and Build
- Dedicated `light` profile test job: validates the default user-facing build separately from the full-feature build
- Consolidated CI: merged `code_style.yml` and `test.yml` into a single `ci.yml` workflow
- Codecov integration for coverage tracking
- `build.rs` for compile-time asset embedding and version metadata

#### WebUI Overhaul
- Routing strategy configuration: mode selector tiles (Solo / Failover / AdvisorExecutor) with conditional advisor config fields
- Provider management redesign: slot-based editor with credential sync, connection testing, and model discovery
- Cost dashboard: daily vertical bar chart, time-range selector (7d/30d/90d), budget progress bars
- Experiments UI: create, monitor, and compare A/B experiments with cost breakdown
- Learning insights panel: review extracted patterns and agent self-improvement metrics
- Modern toggle switches replacing all binary checkboxes
- Responsive layout overhaul with mobile-first grid system

#### LLM Infrastructure
- `credential_sync`: encrypted at-rest credential storage with secure keychain integration
- `model_guidance`: per-model capability hints (supports vision, supports tools, max context, etc.)
- `runtime_manager`: centralised LLM lifecycle management with connection pooling
- `usage_tracking`: per-session and per-agent token and cost accounting
- Enhanced `reasoning` module with chain-of-thought extraction and structured output parsing
- Enhanced `provider_factory` with lazy initialisation and provider health probing

### Fixed

- Learning conversation recall store path was missing from the database layer
- Subagent executor now correctly propagates cancellation signals to child tasks
- WebUI settings page uses full browser width (removed legacy `max-width` constraint)
- Telegram HTML formatter handles nested bold/italic/code spans correctly
- WASM channel wrapper correctly forwards capability JSON updates
- Snapshot tests updated to reflect new settings schema
- Provider editor credential fields preserve values across tab switches

### Changed

- Bumped Rust edition to 2024, minimum toolchain to 1.92
- Cargo.lock updated with 50+ dependency upgrades
- Removed stale planning documents: `rewrite-docs/`, `docs-audit/`, `Agent_flow.md`, `audit.md`, `database_divergence_plan.md`
- Comprehensive README rewrite with updated architecture diagrams and feature matrix
- CLAUDE.md updated with current module map, test patterns, and contribution guidelines
- FEATURE_PARITY.md refreshed against current codebase capabilities
- Expanded `.gitignore` with OS files, local databases, IDE settings, log patterns, and build artifacts

## [0.13.2](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.2) - 2026-03-28

### Added

- Multimodal media pipeline for all channels — images, audio, video, and documents are downloaded and routed to the LLM across Telegram, Discord, Signal, iMessage, WhatsApp, and Slack
- Discord native channel: CDN attachment download with 20MB size limit
- Signal channel: typed `SignalAttachment` struct, reads binary from signal-cli's local attachment store
- iMessage channel: queries `attachment` + `message_attachment_join` tables from chat.db, reads files from disk
- WhatsApp WASM channel: 2-step Cloud API media download (media URL → binary), supports image/audio/video/document/sticker with captions
- Slack WASM channel: file download via `url_private_download` with Bearer auth
- WIT `media-attachment` record and `attachments` field on `emitted-message` for WASM channel binary media transport
- WASM host boundary: 20MB per-file and 50MB per-message attachment size limits
- BOOT.md startup hook: pre-reads workspace docs instead of relying on tool calls
- Multi-provider LLM routing: Provider Vault, agent model switching, wizard fallback step
- Runtime-configurable Claude Code model/max-turns via WebUI
- Active channel names injected into LLM system prompt
- Settings page UX overhaul: subtabs, collapsible sections, search
- Provider Vault moved to dedicated tab with enhanced broadcast logging
- Broadcast support for WASM channels via `on_respond`
- BOOT.md startup briefing: daily logs, memory, heartbeat greeting with DB persistence

### Fixed

- Apple Mail timestamps showing year 2057 (+31 year offset)
- Telegram polling timeout mismatch causing 409 conflicts
- Telegram webhooks unreachable — tunnel forwards to wrong port
- Telegram falls back to polling when webhook fails + Provider UX fixes
- Numeric-looking strings (chat IDs) in `Option<String>` settings
- Detect and bail on tailscale funnel startup failures
- Bail on empty Tailscale hostname instead of producing broken URL
- Telegram broadcast delivery + thread deletion in WebUI
- Boot hook delivery: Telegram, WebUI persistence, BOOT.md migration
- `memory_tree` fails with 'Input cannot be empty' on default params
- Settings array parsing, model reset, provider auto-enable, XSS
- Apple Mail polling crash + add apple_mail search/send tool
- Apple Mail `allow_from` wired to DB settings + security warning
- Prevent BOOTSTRAP.md from re-executing on every restart

### Other

- Update WASM artifact URLs and SHA256 checksums

## [0.13.0](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.0) - 2026-03-26

### Added

- Apple Mail channel + auto-start for macOS apps
- Notion WASM tool with full API coverage
- WebUI model routing settings + wizard cross-provider API key collection
- `/restart` command, hardened auto-approve, improved deployment docs
- DB-backed config resolvers, wizard local tools/sandbox clarity, bootstrap identity injection
- WebUI Settings tab, agent bootstrap/cleanup fixes, favicon update
- MCP stdio transport for MCP servers
- Dual WASM extension deployment options (download from releases vs bundled in binary)
- Wasmtime 36 upgrade, sysinfo API migration, module restructuring
- Full IronClaw agent engine integration + codebase audit

### Fixed

- `super::` prefix for test references to private constants
- Removed placeholder OAuth secrets, correctly documented auth models
- Corrected registry URLs and CI manifest lookup for slack-tool and telegram-mtproto
- Complete IronClaw → ThinClaw rebrand + fixed failing streaming test
- Stripped NEAR AI author references, fixed Linux build errors
- Updated cargo-dist version to 0.31.0 in release.yml
- Corrected WiX manufacturer whitespace to match cargo-dist expectation
- Updated WiX installer manifest from ironclaw to thinclaw
- Installed libasound2-dev for voice feature in CI
- Patched artifact download URLs into manifests, updated repo URLs
- Clear error when cargo-component missing, soft-fail bundled channels
- Wizard: 3 bugs — missing persist, silent fallback skip, loop allocations
- Wizard: bundled-wasm extraction for tool install step (top-priority install path)
- Registry: removed placeholder artifacts, allow sha256-null fallback
- Deployment bugs — desktop feature, repl guards, registry artifacts
- Rebranded Telegram/Discord channels and docs to ThinClaw

### Other

- Added `channels-docs/` with documentation for all 12 channels
- Added `tools-docs/` with documentation for all 11 WASM tools
- Added Gmail setup guide and fixed stale libSQL limitation claims
- Updated FEATURE_PARITY, CLAUDE.md, and setup README
- Added consolidated External Dependencies guide
- Added data directory layout, upgrade workflow, and reset procedures
- Docker worker & Claude Code infrastructure
- Applied cargo fmt to entire codebase
- Updated WASM artifact URLs and SHA256 checksums
- Removed dead code, wired full search pipeline into libsql
- Comprehensive CLAUDE.md update against actual codebase
- Renamed all IRONCLAW_* env vars to THINCLAW_*
- Rebranded all extension sources from IronClaw to ThinClaw
- Added comprehensive deployment guide

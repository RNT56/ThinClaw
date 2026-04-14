# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ThinClaw is a fork of [IronClaw](https://github.com/nearai/ironclaw) by NEAR AI.
> Releases prior to v0.13.0 were published under the IronClaw name and are not listed here.
> See the [IronClaw releases](https://github.com/nearai/ironclaw/releases) for earlier history.

## [0.13.6](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.6) - 2026-04-14

### Added

#### Routing Engine V2
- Unified `RoutePlanner` with three strategies: `Solo`, `Failover`, and new `AdvisorExecutor` mode (lightweight model drafts, heavyweight model verifies)
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
- **Vibe engine**: adaptive tone/style selection based on conversation context and channel type
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
- `advisor`: lightweight pre-screening tool for the AdvisorExecutor routing strategy
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

#### Persona Seed Library
- 6 production-ready persona seeds: `default`, `creative_partner`, `mentor`, `minimal`, `professional`, `research_assistant`
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

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ThinClaw is a fork of [IronClaw](https://github.com/nearai/ironclaw) by NEAR AI.
> Releases prior to v0.13.0 were published under the IronClaw name and are not listed here.
> See the [IronClaw releases](https://github.com/nearai/ironclaw/releases) for earlier history.

## [Unreleased]

### Added

- Channel-aware message formatting: Markdown auto-converted to platform-native format for Telegram (HTML), Slack (mrkdwn), and WhatsApp (bold/italic text)
- WhatsApp reply threading: responses appear as replies to the original message via Cloud API `context.message_id`
- Comprehensive `build-all.sh` script: auto-discovers and builds all WASM channels, deploys to `~/.thinclaw/channels/`
- Unit tests for `markdown_to_slack_mrkdwn()` converter (bold, strikethrough, links, headings, code blocks, mixed formatting)

### Fixed

- WebUI settings page now uses full browser width like other pages (removed `max-width: 800px` constraint)
- WhatsApp `reply_to_message` config was parsed but unused; now wired to the Cloud API `context` field

### Changed

- Updated channel documentation: added Message Formatting section, WhatsApp entry, and `build-all.sh` usage
- Rebranded remaining IronClaw references in user-facing docs (`BUILDING_CHANNELS.md`, `LLM_PROVIDERS.md`, `DEPLOYMENT.md`, `CONTRIBUTING.md`)

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

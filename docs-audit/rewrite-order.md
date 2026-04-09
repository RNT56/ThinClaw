# Rewrite Order

## P0 Canonical

- `README.md`
- `docs/DEPLOYMENT.md`
- `docs/CHANNEL_ARCHITECTURE.md`
- `docs/EXTENSION_SYSTEM.md`
- `channels-docs/README.md`
- `tools-docs/README.md`
- `src/setup/README.md`
- `Agent_flow.md`

## P1 Important

- `CLAUDE.md`
- `CONTRIBUTING.md`
- channel-specific guides with known transport drift
- tool-specific guides with stale auth commands
- security/trust overview routing
- CLI and Web UI operator reference cleanup

## P2 Cleanup

- historical cross-links
- duplicate setup explanations
- overly detailed or brittle counts
- low-signal narrative repetition
- stale command examples in leaf docs
- archive and roadmap scratchpads that compete with current docs

## Archive

- `rewrite-docs/`
- scratch or roadmap docs that are not current references

## Wave Plan

### Wave 1

- Rewrite `README.md` as the front door and docs router
- Normalize setup/deployment canonicals
- Normalize channel and extension canonicals
- Fix top-level tool/channel indexes

### Wave 2

- Rewrite stale maintainer docs (`CLAUDE.md`, `Agent_flow.md`)
- Fix tool leaf docs with stale CLI/auth commands
- Fix channel leaf docs with stale transport framing
- Refresh contributor guidance

### Wave 3

- Add or tighten security/trust overview routing
- Sweep for broken links, stale defaults, and unsupported claims
- Cross-check `FEATURE_PARITY.md` references where behavior-facing wording changed

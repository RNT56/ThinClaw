# Contributing to ThinClaw

Contributor setup, source-build profiles, and maintainer workflow notes live in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Local Checks

Run the Rust checks before opening a PR:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If you changed packaged WASM channels or tools, also rebuild the artifacts that depend on them:

```bash
./scripts/build-all.sh
```

## Before Opening a PR

1. Run `fmt`, `clippy`, and `test` locally.
2. Update the relevant docs when behavior, defaults, commands, or trust boundaries changed.
3. Review `FEATURE_PARITY.md` if the change affects a tracked capability.
4. Update `CHANGELOG.md` for user-facing changes.

## Code Quality

- Keep clippy clean.
- Prefer explicit error handling over unchecked production `unwrap()`.
- Avoid adding stale inventories or brittle counts to docs.
- Treat code-adjacent docs as part of the change, not follow-up work.

## Feature Parity Requirement

When a change affects tracked ThinClaw behavior, update `FEATURE_PARITY.md` in the same branch.

This includes:

- status changes such as `❌`, `🚧`, or `✅`
- notes that describe changed behavior or scope
- references that point readers to the new implementation

## Docs and Canonicals

Before editing broad docs, check the canonical doc for the subsystem:

- onboarding: `src/setup/README.md`
- identity/personality: `docs/IDENTITY_AND_PERSONALITY.md`
- memory/growth: `docs/MEMORY_AND_GROWTH.md`
- research/experiments: `docs/RESEARCH_AND_EXPERIMENTS.md`
- shared command vocabulary: `docs/SURFACES_AND_COMMANDS.md`
- deployment: `docs/DEPLOYMENT.md`
- channels: `docs/CHANNEL_ARCHITECTURE.md`
- extensions: `docs/EXTENSION_SYSTEM.md`
- tools: `src/tools/README.md`
- security/networking: `src/NETWORK_SECURITY.md`

## Dependency and Security Hygiene

- Run `cargo audit` periodically when working on dependency-heavy areas.
- Treat MCP integrations as operator-trusted external surfaces, not sandboxed extensions.
- Keep secret-handling and trust-boundary docs aligned with the real implementation.

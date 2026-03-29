# Contributing to ThinClaw

## Quick Start

```bash
# Run the full CI pipeline locally:
npm run ci

# Or individual checks:
cargo clippy --all-targets -- -D warnings   # lint (warnings = errors)
cargo fmt --check                            # formatting check
cargo test                                   # run tests
```

## Before Opening a PR

1. **Run clippy + fmt + tests** — must pass cleanly (no clippy warnings, no fmt diffs, all tests green).
2. **Review the relevant parity rows in `FEATURE_PARITY.md`** if your change affects a tracked capability.
3. **Update `CHANGELOG.md`** for user-facing changes.

## Code Quality

- **Zero clippy warnings** — CI runs `cargo clippy --all-targets -- -D warnings`.
  Intentionally allowed lints are configured in `Cargo.toml` under `[lints.clippy]`.
- **Zero production `.unwrap()` calls** — use `.expect("descriptive reason")` or proper error handling.
- **Formatting** — `cargo fmt` with the project's `rustfmt.toml` rules.

### Fixing Lint Issues

```bash
cargo clippy --fix --allow-dirty    # Auto-fix clippy suggestions
cargo fmt                           # Auto-format code
```

## Building WASM Channels

After modifying any channel source code in `channels-src/`, rebuild everything:

```bash
./scripts/build-all.sh
```

This rebuilds all WASM channels and the main ThinClaw binary. See [docs/BUILDING_CHANNELS.md](docs/BUILDING_CHANNELS.md) for the full guide.

## Feature Parity Requirement

When your change affects a tracked capability, update `FEATURE_PARITY.md` in the same branch.

1. Review the relevant parity rows in `FEATURE_PARITY.md`.
2. Update status/notes if behavior changed.
3. Include the `FEATURE_PARITY.md` diff in your commit when applicable.

## CI Workflows

| Workflow | Trigger | Duration | Purpose |
|----------|---------|----------|---------|
| `ci.yml` | Push/PR to `main`, `develop` | ~3 min | Lint + test quality gate |
| `build-release.yml` | Tags / manual | ~20 min | Multi-platform build + release |

## Dependency Security

Run `cargo audit` periodically to check for known vulnerabilities.
Known advisories are tracked in the project audit report.

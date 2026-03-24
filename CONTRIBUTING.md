# Contributing to IronClaw

## Quick Start

```bash
# Run the full CI pipeline locally:
npm run ci

# Or individual checks:
npm run lint:ironclaw    # cargo clippy (warnings = errors)
npm run lint:fmt         # cargo fmt --check
npm run lint:ts          # TypeScript type-check
npm run test:ironclaw    # cargo test
```

## Before Opening a PR

1. **Run `npm run ci`** — must pass cleanly (no clippy warnings, no fmt diffs, all tests green).
2. **Review the relevant parity rows in `FEATURE_PARITY.md`** if your change affects a tracked capability.
3. **Update `CHANGELOG.md`** for user-facing changes.

## Code Quality

- **Zero clippy warnings** — CI runs `cargo clippy --all-targets -- -D warnings`.
  Intentionally allowed lints are configured in `Cargo.toml` under `[lints.clippy]`.
- **Zero production `.unwrap()` calls** — use `.expect("descriptive reason")` or proper error handling.
- **Formatting** — `cargo fmt` with the project's `rustfmt.toml` rules.

### Fixing Lint Issues

```bash
npm run lint:ironclaw:fix   # Auto-fix clippy suggestions
npm run lint:fmt:fix        # Auto-format code
```

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

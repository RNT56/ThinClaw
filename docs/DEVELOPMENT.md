# ThinClaw Development

This page is for contributors and maintainers working from a source checkout.
For normal installation and first-run usage, start with [../README.md](../README.md).

## Local Setup

```bash
./scripts/dev-setup.sh
```

The setup script checks the Rust toolchain and common build prerequisites. If
you install dependencies manually, use the repository `rust-toolchain.toml` and
keep `cargo-component` available when working on packaged WASM tools or
channels.

## Source Builds

Use release builds for binaries you intend to run or install:

```bash
cargo build --release --bin thinclaw
cargo build --release --features full --bin thinclaw
```

Plain `cargo build` intentionally uses Cargo's development profile. That is
useful for fast local iteration, but it is not the user-facing build path and it
keeps debug and incremental artifacts under `target/debug`.

For a low-disk source install, keep Cargo artifacts in a temporary target dir:

```bash
tmp="$(mktemp -d)"
cargo install --path . --locked --features full --bin thinclaw --target-dir "$tmp"
rm -rf "$tmp"
```

Feature profiles, disk expectations, custom combinations, and CI matrix details
are canonical in [BUILD_PROFILES.md](BUILD_PROFILES.md).

## Local Checks

Run the Rust checks before opening a PR:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If you changed packaged WASM channels or tools, also rebuild the artifacts that
depend on them:

```bash
./scripts/build-all.sh
```

## Documentation

When behavior changes, update the relevant canonical docs in the same branch. If
the change affects a tracked feature, update [../FEATURE_PARITY.md](../FEATURE_PARITY.md)
too.

Useful contributor references:

- [../CONTRIBUTING.md](../CONTRIBUTING.md)
- [BUILD_PROFILES.md](BUILD_PROFILES.md)
- [CRATE_OWNERSHIP.md](CRATE_OWNERSHIP.md)
- [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- [../src/tools/README.md](../src/tools/README.md)

## Repository Layout

| Path | Purpose |
|---|---|
| [../src/](../src/) | Core runtime, CLI, gateway, channels, tools, memory, policy, and platform integration |
| [../crates/](../crates/) | Workspace crates that own extracted subsystem traits, DTOs, and runtime helpers |
| [./](./) | Canonical user, operator, architecture, security, and deployment docs |
| [../deploy/](../deploy/) | Linux, Docker, Raspberry Pi, and service helper assets |
| [../channels-src/](../channels-src/) | Source crates for packaged channel integrations |
| [../tools-src/](../tools-src/) | Source crates for packaged tool integrations |
| [../channels-docs/](../channels-docs/) | Channel setup and operation docs |
| [../tools-docs/](../tools-docs/) | Tool setup and operation docs |
| [../patches/](../patches/) | Vendored or patched dependency material |

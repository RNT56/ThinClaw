# Build Profiles

ThinClaw supports multiple build profiles to optimize binary size and compile time
for different deployment scenarios.

## Quick Reference

| Profile | Command | Use Case |
|---------|---------|----------|
| **light** (default) | `cargo build` | CLI agent, local gateway, API-only, cron agents |
| **full** | `cargo build --features full` | Production runtime with tunnel, Docker sandbox, browser, Nostr |
| **desktop** | `cargo build --features desktop` | Tauri/Scrappy desktop embedding |
| **minimal** | `cargo build --no-default-features --features libsql` | Embedded, air-gapped, IoT |
| **custom** | `cargo build --features light,browser` | Mix and match |

## Profile Details

### `light` (default)

**Included:** PostgreSQL, libSQL, local HTTP gateway/web UI, HTML-to-Markdown, document extraction (PDF/DOCX).

**Excluded:** REPL/TUI boot screen, ACP server, tunnel providers, Docker sandbox, browser automation, Nostr, voice wake.

Best for: CLI-only usage, cron-driven agents, headless deployments, API-only servers.

```bash
# Default build — this is what you get with plain cargo commands
cargo build --release
cargo install thinclaw

# Run
thinclaw
```

### `full`

Everything in `light` **plus**: ACP integration, REPL/TUI mode (interactive terminal
with boot screen), tunnel providers (Tailscale/Cloudflare), Docker sandbox
(container isolation for untrusted code), browser automation (Chromium-based),
and Nostr protocol integration.

Best for: Full production deployments with web UI and all channel support.

```bash
cargo build --release --features full

# Or for development
cargo run --features full
```

### `desktop`

Minimal footprint for Tauri/Scrappy desktop app embedding. Includes libSQL (no
PostgreSQL), HTML-to-Markdown, document extraction, and REPL mode.

```bash
cargo build --release --features desktop
```

### Minimal

Absolute minimum viable agent. Only a single database backend, no media processing,
no web capabilities. Useful for embedded systems or edge deployments.

```bash
# libSQL only (embedded database, zero external deps)
cargo build --release --no-default-features --features libsql

# PostgreSQL only
cargo build --release --no-default-features --features postgres
```

## Custom Combinations

Individual features can be combined freely:

```bash
# Light + browser automation only (no tunnel, no Docker sandbox)
cargo build --features light,browser

# Light + tunnel for public webhook access
cargo build --features light,tunnel

# Light + voice wake word detection
cargo build --features light,voice

# Light + AWS Bedrock embeddings
cargo build --features light,bedrock

# Embed WASM extensions into binary (air-gapped deploy)
cargo build --features full,bundled-wasm

# All features including voice, Bedrock, and bundled WASM
cargo build --all-features
```

## Feature Flag Reference

| Flag | Description | Extra Dependencies |
|------|-------------|-------------------|
| `postgres` | PostgreSQL backend with TLS | deadpool-postgres, tokio-postgres, rustls |
| `libsql` | Embedded libSQL/Turso database | libsql |
| `html-to-markdown` | Web page → markdown conversion | html-to-markdown-rs, readabilityrs |
| `document-extraction` | PDF/DOCX/PPTX/XLSX text extraction | pdf-extract, zip |
| `timezones` | Timezone handling via chrono-tz | chrono-tz |
| `web-gateway` | Compatibility flag for the always-available local HTTP web UI + API server | (uses axum, already a base dep) |
| `acp` | ACP integration surface | (no extra system deps) |
| `repl` | Interactive REPL mode + boot screen | (no extra deps) |
| `tunnel` | VPN tunnel integration | (uses tailscale binary externally) |
| `docker-sandbox` | Docker container sandboxing | (uses bollard, already a base dep) |
| `browser` | Chromium-based browser automation | chromiumoxide |
| `nostr` | Nostr protocol integration (NIP-04, NIP-59) | nostr-sdk |
| `bundled-wasm` | Embed all WASM extensions in binary | (compile-time includes, +6-13 MB) |
| `voice` | Voice wake word detection | cpal (audio capture) |
| `bedrock` | AWS Bedrock Titan embeddings | aws-config, aws-sdk-bedrockruntime |
| `integration` | Gate for integration tests | (no extra deps) |

Linux notes:

- The default `light` profile includes the local gateway and does not need any extra system packages beyond the normal Rust build toolchain.
- `full` needs Docker for Docker sandbox jobs and Docker Chromium fallback, plus a local Chrome/Chromium/Brave/Edge browser if you set `BROWSER_DOCKER=never`.
- `--features light,voice` or `--all-features` requires `libasound2-dev`.
- `--features bedrock` or `--all-features` requires AWS credentials (`AWS_PROFILE` or AWS access keys).
- `--features bundled-wasm` or `--all-features` requires `rustup target add wasm32-wasip2` and `cargo install wasm-tools --locked`.

## Profile Composition

```
light    = postgres + libsql + gateway + html-to-markdown + document-extraction + timezones
desktop  = libsql + html-to-markdown + document-extraction + repl + timezones
full     = light + acp + repl/tui + tunnel + docker-sandbox + browser + nostr
```

## `full` vs `--all-features`

`full` is the production-ready profile with all runtime modules. `--all-features`
enables everything in `full` **plus** niche/platform-specific capabilities that most
users don't need:

| Extra flag (not in `full`) | Why it's opt-in |
|---|---|
| `voice` | Adds `cpal` for audio capture; only for headless/remote mode. Requires ALSA headers on Linux. |
| `bedrock` | Adds AWS SDK deps; only useful with an AWS account for Bedrock Titan embeddings. |
| `bundled-wasm` | Embeds all WASM extensions into the binary (+6-13 MB); only for air-gapped deploys. |
| `integration` | Gate for integration tests; not a runtime capability. |

Use `full` for production. Use `--all-features` for CI test coverage or when you
specifically need one of the extras above. On Linux, run
`thinclaw doctor --profile all-features` before using `--all-features` locally.

## CI/CD

CI runs a **feature-matrix** job that verifies every documented profile compiles,
passes clippy, and compiles tests:

| CI Check | Profiles Tested |
|----------|----------------|
| `cargo check` | light (default), full, all-features, desktop, minimal-libsql, minimal-postgres |
| `cargo clippy` | All of the above |
| `cargo test --no-run` | All of the above (compile-only) |
| Host/deploy smoke | Linux host runtime, Docker image build, Docker Compose `/api/health` |

- **Feature matrix:** Catches broken `#[cfg(feature = "...")]` gates before they ship.
  The `light` profile is especially important since it's what `cargo install thinclaw` produces.
- **Full test suite:** Runs with `--all-features` and a live PostgreSQL service for
  integration coverage.
- **Release builds:** Produce binaries with the `full` profile for maximum compatibility.

## Migration from `full` Default

Prior to v0.14, the default build profile was `full` (all features enabled).
Starting with v0.14, the default is `light`. If you were previously running
`cargo build` without feature flags:

```bash
# Old behavior (< v0.14)
cargo build  # was equivalent to --features full

# New behavior (>= v0.14) — same result, explicit flag
cargo build --features full
```

Deployment scripts use explicit feature flags appropriate to their context:
- `mac-deploy.sh` — uses `--features libsql` (zero-dependency Mac Mini deploy)
- `dev-setup.sh` — uses default (`light`) for development
- Release CI (`cargo dist`) — uses `full` profile for maximum compatibility

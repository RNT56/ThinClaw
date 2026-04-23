# Build Profiles

ThinClaw supports multiple build profiles to optimize binary size and compile time
for different deployment scenarios.

## Quick Reference

| Profile | Command | Use Case |
|---------|---------|----------|
| **light** (default) | `cargo build` | CLI agent, API-only, cron agents |
| **full** | `cargo build --features full` | Production with web UI, tunnel, Docker |
| **desktop** | `cargo build --features desktop` | Tauri/Scrappy desktop embedding |
| **minimal** | `cargo build --no-default-features --features libsql` | Embedded, air-gapped, IoT |
| **custom** | `cargo build --features light,web-gateway` | Mix and match |

## Profile Details

### `light` (default)

**Included:** PostgreSQL, libSQL, HTML-to-Markdown, document extraction (PDF/DOCX).

**Excluded:** Web gateway, REPL boot screen, tunnel providers, Docker sandbox, voice wake.

Best for: CLI-only usage, cron-driven agents, headless deployments, API-only servers.

```bash
# Default build — this is what you get with plain cargo commands
cargo build --release
cargo install thinclaw

# Run
thinclaw
```

### `full`

Everything in `light` **plus**: web gateway (browser UI with SSE/WebSocket), REPL mode
(interactive terminal with boot screen), tunnel providers (Tailscale/Cloudflare),
Docker sandbox (container isolation for untrusted code), browser automation
(Chromium-based), and Nostr protocol integration.

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
# Light + web gateway only (no tunnel, no Docker)
cargo build --features light,web-gateway

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
| `web-gateway` | HTTP web UI + API server | (uses axum, already a base dep) |
| `repl` | Interactive REPL mode + boot screen | (no extra deps) |
| `tunnel` | VPN tunnel integration | (uses tailscale binary externally) |
| `docker-sandbox` | Docker container sandboxing | (uses bollard, already a base dep) |
| `browser` | Chromium-based browser automation | chromiumoxide |
| `nostr` | Nostr protocol integration (NIP-04, NIP-59) | nostr-sdk |
| `bundled-wasm` | Embed all WASM extensions in binary | (compile-time includes, +6-13 MB) |
| `voice` | Voice wake word detection | cpal (audio capture) |
| `bedrock` | AWS Bedrock Titan embeddings | aws-config, aws-sdk-bedrockruntime |
| `integration` | Gate for integration tests | (no extra deps) |

Linux note: the default `light` profile does not include `voice`, so a normal
Linux build does not need ALSA development headers. If you opt into
`--features light,voice` or `--all-features`, install `libasound2-dev`.

## Profile Composition

```
light    = postgres + libsql + html-to-markdown + document-extraction + timezones
desktop  = libsql + html-to-markdown + document-extraction + repl + timezones
full     = light + web-gateway + repl + tunnel + docker-sandbox + browser + nostr
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
specifically need one of the extras above.

## CI/CD

CI runs a **feature-matrix** job that verifies every documented profile compiles,
passes clippy, and compiles tests:

| CI Check | Profiles Tested |
|----------|----------------|
| `cargo check` | light (default), full, desktop, minimal-libsql, minimal-postgres |
| `cargo clippy` | All of the above |
| `cargo test --no-run` | All of the above (compile-only) |
| `cargo test` (execution) | `--all-features` (with PostgreSQL service) |

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

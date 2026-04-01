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
Docker sandbox (container isolation for untrusted code).

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

# Embed WASM extensions into binary (air-gapped deploy)
cargo build --features full,bundled-wasm

# All features including voice
cargo build --all-features
```

## Feature Flag Reference

| Flag | Description | Extra Dependencies |
|------|-------------|-------------------|
| `postgres` | PostgreSQL backend with TLS | deadpool-postgres, tokio-postgres, rustls |
| `libsql` | Embedded libSQL/Turso database | libsql |
| `html-to-markdown` | Web page → markdown conversion | html-to-markdown-rs, readabilityrs |
| `document-extraction` | PDF/DOCX/PPTX/XLSX text extraction | pdf-extract, zip |
| `web-gateway` | HTTP web UI + API server | (uses axum, already a base dep) |
| `repl` | Interactive REPL mode + boot screen | (no extra deps) |
| `tunnel` | VPN tunnel integration | (uses tailscale binary externally) |
| `docker-sandbox` | Docker container sandboxing | (uses bollard, already a base dep) |
| `bundled-wasm` | Embed all WASM extensions in binary | (compile-time includes, +6-13 MB) |
| `voice` | Voice wake word detection | cpal (audio capture) |

## Profile Composition

```
light = postgres + libsql + html-to-markdown + document-extraction
desktop = libsql + html-to-markdown + document-extraction + repl
full = light + web-gateway + repl + tunnel + docker-sandbox
```

## CI/CD

- **Tests:** Always run with `--all-features` to ensure nothing is broken.
- **Release builds:** Produce binaries with the `full` profile for maximum compatibility.
- **Musl targets:** Fully static Linux binaries (no libc dependency) for
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`.

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

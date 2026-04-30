# Build Profiles

ThinClaw supports multiple build profiles to optimize binary size and compile time
for different deployment scenarios. Use `--release` for source builds you intend
to run or install; plain `cargo build` is a development/debug build and can leave
large debug and incremental compiler artifacts under `target/debug`.

For the normal user-facing CLI, build only `--bin thinclaw`. That avoids
compiling auxiliary binaries that are useful for development or protocol-specific
testing but are not needed for a standard install. Build auxiliary binaries such
as `thinclaw-acp` explicitly when you need them.

## Quick Reference

| Profile | Command | Use Case |
|---------|---------|----------|
| **light** (default) | `cargo build --release --bin thinclaw` | CLI agent, local gateway, API-only, cron agents |
| **full** | `cargo build --release --features full --bin thinclaw` | Production runtime with tunnel, Docker sandbox, browser, Nostr |
| **desktop** | `cargo build --release --features desktop --bin thinclaw` | Tauri/Scrappy desktop embedding |
| **minimal** | `cargo build --release --no-default-features --features libsql --bin thinclaw` | Embedded, air-gapped, IoT |
| **custom** | `cargo build --release --features light,browser --bin thinclaw` | Mix and match |

## Profile Details

### `light` (default)

**Included:** PostgreSQL, libSQL, local HTTP gateway/web UI, HTML-to-Markdown, document extraction (PDF/DOCX).

**Excluded:** REPL/TUI boot screen, ACP server, tunnel providers, Docker sandbox, browser automation, Nostr, voice wake.

Best for: CLI-only usage, cron-driven agents, headless deployments, API-only servers.

```bash
# Default release build
cargo build --release --bin thinclaw
cargo install --path . --locked --bin thinclaw

# Run
thinclaw
```

### `full`

Everything in `light` **plus**: ACP integration, REPL/TUI mode (interactive terminal
with boot screen), tunnel providers (Tailscale/Cloudflare), Docker sandbox
(container isolation for untrusted code), browser automation (Chromium-based),
and Nostr protocol integration.

Best for: Full production deployments with web UI and all non-desktop channel
support. This is the expected native build profile for Raspberry Pi OS Lite
64-bit.

```bash
cargo build --release --features full --bin thinclaw

# Or for development
cargo run --features full
```

### `desktop`

Minimal footprint for Tauri/Scrappy desktop app embedding. Includes libSQL (no
PostgreSQL), HTML-to-Markdown, document extraction, and REPL mode.

```bash
cargo build --release --features desktop --bin thinclaw
```

### Minimal

Absolute minimum viable agent. Only a single database backend, no media processing,
no web capabilities. Useful for embedded systems or edge deployments.

```bash
# libSQL only (embedded database, zero external deps)
cargo build --release --no-default-features --features libsql --bin thinclaw

# PostgreSQL only
cargo build --release --no-default-features --features postgres --bin thinclaw
```

## Custom Combinations

Individual features can be combined freely:

```bash
# Light + browser automation only (no tunnel, no Docker sandbox)
cargo build --release --features light,browser --bin thinclaw

# Light + tunnel for public webhook access
cargo build --release --features light,tunnel --bin thinclaw

# Light + voice wake word detection
cargo build --release --features light,voice --bin thinclaw

# Light + AWS Bedrock embeddings
cargo build --release --features light,bedrock --bin thinclaw

# Embed WASM extensions into binary (air-gapped deploy)
cargo build --release --features full,bundled-wasm --bin thinclaw

# All features including voice, Bedrock, and bundled WASM
cargo build --release --all-features --bin thinclaw
```

## Disk Usage

A clean `cargo build --release --features full --bin thinclaw` build should have
roughly 15-20 GiB free. Cargo needs space for optimized dependency artifacts,
native build outputs, linker scratch files, and the final binary. This is far
smaller than a full debug/test tree, but it is still a large Rust build because
`full` includes Wasmtime/Cranelift, browser automation, libSQL, networking, and
database backends.

The installed artifact is only `target/release/thinclaw`. After copying or
installing that binary, reclaim the release build tree with:

```bash
cargo clean --release
```

If a previous debug build has filled the checkout, remove only the debug artifacts
with:

```bash
rm -rf target/debug target/flycheck0
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
- `full` needs Docker for Docker sandbox jobs and Docker Chromium fallback. The default `CHROMIUM_IMAGE=chromedp/headless-shell:latest` is multi-arch; use a local Chrome/Chromium/Brave/Edge browser if you set `BROWSER_DOCKER=never`.
- Raspberry Pi OS Lite 64-bit should use the `aarch64-unknown-linux-gnu` release artifact or `cargo build --release --features full --bin thinclaw` for native installs.
- `--features light,voice` or `--all-features` requires `libasound2-dev`.
- `--features bedrock` or `--all-features` requires AWS credentials (`AWS_PROFILE` or AWS access keys).
- `--features bundled-wasm` or `--all-features` requires `rustup target add wasm32-wasip2` and `cargo install wasm-tools --locked`.

## Raspberry Pi OS Lite 64-Bit Builds

Pi OS Lite is a headless target, not a desktop-autonomy target.

Use the release artifact for production:

```bash
export THINCLAW_VERSION=v0.13.7  # replace with the latest release tag
curl -L -o thinclaw-pi.tar.gz \
  "https://github.com/RNT56/ThinClaw/releases/download/${THINCLAW_VERSION}/thinclaw-aarch64-unknown-linux-gnu.tar.gz"
tar -xzf thinclaw-pi.tar.gz
```

Build from source on a Pi only when you need local patches:

```bash
cargo build --release --features full --bin thinclaw
```

Docker users should prefer the multi-arch image:

```bash
docker pull ghcr.io/rnt56/thinclaw:latest
```

Use the `light` profile only for intentionally smaller gateway/libSQL installs.
It omits ACP, tunnels, Docker sandbox, browser automation, and Nostr.

Before deploying or enabling new runtime capabilities:

```bash
thinclaw onboard --profile pi-os-lite-64
thinclaw doctor --profile pi-os-lite-64
thinclaw status --profile pi-os-lite-64
```

That onboarding profile writes `THINCLAW_RUNTIME_PROFILE=pi-os-lite-64` and
`THINCLAW_HEADLESS=true`, which keep Pi OS Lite on the remote/headless path.

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
  The `light` profile is especially important since it's what `cargo install --path . --bin thinclaw` produces.
- **Full test suite:** Runs with `--all-features` and a live PostgreSQL service for
  integration coverage.
- **Release builds:** Produce binaries with the `full` profile for maximum compatibility.
- **Raspberry Pi OS Lite 64-bit:** Uses the `aarch64-unknown-linux-gnu` release artifact for native installs and the multi-arch `ghcr.io/rnt56/thinclaw:<version>` / `latest` image for Docker installs.

## Migration from `full` Default

Prior to v0.14, the default build profile was `full` (all features enabled).
Starting with v0.14, the default is `light`. If you were previously building
release artifacts without feature flags:

```bash
# Old behavior (< v0.14)
cargo build --release --bin thinclaw  # was equivalent to --features full

# New behavior (>= v0.14) — same result, explicit flag
cargo build --release --features full --bin thinclaw
```

Deployment scripts use explicit feature flags appropriate to their context:
- `mac-deploy.sh` — uses `--features libsql` (zero-dependency Mac Mini deploy)
- `dev-setup.sh` — uses default (`light`) for development
- Release CI (`cargo dist`) — uses `full` profile for maximum compatibility

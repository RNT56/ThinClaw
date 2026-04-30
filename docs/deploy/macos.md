# ThinClaw On macOS

Use this path for a Mac laptop, workstation, or Mac Mini. macOS is the most
polished host for local use, service mode, and desktop autonomy.

## Choose A Path

| Goal | Recommended Path |
|---|---|
| Fast local install | Release installer, then `thinclaw onboard` |
| Fresh Mac Mini setup from source | `scripts/mac-deploy.sh` |
| Always-on background runtime | `thinclaw onboard --profile remote`, then launchd service |
| Host-level desktop automation | Enable the reckless desktop autonomy profile after local setup |
| Remote Scrappy access | Bind the gateway to a private address or Tailscale |

## Prerequisites

Required for release install:

- macOS on Apple Silicon or Intel
- Terminal or another shell
- `curl` with TLS access to GitHub Releases
- a writable install location in `PATH`

Required for source builds:

- Xcode Command Line Tools: `xcode-select --install`
- Rust 1.92+ through `rustup`
- Git
- `wasm32-wasip2` when building bundled WASM extensions

Optional feature prerequisites:

- Chrome, Brave, or Edge for local browser automation
- Docker Desktop for Docker sandbox jobs or Docker Chromium fallback
- Tailscale, Cloudflare Tunnel, ngrok, or another tunnel when remote webhooks need a public HTTPS URL
- `ffmpeg` for richer audio/video media handling
- Calendar, Numbers, Pages, TextEdit, and one-time macOS privacy/accessibility permissions for desktop autonomy

Verify after install:

```bash
thinclaw status
thinclaw doctor
```

## Fast Local Install

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

thinclaw onboard
thinclaw
```

Open the local gateway:

```text
http://127.0.0.1:3000
```

For a full-screen terminal runtime:

```bash
thinclaw tui
```

Common post-install checks:

```bash
thinclaw status
thinclaw doctor
thinclaw logs tail
```

Verbose startup diagnostics:

```bash
thinclaw --debug
thinclaw --debug tui
```

## Fresh Mac Or Mac Mini Source Setup

The one-click macOS script installs prerequisites, clones or locates the repo,
builds the binary, and offers to launch onboarding.

```bash
curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash
```

From a checkout:

```bash
./scripts/mac-deploy.sh
```

Useful options:

```bash
./scripts/mac-deploy.sh --bundled
./scripts/mac-deploy.sh --install-only
./scripts/mac-deploy.sh --skip-build
./scripts/mac-deploy.sh --no-launch
```

The script builds with `libsql` by default. Use `--bundled` when you want WASM
extensions embedded for an air-gapped or mostly offline host.

## Build From Source Manually

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustup target add wasm32-wasip2

git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw
cargo build --release --features full --bin thinclaw
./target/release/thinclaw onboard
```

Use `cargo build --release --bin thinclaw` only when you intentionally want the
smaller default `light` profile. See [../BUILD_PROFILES.md](../BUILD_PROFILES.md).

## Run As A launchd Service

For a Mac Mini reached over SSH, prefer the remote profile first:

```bash
thinclaw onboard --profile remote
thinclaw gateway access
```

After onboarding:

```bash
thinclaw service install
thinclaw service start
thinclaw service status
```

Service management:

```bash
thinclaw service stop
thinclaw service uninstall
```

The service uses launchd and runs:

```bash
thinclaw run --no-onboard
```

If the service fails to start, inspect launchd status first, then run the same
command in the foreground:

```bash
thinclaw --debug run --no-onboard
```

Useful service log commands:

```bash
thinclaw logs tail
thinclaw logs tail -l error
```

## Remote Gateway Access

Remote access is opt-in. For private LAN or Tailscale access, configure:

```env
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
CLI_ENABLED=false
```

Prefer Tailscale or a trusted private network over public exposure. For webhook
tunnels and provider-specific tunnel setup, use [remote-access.md](remote-access.md).

For the safer SSH-tunnel default, keep `GATEWAY_HOST=127.0.0.1` and run:

```bash
ssh -L 3000:127.0.0.1:3000 user@host
```

## Desktop Autonomy

macOS is the primary desktop-autonomy path today. It expects a real logged-in
GUI session and the relevant app permissions.

Use [../DESKTOP_AUTONOMY.md](../DESKTOP_AUTONOMY.md) as the canonical guide.

## Troubleshooting

If the gateway is not available:

```bash
thinclaw logs tail
thinclaw logs tail -l error
```

If Tailscale crashes with a `BundleIdentifiers.swift` fatal error, you launched
the GUI app binary instead of the CLI. Install the CLI from the Tailscale menu
bar app settings, or install Tailscale through Homebrew:

```bash
brew install tailscale
```

For the full command surface after deployment, use
[../CLI_REFERENCE.md](../CLI_REFERENCE.md).

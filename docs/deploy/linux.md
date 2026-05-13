# ThinClaw On Linux

Use this path for generic Linux laptops, workstations, home servers, VPS hosts,
and development boxes. For Raspberry Pi OS Lite, use
[raspberry-pi-os-lite.md](raspberry-pi-os-lite.md).

## Choose A Path

| Goal | Recommended Path |
|---|---|
| Fast local install | Release installer, then `thinclaw onboard` |
| Generic always-on server | Release installer, `thinclaw onboard --profile remote`, then service install |
| Small VPS / SD-card host | Release installer with `--profile edge` |
| Container deployment | [docker.md](docker.md) |
| Raspberry Pi OS Lite 64-bit | [raspberry-pi-os-lite.md](raspberry-pi-os-lite.md) |
| Remote Scrappy access | [remote-access.md](remote-access.md) |

## Prerequisites

Required for release install:

- Linux on a supported release target
- shell access as the operator user
- `curl` with TLS access to GitHub Releases
- a writable install location in `PATH`

Required for source builds:

- C/C++ build tools
- `pkg-config`
- `curl`, Git, and CA certificates
- Rust 1.92+ through `rustup`
- `wasm32-wasip2` if you are building WASM extensions or bundled channel/tool artifacts

Debian/Ubuntu baseline:

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config ca-certificates
```

Fedora baseline:

```bash
sudo dnf install -y gcc gcc-c++ make curl git pkg-config ca-certificates
```

Required for `systemd --user` service mode:

- a normal user login session with `pam_systemd`
- `systemctl --user` reachable for the operator account
- onboarding completed under the same account
- persisted gateway/provider settings before service start

Optional feature prerequisites:

- Docker Engine and Compose V2 for Docker sandbox jobs, Docker Chromium fallback, or Compose deployment
- Chrome, Chromium, Brave, or Edge for local browser automation
- `ffmpeg` for richer audio/video media handling
- device permissions for camera and microphone tools
- Tailscale, Cloudflare Tunnel, ngrok, or another tunnel for remote webhook URLs
- a logged-in supported Linux desktop session plus the packages listed in [../DESKTOP_AUTONOMY.md](../DESKTOP_AUTONOMY.md) for Linux desktop autonomy
- `libasound2-dev` when compiling with the optional `voice` feature

Verify after install:

```bash
thinclaw doctor --profile server
thinclaw status --profile server
```

## Fast Local Install

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

thinclaw onboard
thinclaw
```

Small-machine install:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh -s -- --profile edge
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
thinclaw status --profile server
thinclaw doctor --profile server
thinclaw logs tail
```

Verbose startup diagnostics:

```bash
thinclaw --debug
thinclaw --debug run --no-onboard
```

## Build From Source

Build from source only when you need local code changes. Debian or Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config ca-certificates
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustup target add wasm32-wasip2

git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw
cargo build --release --features full --bin thinclaw
./target/release/thinclaw onboard
```

Fedora:

```bash
sudo dnf install -y gcc gcc-c++ make curl git pkg-config ca-certificates
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

## Run As A systemd User Service

For an SSH-managed server or VPS, prefer the remote profile first:

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

The service uses `systemd --user` and runs:

```bash
thinclaw run --no-onboard
```

If `systemd --user` is not reachable, start from a normal user login session
with `pam_systemd`, then retry:

```bash
systemctl --user status
```

For a dedicated system-level service with an unprivileged `thinclaw` user, use
the Pi OS Lite native installer pattern in
[raspberry-pi-os-lite.md](raspberry-pi-os-lite.md) as the closest maintained
template.

## Docker Deployment

Use [docker.md](docker.md) for Compose, GHCR images, source-build images, and
PostgreSQL profile details.

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

Linux desktop autonomy is best-effort and prerequisite-heavy. It expects a
compatible desktop session plus LibreOffice, Evolution, and accessibility
tooling. Headless Linux hosts should leave desktop autonomy disabled.

Use [../DESKTOP_AUTONOMY.md](../DESKTOP_AUTONOMY.md) as the canonical guide.

## Troubleshooting

If the source build runs but integrations are missing, rebuild with:

```bash
cargo build --release --features full --bin thinclaw
```

If the gateway is reachable locally but not from another machine, check:

- `GATEWAY_HOST` is not loopback-only
- your firewall allows the selected port
- you are using the correct host address
- your network path is private or intentionally secured

For the full command surface after deployment, use
[../CLI_REFERENCE.md](../CLI_REFERENCE.md).

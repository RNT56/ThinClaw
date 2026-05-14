# ThinClaw On Raspberry Pi OS Lite 64-Bit

Use this path for Pi 4/5 or ARM64 Raspberry Pi servers running Raspberry Pi OS
Lite 64-bit Bookworm.

Pi OS Lite is a first-class headless Linux target. It supports the gateway,
libSQL, channels, routines, tunnels, and Docker sandbox/browser fallback when
Docker is installed. It does not support reckless desktop autonomy because Lite
has no interactive Linux desktop session.

## What Works

Supported:

- Web gateway and remote Scrappy connection on port `3000`
- libSQL local database
- routines and scheduled background work
- native and WASM channels that do not require a desktop session
- tunnels, including Tailscale, when installed
- Docker sandbox jobs and Docker Chromium browser fallback when Docker is installed
- ACP and Nostr when using the `full` release artifact or a `--features full` source build

Not supported:

- reckless desktop autonomy
- Linux desktop action stack
- native Apple Mail and native iMessage channels

Use Gmail for mail and BlueBubbles for iMessage-compatible messaging through a
Mac-hosted BlueBubbles server.

## Prerequisites

Required host shape:

- Raspberry Pi 4/5 or another ARM64 Pi-class host
- Raspberry Pi OS Lite 64-bit Bookworm
- SSH or local shell access
- `sudo`
- `systemd`
- `curl`, `tar`, and CA certificates
- `openssl` for the token-generation examples, or another secure random-token generator
- port `3000` reachable only through the access method you choose

Recommended:

- 4 GB RAM or more for `full` plus Docker
- additional swap before large on-device source builds
- release artifact or GHCR image for production installs instead of on-device builds

Optional feature prerequisites:

- Docker Engine and Compose V2 for container deployment, Docker sandbox jobs, or Docker Chromium fallback
- Tailscale for private access without an SSH tunnel
- public HTTPS tunnel only when webhook providers require callbacks
- explicitly configured local browser or Docker Chromium fallback for browser automation

Not supported on Pi OS Lite:

- reckless desktop autonomy
- Linux desktop action stack
- native Apple Mail and native iMessage channels

## Readiness

Check readiness with:

```bash
thinclaw doctor --profile pi-os-lite-64
thinclaw status --profile pi-os-lite-64
```

Run readiness again after changing `.env`, installing Docker, or enabling a new
channel.

For an interactive first-run setup on the Pi instead of the root installer path:

```bash
thinclaw onboard --profile pi-os-lite-64
thinclaw gateway access
thinclaw doctor --profile pi-os-lite-64
```

## Native Release Install

Native install is the preferred low-overhead path. For Pi, VPS, and SD-card
hosts, prefer the edge artifact. The root setup script downloads the matching
prebuilt release binary, verifies its checksum, installs it under
`/usr/local/bin`, and keeps state under `/var/lib/thinclaw/.thinclaw`:

```bash
curl -L -o deploy-setup.sh \
  https://raw.githubusercontent.com/RNT56/ThinClaw/main/deploy/setup.sh

sudo bash deploy-setup.sh --mode native --profile edge \
  --token "$(openssl rand -hex 32)"
```

Use the full `aarch64-unknown-linux-gnu` release artifact when you need ACP,
tunnels, Docker sandbox, browser automation, Nostr, PostgreSQL, or the local
WASM runtime.

```bash
sudo bash deploy-setup.sh --mode native --profile full \
  --token "$(openssl rand -hex 32)"
```

Use `--binary /path/to/thinclaw` only when installing a locally patched binary.

If you are running from a repo checkout on the Pi, `--mode auto` selects native
mode automatically on Raspberry Pi OS Lite 64-bit:

```bash
sudo bash deploy/setup.sh --mode auto --token "$(openssl rand -hex 32)"
```

The native installer:

- installs `/usr/local/bin/thinclaw`
- creates the unprivileged `thinclaw` system user
- writes `/var/lib/thinclaw/.thinclaw/.env`
- generates a headless `SECRETS_MASTER_KEY`
- enables `THINCLAW_ALLOW_ENV_MASTER_KEY=1`
- defaults to `DATABASE_BACKEND=libsql`
- stores the database at `/var/lib/thinclaw/.thinclaw/thinclaw.db`
- installs `/etc/systemd/system/thinclaw.service`

The system service runs:

```text
ExecStart=/usr/local/bin/thinclaw run --no-onboard
```

After installation:

```bash
sudo systemctl status thinclaw
sudo journalctl -u thinclaw -f
curl http://localhost:3000/api/health
```

Edit the Pi runtime environment here:

```bash
sudoedit /var/lib/thinclaw/.thinclaw/.env
sudo systemctl restart thinclaw
```

At minimum, replace `OPENROUTER_API_KEY=CHANGE_ME` or configure another LLM
provider before expecting agent replies.

## WebUI Access Over SSH

Tailscale is optional for Pi WebUI access. If you can SSH to the Pi, use an SSH
local port forward from your laptop. For an SSH-only posture, set the Pi gateway
bind address to loopback:

```env
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3000
```

Then restart the service after editing `/var/lib/thinclaw/.thinclaw/.env`:

```bash
sudo systemctl restart thinclaw
```

Forward the gateway port from your laptop:

```bash
ssh -L 3000:127.0.0.1:3000 pi@<pi-host-or-ip>
```

Then open this on your laptop:

```text
http://127.0.0.1:3000/?token=<gateway-token>
```

See [SSH Tunnel Recommended](remote-access.md#ssh-tunnel-recommended) for the
shared remote access guidance and the distinction between WebUI access and public
webhook tunnels.

## Docker Install

Docker is supported when you want container deployment or Docker-backed
sandbox/browser features. On Pi, prefer the published multi-arch image instead
of building on-device:

```bash
sudo bash deploy/setup.sh --mode docker --token replace-with-a-long-random-token \
  --image ghcr.io/rnt56/thinclaw:latest
```

Manual Compose path from a repo checkout:

```bash
cd deploy
cp env.example .env
sed -i "s/^GATEWAY_AUTH_TOKEN=.*/GATEWAY_AUTH_TOKEN=$(openssl rand -hex 32)/" .env
sed -i "s|^THINCLAW_IMAGE=.*|THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest|" .env

docker compose pull thinclaw
docker compose up -d
curl http://localhost:3000/api/health
```

Source-build Compose is available, but it is slower on Pi:

```bash
BUILD_FEATURES=full docker compose up -d --build
```

For the shared Compose reference, use [docker.md](docker.md).

## Source Build On Pi

Source builds are useful for development or local patches. For production Pi
installs, prefer the release artifact or GHCR image.

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config ca-certificates
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustup target add wasm32-wasip2

git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw
cargo build --release --no-default-features --features edge --bin thinclaw

sudo bash deploy/setup.sh --mode native \
  --binary target/release/thinclaw \
  --token "$(openssl rand -hex 32)"
```

Use `cargo build --release --features full --bin thinclaw` only if you need the
full native runtime surface on the Pi.

## Private Access With Tailscale

For private access from Scrappy or devices that should connect without opening an
SSH tunnel, use Tailscale:

```bash
sudo bash deploy/setup.sh --mode auto --token replace-with-a-long-random-token \
  --tailscale tskey-auth-...
```

You can also install Tailscale yourself and bind the gateway to the tailnet IP:

```bash
tailscale ip -4
sudoedit /var/lib/thinclaw/.thinclaw/.env
sudo systemctl restart thinclaw
```

Set:

```env
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
```

Then connect from a tailnet device with:

```text
http://<pi-tailscale-ip>:3000/?token=<gateway-token>
```

For SSH-only WebUI access, use
[SSH Tunnel Recommended](remote-access.md#ssh-tunnel-recommended). For public
webhook tunnels, use [remote-access.md](remote-access.md).

## Pi Notes

- Use Raspberry Pi OS Lite 64-bit Bookworm.
- Keep the default `GATEWAY_PORT=3000` unless you intentionally change it.
- 4 GB RAM or more is recommended for `full` plus Docker.
- Increase swap before large on-device source builds.
- Browser automation on Lite needs Docker Chromium fallback (`CHROMIUM_IMAGE=chromedp/headless-shell:latest` by default) or an explicitly configured local browser.
- Use `thinclaw onboard --profile pi-os-lite-64` for interactive setup on-device.
- Native installs write `THINCLAW_RUNTIME_PROFILE=pi-os-lite-64` and `THINCLAW_HEADLESS=true`; with those markers, reckless desktop autonomy tools are blocked at runtime even if misconfigured.
- Keep `DESKTOP_AUTONOMY_ENABLED=false` on Pi OS Lite.

For the full command surface after deployment, use
[../CLI_REFERENCE.md](../CLI_REFERENCE.md).

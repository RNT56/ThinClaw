# ThinClaw Deployment Guide

This guide is the canonical operator reference for running ThinClaw outside the development loop.

It covers:

- local standalone use
- long-running service mode
- remote gateway access
- reckless desktop autonomy
- Scrappy-connected deployments
- source-build feature choices

For onboarding details, use [../src/setup/README.md](../src/setup/README.md). For identity and surface behavior, use [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md) and [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md).

## Deployment Modes

| Mode | Best For | Main Shape |
|---|---|---|
| Local standalone | laptop, workstation, single-user local run | `thinclaw` or `thinclaw tui` |
| Long-running service | Mac Mini, home server, Linux box, Windows workstation/server, VPS | launchd, systemd user service, or Windows Service Control Manager |
| Remote gateway | controlled LAN or Tailscale access | bind gateway to non-loopback host |
| Reckless desktop autonomy | operator-approved host-level desktop agenting | `desktop_autonomy.profile = "reckless_desktop"` plus bootstrap |
| Scrappy backend | desktop app + remote or local ThinClaw runtime | Scrappy talks to ThinClaw over the gateway |

## Defaults And Important Truths

- Code-backed default gateway port: `3000`
- Local default gateway URL: `http://127.0.0.1:3000`
- Remote access is opt-in through host/bind settings, not the default
- Source builds default to the `light` feature set, which includes the local gateway but does **not** include ACP, tunnel support, Docker sandbox, browser automation, or Nostr
- If you need the full production/runtime surface from source, build with `--features full`
- Desktop autonomy is a separate privileged operator mode; enabling it grants host-level app/UI/screen control and managed build promotion/rollback

## Fast Local Path

If you installed a release build:

```bash
thinclaw onboard
# onboarding now continues directly into runtime by default
# later, start the standard local runtime with:
thinclaw
# or launch the full-screen runtime directly:
thinclaw tui
```

Startup logging defaults to a quiet operator experience: `thinclaw` and `thinclaw tui` show warnings and errors, not the full initialization trace. For a verbose startup session, use either:

```bash
thinclaw --debug
thinclaw --debug tui
```

If the runtime is already up and you want to inspect logs without restarting, use the logs CLI:

```bash
thinclaw logs tail
thinclaw logs tail -l error
```

Then open:

```text
http://127.0.0.1:3000
```

Use this path when ThinClaw and your browser are on the same machine.

Windows quick path:

```powershell
thinclaw onboard
# onboarding now continues directly into runtime by default
# later, start the standard local runtime with:
thinclaw
# or launch the full-screen runtime directly:
thinclaw tui
```

If you are using a release install on Windows, prefer the MSI for PATH integration and service-friendly installs. The portable ZIP is supported for manual or side-by-side installs.

## Build From Source

The default source build is intentionally lightweight:

```bash
cargo build --release
```

That default maps to the `light` feature set. The local gateway is available by
default; the profile excludes:

- ACP integration
- tunnel integrations
- Docker sandbox
- browser automation
- Nostr

If you want the full operator-facing runtime from source:

```bash
cargo build --release --features full
```

If you want a more selective source build, combine features explicitly:

```bash
cargo build --release --features "light browser repl"
```

Relevant reference:

- [BUILD_PROFILES.md](BUILD_PROFILES.md)
- [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)
- [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md)

## Configuration Layers

ThinClaw starts from bootstrap config first, then runtime settings:

1. process environment
2. `./.env`
3. `~/.thinclaw/.env`
4. optional TOML overlay
5. injected secrets
6. database-backed settings

Do not treat this guide as the source of truth for onboarding step order. The canonical setup spec is [../src/setup/README.md](../src/setup/README.md), backed by `src/setup/wizard/mod.rs`.

## Running As A Service

ThinClaw ships with service helpers for:

- macOS `launchd`
- Linux `systemd --user`
- Windows Service Control Manager

The service path runs the normal agent bootstrap without interactive local runtime surfaces:

```bash
thinclaw run --no-onboard
```

If you are diagnosing a service startup problem, prefer your service manager's journal first, then run the same command manually with `--debug` for a verbose foreground boot.

Useful commands:

```bash
thinclaw service install
thinclaw service start
thinclaw service status
thinclaw service stop
thinclaw service uninstall
```

The service manager is the right path when you want ThinClaw always available on a dedicated host.

Windows notes:

- `thinclaw service install` registers ThinClaw with the Windows Service Control Manager.
- Service logs are written under the ThinClaw runtime logs directory.
- Setup, status, and reset flows use OS secure-store wording on Windows; they do not require Unix shell syntax.

## Reckless Desktop Autonomy

Desktop autonomy is the deployment path for operators who want ThinClaw to act inside the host GUI session, use local productivity apps, capture evidence, and manage its own local code rollout path.

This mode is controlled through the top-level `desktop_autonomy` settings group. The relevant operator settings are:

- `desktop_autonomy.enabled`
- `desktop_autonomy.profile`
- `desktop_autonomy.deployment_mode`
- `desktop_autonomy.target_username`
- `desktop_autonomy.desktop_max_concurrent_jobs`
- `desktop_autonomy.desktop_action_timeout_secs`
- `desktop_autonomy.capture_evidence`
- `desktop_autonomy.emergency_stop_path`
- `desktop_autonomy.pause_on_bootstrap_failure`
- `desktop_autonomy.kill_switch_hotkey`

`reckless_desktop` is intentionally stronger than a normal local run:

- it enables host desktop tools
- it seeds desktop routines and skills
- it captures screenshots/OCR/action evidence
- it allows managed local code autorollout with canaries, promotion, and rollback

Use [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md) as the canonical operator guide for that mode. The rest of this section is the deployment summary.

### Deployment Modes

Desktop autonomy supports two operator-facing deployment shapes:

| Mode | Intended Use | Notes |
|---|---|---|
| `whole_machine_admin` | Main logged-in machine account | Default |
| `dedicated_user` | Separate desktop session reserved for autonomy | Requires `target_username` and an active GUI login |

Dedicated-user deployment still uses a real GUI session. ThinClaw can install the launcher and create the user when privilege is available, but it does not auto-login that user or bypass the one-time permission grant.

### Bootstrap Expectations

Desktop autonomy bootstrap:

1. checks sidecar health and permissions
2. validates platform-specific prerequisites
3. prepares the dedicated-user path when requested
4. creates canary fixtures
5. seeds starter skills and routines
6. writes and optionally loads the desktop session launcher

If bootstrap fails and `desktop_autonomy.pause_on_bootstrap_failure = true`, ThinClaw pauses desktop autonomy automatically until the operator fixes the blocker and resumes it.

### Platform Notes

Desktop autonomy exists across platforms, but it is not equally mature everywhere.

- macOS is the most polished path today and expects Calendar, Numbers, Pages, and TextEdit
- Windows is supported through the native Windows bridge path and expects Outlook, Excel, Word, and Notepad plus an interactive session
- Linux is best-effort and prerequisite-heavy, requiring a compatible desktop session plus LibreOffice/Evolution/accessibility tooling

Treat bootstrap output as the source of truth for whether a given host is ready.

### Emergency Stop And Live Smoke

Desktop autonomy checks the emergency-stop file before routines fire and before actions run. The default path is:

```text
~/.thinclaw/AUTONOMY_DISABLED
```

Live desktop smoke coverage is intentionally ignored by default and should be run only on a sacrificial host with permissions already granted:

```bash
THINCLAW_LIVE_DESKTOP_SMOKE=1 cargo test --test desktop_autonomy_live_smoke -- --ignored
```

## Remote Gateway Access

Remote access is a deployment choice, not a default.

If you want ThinClaw reachable from another device:

1. enable the gateway
2. bind it to the right host
3. choose an auth token
4. restrict network exposure

Typical environment shape:

```env
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
```

Notes:

- `3000` is the code-backed default
- using a different port is fine if your deployment intentionally sets `GATEWAY_PORT`
- do not expose the gateway to the public internet without an intentional security layer

For safer remote access, prefer:

- Tailscale
- a trusted private network
- a reverse proxy you control

Treat tunnels as optional integrations, not default behavior.

PowerShell equivalent:

```powershell
$env:GATEWAY_ENABLED = "true"
$env:GATEWAY_HOST = "0.0.0.0"
$env:GATEWAY_PORT = "3000"
$env:GATEWAY_AUTH_TOKEN = "replace-with-a-long-random-token"
thinclaw
```

## Tunnels And Webhook Delivery

Channels like Telegram, Slack, and Discord support two message delivery modes:

| Mode | Latency | Requirements | When Used |
|---|---|---|---|
| **Polling** | ~5 seconds | None — works behind any NAT/firewall | Default when no tunnel is configured |
| **Webhook** | < 200ms | Publicly reachable HTTPS URL | When a tunnel provides a public URL |

Polling is reliable and zero-config. Webhook mode requires a tunnel because most home networks use NAT — external servers (e.g. Telegram's API) cannot reach your machine directly.

### Supported Tunnel Providers

| Provider | Prerequisites | Persistent URL | Config |
|---|---|---|---|
| **Tailscale Funnel** | Tailscale app + CLI installed, Funnel enabled in admin console | Yes | `TUNNEL_PROVIDER=tailscale`, `TUNNEL_TS_FUNNEL=true` |
| **ngrok** | `ngrok` binary, auth token | Paid plan only | `TUNNEL_PROVIDER=ngrok`, `TUNNEL_NGROK_TOKEN=...` |
| **Cloudflare Tunnel** | `cloudflared` binary, tunnel token | Yes | `TUNNEL_PROVIDER=cloudflare`, `TUNNEL_CF_TOKEN=...` |
| **Custom** | Your own tunnel command | Depends | `TUNNEL_PROVIDER=custom`, `TUNNEL_CUSTOM_COMMAND=...` |
| **Static URL** | You manage the tunnel yourself | Depends | `TUNNEL_URL=https://...` |

### Tailscale Setup (Recommended)

Tailscale Funnel is the recommended tunnel for most users: free, zero-config persistent hostname, and built-in HTTPS.

**Important:** Tailscale offers two modes:

- **Funnel (public)** — makes your machine reachable from the public internet. Required for webhooks.
- **Serve (tailnet-only)** — only reachable from devices on your Tailscale network. Good for private Web UI access, but webhook channels will fall back to polling.

Prerequisites:

1. Install Tailscale: https://tailscale.com/download
2. Install the Tailscale CLI:
   - **macOS (App Store / standalone):** Click the Tailscale menu bar icon → settings → **Install CLI**. Do NOT run the GUI binary at `/Applications/Tailscale.app/Contents/MacOS/Tailscale` directly — it is not a CLI and will crash.
   - **macOS (Homebrew):** `brew install tailscale`
   - **Linux:** `curl -fsSL https://tailscale.com/install.sh | sh`
3. Enable HTTPS in the Tailscale admin console: https://login.tailscale.com/admin/dns
4. Enable Funnel in your ACL policy: https://login.tailscale.com/admin/acls/file — ensure `"attr": ["funnel"]` is present in `nodeAttrs`

Config:

```env
TUNNEL_PROVIDER=tailscale
TUNNEL_TS_FUNNEL=true
# Optional: override auto-detected hostname
# TUNNEL_TS_HOSTNAME=my-host.tail1234.ts.net
```

### ngrok Setup

```env
TUNNEL_PROVIDER=ngrok
TUNNEL_NGROK_TOKEN=your-auth-token
# Optional: custom domain (paid plan)
# TUNNEL_NGROK_DOMAIN=my-agent.ngrok.app
```

Get your auth token from: https://dashboard.ngrok.com/get-started/your-authtoken

Install: `brew install ngrok` (macOS), `snap install ngrok` (Linux), or https://ngrok.com/download

### Cloudflare Tunnel Setup

```env
TUNNEL_PROVIDER=cloudflare
TUNNEL_CF_TOKEN=your-tunnel-token
```

Get your tunnel token from: https://one.dash.cloudflare.com/ → Networks → Tunnels

Install: `brew install cloudflare/cloudflare/cloudflared` (macOS) or https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/

### No Tunnel (Polling Mode)

If no tunnel is configured, webhook channels use polling mode automatically. No action needed. This is the safest default and works from any network.

### Full Environment Variable Reference

| Variable | Provider | Required | Description |
|---|---|---|---|
| `TUNNEL_PROVIDER` | All | Yes | `tailscale`, `ngrok`, `cloudflare`, `custom`, or `none` |
| `TUNNEL_URL` | Static | No | Skip managed tunnel, use this URL directly |
| `TUNNEL_TS_FUNNEL` | Tailscale | For webhooks | `true` for public Funnel, `false` for tailnet-only Serve |
| `TUNNEL_TS_HOSTNAME` | Tailscale | No | Override auto-detected hostname |
| `TUNNEL_NGROK_TOKEN` | ngrok | Yes | ngrok auth token |
| `TUNNEL_NGROK_DOMAIN` | ngrok | No | Custom domain (paid plan) |
| `TUNNEL_CF_TOKEN` | Cloudflare | Yes | Cloudflare Zero Trust tunnel token |
| `TUNNEL_CUSTOM_COMMAND` | Custom | Yes | Shell command with `{host}`/`{port}` placeholders |
| `TUNNEL_CUSTOM_HEALTH_URL` | Custom | No | HTTP endpoint for health checks |
| `TUNNEL_CUSTOM_URL_PATTERN` | Custom | No | Substring to match in stdout for URL extraction |

## Scrappy Connection Model

ThinClaw can run behind Scrappy in two shapes:

- embedded directly inside Scrappy
- remotely, with Scrappy connecting over the gateway

For remote mode, Scrappy needs the ThinClaw gateway URL and auth token. The gateway is the control plane for chat, memory, routines, logs, providers, settings, and operator actions.

## Docker And External Dependencies

Docker is optional and only matters if you want Docker-backed sandbox execution or container-based deployment. It is not required for a basic ThinClaw install.

The deployment Dockerfile intentionally builds the `full` profile by default:

```bash
docker build --build-arg BUILD_FEATURES=full -t thinclaw:latest .
docker run --env-file deploy/.env -p 3000:3000 thinclaw:latest
```

Use `BUILD_FEATURES=light` only when you want a smaller image without the full
runtime integrations:

```bash
docker build --build-arg BUILD_FEATURES=light -t thinclaw:light .
```

Other optional external dependencies include:

- Signal CLI
- local inference engines
- tunnel providers
- browser automation dependencies

See [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md) for the current dependency matrix.

## Operator Surfaces

ThinClaw exposes several operator-facing surfaces:

- CLI commands under `thinclaw ...`
- the web gateway UI
- the gateway API
- channel delivery surfaces

Canonical references:

- [../README.md](../README.md)
- [../tools-docs/README.md](../tools-docs/README.md)
- [../channels-docs/README.md](../channels-docs/README.md)
- [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)

## Troubleshooting

### The gateway is not available on `127.0.0.1:3000`

Check:

- you are running a current build; the local gateway is part of the default `light` profile
- ThinClaw is actually running
- you did not override `GATEWAY_PORT`
- the gateway is enabled for the current deployment

For deeper startup diagnostics, run `thinclaw --debug` or `thinclaw --debug tui` in the foreground.

### The host is reachable locally but not from another machine

Check:

- `GATEWAY_HOST` is not loopback-only
- your firewall allows the chosen port
- you are using the correct host address
- your network path is private or explicitly secured

### The source build runs but is missing full-runtime integrations

The default `light` profile includes the gateway but not browser automation,
tunnels, Docker sandbox jobs, Nostr, or ACP. Rebuild with:

```bash
cargo build --release --features full
```

### Windows browser or sandbox fallback is unavailable

Check:

- Docker Desktop is installed and running if you need the browser fallback or Docker-backed sandbox
- Chrome, Edge, or Brave is installed for local browser automation
- `thinclaw doctor` or `thinclaw status` output is being read on Windows-native terms rather than Unix shell setup examples

### Windows secrets or service setup looks wrong

Check:

- onboarding completed on the same Windows account that will run ThinClaw
- the Windows OS secure store is available for local installs, or `SECRETS_MASTER_KEY` is set for CI/container flows
- `thinclaw service status` reflects the Windows Service Control Manager state after install/start

### Tailscale tunnel crashes with `BundleIdentifiers.swift` fatal error

The binary at `/Applications/Tailscale.app/Contents/MacOS/Tailscale` is the GUI app, not the CLI tool. Running it directly from a terminal crashes with:

```
Fatal error: The current bundleIdentifier is unknown to the registry
```

Fix: install the Tailscale CLI through the Tailscale menubar app:

1. Click the Tailscale icon in your menu bar
2. Go to settings
3. Click **Install CLI**

Do NOT create manual symlinks to the GUI binary. The CLI must be installed through Tailscale's own mechanism.

### Telegram webhook fails with "Failed to resolve host"

This means Telegram's servers cannot reach your tunnel URL. Common causes:

- **Tailscale Serve (not Funnel):** `.ts.net` hostnames are only resolvable within your tailnet. Set `TUNNEL_TS_FUNNEL=true` and enable Funnel in the admin console.
- **No tunnel configured:** Without a tunnel, Telegram automatically falls back to polling. The error is logged but not fatal.
- **Funnel not enabled in admin console:** Visit https://login.tailscale.com/admin/dns and enable HTTPS, then ensure your ACL policy includes `"attr": ["funnel"]`.

### Setup docs and deployment docs disagree

When there is a disagreement:

- setup behavior is owned by [../src/setup/README.md](../src/setup/README.md) and the wizard code
- deployment defaults are owned by the config/runtime code and this guide
- broad overview docs should be updated to match those canonicals

## Logging Expectations

- `thinclaw` and `thinclaw run` share the same default startup behavior and only show warnings and errors
- `thinclaw --debug` and `thinclaw --debug run` enable verbose terminal logs for troubleshooting
- `RUST_LOG=...` remains available for custom filtering and overrides the default terminal behavior
- after startup, prefer `thinclaw logs ...` or the gateway log viewer when you want to inspect runtime events without restarting

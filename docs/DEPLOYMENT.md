# ThinClaw Deployment Guide

This guide is the canonical operator reference for running ThinClaw outside the development loop.

It covers:

- local standalone use
- long-running service mode
- remote gateway access
- Scrappy-connected deployments
- source-build feature choices

For onboarding details, use [../src/setup/README.md](../src/setup/README.md). For identity and surface behavior, use [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md) and [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md).

## Deployment Modes

| Mode | Best For | Main Shape |
|---|---|---|
| Local standalone | laptop, workstation, single-user local run | `thinclaw run --no-onboard` |
| Long-running service | Mac Mini, home server, Linux box, Windows workstation/server, VPS | launchd, systemd user service, or Windows Service Control Manager |
| Remote gateway | controlled LAN or Tailscale access | bind gateway to non-loopback host |
| Scrappy backend | desktop app + remote or local ThinClaw runtime | Scrappy talks to ThinClaw over the gateway |

## Defaults And Important Truths

- Code-backed default gateway port: `3000`
- Local default gateway URL: `http://127.0.0.1:3000`
- Remote access is opt-in through host/bind settings, not the default
- Source builds default to the `light` feature set, which does **not** include the web gateway, tunnel support, or Docker sandbox
- If you need the gateway from source, build with `--features full` or explicitly add `web-gateway`

## Fast Local Path

If you installed a release build:

```bash
thinclaw onboard
thinclaw run --no-onboard
```

Startup logging defaults to a quiet operator experience: `thinclaw` and `thinclaw run` both show warnings and errors, not the full initialization trace. For a verbose startup session, use either:

```bash
thinclaw --debug --no-onboard
thinclaw --debug run --no-onboard
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
thinclaw run --no-onboard
```

If you are using a release install on Windows, prefer the MSI for PATH integration and service-friendly installs. The portable ZIP is supported for manual or side-by-side installs.

## Build From Source

The default source build is intentionally lightweight:

```bash
cargo build --release
```

That default maps to the `light` feature set and excludes:

- web gateway
- tunnel integrations
- Docker sandbox

If you want the full operator-facing runtime from source:

```bash
cargo build --release --features full
```

If you want a more selective source build, combine features explicitly:

```bash
cargo build --release --features "light web-gateway repl"
```

Relevant reference:

- [BUILD_PROFILES.md](BUILD_PROFILES.md)
- [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)

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

The service path runs:

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
- using a different port, including `18789`, is fine if your deployment wants that
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
thinclaw run --no-onboard
```

## Scrappy Connection Model

ThinClaw can run behind Scrappy in two shapes:

- embedded directly inside Scrappy
- remotely, with Scrappy connecting over the gateway

For remote mode, Scrappy needs the ThinClaw gateway URL and auth token. The gateway is the control plane for chat, memory, routines, logs, providers, settings, and operator actions.

## Docker And External Dependencies

Docker is optional and only matters if you want Docker-backed sandbox execution or container-based deployment. It is not required for a basic ThinClaw install.

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

- you built or installed a runtime that includes the web gateway
- ThinClaw is actually running
- you did not override `GATEWAY_PORT`
- the gateway is enabled for the current deployment

For deeper startup diagnostics, run `thinclaw --debug --no-onboard` or `thinclaw --debug run --no-onboard` in the foreground.

### The host is reachable locally but not from another machine

Check:

- `GATEWAY_HOST` is not loopback-only
- your firewall allows the chosen port
- you are using the correct host address
- your network path is private or explicitly secured

### The source build runs but has no gateway

You probably built the default `light` profile. Rebuild with:

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

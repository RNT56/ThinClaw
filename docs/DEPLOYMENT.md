# ThinClaw Deployment Guide

This is the deployment entry point. Pick the host or deployment shape that
matches your situation, then follow that runbook end to end.

For onboarding behavior, use [../src/setup/README.md](../src/setup/README.md).
For identity and surface behavior, use
[IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md) and
[SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md).

## Choose Your Path

| Situation | Start Here |
|---|---|
| Mac laptop, workstation, or Mac Mini | [deploy/macos.md](deploy/macos.md) |
| Windows native install | [deploy/windows.md](deploy/windows.md) |
| WSL 2 on Windows | [deploy/windows.md#wsl-guidance](deploy/windows.md#wsl-guidance) |
| Generic Linux laptop, workstation, server, or VPS | [deploy/linux.md](deploy/linux.md) |
| Raspberry Pi OS Lite 64-bit | [deploy/raspberry-pi-os-lite.md](deploy/raspberry-pi-os-lite.md) |
| Docker Compose or container deployment | [deploy/docker.md](deploy/docker.md) |
| Remote gateway, Tailscale, or webhook tunnels | [deploy/remote-access.md](deploy/remote-access.md) |
| Reckless desktop autonomy | [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md) |
| Scrappy connected to a local or remote ThinClaw runtime | Platform runbook plus [deploy/remote-access.md](deploy/remote-access.md) |

## Defaults And Important Truths

- Code-backed default gateway port: `3000`
- Local default gateway URL: `http://127.0.0.1:3000`
- Remote access is opt-in through host/bind settings, not the default
- Source builds default to the `light` feature set
- If you need the full production/runtime surface from source, build with `--features full`
- Desktop autonomy is a separate privileged operator mode

The default `light` source profile includes the local gateway but excludes ACP,
tunnel support, Docker sandbox, browser automation, and Nostr. For source build
choices, use [BUILD_PROFILES.md](BUILD_PROFILES.md).

Raspberry Pi OS Lite 64-bit uses the same headless runtime surface with Pi
specific defaults. Prefer the published `aarch64-unknown-linux-gnu` release
artifact for native installs, or the multi-arch Docker image for Compose:

```bash
thinclaw doctor --profile pi-os-lite-64
cargo build --release --features full
docker compose pull thinclaw
```

Use `thinclaw onboard --profile pi-os-lite-64` for interactive Pi setup. Keep
`DESKTOP_AUTONOMY_ENABLED=false` on Pi OS Lite; the native installer also writes
`THINCLAW_RUNTIME_PROFILE=pi-os-lite-64` and `THINCLAW_HEADLESS=true`, which
block desktop autonomy tool registration at runtime. The detailed native and
Docker paths live in [deploy/raspberry-pi-os-lite.md](deploy/raspberry-pi-os-lite.md).

## Deployment Prerequisites Matrix

Use this as the high-level checklist before choosing a runbook. Platform pages
below repeat the prerequisites that matter for that host.

| Deployment Shape | Required Before You Start | Optional / Feature-Specific |
|---|---|---|
| Release install on macOS or Linux | `curl`, TLS access to GitHub Releases, normal user shell, writable install target in `PATH` | Chrome/Brave/Edge for local browser automation; Docker for sandbox jobs or Docker Chromium fallback; Tailscale/tunnel provider for remote access |
| Release install on Windows | MSI or portable ZIP from GitHub Releases, PowerShell, writable install location in `PATH` | Docker Desktop for sandbox jobs or Docker Chromium fallback; Chrome/Edge/Brave for local browser automation; `ffmpeg` for media capture/processing |
| Source build on macOS | Xcode Command Line Tools, Rust 1.92+, `wasm32-wasip2` target, Git | `wasm-tools`/`cargo-component` for building WASM extensions; Docker for sandbox jobs; optional feature dependencies from [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md) |
| Source build on Linux | C/C++ build tools, `pkg-config`, `curl`, Git, Rust 1.92+ | `wasm32-wasip2` for WASM extension builds; `libasound2-dev` when enabling `voice`; Docker, browser, and desktop packages as needed |
| Native service on macOS | Completed onboarding, launchd-capable logged-in user, secrets available to that user | Remote profile for SSH-managed Mac Mini hosts; Tailscale/private network for remote WebUI |
| Native service on Windows | Completed onboarding under the service account, Windows Service Control Manager access, secrets available to that account | Persist gateway/env settings before service mode; Docker Desktop and browser/media dependencies as needed |
| Linux `systemd --user` service | Completed onboarding, user systemd session with `pam_systemd`, `systemctl --user` reachable | Linger/session management if the service must survive logout; system-level service pattern from the Pi runbook when needed |
| Pi OS Lite native system service | Raspberry Pi OS Lite 64-bit Bookworm, ARM64 release binary or local build, `sudo`, `systemd`, `curl`, `tar`, token generation via `openssl` or `/dev/urandom` | Docker for sandbox/browser fallback; Tailscale for private network access; more swap for on-device source builds |
| Docker Compose deployment | Docker Engine/Desktop running, Compose V2, repo checkout or deploy assets, network access to GHCR or build context, `GATEWAY_AUTH_TOKEN` | PostgreSQL Compose profile; systemd wrapper; Docker Chromium fallback; firewall/Tailscale exposure |
| Remote WebUI over SSH | SSH access to the host, gateway enabled, auth token, local port forward | No public inbound port needed; not sufficient for public webhooks |
| Remote LAN/Tailscale access | Gateway bound to reachable interface, auth token, private network or tailnet route | Firewall rules, Tailscale CLI, Tailscale Funnel/Serve depending on access mode |
| Public webhook tunnel | Public HTTPS URL, provider token/binary for Tailscale Funnel, ngrok, Cloudflare Tunnel, or custom tunnel | DNS, TLS, proxy auth, rate limits, and firewall policy owned by the operator |
| Desktop autonomy | Interactive GUI session, platform desktop apps, platform permissions/accessibility approvals, `desktop_autonomy.profile = "reckless_desktop"` | Dedicated-user mode on macOS/Windows/Linux; Linux desktop packages and input backend; emergency stop path and bootstrap checks |

Core ThinClaw can start without optional external tools. Optional features simply
stay unavailable until their dependencies are installed and configured. The
complete optional dependency catalog is [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md).

## Platform Capability Matrix

| Capability | macOS | Windows | Linux | Raspberry Pi OS Lite |
|---|---|---|---|---|
| Local CLI and gateway | Supported | Supported | Supported | Supported |
| Full-screen TUI | Supported | Supported | Supported | Supported when built with TUI support |
| OS secure store | Supported | Supported | Supported | Headless env master key by default |
| `thinclaw service` lifecycle | launchd | Windows Service Control Manager | `systemd --user` | system-level systemd through `deploy/setup.sh` |
| Local browser automation | Chrome, Brave, Edge | Chrome, Edge, Brave | Chrome, Chromium, Brave, Edge | Local browser only if installed |
| Docker browser fallback | Supported | Docker Desktop | Supported | Supported when Docker is installed |
| Docker sandbox jobs | Supported when Docker is installed | Docker Desktop | Supported when Docker is installed | Supported when Docker is installed |
| Camera and microphone tools | Supported | Supported with `ffmpeg` | Supported with device permissions | Usually disabled/headless |
| Native Apple Mail and iMessage | Supported | Unsupported | Unsupported | Unsupported |
| BlueBubbles iMessage bridge | Supported | Supported | Supported | Supported |
| Desktop autonomy | Most mature path | Supported, prerequisite-driven | Best-effort desktop session path | Unsupported |

## Fast Local Reminder

macOS and Linux release install:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

thinclaw onboard
thinclaw
```

Windows release install:

```text
https://github.com/RNT56/ThinClaw/releases
```

Then in PowerShell:

```powershell
thinclaw onboard
thinclaw
```

Open the local gateway:

```text
http://127.0.0.1:3000
```

## Common Operator Commands

These commands apply across platforms unless the command itself reports that a
feature was not built or is unavailable on the current host.

| Task | Command |
|---|---|
| Start standard runtime | `thinclaw` or `thinclaw run` |
| Start full-screen runtime | `thinclaw tui` |
| Run onboarding | `thinclaw onboard` |
| Force TUI onboarding | `thinclaw onboard --ui tui` |
| Revisit guided setup | `thinclaw onboard --guide` |
| Reconfigure channels only | `thinclaw onboard --channels-only` |
| Reset local state | `thinclaw reset --yes` |
| Verbose startup | `thinclaw --debug run --no-onboard` |
| Tail logs | `thinclaw logs tail` |
| Tail error logs | `thinclaw logs tail -l error` |
| Health check | `thinclaw status` |
| Dependency probe | `thinclaw doctor` |
| Linux server readiness | `thinclaw doctor --profile server` |
| Remote/headless readiness | `thinclaw doctor --profile remote` |
| Linux desktop readiness | `thinclaw doctor --profile desktop-linux` |
| Pi OS Lite readiness | `thinclaw doctor --profile pi-os-lite-64` |
| All optional Linux feature readiness | `thinclaw doctor --profile all-features` |
| Manage config | `thinclaw config list`, `get`, `set` |
| Manage secrets | `thinclaw secrets status`, `list`, `set`, `delete` |
| Inspect providers/models | `thinclaw models` |
| Manage gateway | `thinclaw gateway` |
| Inspect channels | `thinclaw channels list` |
| Manage tools | `thinclaw tool list`, `install`, `remove` |
| Manage MCP servers | `thinclaw mcp list`, `add`, `auth`, `test` |
| Manage routines | `thinclaw cron` |
| Manage service | `thinclaw service install`, `start`, `status`, `stop`, `uninstall` |
| Self-update | `thinclaw update` |

For the complete command surface, use [CLI_REFERENCE.md](CLI_REFERENCE.md).

## Service Mode

ThinClaw ships one service command surface:

```bash
thinclaw service install
thinclaw service start
thinclaw service status
thinclaw service stop
thinclaw service uninstall
```

Platform backends:

| Platform | Service Backend |
|---|---|
| macOS | launchd |
| Linux | `systemd --user` |
| Raspberry Pi OS Lite native installer | system-level systemd service |
| Windows | Windows Service Control Manager |

The service path runs:

```bash
thinclaw run --no-onboard
```

If you are diagnosing service startup, inspect the platform service manager
first, then run the same command in the foreground with `--debug`.

## Configuration Layers

ThinClaw starts from bootstrap config first, then runtime settings:

1. process environment
2. `./.env`
3. `~/.thinclaw/.env`
4. optional TOML overlay
5. injected secrets
6. database-backed settings

Do not treat this guide as the source of truth for onboarding step order. The
canonical setup spec is [../src/setup/README.md](../src/setup/README.md), backed
by `src/setup/wizard/mod.rs`.

## Shared References

- [deploy/macos.md](deploy/macos.md)
- [deploy/windows.md](deploy/windows.md)
- [deploy/linux.md](deploy/linux.md)
- [deploy/raspberry-pi-os-lite.md](deploy/raspberry-pi-os-lite.md)
- [deploy/docker.md](deploy/docker.md)
- [deploy/remote-access.md](deploy/remote-access.md)
- [BUILD_PROFILES.md](BUILD_PROFILES.md)
- [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)
- [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md)

## Troubleshooting Ownership

When docs disagree:

- setup behavior is owned by [../src/setup/README.md](../src/setup/README.md)
  and the wizard code
- deployment defaults are owned by config/runtime code and these deploy runbooks
- external dependency details are owned by
  [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)
- broad overview docs should point here rather than duplicate platform details

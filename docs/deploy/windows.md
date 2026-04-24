# ThinClaw On Windows

Use this path for native Windows installs. WSL is supported as an advanced
Linux-like environment, but the primary Windows user path is native Windows.

## Choose A Path

| Goal | Recommended Path |
|---|---|
| Normal Windows install | MSI from GitHub Releases |
| Portable or side-by-side install | Portable ZIP from GitHub Releases |
| Always-on background runtime | `thinclaw service install` using Windows Service Control Manager |
| Docker-backed browser or sandbox fallback | Docker Desktop |
| Linux-style development | WSL 2, then follow the Linux docs inside WSL |

## Native Windows Install

Install the latest MSI or portable ZIP from GitHub Releases:

```text
https://github.com/RNT56/ThinClaw/releases
```

Then run in PowerShell:

```powershell
thinclaw onboard
thinclaw
```

Open the local gateway:

```text
http://127.0.0.1:3000
```

For a full-screen terminal runtime:

```powershell
thinclaw tui
```

Common post-install checks:

```powershell
thinclaw status
thinclaw doctor
thinclaw logs tail
```

Verbose startup diagnostics:

```powershell
thinclaw --debug
thinclaw --debug run --no-onboard
```

## Windows Service

After onboarding:

```powershell
thinclaw service install
thinclaw service start
thinclaw service status
```

Service management:

```powershell
thinclaw service stop
thinclaw service uninstall
```

The service registers with the Windows Service Control Manager. Service logs
are written under the ThinClaw runtime logs directory.

If service setup looks wrong, check:

- onboarding completed on the same Windows account that will run ThinClaw
- the Windows OS secure store is available for local installs
- `SECRETS_MASTER_KEY` is set only for CI, container, or intentionally headless flows
- `thinclaw service status` reflects the Service Control Manager state

## Remote Gateway Access

Remote access is opt-in. For LAN or private-network access:

```powershell
$env:GATEWAY_ENABLED = "true"
$env:GATEWAY_HOST = "0.0.0.0"
$env:GATEWAY_PORT = "3000"
$env:GATEWAY_AUTH_TOKEN = "replace-with-a-long-random-token"
thinclaw
```

Persist those values through your preferred Windows environment or ThinClaw
configuration path before using them in service mode.

For Tailscale, tunnels, and webhook delivery, use [remote-access.md](remote-access.md).

## Docker Desktop

Docker Desktop is optional. Install it when you need Docker-backed sandbox jobs,
Docker Chromium fallback, or container development.

```powershell
winget install Docker.DockerDesktop
```

Then restart PowerShell and verify:

```powershell
docker version
docker compose version
```

## WSL Guidance

Use WSL 2 when you specifically want Linux tooling, source builds, or Linux-like
automation from a Windows machine.

Inside WSL:

1. Follow [linux.md](linux.md) for local or source builds.
2. Treat the WSL filesystem and Windows filesystem as separate runtime homes.
3. Prefer Windows native ThinClaw when you need Windows desktop autonomy or
   Windows Service Control Manager integration.
4. Prefer WSL ThinClaw when your target is Linux development or Linux service
   behavior.

Do not mix one ThinClaw home between native Windows and WSL unless you have a
specific migration plan.

## Desktop Autonomy

Windows desktop autonomy uses the native Windows bridge path and expects an
interactive Windows session plus Microsoft Office or compatible local apps.

Use [../DESKTOP_AUTONOMY.md](../DESKTOP_AUTONOMY.md) as the canonical guide.

## Troubleshooting

If browser or sandbox fallback is unavailable:

- install and start Docker Desktop if you need Docker fallback
- install Chrome, Edge, or Brave for local browser automation
- read `thinclaw doctor` and `thinclaw status` output as Windows-native checks,
  not Unix shell setup instructions

For the full command surface after deployment, use
[../CLI_REFERENCE.md](../CLI_REFERENCE.md).

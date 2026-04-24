# ThinClaw Deploy Assets

This directory contains deploy helpers and examples. User-facing runbooks live
under [../docs/deploy/](../docs/deploy/).

## Which File Applies?

| File | Applies To | Notes |
|---|---|---|
| [setup.sh](setup.sh) | Linux server, Docker mode, Raspberry Pi OS Lite native mode | Main Linux/Pi helper script |
| [docker-compose.yml](docker-compose.yml) | Docker Compose deployments | Uses GHCR image by default and can build from source |
| [env.example](env.example) | Direct binary and Compose environment examples | Copy to `~/.thinclaw/.env` or `deploy/.env` |
| [thinclaw.service](thinclaw.service) | Legacy/internal Docker plus Cloud SQL shape | Not the current public default |
| [cloud-sql-proxy.service](cloud-sql-proxy.service) | Legacy/internal Cloud SQL shape | Not needed for normal self-hosting |

## Platform Runbooks

- [macOS](../docs/deploy/macos.md)
- [Windows](../docs/deploy/windows.md)
- [Linux](../docs/deploy/linux.md)
- [Raspberry Pi OS Lite 64-bit](../docs/deploy/raspberry-pi-os-lite.md)
- [Docker](../docs/deploy/docker.md)
- [Remote access](../docs/deploy/remote-access.md)

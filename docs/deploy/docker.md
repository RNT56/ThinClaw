# ThinClaw Docker Deployment

Use this path for container deployment on Linux, Pi OS Lite, a VPS, or a host
where Docker is your preferred service boundary.

Docker is optional. It is only required for container deployment, Docker-backed
sandbox execution, or Docker Chromium browser fallback.

## Prerequisites

Required for Compose:

- Docker Engine on Linux/Pi, or Docker Desktop on macOS/Windows
- Docker Compose V2 (`docker compose version`)
- a repo checkout or the `deploy/` assets
- network access to pull `ghcr.io/rnt56/thinclaw:latest`, unless you build locally
- a 32-byte random `GATEWAY_AUTH_TOKEN` encoded as 64 hexadecimal characters
- a separate, stable 32-byte `SECRETS_MASTER_KEY` encoded as 64 hexadecimal
  characters; losing it makes encrypted values unreadable
- `curl` for health checks
- `openssl` for the token-generation examples, or another secure random-token generator
- `sed` for the shell snippets shown below

Verify before starting:

```bash
docker version
docker compose version
docker pull ghcr.io/rnt56/thinclaw:latest
```

Windows notes:

- Docker Desktop must be installed and running before Compose commands work.
- WSL 2 integration is recommended for Linux-container workflows.
- Run commands from PowerShell or a WSL shell consistently; do not mix runtime homes.

Linux server notes:

- The setup script can install Docker on `apt`, `dnf`, or `yum` hosts.
- If you manage Docker manually, make sure the daemon is running and the operator
  account can run Docker commands.
- Expose port `3000` only on a trusted network, behind a reverse proxy, or behind
  Tailscale unless you intentionally publish it.

Optional:

- PostgreSQL overlay if you want a separately managed database instead of libSQL.
- systemd wrapper if you want Compose managed by the host service manager.
- Docker Chromium fallback for browser automation on headless hosts.

## Compose Quick Start

From a repo checkout:

```bash
cd deploy
cp env.example .env
sed -i "s/^GATEWAY_AUTH_TOKEN=.*/GATEWAY_AUTH_TOKEN=$(openssl rand -hex 32)/" .env
sed -i "s/^SECRETS_MASTER_KEY=.*/SECRETS_MASTER_KEY=$(openssl rand -hex 32)/" .env
chmod 0600 .env

docker compose pull thinclaw
docker compose up -d
curl http://localhost:3000/api/health
```

ThinClaw Desktop connects to:

```text
http://<server-ip>:3000
```

Use the value of `GATEWAY_AUTH_TOKEN` from `deploy/.env`.

Common Compose operations:

```bash
docker compose ps
docker compose logs -f thinclaw
docker compose restart thinclaw
docker compose down
```

The public Compose file is directly bootable: it runs `thinclaw run
--skip-setup-check`, enables the intentional headless secrets fallback, and
stores the libSQL database and canonical ThinClaw home in `thinclaw-data`.
The workspace is stored independently in `thinclaw-workspace`. Keep the same
`.env` master key when restarting or recreating the container.

## Image And Local Build

The Compose file defaults to:

```env
THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest
```

The deployment Dockerfile packages a prebuilt ThinClaw binary; it does not
compile Rust in the container. For a locally patched build:

```bash
cargo build --locked --release --features full --bin thinclaw
cp target/release/thinclaw ./thinclaw
docker build --build-arg THINCLAW_BINARY=thinclaw -t thinclaw:local .
THINCLAW_IMAGE=thinclaw:local docker compose -f deploy/docker-compose.yml up -d
```

For a light-profile local image, build that binary on the host first:

```bash
cargo build --locked --release --features light --bin thinclaw
cp target/release/thinclaw ./thinclaw
docker build --build-arg THINCLAW_BINARY=thinclaw -t thinclaw:light .
```

## Raspberry Pi

On Pi OS Lite, prefer the published multi-arch image instead of building
on-device:

```bash
printf '%s\n\n' "$(openssl rand -hex 32)" | \
  sudo bash deploy/setup.sh --secrets-stdin --mode docker --allow-public-http \
    --image ghcr.io/rnt56/thinclaw:latest
```

`--allow-public-http` is an explicit unsafe opt-in. Prefer supplying a
Tailscale auth key on the second stdin line as described in the remote-access
guide.

For the full Pi path, use [raspberry-pi-os-lite.md](raspberry-pi-os-lite.md).

## PostgreSQL Overlay

PostgreSQL is deliberately absent from the public libSQL base. Start the pinned
pgvector overlay only when you choose that database. The password must be a
separate URL-safe, high-entropy value; the command below generates the exact
64-hex-character form enforced by the container entrypoint and stores it in the
private Compose environment for future operator commands:

```bash
sed -i "s/^# POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 32)/" .env
chmod 0600 .env
docker compose -f docker-compose.yml -f docker-compose.postgres.yml up -d
```

The default deployment uses libSQL:

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=/data/thinclaw.db
```

The overlay wires the service hostname `postgres`, waits for database health,
and persists the cluster in `thinclaw-postgres`. It intentionally fails Compose
configuration when `POSTGRES_PASSWORD` is absent; there is no development
password fallback. Keep this password stable while the volume exists. Rotate it
as a coordinated PostgreSQL credential change, never by replacing only `.env`.

To stop the overlay without deleting its volumes:

```bash
docker compose -f docker-compose.yml -f docker-compose.postgres.yml down
```

## Environment File

Use [../../deploy/env.example](../../deploy/env.example) as the starter:

```bash
cp deploy/env.example deploy/.env
```

Set at least:

```env
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
SECRETS_MASTER_KEY=replace-with-a-different-64-character-hex-key
THINCLAW_ALLOW_ENV_MASTER_KEY=1
ONBOARD_COMPLETED=true
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
OPENROUTER_API_KEY=sk-or-CHANGE_ME
```

For local direct binary installs, copy the same shape to `~/.thinclaw/.env`
instead of `deploy/.env`.

The public stack also applies configurable defaults of 2 GiB memory, 2 CPUs,
512 PIDs, and five 10 MiB JSON log files. Override `THINCLAW_MEMORY_LIMIT`,
`THINCLAW_CPU_LIMIT`, `THINCLAW_PIDS_LIMIT`, `THINCLAW_LOG_MAX_SIZE`, or
`THINCLAW_LOG_MAX_FILES` in `.env` when the host needs different bounds.

## systemd Wrapper For Compose

The Linux setup script can create a systemd service for Docker Compose mode:

```bash
printf '%s\n\n' "$(openssl rand -hex 32)" | \
  sudo bash deploy/setup.sh --secrets-stdin --mode docker \
    --allow-public-http --systemd
```

That script installs Docker when needed, configures UFW and Fail2ban when
available, writes `deploy/.env`, generates a master key only when no valid one
already exists, starts Compose, and optionally enables the systemd wrapper.

## Legacy Service Files

The files [../../deploy/thinclaw.service](../../deploy/thinclaw.service) and
[../../deploy/cloud-sql-proxy.service](../../deploy/cloud-sql-proxy.service)
appear to describe an older GCP Artifact Registry plus Cloud SQL deployment.
They are not the current public default path. Prefer `deploy/docker-compose.yml`
or `deploy/setup.sh` unless you are intentionally maintaining that legacy shape.

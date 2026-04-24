# ThinClaw Docker Deployment

Use this path for container deployment on Linux, Pi OS Lite, a VPS, or a host
where Docker is your preferred service boundary.

Docker is optional. It is only required for container deployment, Docker-backed
sandbox execution, or Docker Chromium browser fallback.

## Compose Quick Start

From a repo checkout:

```bash
cd deploy
cp env.example .env
sed -i "s/^GATEWAY_AUTH_TOKEN=.*/GATEWAY_AUTH_TOKEN=$(openssl rand -hex 32)/" .env

docker compose pull thinclaw
docker compose up -d
curl http://localhost:3000/api/health
```

Scrappy connects to:

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

## Image And Build Profile

The Compose file defaults to:

```env
THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest
BUILD_FEATURES=full
```

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

## Raspberry Pi

On Pi OS Lite, prefer the published multi-arch image instead of building
on-device:

```bash
sudo bash deploy/setup.sh --mode docker --token replace-with-a-long-random-token \
  --image ghcr.io/rnt56/thinclaw:latest
```

For the full Pi path, use [raspberry-pi-os-lite.md](raspberry-pi-os-lite.md).

## PostgreSQL Profile

The Compose file includes an optional PostgreSQL service. It starts only when
the `postgres` profile is enabled:

```bash
docker compose --profile postgres up -d
```

The default deployment uses libSQL:

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=/data/thinclaw.db
```

Use PostgreSQL when you intentionally want a separately managed database:

```env
DATABASE_BACKEND=postgres
DATABASE_URL=postgres://thinclaw:CHANGE_ME@postgres:5432/thinclaw
```

## Environment File

Use [../../deploy/env.example](../../deploy/env.example) as the starter:

```bash
cp deploy/env.example deploy/.env
```

Set at least:

```env
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
OPENROUTER_API_KEY=sk-or-CHANGE_ME
```

For local direct binary installs, copy the same shape to `~/.thinclaw/.env`
instead of `deploy/.env`.

## systemd Wrapper For Compose

The Linux setup script can create a systemd service for Docker Compose mode:

```bash
sudo bash deploy/setup.sh --mode docker --token replace-with-a-long-random-token --systemd
```

That script installs Docker when needed, configures UFW and Fail2ban when
available, writes `deploy/.env`, starts Compose, and optionally enables the
systemd wrapper.

## Legacy Service Files

The files [../../deploy/thinclaw.service](../../deploy/thinclaw.service) and
[../../deploy/cloud-sql-proxy.service](../../deploy/cloud-sql-proxy.service)
appear to describe an older GCP Artifact Registry plus Cloud SQL deployment.
They are not the current public default path. Prefer `deploy/docker-compose.yml`
or `deploy/setup.sh` unless you are intentionally maintaining that legacy shape.

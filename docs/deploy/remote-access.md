# ThinClaw Remote Access

Use this path when ThinClaw runs on one machine and you want to reach it from
another device, Scrappy, or a webhook-capable channel.

Remote access is a deployment choice, not a default. The first-class setup path is:

```bash
thinclaw onboard --profile remote
```

That profile enables the gateway, disables the interactive CLI channel for
service/headless runtime, generates `GATEWAY_AUTH_TOKEN` when missing, and
prints access instructions. It ships inside the normal `thinclaw` binary from
GitHub Releases; there is no separate remote binary.

## Gateway Basics

Local default:

```text
http://127.0.0.1:3000
```

Remote access requires:

1. enable the gateway
2. bind it to the right host
3. choose an auth token
4. restrict network exposure

Default remote profile environment shape:

```env
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=generated-by-onboarding
CLI_ENABLED=false
```

Check access at any time:

```bash
thinclaw gateway access
thinclaw gateway access --show-token
```

## SSH Tunnel Recommended

This is the safest default for VPS, Pi, and Mac Mini hosts managed over SSH.
Keep the gateway bound to loopback:

```env
GATEWAY_HOST=127.0.0.1
```

From your laptop:

```bash
ssh -L 3000:127.0.0.1:3000 user@host
```

Then open:

```text
http://127.0.0.1:3000/?token=<gateway-token>
```

## Private LAN Or Tailscale

Use this when every client is on a trusted LAN or tailnet:

```env
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
CLI_ENABLED=false
```

Then connect from the private address:

```text
http://<host-or-tailnet-ip>:3000/?token=<gateway-token>
```

## Reverse Proxy Or Public Exposure

Keep `GATEWAY_AUTH_TOKEN` required. TLS, proxy auth, firewall rules, rate
limits, DNS, and public exposure policy are operator-owned. Prefer binding the
gateway to `127.0.0.1` behind a local reverse proxy unless you intentionally
need a broader bind.

## Runtime And Service Handoff

Remote/headless hosts should run:

```bash
thinclaw run --no-onboard
```

Or install the OS service:

```bash
thinclaw service install
thinclaw service start
thinclaw service status
```

The service path sets `CLI_ENABLED=false` so stdin EOF does not stop the
runtime. Service install blocks explicitly configured remote gateways that lack
`GATEWAY_AUTH_TOKEN`.

## Tailscale For Private Access

Private access from Scrappy or another device is usually simplest through
Tailscale.

Linux or Pi installer path:

```bash
sudo bash deploy/setup.sh --mode auto --token replace-with-a-long-random-token \
  --tailscale tskey-auth-...
```

Manual path:

```bash
tailscale ip -4
```

Set:

```env
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=replace-with-a-long-random-token
```

Then connect from a tailnet device:

```text
http://<tailscale-ip>:3000/?token=<gateway-token>
```

## Webhook Delivery

Channels like Telegram, Slack, and Discord support two message delivery modes:

| Mode | Latency | Requirements | When Used |
|---|---|---|---|
| Polling | Around 5 seconds | None, works behind NAT/firewalls | Default when no tunnel is configured |
| Webhook | Under 200 ms | Publicly reachable HTTPS URL | When a tunnel provides a public URL |

Polling is reliable and zero-config. Webhook mode requires a tunnel because
most home networks use NAT and external services cannot reach your machine
directly.

## Supported Tunnel Providers

| Provider | Prerequisites | Persistent URL | Config |
|---|---|---|---|
| Tailscale Funnel | Tailscale app plus CLI installed, Funnel enabled in admin console | Yes | `TUNNEL_PROVIDER=tailscale`, `TUNNEL_TS_FUNNEL=true` |
| ngrok | `ngrok` binary, auth token | Paid plan only | `TUNNEL_PROVIDER=ngrok`, `TUNNEL_NGROK_TOKEN=...` |
| Cloudflare Tunnel | `cloudflared` binary, tunnel token | Yes | `TUNNEL_PROVIDER=cloudflare`, `TUNNEL_CF_TOKEN=...` |
| Custom | Your own tunnel command | Depends | `TUNNEL_PROVIDER=custom`, `TUNNEL_CUSTOM_COMMAND=...` |
| Static URL | You manage the tunnel yourself | Depends | `TUNNEL_URL=https://...` |

## Tailscale Funnel

Tailscale has two relevant modes:

- Funnel, public, required for webhooks
- Serve, tailnet-only, good for private Web UI access but not public webhooks

Prerequisites:

1. Install Tailscale: https://tailscale.com/download
2. Install the Tailscale CLI:
   - macOS App Store or standalone: use the menu bar app settings and click
     Install CLI
   - macOS Homebrew: `brew install tailscale`
   - Linux: `curl -fsSL https://tailscale.com/install.sh | sh`
3. Enable HTTPS in the Tailscale admin console: https://login.tailscale.com/admin/dns
4. Enable Funnel in your ACL policy: https://login.tailscale.com/admin/acls/file

Config:

```env
TUNNEL_PROVIDER=tailscale
TUNNEL_TS_FUNNEL=true
# Optional:
# TUNNEL_TS_HOSTNAME=my-host.tail1234.ts.net
```

If Tailscale crashes with a `BundleIdentifiers.swift` fatal error on macOS, you
launched the GUI app binary instead of the CLI. Install the CLI through the
Tailscale menu bar app settings. Do not symlink the GUI binary.

## ngrok

```env
TUNNEL_PROVIDER=ngrok
TUNNEL_NGROK_TOKEN=your-auth-token
# Optional paid custom domain:
# TUNNEL_NGROK_DOMAIN=my-agent.ngrok.app
```

Get your auth token from:

```text
https://dashboard.ngrok.com/get-started/your-authtoken
```

Install:

```bash
brew install ngrok
# or on Linux:
snap install ngrok
```

## Cloudflare Tunnel

```env
TUNNEL_PROVIDER=cloudflare
TUNNEL_CF_TOKEN=your-tunnel-token
```

Get your tunnel token from Cloudflare Zero Trust:

```text
https://one.dash.cloudflare.com/
```

Install:

```bash
brew install cloudflare/cloudflare/cloudflared
```

Linux packages are available from Cloudflare's download page.

## No Tunnel

If no tunnel is configured, webhook channels use polling mode automatically.
No action is needed.

## Environment Variable Reference

| Variable | Provider | Required | Description |
|---|---|---|---|
| `TUNNEL_PROVIDER` | All | Yes | `tailscale`, `ngrok`, `cloudflare`, `custom`, or `none` |
| `TUNNEL_URL` | Static | No | Skip managed tunnel, use this URL directly |
| `TUNNEL_TS_FUNNEL` | Tailscale | For webhooks | `true` for public Funnel, `false` for tailnet-only Serve |
| `TUNNEL_TS_HOSTNAME` | Tailscale | No | Override auto-detected hostname |
| `TUNNEL_NGROK_TOKEN` | ngrok | Yes | ngrok auth token |
| `TUNNEL_NGROK_DOMAIN` | ngrok | No | Custom domain |
| `TUNNEL_CF_TOKEN` | Cloudflare | Yes | Cloudflare Zero Trust tunnel token |
| `TUNNEL_CUSTOM_COMMAND` | Custom | Yes | Shell command with `{host}` and `{port}` placeholders |
| `TUNNEL_CUSTOM_HEALTH_URL` | Custom | No | HTTP endpoint for health checks |
| `TUNNEL_CUSTOM_URL_PATTERN` | Custom | No | Substring to match in stdout for URL extraction |

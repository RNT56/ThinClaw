# Web Gateway Channel

> Built-in control plane with web UI, API, SSE streaming, WebSocket, and OpenAI-compatible access.

## Overview

The Gateway is ThinClaw's operator control plane. It serves a built-in web UI
and exposes a programmatic API for chat, memory, jobs, routines, extensions,
settings, and OpenAI-compatible access. Availability depends on the build
profile and gateway-enabled configuration.

The gateway follows the shared cross-surface vocabulary: `/personality` is the
primary session overlay command, `/compress` is the primary compaction command,
and the WebUI settings surface exposes `agent.name`, `agent.personality_pack`,
and the shared skin controls.

## Configuration

```bash
# Enable or disable the gateway with your deployment configuration

# Host and port (defaults: 127.0.0.1:3000)
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3000

# Auth token (auto-generated random hex if unset)
GATEWAY_AUTH_TOKEN=my-secret-token

# User ID for this gateway session
GATEWAY_USER_ID=default
```

## Features

### Web UI
- Modern chat interface with thread support
- Real-time streaming responses via SSE/WebSocket
- Tool call visualization with approve/deny
- Extension management UI
- Job monitoring for sandboxed tasks
- Memory browser (workspace file tree)
- Routine management
- Settings management
- Dark mode, responsive design

### API Surface

The gateway API groups include:

- chat and streaming
- memory and workspace access
- jobs and approvals
- extensions and installation
- routines and triggers
- settings and health
- OpenAI-compatible chat/model endpoints

The exact route table is maintained in the deployment and server docs so this
page can stay focused on the control-plane model.

### Authentication

All protected endpoints require a Bearer token:
```
Authorization: Bearer <token>
```

The token is displayed in the boot screen URL when the gateway is running.

### SSE Events

Streaming events include:
- `message_chunk` — Streaming text chunks
- `message_complete` — Full response ready
- `tool_call_start` / `tool_call_result` — Tool execution progress
- `approval_required` — Awaiting user approval
- `auth_required` — Extension needs authentication
- `status_update` — Typing indicators, progress

### WebSocket

The WebSocket channel provides bidirectional communication with the same event
types as SSE. Origin validation is enforced for local-first deployments.

### Rate Limiting

Chat endpoints are rate-limited to 30 messages per 60-second window.

### Security

- CORS restricted to localhost origins
- `X-Content-Type-Options: nosniff` and `X-Frame-Options: DENY` headers
- WebSocket origin validation (localhost only)
- Optional trusted-proxy auth mode for reverse proxies

## Exposing Remotely

To access the Gateway from outside the machine:

```bash
# Bind to all interfaces
GATEWAY_HOST=0.0.0.0 thinclaw
```

For wider access, use the deployment guide's tunnel and remote-access options.

# Web Gateway Channel

> Built-in web UI with REST API, SSE streaming, WebSocket, and OpenAI-compatible endpoint.

## Overview

The Gateway is a full-featured web interface for the agent. It serves a built-in
web UI and exposes a comprehensive REST API for programmatic access. Enabled by default.

## Configuration

```bash
# Enabled by default. To disable:
GATEWAY_ENABLED=false

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

### REST API Endpoints

| Category | Endpoint | Method | Description |
|----------|----------|--------|-------------|
| **Chat** | `/api/chat/send` | POST | Send a message |
| | `/api/chat/events` | GET | SSE event stream |
| | `/api/chat/ws` | GET | WebSocket upgrade |
| | `/api/chat/history` | GET | Paginated chat history |
| | `/api/chat/threads` | GET | List threads |
| | `/api/chat/thread/new` | POST | Create new thread |
| | `/api/chat/approval` | POST | Approve/deny tool execution |
| **Memory** | `/api/memory/tree` | GET | Workspace file tree |
| | `/api/memory/read` | GET | Read a file |
| | `/api/memory/write` | POST | Write a file |
| | `/api/memory/search` | POST | Search files |
| **Jobs** | `/api/jobs` | GET | List sandbox jobs |
| | `/api/jobs/{id}` | GET | Job details |
| | `/api/jobs/{id}/cancel` | POST | Cancel a job |
| **Extensions** | `/api/extensions` | GET | List extensions |
| | `/api/extensions/install` | POST | Install extension |
| **Routines** | `/api/routines` | GET | List routines |
| | `/api/routines/{id}/trigger` | POST | Trigger a routine |
| **Settings** | `/api/settings` | GET | List settings |
| | `/api/settings/{key}` | PUT | Update a setting |
| **Health** | `/api/health` | GET | Health check (no auth) |
| **OpenAI** | `/v1/chat/completions` | POST | OpenAI-compatible proxy |
| | `/v1/models` | GET | List available models |

### Authentication

All protected endpoints require a Bearer token:
```
Authorization: Bearer <token>
```

The token is displayed in the boot screen URL: `http://127.0.0.1:3000/?token=<token>`

### SSE Events

The `/api/chat/events` endpoint streams:
- `message_chunk` — Streaming text chunks
- `message_complete` — Full response ready
- `tool_call_start` / `tool_call_result` — Tool execution progress
- `approval_required` — Awaiting user approval
- `auth_required` — Extension needs authentication
- `status_update` — Typing indicators, progress

### WebSocket

`/api/chat/ws` provides bidirectional communication with the same event types as SSE.
Origin validation is enforced (localhost only).

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

# Or use a tunnel
thinclaw --tunnel cloudflare
```

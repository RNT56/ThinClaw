# Extension System Architecture

ThinClaw's extension system unifies three runtime types — **WASM channels**, **WASM tools**, and **MCP servers** — under a single discovery, installation, and lifecycle model.

## Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Extension System                               │
│                                                                     │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────────────────┐  │
│  │ WASM        │   │ WASM        │   │ MCP Servers             │  │
│  │ Channels    │   │ Tools       │   │ (Model Context Protocol)│  │
│  │             │   │             │   │                         │  │
│  │ • Telegram  │   │ • GitHub    │   │ • Any MCP-compatible    │  │
│  │ • WhatsApp  │   │ • Gmail     │   │   server (local/remote) │  │
│  │ • Slack     │   │ • Notion    │   │ • OAuth 2.1 / stdio     │  │
│  │ • Discord   │   │ • Okta      │   │ • Language-agnostic     │  │
│  └──────┬──────┘   └──────┬──────┘   └──────────┬──────────────┘  │
│         │                 │                      │                  │
│         └─────────────────┴──────────────────────┘                  │
│                           │                                         │
│                    ┌──────┴──────┐                                  │
│                    │ Extension   │                                  │
│                    │ Manager     │                                  │
│                    └──────┬──────┘                                  │
│                           │                                         │
│              ┌────────────┼────────────┐                           │
│              │            │            │                            │
│         ┌────┴────┐ ┌────┴────┐ ┌────┴────┐                      │
│         │Registry │ │Installer│ │Discovery│                      │
│         │Catalog  │ │         │ │(online) │                      │
│         └─────────┘ └─────────┘ └─────────┘                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Extension Types

### WASM Channels

Messaging integrations compiled to `.wasm` and loaded at runtime from `~/.thinclaw/channels/`.

| Channel | Description | Auth |
|---------|-------------|------|
| Telegram | Bot API via webhooks | Bot token |
| WhatsApp | Business Cloud API | Bearer token |
| Slack | Events API + Web API | Bot token, signing secret |
| Discord | Interactions (slash commands) | Bot token, public key |

**Key feature: Hot-reload** — The channel watcher polls `~/.thinclaw/channels/` every 3 seconds. Adding, modifying, or deleting a `.wasm` file triggers automatic channel loading/reloading/removal without restart.

### WASM Tools

Sandboxed tool modules compiled to `.wasm` and installed to `~/.thinclaw/tools/`.

| Tool | Description |
|------|-------------|
| GitHub | Issues, PRs, repositories |
| Gmail | Email send/receive |
| Google Calendar | Calendar management |
| Google Docs/Sheets/Slides | Document tools |
| Notion | Workspace integration |
| Slack (tool) | Post messages |
| Telegram (tool) | Send messages |
| Okta | Identity management |

### MCP Servers

External processes or hosted services that expose tools via the [Model Context Protocol](https://modelcontextprotocol.io/) (JSON-RPC). MCP is the industry standard created by Anthropic — any MCP-compatible server written in any language (Python, Node, Go, etc.) works with ThinClaw out of the box.

MCP servers support:
- **stdio transport** — local processes spawned as children, communicating via stdin/stdout
- **HTTP transport** — remote/hosted servers via Streamable HTTP
- **OAuth 2.1** — Dynamic Client Registration (DCR) or pre-configured clients with PKCE
- **Session management** — multiple concurrent MCP connections with automatic reconnection

#### Adding MCP Servers via CLI

```bash
# ── Stdio transport (local process) ──────────────────────────
# The agent spawns the process and communicates via stdin/stdout

# Official filesystem server
thinclaw mcp add filesystem --command npx \
  --args '-y,@modelcontextprotocol/server-filesystem,/tmp'

# Python-based server with environment variables
thinclaw mcp add my-jira --command uvx \
  --args 'mcp-server-jira' \
  --env 'JIRA_URL=https://mycompany.atlassian.net,JIRA_TOKEN=xxx'

# Any local MCP server
thinclaw mcp add my-tools --command python \
  --args 'my_mcp_server.py' \
  --description 'Custom company tools'

# ── HTTP transport (remote/hosted server) ────────────────────
# The agent connects to a running HTTP server

# Public server (no auth — localhost is allowed over plain HTTP)
thinclaw mcp add local-dev http://localhost:8080

# Hosted server with OAuth (auto-discovers endpoints via RFC 8414)
thinclaw mcp add notion https://mcp.notion.com \
  --client-id 'YOUR_CLIENT_ID' \
  --scopes 'read,write'

# Hosted server with pre-configured OAuth endpoints
thinclaw mcp add github https://api.github.com/mcp \
  --client-id 'YOUR_CLIENT_ID' \
  --auth-url 'https://github.com/login/oauth/authorize' \
  --token-url 'https://github.com/login/oauth/access_token' \
  --scopes 'repo,read:org'

# After adding an OAuth server, authenticate:
thinclaw mcp auth notion
```

#### Managing MCP Servers

```bash
thinclaw mcp list              # Show all configured servers
thinclaw mcp list --verbose    # Show URLs, auth status, commands, env vars
thinclaw mcp test notion       # Test connection + list available tools
thinclaw mcp toggle notion --disable  # Temporarily disable without removing
thinclaw mcp toggle notion --enable   # Re-enable
thinclaw mcp remove notion     # Delete configuration permanently
```

#### Config File Format

MCP servers are stored in `~/.thinclaw/mcp-servers.json` (or in the database when a DB backend is configured). The file is loaded automatically at startup.

```json
{
  "schema_version": 1,
  "servers": [
    {
      "name": "filesystem",
      "url": "",
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "enabled": true,
      "description": "Local filesystem access"
    },
    {
      "name": "notion",
      "url": "https://mcp.notion.com",
      "transport": "http",
      "oauth": {
        "client_id": "YOUR_CLIENT_ID",
        "scopes": ["read", "write"],
        "use_pkce": true
      },
      "enabled": true
    },
    {
      "name": "my-jira",
      "url": "",
      "transport": "stdio",
      "command": "uvx",
      "args": ["mcp-server-jira"],
      "env": {
        "JIRA_URL": "https://mycompany.atlassian.net",
        "JIRA_TOKEN": "your-token"
      },
      "enabled": true
    }
  ]
}
```

#### WebUI API

MCP servers can also be managed through the extension system REST API:

```bash
# Add an MCP server via API
curl -X POST http://localhost:3000/api/extensions/install \
  -H "Content-Type: application/json" \
  -d '{"name": "notion", "kind": "mcp_server", "url": "https://mcp.notion.com"}'

# Activate (connects and registers tools)
curl -X POST http://localhost:3000/api/extensions/notion/activate
```

#### What Happens at Startup

When ThinClaw starts (in `app.rs` Phase 5 — `init_extensions`):

1. Reads `~/.thinclaw/mcp-servers.json` (falls back to DB settings if available)
2. **Concurrently connects** to all enabled servers using a `JoinSet`
3. Auto-dispatches transport: **stdio** → spawns child process; **HTTP** → connects to URL
4. For OAuth servers: checks for stored access/refresh tokens, uses them if valid
5. Calls `client.list_tools()` → `client.create_tools()` to discover and register tools
6. All discovered tools are added to the `ToolRegistry` and immediately available to the LLM

If a server is unreachable at startup, the agent logs a warning and continues — other servers and channels are unaffected.

#### OAuth Authentication Flow

For hosted HTTPS servers that require OAuth:

1. `thinclaw mcp add <name> <url> --client-id <id>` — saves config
2. `thinclaw mcp auth <name>` — initiates the OAuth 2.1 flow:
   - Discovers authorization/token endpoints via `.well-known/oauth-authorization-server` (RFC 8414)
   - Falls back to Dynamic Client Registration (DCR) if no client ID is pre-configured
   - Opens browser for user authorization with PKCE challenge
   - Stores access + refresh tokens in the encrypted secrets store
3. On subsequent startups, tokens are automatically refreshed if expired

#### Implementation Reference

| Component | File | Size |
|-----------|------|------|
| Config schema & persistence | [`src/tools/mcp/config.rs`](../src/tools/mcp/config.rs) | 719 LOC |
| MCP client (HTTP + tool creation) | [`src/tools/mcp/client.rs`](../src/tools/mcp/client.rs) | 650+ LOC |
| OAuth 2.1 + DCR + PKCE | [`src/tools/mcp/auth.rs`](../src/tools/mcp/auth.rs) | 27K |
| Stdio transport (child process) | [`src/tools/mcp/stdio.rs`](../src/tools/mcp/stdio.rs) | 10K |
| Session management | [`src/tools/mcp/session.rs`](../src/tools/mcp/session.rs) | 9K |
| JSON-RPC protocol types | [`src/tools/mcp/protocol.rs`](../src/tools/mcp/protocol.rs) | 11K |
| CLI commands | [`src/cli/mcp.rs`](../src/cli/mcp.rs) | 713 LOC |
| ExtensionManager MCP integration | [`src/extensions/manager.rs`](../src/extensions/manager.rs) | (integrated) |

## Registry & Catalog

### Manifest Format

Each extension has a JSON manifest in `registry/{tools,channels}/<name>.json`:

```json
{
  "name": "telegram",
  "display_name": "Telegram",
  "kind": "channel",
  "version": "0.1.0",
  "description": "Telegram Bot API channel",
  "keywords": ["messaging", "chat", "bot"],
  "source": {
    "dir": "channels-src/telegram",
    "capabilities": "telegram.capabilities.json",
    "crate_name": "telegram-channel"
  },
  "artifacts": {
    "wasm32-wasip2": {
      "url": "https://github.com/example/releases/telegram.tar.gz",
      "sha256": null
    }
  },
  "auth_summary": {
    "method": "manual",
    "provider": "Telegram",
    "secrets": ["telegram_bot_token"],
    "setup_url": "https://t.me/BotFather"
  },
  "tags": ["messaging", "default"]
}
```

### Bundles

Bundle definitions in `registry/_bundles.json` group related extensions:

```json
{
  "bundles": {
    "default": {
      "display_name": "Recommended",
      "extensions": ["tools/github", "tools/gmail", "channels/telegram"]
    },
    "google": {
      "display_name": "Google Suite",
      "extensions": ["tools/gmail", "tools/google-calendar", "tools/google-docs"],
      "shared_auth": "google_oauth_token"
    }
  }
}
```

## CLI Commands

### Registry Commands

```bash
# List all available extensions
thinclaw registry list
thinclaw registry list --kind channel
thinclaw registry list --tag messaging --verbose

# Search extensions
thinclaw registry search "messaging"
thinclaw registry search "email"

# Show extension details
thinclaw registry info telegram
thinclaw registry info google          # shows bundle contents

# Install an extension
thinclaw registry install telegram
thinclaw registry install google       # installs entire bundle
thinclaw registry install-defaults     # installs "default" bundle
thinclaw registry install slack --force # overwrite existing
thinclaw registry install github --build # build from source

# Remove an installed extension
thinclaw registry remove telegram
thinclaw registry remove github
```

### Channel Commands

```bash
# List configured channels
thinclaw channels list
thinclaw channels list --format json

# Show channel details
thinclaw channels info telegram
thinclaw channels info signal
```

## REST API

The Gateway exposes extension management endpoints:

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/extensions` | List installed extensions |
| `GET` | `/api/extensions/tools` | List registered tools |
| `GET` | `/api/extensions/registry` | Search available extensions |
| `POST` | `/api/extensions/install` | Install extension by name |
| `POST` | `/api/extensions/{name}/activate` | Activate (with auto-auth) |
| `DELETE` | `/api/extensions/{name}/remove` | Remove extension |
| `GET` | `/api/extensions/{name}/setup` | Get setup schema |
| `POST` | `/api/extensions/{name}/setup` | Submit credentials |

### Example: Search and Install

```bash
# Search available extensions
curl http://localhost:3000/api/extensions/registry?query=messaging

# Install Telegram channel
curl -X POST http://localhost:3000/api/extensions/install \
  -H "Content-Type: application/json" \
  -d '{"name": "telegram", "kind": "wasm_channel"}'

# Submit credentials
curl -X POST http://localhost:3000/api/extensions/telegram/setup \
  -H "Content-Type: application/json" \
  -d '{"secrets": {"telegram_bot_token": "123456:ABC-DEF"}}'

# Activate
curl -X POST http://localhost:3000/api/extensions/telegram/activate
```

## Hot-Reload

### How It Works

The `ChannelWatcher` polls `~/.thinclaw/channels/` every 3 seconds using `mtime` comparison:

1. **New `.wasm` file** → Loads via `WasmChannelLoader`, calls `on_start`, registers with `ChannelManager`
2. **Modified `.wasm` file** → Shuts down old channel, loads new version, swaps in `ChannelManager`
3. **Deleted `.wasm` file** → Shuts down channel, removes from `ChannelManager`

Changes are debounced with a 1-second minimum interval to handle atomic writes.

### Usage

```bash
# Install a channel (hot-reload picks it up automatically)
thinclaw registry install telegram

# Or manually copy a WASM file
cp telegram.wasm ~/.thinclaw/channels/

# Update a channel (hot-reload detects the change)
cp updated-telegram.wasm ~/.thinclaw/channels/telegram.wasm

# Remove a channel (hot-reload detects deletion)
rm ~/.thinclaw/channels/telegram.wasm
```

### SSE Events

Channel status changes are broadcast via Server-Sent Events:

```json
{"type": "ChannelStatusChange", "channel": "telegram", "status": "online", "message": "Channel 'telegram' activated"}
{"type": "ChannelStatusChange", "channel": "telegram", "status": "removed", "message": "Channel 'telegram' removed"}
```

## Security Model

| Concern | Mitigation |
|---------|------------|
| Credential exposure | WASM channels never see raw tokens; host injects at boundary |
| Malicious WASM | Fuel metering, memory limits, restricted filesystem access |
| Path hijacking | `allowed_paths` restricts HTTP endpoints channels can register |
| State pollution | Fresh WASM instance per callback (no shared mutable state) |
| Resource exhaustion | Rate limiting on `emit_message`, minimum polling intervals |

## File Layout

```
~/.thinclaw/
├── channels/                  # WASM channel modules (hot-reloaded)
│   ├── telegram.wasm
│   ├── telegram.capabilities.json
│   ├── whatsapp.wasm
│   └── whatsapp.capabilities.json
├── tools/                     # WASM tool modules
│   ├── github.wasm
│   ├── github-tool.capabilities.json
│   └── ...
├── mcp-servers.json           # MCP server configurations
└── ...

registry/                      # Source manifests (in repo)
├── channels/
│   ├── telegram.json
│   ├── whatsapp.json
│   ├── slack.json
│   └── discord.json
├── tools/
│   ├── github.json
│   ├── gmail.json
│   └── ...
└── _bundles.json
```

## Architecture Comparison

| Feature | WASM Channels | WASM Tools | MCP Servers |
|---------|--------------|------------|-------------|
| Runtime | wasmtime sandbox | wasmtime sandbox | External process/HTTP |
| Install | Copy `.wasm` to dir | Copy `.wasm` to dir | Configure URL/command |
| Auth | Host-injected credentials | Capabilities file | OAuth 2.1 / manual |
| Hot-reload | ✅ File watcher | ❌ Requires restart | ✅ Reconnect on failure |
| Isolation | Full sandbox | Full sandbox | Process boundary |
| Language | Rust → WASM | Rust → WASM | Any (Python, Node, etc.) |
| Use case | Messaging transports | Agent capabilities | Third-party integrations |

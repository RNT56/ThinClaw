# WASM Channels

> Plugin-based channels loaded dynamically from WASM modules at runtime.

## Overview

WASM channels extend ThinClaw with custom channel implementations without
modifying core code. Each channel is a WASM component that defines behavior
via callbacks, while the host manages all infrastructure (HTTP, polling, timing).

## Configuration

```bash
# Enabled by default
WASM_CHANNELS_ENABLED=true

# Directory containing WASM channel modules (default: ~/.thinclaw/channels/)
WASM_CHANNELS_DIR=~/.thinclaw/channels/
```

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                  Host-Managed Event Loop                   │
│                                                            │
│   ┌──────────┐     ┌───────────┐     ┌──────────┐        │
│   │  HTTP    │     │  Polling  │     │  Timer   │        │
│   │  Router  │     │  Schedule │     │  Schedule│        │
│   └────┬─────┘     └─────┬─────┘     └────┬─────┘        │
│        └──────────────────┴────────────────┘               │
│                           │                                │
│          ┌────────────────┼────────────────┐              │
│          ▼                ▼                ▼               │
│   ┌────────────┐   ┌───────────┐   ┌────────────┐       │
│   │ on_http_req│   │  on_poll  │   │ on_respond │ WASM  │
│   └────────────┘   └───────────┘   └────────────┘       │
│                           │                                │
│                    ┌──────┴──────┐                         │
│                    │ Host Imports│                         │
│                    │ emit_message│──▶ MessageStream        │
│                    │ http_request│                         │
│                    └─────────────┘                         │
└──────────────────────────────────────────────────────────┘
```

## Security Model

| Threat | Mitigation |
|--------|------------|
| Path hijacking | `allowed_paths` restricts registrable endpoints |
| Token exposure | Injected at host boundary, WASM never sees |
| State pollution | Fresh instance per callback |
| Workspace escape | Paths prefixed with `channels/<name>/` |
| Message spam | Rate limiting on `emit_message` |
| Resource exhaustion | Fuel metering, memory limits, callback timeout |
| Polling abuse | Minimum 30s interval enforced |

## Creating a WASM Channel

1. Write a WASM component implementing the channel interface (exports: `on_http_request`, `on_poll`, `on_respond`)
2. Create a capabilities JSON file defining HTTP endpoints, polling config, and auth
3. Place the `.wasm` file and capabilities in `~/.thinclaw/channels/`

## Bundled Channels

Some WASM channels come bundled with ThinClaw and can be installed via:
```bash
thinclaw registry install <channel-name>
```

## Managing Channels

- WASM channels are auto-discovered from the channels directory at startup
- Hot-activation is supported via the Extension Manager
- The Gateway UI can manage channel activation/deactivation

## Notes

- WASM channels share the same Wasmtime engine as WASM tools
- Each callback gets a fresh WASM instance (no shared mutable state)
- Channels can register HTTP webhook endpoints for inbound messages
- Polling intervals have a minimum of 30 seconds (enforced by the host)
